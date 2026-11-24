// SPDX-License-Identifier: Apache-2.0
//! Trusted release-root access and private snapshot sealing.
//!
//! Directory descent uses descriptor-relative `openat` with `O_NOFOLLOW`. The
//! configured root itself is opened with `O_NOFOLLOW | O_DIRECTORY` through the
//! standard library's safe `OpenOptionsExt`. Files are hashed from the opened
//! descriptor and copied into a caller-provided private staging directory
//! before any downstream stage runs. No stage after sealing reopens a
//! published path.

use crate::error::CleanupPending;
use crate::error::VerifierError;
use crate::trust::VerifierResourcePolicy;
use rustix::fd::AsFd;
use rustix::fd::OwnedFd;
use rustix::fs::AtFlags;
use rustix::fs::Dir;
use rustix::fs::Mode;
use rustix::fs::OFlags;
use rustix::fs::openat;
use rustix::fs::statat;
use rustix::fs::unlinkat;
use sha2::Digest;
use sha2::Sha256;
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;

#[cfg(test)]
macro_rules! fault {
    ($probe:ident) => {
        $crate::files::faults::$probe()
    };
}

#[cfg(not(test))]
macro_rules! fault {
    ($probe:ident) => {
        false
    };
}

#[cfg(test)]
fn removal_fault(attempt: usize) -> bool {
    faults::removal_fails_at() == attempt
}

#[cfg(not(test))]
fn removal_fault(_attempt: usize) -> bool {
    false
}

/// Deterministic failure points for the snapshot and cleanup unit tests.
#[cfg(test)]
pub(crate) mod faults {
    use std::cell::RefCell;

    /// One fault schedule, applied to the current test thread only.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) struct FaultConfig {
        pub(crate) creation_fails: bool,
        pub(crate) partial_write_fails: bool,
        pub(crate) file_sync_fails: bool,
        pub(crate) reopen_fails: bool,
        pub(crate) copy_verification_fails: bool,
        pub(crate) removal_fails_at: usize,
        pub(crate) cleanup_directory_sync_fails: bool,
        pub(crate) cleanup_enumeration_fails: bool,
        pub(crate) seal_sync_fails: bool,
        pub(crate) binding_verification_fails: bool,
    }

    impl FaultConfig {
        pub(crate) const fn empty() -> Self {
            Self {
                creation_fails: false,
                partial_write_fails: false,
                file_sync_fails: false,
                reopen_fails: false,
                copy_verification_fails: false,
                removal_fails_at: 0,
                cleanup_directory_sync_fails: false,
                cleanup_enumeration_fails: false,
                seal_sync_fails: false,
                binding_verification_fails: false,
            }
        }
    }

    thread_local! {
        static CONFIG: RefCell<FaultConfig> = const { RefCell::new(FaultConfig::empty()) };
    }

    pub(crate) fn set(config: FaultConfig) {
        CONFIG.with(|slot| *slot.borrow_mut() = config);
    }

    pub(crate) fn reset() {
        CONFIG.with(|slot| *slot.borrow_mut() = FaultConfig::empty());
    }

    fn read() -> FaultConfig {
        CONFIG.with(|slot| *slot.borrow())
    }

    pub(crate) fn creation_fails() -> bool {
        read().creation_fails
    }

    pub(crate) fn partial_write_fails() -> bool {
        read().partial_write_fails
    }

    pub(crate) fn file_sync_fails() -> bool {
        read().file_sync_fails
    }

    pub(crate) fn reopen_fails() -> bool {
        read().reopen_fails
    }

    pub(crate) fn copy_verification_fails() -> bool {
        read().copy_verification_fails
    }

    pub(crate) fn removal_fails_at() -> usize {
        read().removal_fails_at
    }

    pub(crate) fn cleanup_directory_sync_fails() -> bool {
        read().cleanup_directory_sync_fails
    }

    pub(crate) fn cleanup_enumeration_fails() -> bool {
        read().cleanup_enumeration_fails
    }

    pub(crate) fn seal_sync_fails() -> bool {
        read().seal_sync_fails
    }

    pub(crate) fn binding_verification_fails() -> bool {
        read().binding_verification_fails
    }
}

/// A conservative budget for descriptor opens during one verification.
#[derive(Debug, Clone)]
pub struct FdBudget {
    remaining: u32,
}

impl FdBudget {
    /// Create a budget from the caller resource policy.
    pub fn new(policy: &VerifierResourcePolicy) -> Result<Self, VerifierError> {
        if policy.max_open_file_descriptors < 4 {
            return Err(VerifierError::ResourceLimit {
                limit: "max_open_file_descriptors".to_owned(),
            });
        }
        Ok(Self {
            remaining: policy.max_open_file_descriptors,
        })
    }

    /// Create a budget from an effective streaming descriptor limit.
    ///
    /// The structural minimum is the artifact descriptor, the staging writer
    /// and the verify descriptor held in parallel, as required by WS-STREAM-001
    /// D9. The legacy entry point keeps its four-descriptor minimum.
    pub fn with_minimum(limit: u32, minimum: u32) -> Result<Self, VerifierError> {
        if limit < minimum {
            return Err(VerifierError::ResourceLimit {
                limit: "max_open_file_descriptors".to_owned(),
            });
        }
        Ok(Self { remaining: limit })
    }

    fn spend(&mut self) -> Result<(), VerifierError> {
        if self.remaining == 0 {
            return Err(VerifierError::ResourceLimit {
                limit: "max_open_file_descriptors".to_owned(),
            });
        }
        self.remaining -= 1;
        Ok(())
    }
}

/// An opened and configured trusted release root.
#[derive(Debug)]
pub struct ReleaseRoot {
    /// Descriptor for the configured root directory.
    pub dir: File,
    /// Configured root path, retained for diagnostics only.
    pub path: PathBuf,
}

impl ReleaseRoot {
    /// Open the configured root without following a final symlink.
    pub fn open(path: &Path, budget: &mut FdBudget) -> Result<Self, VerifierError> {
        budget.spend()?;
        let dir = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open(path)
            .map_err(|error| VerifierError::Io {
                detail: format!("open trusted root: {}", error),
            })?;
        ensure_directory(&dir)?;
        Ok(Self {
            dir,
            path: path.to_path_buf(),
        })
    }

    /// Descend through publisher, dataset and release directory components.
    pub fn release_dir(
        &self,
        publisher_id: &str,
        dataset_id: &str,
        release_id: &str,
        budget: &mut FdBudget,
    ) -> Result<File, VerifierError> {
        for component in [publisher_id, dataset_id, release_id] {
            if !valid_component(component) {
                return Err(VerifierError::UnsafePath {
                    detail: "release locator component outside the component alphabet".to_owned(),
                });
            }
        }
        let publisher = open_dir_at(&self.dir, publisher_id, budget)?;
        let dataset = open_dir_at(&publisher, dataset_id, budget)?;
        open_dir_at(&dataset, release_id, budget)
    }
}

/// Facts captured from a descriptor before and after a bounded read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFacts {
    /// Byte length reported by `fstat`.
    pub size: u64,
    /// Device identity.
    pub device: u64,
    /// Inode identity.
    pub inode: u64,
    /// Link count.
    pub links: u64,
}

/// Validate a locator or artifact component against the declared alphabet.
pub fn valid_component(component: &str) -> bool {
    if component.is_empty() || component.len() > 128 || !component.is_ascii() {
        return false;
    }
    let mut chars = component.chars();
    let first = match chars.next() {
        Some(value) => value,
        None => return false,
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    chars.all(|value| value.is_ascii_alphanumeric() || value == '.' || value == '_' || value == '-')
}

/// Validate a declared artifact path and return its file component.
pub fn artifact_component(path: &str) -> Result<String, VerifierError> {
    let rest = match path.strip_prefix("artifacts/") {
        Some(value) => value,
        None => {
            return Err(VerifierError::UnsafePath {
                detail: "artifact path must begin with artifacts/".to_owned(),
            });
        }
    };
    if rest.contains('/') || rest.contains('\\') || rest.contains('\0') {
        return Err(VerifierError::UnsafePath {
            detail: "artifact path has an invalid separator".to_owned(),
        });
    }
    if !valid_component(rest) || rest == "." || rest == ".." {
        return Err(VerifierError::UnsafePath {
            detail: "artifact component outside the component alphabet".to_owned(),
        });
    }
    Ok(rest.to_owned())
}

fn ensure_directory(file: &File) -> Result<(), VerifierError> {
    let metadata = file.metadata().map_err(|error| VerifierError::Io {
        detail: format!("fstat directory: {}", error),
    })?;
    if !metadata.is_dir() {
        return Err(VerifierError::SpecialFile);
    }
    Ok(())
}

/// Open a child directory relative to a verified parent descriptor.
pub fn open_dir_at(
    parent: &File,
    name: &str,
    budget: &mut FdBudget,
) -> Result<File, VerifierError> {
    budget.spend()?;
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let opened = openat(parent, name, flags, Mode::empty()).map_err(|error| {
        VerifierError::SymlinkEscape {
            detail: format!("{}: {}", name, error),
        }
    })?;
    let file = File::from(opened);
    ensure_directory(&file)?;
    Ok(file)
}

/// Enumerate directory entry names from a verified directory descriptor.
pub fn enumerate_names(dir: &File) -> Result<Vec<String>, VerifierError> {
    let mut reader = Dir::read_from(dir.as_fd()).map_err(|error| VerifierError::Io {
        detail: format!("open directory reader: {}", error),
    })?;
    let mut names = Vec::new();
    while let Some(entry) = reader.read() {
        let entry = entry.map_err(|error| VerifierError::Io {
            detail: format!("read directory entries: {}", error),
        })?;
        let raw = entry.file_name().to_bytes();
        if raw == b"." || raw == b".." {
            continue;
        }
        let name = std::str::from_utf8(raw).map_err(|_| VerifierError::UnsafePath {
            detail: "directory entry name is not UTF-8".to_owned(),
        })?;
        names.push(name.to_owned());
    }
    let unique: HashSet<&String> = names.iter().collect();
    if unique.len() != names.len() {
        return Err(VerifierError::UnsafePath {
            detail: "directory has duplicate normalized entry names".to_owned(),
        });
    }
    Ok(names)
}

/// Open a regular file relative to a verified directory descriptor.
pub fn open_regular_file(
    parent: &File,
    name: &str,
    budget: &mut FdBudget,
) -> Result<(File, FileFacts), VerifierError> {
    if !valid_component(name) || name == "." || name == ".." {
        return Err(VerifierError::UnsafePath {
            detail: "file name outside the component alphabet".to_owned(),
        });
    }
    budget.spend()?;
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    let opened: OwnedFd = openat(parent, name, flags, Mode::empty()).map_err(|error| {
        VerifierError::SymlinkEscape {
            detail: format!("{}: {}", name, error),
        }
    })?;
    let file = File::from(opened);
    let facts = file_facts(&file)?;
    if !file
        .metadata()
        .map_err(|error| VerifierError::Io {
            detail: format!("fstat file: {}", error),
        })?
        .file_type()
        .is_file()
    {
        return Err(VerifierError::SpecialFile);
    }
    if facts.links > 1 {
        return Err(VerifierError::HardLink {
            detail: name.to_owned(),
        });
    }
    Ok((file, facts))
}

pub(crate) fn file_facts(file: &File) -> Result<FileFacts, VerifierError> {
    let metadata = file.metadata().map_err(|error| VerifierError::Io {
        detail: format!("fstat file: {}", error),
    })?;
    Ok(FileFacts {
        size: metadata.len(),
        device: metadata.dev(),
        inode: metadata.ino(),
        links: metadata.nlink(),
    })
}

/// Read all bytes from a handle with a declared size and a policy maximum.
pub fn read_bounded(
    file: &File,
    facts: &FileFacts,
    declared: u64,
    maximum: u64,
    limit: &str,
) -> Result<Vec<u8>, VerifierError> {
    if declared > maximum {
        return Err(VerifierError::ResourceLimit {
            limit: limit.to_owned(),
        });
    }
    if facts.size != declared {
        return Err(VerifierError::ByteCount {
            detail: format!(
                "fstat size {} differs from declared {}",
                facts.size, declared
            ),
        });
    }
    let mut bytes = Vec::new();
    let allowed = declared.saturating_add(1);
    let read = file
        .take(allowed)
        .read_to_end(&mut bytes)
        .map_err(|error| VerifierError::Io {
            detail: format!("read verified handle: {}", error),
        })?;
    if read as u64 != declared {
        return Err(VerifierError::ByteCount {
            detail: format!("read {} bytes from a declared {} byte file", read, declared),
        });
    }
    let after = file_facts(file)?;
    if after != *facts {
        return Err(VerifierError::ByteCount {
            detail: "descriptor metadata changed during the read".to_owned(),
        });
    }
    Ok(bytes)
}

/// SHA-256 over a byte slice.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut out = [0_u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// Render a digest as lowercase hex.
pub fn hex_lower(digest: &[u8; 32]) -> String {
    let mut text = String::with_capacity(64);
    for byte in digest {
        let _ = write!(text, "{:02x}", byte);
    }
    text
}

/// Parse a lowercase 64-hex digest string.
pub fn parse_hex_lower(text: &str, field: &str) -> Result<[u8; 32], VerifierError> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(VerifierError::MalformedDigest {
            detail: field.to_owned(),
        });
    }
    let mut out = [0_u8; 32];
    for (index, chunk) in text.as_bytes().chunks(2).enumerate() {
        let pair = std::str::from_utf8(chunk).unwrap_or("");
        let value = u8::from_str_radix(pair, 16).map_err(|_| VerifierError::MalformedDigest {
            detail: field.to_owned(),
        })?;
        out[index] = value;
    }
    Ok(out)
}

/// Observable state of one private snapshot cleanup attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupReport {
    /// Tracked entries present when the attempt started.
    pub files_expected: usize,
    /// Files removed by descriptor-relative unlink during this attempt.
    pub files_removed: usize,
    /// Tracked entries that were already absent at their removal turn.
    pub files_already_absent: usize,
    /// Sum of no-follow stat sizes for the files removed by this attempt.
    pub bytes_removed: u64,
    /// Result of the final directory re-enumeration.
    pub directory_empty: bool,
    /// Result of the directory sync after removals.
    pub directory_sync_completed: bool,
}

impl CleanupReport {
    /// Report completion only when every tracked entry ended absent and the
    /// directory was synced after an empty re-enumeration.
    pub fn is_complete(&self) -> bool {
        self.directory_empty
            && self.directory_sync_completed
            && self.files_removed + self.files_already_absent == self.files_expected
    }
}

/// Metadata for one private snapshot entry.
///
/// The entry holds no byte vector. The bytes live in the owned staging
/// directory and are read descriptor-relative with `O_NOFOLLOW` by
/// [`PrivateSnapshot::verify_entry`]. `verified` records that
/// [`SnapshotBuilder::finalize_entry`] proved the copy at write time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotEntry {
    /// Private staging file name.
    pub name: String,
    /// Count of bytes written to the staging file.
    pub stored_bytes: u64,
    /// SHA-256 of the staged bytes, proven by `finalize_entry`.
    pub stored_sha256: [u8; 32],
    /// Copy verification completed before the entry was sealed.
    pub verified: bool,
}

impl SnapshotEntry {
    /// Return the metadata digest used by receipt bindings.
    pub fn digest(&self) -> &[u8; 32] {
        &self.stored_sha256
    }
}

/// Fixed bounds applied by the private staging writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StagingLimits {
    /// Maximum bytes written to one staged entry.
    pub max_stored_bytes: u64,
    /// Maximum bytes written to the whole staging directory.
    pub max_temp_disk_bytes: u64,
    /// Fixed chunk size for staged writes and copy reads.
    pub io_chunk_bytes: u64,
}

/// Proof returned by a descriptor-relative snapshot entry verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryProof {
    /// Byte count re-read from the sealed copy.
    pub stored_bytes: u64,
    /// Digest re-computed from the sealed copy.
    pub stored_sha256: [u8; 32],
}

/// An immutable private snapshot of an accepted release.
#[derive(Debug)]
pub struct PrivateSnapshot {
    dir: File,
    entries: Vec<SnapshotEntry>,
    staging_path: PathBuf,
    limits: StagingLimits,
    available_fds: u32,
}

impl PrivateSnapshot {
    /// Return the verified private directory descriptor.
    pub fn directory(&self) -> &File {
        &self.dir
    }

    /// Return all sealed entries.
    pub fn entries(&self) -> &[SnapshotEntry] {
        &self.entries
    }

    /// Return one entry by private staging name.
    pub fn entry(&self, name: &str) -> Result<&SnapshotEntry, VerifierError> {
        self.entries
            .iter()
            .find(|entry| entry.name == name)
            .ok_or(VerifierError::SnapshotMismatch)
    }

    /// Return the configured private staging path.
    pub fn staging_path(&self) -> &Path {
        &self.staging_path
    }

    /// Re-verify one sealed entry from its descriptor without materializing bytes.
    ///
    /// The entry must carry a verified state from `finalize_entry`. The copy is
    /// reopened with `O_NOFOLLOW` relative to the sealed staging descriptor and
    /// streamed through an incremental SHA-256 in fixed profile-sized chunks.
    pub fn verify_entry(&self, name: &str) -> Result<EntryProof, VerifierError> {
        let entry = self.entry(name)?;
        if !entry.verified {
            return Err(VerifierError::SnapshotMismatch);
        }
        let mut budget = FdBudget {
            remaining: self.available_fds,
        };
        let (digest, bytes) = stream_copy_digest(
            &self.dir,
            name,
            entry.stored_bytes,
            self.limits.io_chunk_bytes,
            &mut budget,
            fault!(binding_verification_fails),
        )?;
        if digest != entry.stored_sha256 || bytes != entry.stored_bytes {
            return Err(VerifierError::SnapshotMismatch);
        }
        Ok(EntryProof {
            stored_bytes: bytes,
            stored_sha256: digest,
        })
    }

    /// Remove staged files after a refusal and prove the directory is empty.
    ///
    /// Every removal error is reported. A successful result is returned only
    /// after a post-removal directory sync and an empty re-enumeration.
    pub fn discard(&self) -> Result<CleanupReport, CleanupPending> {
        cleanup_directory(&self.dir, &self.staging_path, &self.entries)
    }
}

fn remaining_from(entries: &[SnapshotEntry], from: usize) -> Vec<String> {
    entries[from..]
        .iter()
        .map(|entry| entry.name.clone())
        .collect()
}

fn pending(staging_path: &Path, remaining: Vec<String>, detail: String) -> CleanupPending {
    CleanupPending {
        staging_path: staging_path.to_path_buf(),
        remaining_entries: remaining,
        detail,
    }
}

fn cleanup_directory(
    dir: &File,
    staging_path: &Path,
    entries: &[SnapshotEntry],
) -> Result<CleanupReport, CleanupPending> {
    let mut report = CleanupReport {
        files_expected: entries.len(),
        files_removed: 0,
        files_already_absent: 0,
        bytes_removed: 0,
        directory_empty: false,
        directory_sync_completed: false,
    };
    let mut attempts = 0_usize;
    for (index, entry) in entries.iter().enumerate() {
        let before = match statat(dir.as_fd(), &entry.name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                report.files_already_absent += 1;
                continue;
            }
            Err(error) => {
                return Err(pending(
                    staging_path,
                    remaining_from(entries, index),
                    format!("stat staged entry before removal: {}", error),
                ));
            }
        };
        attempts += 1;
        if removal_fault(attempts) {
            return Err(pending(
                staging_path,
                remaining_from(entries, index),
                "injected removal failure".to_owned(),
            ));
        }
        unlinkat(dir.as_fd(), &entry.name, AtFlags::empty()).map_err(|error| {
            pending(
                staging_path,
                remaining_from(entries, index),
                format!("unlink staged entry {}: {}", entry.name, error),
            )
        })?;
        report.files_removed += 1;
        report.bytes_removed = report
            .bytes_removed
            .saturating_add(before.st_size.max(0) as u64);
    }
    if fault!(cleanup_directory_sync_fails) {
        return Err(pending(
            staging_path,
            Vec::new(),
            "injected directory sync failure after deletion".to_owned(),
        ));
    }
    rustix::fs::fsync(dir.as_fd()).map_err(|error| {
        pending(
            staging_path,
            Vec::new(),
            format!("fsync private staging directory after deletion: {}", error),
        )
    })?;
    report.directory_sync_completed = true;
    if fault!(cleanup_enumeration_fails) {
        return Err(pending(
            staging_path,
            Vec::new(),
            "injected final enumeration failure".to_owned(),
        ));
    }
    let remaining = enumerate_names(dir).map_err(|error| {
        pending(
            staging_path,
            Vec::new(),
            format!("re-enumerate private staging directory: {}", error),
        )
    })?;
    if !remaining.is_empty() {
        return Err(pending(
            staging_path,
            remaining,
            "private staging directory is not empty after cleanup".to_owned(),
        ));
    }
    report.directory_empty = true;
    Ok(report)
}

/// Active entry held between `begin_entry` and `finalize_entry`.
#[derive(Debug)]
struct ActiveEntry {
    file: File,
    name: String,
    stored_bytes: u64,
    hasher: Sha256,
}

/// Builder that streams verified bytes into private staging and seals them.
#[derive(Debug)]
pub struct SnapshotBuilder {
    dir: File,
    staging_path: PathBuf,
    entries: Vec<SnapshotEntry>,
    total_bytes: u64,
    budget: FdBudget,
    limits: StagingLimits,
    active: Option<ActiveEntry>,
}

impl SnapshotBuilder {
    /// Open an empty, caller-provided private staging directory with limits
    /// derived from the legacy caller policy.
    pub fn new(
        staging_path: &Path,
        budget: &mut FdBudget,
        policy: &VerifierResourcePolicy,
    ) -> Result<Self, VerifierError> {
        let chunk = policy.max_record_bytes.min(policy.max_stored_bytes).max(1);
        let limits = StagingLimits {
            max_stored_bytes: policy.max_stored_bytes,
            max_temp_disk_bytes: policy.max_temp_disk_bytes,
            io_chunk_bytes: chunk,
        };
        Self::open(staging_path, budget, limits)
    }

    /// Open an empty staging directory with limits from a validated profile.
    pub fn with_limits(
        staging_path: &Path,
        budget: &mut FdBudget,
        limits: StagingLimits,
    ) -> Result<Self, VerifierError> {
        if limits.io_chunk_bytes == 0
            || limits.max_stored_bytes == 0
            || limits.max_temp_disk_bytes == 0
        {
            return Err(VerifierError::ResourceLimit {
                limit: "staging profile limits".to_owned(),
            });
        }
        Self::open(staging_path, budget, limits)
    }

    fn open(
        staging_path: &Path,
        budget: &mut FdBudget,
        limits: StagingLimits,
    ) -> Result<Self, VerifierError> {
        budget.spend()?;
        let dir = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open(staging_path)
            .map_err(|error| VerifierError::Io {
                detail: format!("open private staging directory: {}", error),
            })?;
        ensure_directory(&dir)?;
        let existing = enumerate_names(&dir)?;
        if !existing.is_empty() {
            return Err(VerifierError::FileSet {
                detail: "private staging directory is not empty".to_owned(),
            });
        }
        Ok(Self {
            dir,
            staging_path: staging_path.to_path_buf(),
            entries: Vec::new(),
            total_bytes: 0,
            budget: FdBudget {
                remaining: budget.remaining,
            },
            limits,
            active: None,
        })
    }

    /// Create a named entry under the staging descriptor and track it at once.
    ///
    /// A failure during a subsequent write, sync or verification leaves the name
    /// tracked so `discard_partial` can remove it.
    pub fn begin_entry(&mut self, name: &str) -> Result<(), VerifierError> {
        if !valid_component(name) || name == "." || name == ".." {
            return Err(VerifierError::UnsafePath {
                detail: "snapshot entry name outside the component alphabet".to_owned(),
            });
        }
        if self.active.is_some() {
            return Err(VerifierError::Io {
                detail: "another snapshot entry is active".to_owned(),
            });
        }
        self.budget.spend()?;
        if fault!(creation_fails) {
            return Err(VerifierError::Io {
                detail: "injected creation failure".to_owned(),
            });
        }
        let flags =
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let opened =
            openat(&self.dir, name, flags, Mode::from_raw_mode(0o600)).map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    VerifierError::SymlinkEscape {
                        detail: format!("{}: staged entry already exists", name),
                    }
                } else {
                    VerifierError::Io {
                        detail: format!("create private snapshot file: {}", error),
                    }
                }
            })?;
        self.entries.push(SnapshotEntry {
            name: name.to_owned(),
            stored_bytes: 0,
            stored_sha256: [0_u8; 32],
            verified: false,
        });
        self.active = Some(ActiveEntry {
            file: File::from(opened),
            name: name.to_owned(),
            stored_bytes: 0,
            hasher: Sha256::new(),
        });
        Ok(())
    }

    /// Write one bounded chunk into the active entry.
    pub fn write_chunk(&mut self, chunk: &[u8]) -> Result<(), VerifierError> {
        let active = self
            .active
            .as_mut()
            .ok_or(VerifierError::SnapshotMismatch)?;
        let added = chunk.len() as u64;
        if added > self.limits.io_chunk_bytes {
            return Err(VerifierError::ResourceLimit {
                limit: "io_chunk_bytes".to_owned(),
            });
        }
        if active.stored_bytes.saturating_add(added) > self.limits.max_stored_bytes
            || self.total_bytes.saturating_add(added) > self.limits.max_temp_disk_bytes
        {
            return Err(VerifierError::ResourceLimit {
                limit: "max_temp_disk_bytes".to_owned(),
            });
        }
        if fault!(partial_write_fails) {
            let half = chunk.len() / 2;
            active
                .file
                .write_all(&chunk[..half])
                .map_err(|error| VerifierError::Io {
                    detail: format!("write private snapshot file: {}", error),
                })?;
            return Err(VerifierError::Io {
                detail: "injected partial write failure".to_owned(),
            });
        }
        active
            .file
            .write_all(chunk)
            .map_err(|error| VerifierError::Io {
                detail: format!("write private snapshot file: {}", error),
            })?;
        active.hasher.update(chunk);
        active.stored_bytes += added;
        Ok(())
    }

    /// Sync, close, reopen and stream-verify the active copy.
    ///
    /// The returned metadata carries no bytes. Verification runs here and again
    /// at snapshot binding through `PrivateSnapshot::verify_entry`.
    pub fn finalize_entry(
        &mut self,
        expected_digest: [u8; 32],
        expected_bytes: u64,
    ) -> Result<SnapshotEntry, VerifierError> {
        let active = self.active.take().ok_or(VerifierError::SnapshotMismatch)?;
        if fault!(file_sync_fails) {
            self.active = Some(active);
            return Err(VerifierError::Io {
                detail: "injected file sync failure".to_owned(),
            });
        }
        if let Err(error) = active.file.sync_all() {
            self.active = Some(active);
            return Err(VerifierError::Io {
                detail: format!("fsync private snapshot file: {}", error),
            });
        }
        drop(active.file);
        if fault!(reopen_fails) {
            return Err(VerifierError::Io {
                detail: "injected reopen failure".to_owned(),
            });
        }
        let (digest, bytes) = stream_copy_digest(
            &self.dir,
            &active.name,
            expected_bytes,
            self.limits.io_chunk_bytes,
            &mut self.budget,
            fault!(copy_verification_fails),
        )?;
        let computed = {
            let bytes_digest = active.hasher.finalize();
            let mut out = [0_u8; 32];
            out.copy_from_slice(&bytes_digest);
            out
        };
        if digest != computed || digest != expected_digest || bytes != expected_bytes {
            return Err(VerifierError::SnapshotMismatch);
        }
        let slot = self
            .entries
            .iter()
            .position(|entry| entry.name == active.name)
            .ok_or(VerifierError::SnapshotMismatch)?;
        let entry = &mut self.entries[slot];
        entry.stored_bytes = bytes;
        entry.stored_sha256 = digest;
        entry.verified = true;
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        Ok(entry.clone())
    }

    /// Compatibility helper for callers that hold one bounded byte sequence.
    ///
    /// The bytes are forwarded through the entry lifecycle and are absent
    /// from snapshot metadata.
    pub fn add(&mut self, name: &str, bytes: &[u8]) -> Result<(), VerifierError> {
        let digest = sha256(bytes);
        self.begin_entry(name)?;
        let chunk = self.limits.io_chunk_bytes as usize;
        for part in bytes.chunks(chunk) {
            self.write_chunk(part)?;
        }
        self.finalize_entry(digest, bytes.len() as u64)?;
        Ok(())
    }

    /// Sync the private directory before sealing.
    pub fn sync_directory(&self) -> Result<(), VerifierError> {
        if fault!(seal_sync_fails) {
            return Err(VerifierError::Io {
                detail: "injected directory sync failure before seal".to_owned(),
            });
        }
        rustix::fs::fsync(self.dir.as_fd()).map_err(|error| VerifierError::Io {
            detail: format!("fsync private snapshot directory: {}", error),
        })
    }

    /// Seal an already synced builder into an immutable snapshot.
    ///
    /// Callers invoke `sync_directory` first. This method performs no I/O, so
    /// the refusal path keeps the sync result observable for cleanup.
    pub fn seal(self) -> PrivateSnapshot {
        PrivateSnapshot {
            dir: self.dir,
            entries: self.entries,
            staging_path: self.staging_path,
            limits: self.limits,
            available_fds: self.budget.remaining,
        }
    }

    /// Remove files copied so far after a refusal during staging.
    pub fn discard_partial(&self) -> Result<CleanupReport, CleanupPending> {
        cleanup_directory(&self.dir, &self.staging_path, &self.entries)
    }
}

fn stream_copy_digest(
    dir: &File,
    name: &str,
    expected_bytes: u64,
    io_chunk_bytes: u64,
    budget: &mut FdBudget,
    force_mismatch: bool,
) -> Result<([u8; 32], u64), VerifierError> {
    budget.spend()?;
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let opened = openat(dir, name, flags, Mode::empty()).map_err(|error| VerifierError::Io {
        detail: format!("reopen private snapshot copy: {}", error),
    })?;
    let file = File::from(opened);
    let facts = file_facts(&file)?;
    if !file
        .metadata()
        .map_err(|error| VerifierError::Io {
            detail: format!("fstat private snapshot copy: {}", error),
        })?
        .file_type()
        .is_file()
        || facts.links > 1
    {
        return Err(VerifierError::SnapshotMismatch);
    }
    let chunk = io_chunk_bytes as usize;
    let mut buffer = vec![0_u8; chunk];
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut reader = file.take(expected_bytes.saturating_add(1));
    loop {
        let allowance = (expected_bytes.saturating_add(1) - total).min(io_chunk_bytes) as usize;
        let read = reader
            .read(&mut buffer[..allowance])
            .map_err(|error| VerifierError::Io {
                detail: format!("stream private snapshot copy: {}", error),
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
    if total != expected_bytes {
        return Err(VerifierError::SnapshotMismatch);
    }
    if force_mismatch {
        return Err(VerifierError::SnapshotMismatch);
    }
    let bytes = hasher.finalize();
    let mut digest = [0_u8; 32];
    digest.copy_from_slice(&bytes);
    Ok((digest, total))
}

#[cfg(test)]
mod tests {
    use super::faults::FaultConfig;
    use super::*;
    use tempfile::TempDir;

    fn test_policy() -> VerifierResourcePolicy {
        VerifierResourcePolicy {
            max_manifest_bytes: 1_024,
            max_sidecar_bytes: 1_024,
            max_artifact_count: 4,
            max_stored_bytes: 1_024,
            max_expanded_bytes: 1_024,
            max_total_expanded_bytes: 4_096,
            max_record_bytes: 256,
            max_record_count: 64,
            max_json_nesting: 8,
            max_decompression_budget: 4_096,
            max_temp_disk_bytes: 4_096,
            max_open_file_descriptors: 32,
        }
    }

    fn staging_builder(temp: &TempDir) -> Result<SnapshotBuilder, VerifierError> {
        let path = temp.path().join("private-staging");
        std::fs::create_dir(&path).map_err(|error| VerifierError::Io {
            detail: format!("create test staging directory: {}", error),
        })?;
        let mut budget = FdBudget::new(&test_policy())?;
        SnapshotBuilder::new(&path, &mut budget, &test_policy())
    }

    fn pending_result(
        result: Result<CleanupReport, CleanupPending>,
    ) -> Result<CleanupPending, Box<dyn std::error::Error>> {
        match result {
            Ok(_) => Err("cleanup was expected to be pending".into()),
            Err(pending) => Ok(pending),
        }
    }

    fn assert_pending_code(pending: &CleanupPending) -> Result<(), Box<dyn std::error::Error>> {
        let error = VerifierError::from(pending.clone());
        assert_eq!(error.code(), "CLEANUP_PENDING", "distinct cleanup code");
        assert_eq!(error.stage(), 5, "cleanup belongs to the staging stage");
        Ok(())
    }

    fn missing(builder: &SnapshotBuilder, name: &str) -> bool {
        matches!(
            statat(builder.dir.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        )
    }

    #[test]
    fn creation_failure_tracks_no_entry() -> Result<(), Box<dyn std::error::Error>> {
        faults::set(FaultConfig {
            creation_fails: true,
            ..FaultConfig::empty()
        });
        let temp = tempfile::tempdir()?;
        let mut builder = staging_builder(&temp)?;
        let error = builder.add("alpha.txt", b"payload");
        faults::reset();
        assert!(error.is_err(), "injected creation failure must be reported");
        let report = builder.discard_partial()?;
        assert_eq!(report.files_expected, 0, "no entry was created");
        assert_eq!(report.files_removed, 0, "no file was removed");
        assert!(
            report.is_complete(),
            "cleanup proved an empty synced directory"
        );
        Ok(())
    }

    #[test]
    fn partial_write_failure_removes_the_tracked_file() -> Result<(), Box<dyn std::error::Error>> {
        faults::set(FaultConfig {
            partial_write_fails: true,
            ..FaultConfig::empty()
        });
        let temp = tempfile::tempdir()?;
        let mut builder = staging_builder(&temp)?;
        let error = builder.add("alpha.txt", b"abcdef");
        assert!(error.is_err(), "injected partial write must be reported");
        let report = builder.discard_partial()?;
        assert_eq!(report.files_expected, 1, "entry was tracked before writing");
        assert_eq!(
            report.files_removed, 1,
            "partially written file was removed"
        );
        assert_eq!(
            report.bytes_removed, 3,
            "only the written bytes were counted"
        );
        assert_eq!(report.files_already_absent, 0, "no entry was absent");
        assert!(report.directory_empty, "final enumeration found no entries");
        assert!(
            report.directory_sync_completed,
            "directory sync after deletion completed"
        );
        assert!(report.is_complete(), "cleanup disposition is complete");
        Ok(())
    }

    #[test]
    fn file_sync_failure_removes_the_tracked_file() -> Result<(), Box<dyn std::error::Error>> {
        faults::set(FaultConfig {
            file_sync_fails: true,
            ..FaultConfig::empty()
        });
        let temp = tempfile::tempdir()?;
        let mut builder = staging_builder(&temp)?;
        let error = builder.add("alpha.txt", b"abcdef");
        assert!(
            error.is_err(),
            "injected file sync failure must be reported"
        );
        let report = builder.discard_partial()?;
        assert_eq!(report.files_expected, 1, "entry was tracked before syncing");
        assert_eq!(report.files_removed, 1, "synced-file failure was cleaned");
        assert_eq!(report.bytes_removed, 6, "full written size was counted");
        assert!(report.is_complete(), "cleanup disposition is complete");
        Ok(())
    }

    #[test]
    fn reopen_failure_removes_the_tracked_file() -> Result<(), Box<dyn std::error::Error>> {
        faults::set(FaultConfig {
            reopen_fails: true,
            ..FaultConfig::empty()
        });
        let temp = tempfile::tempdir()?;
        let mut builder = staging_builder(&temp)?;
        let error = builder.add("alpha.txt", b"abcdef");
        assert!(error.is_err(), "injected reopen failure must be reported");
        let report = builder.discard_partial()?;
        assert_eq!(report.files_expected, 1, "entry was tracked before reopen");
        assert_eq!(report.files_removed, 1, "file left by reopen was removed");
        assert!(report.is_complete(), "cleanup disposition is complete");
        Ok(())
    }

    #[test]
    fn copy_verification_failure_removes_the_tracked_file() -> Result<(), Box<dyn std::error::Error>>
    {
        faults::set(FaultConfig {
            copy_verification_fails: true,
            ..FaultConfig::empty()
        });
        let temp = tempfile::tempdir()?;
        let mut builder = staging_builder(&temp)?;
        let error = builder.add("alpha.txt", b"abcdef");
        assert!(
            matches!(error, Err(VerifierError::SnapshotMismatch)),
            "injected copy verification failure must be reported"
        );
        let report = builder.discard_partial()?;
        assert_eq!(report.files_expected, 1, "entry was tracked before verify");
        assert_eq!(report.files_removed, 1, "unverified file was removed");
        assert!(report.is_complete(), "cleanup disposition is complete");
        Ok(())
    }

    #[test]
    fn removal_of_first_file_is_pending() -> Result<(), Box<dyn std::error::Error>> {
        faults::set(FaultConfig {
            removal_fails_at: 1,
            ..FaultConfig::empty()
        });
        let temp = tempfile::tempdir()?;
        let mut builder = staging_builder(&temp)?;
        builder.add("alpha.txt", b"one")?;
        builder.add("beta.txt", b"two")?;
        let pending = pending_result(builder.discard_partial())?;
        assert_pending_code(&pending)?;
        assert_eq!(
            pending.staging_path, builder.staging_path,
            "exact staging path"
        );
        assert_eq!(
            pending.remaining_entries,
            vec!["alpha.txt".to_owned(), "beta.txt".to_owned()],
            "both tracked files remain"
        );
        assert!(!missing(&builder, "alpha.txt"), "first file survived");
        Ok(())
    }

    #[test]
    fn removal_after_some_files_deleted_is_pending() -> Result<(), Box<dyn std::error::Error>> {
        faults::set(FaultConfig {
            removal_fails_at: 2,
            ..FaultConfig::empty()
        });
        let temp = tempfile::tempdir()?;
        let mut builder = staging_builder(&temp)?;
        builder.add("alpha.txt", b"one")?;
        builder.add("beta.txt", b"two")?;
        builder.add("gamma.txt", b"three")?;
        let pending = pending_result(builder.discard_partial())?;
        assert_pending_code(&pending)?;
        assert_eq!(
            pending.remaining_entries,
            vec!["beta.txt".to_owned(), "gamma.txt".to_owned()],
            "unremoved entries are reported exactly"
        );
        assert!(missing(&builder, "alpha.txt"), "first file was removed");
        assert!(!missing(&builder, "beta.txt"), "second file survived");
        assert!(!missing(&builder, "gamma.txt"), "third file survived");
        Ok(())
    }

    #[test]
    fn directory_sync_failure_after_deletion_is_pending() -> Result<(), Box<dyn std::error::Error>>
    {
        faults::set(FaultConfig {
            cleanup_directory_sync_fails: true,
            ..FaultConfig::empty()
        });
        let temp = tempfile::tempdir()?;
        let mut builder = staging_builder(&temp)?;
        builder.add("alpha.txt", b"one")?;
        let pending = pending_result(builder.discard_partial())?;
        assert_pending_code(&pending)?;
        assert!(
            pending.remaining_entries.is_empty(),
            "no tracked entry remained after deletion"
        );
        assert!(missing(&builder, "alpha.txt"), "the file was removed");
        assert!(
            pending.detail.contains("sync"),
            "the sync failure is the recorded cause"
        );
        Ok(())
    }

    #[test]
    fn final_enumeration_failure_is_pending() -> Result<(), Box<dyn std::error::Error>> {
        faults::set(FaultConfig {
            cleanup_enumeration_fails: true,
            ..FaultConfig::empty()
        });
        let temp = tempfile::tempdir()?;
        let mut builder = staging_builder(&temp)?;
        builder.add("alpha.txt", b"one")?;
        let pending = pending_result(builder.discard_partial())?;
        assert_pending_code(&pending)?;
        assert!(
            pending.remaining_entries.is_empty(),
            "directory held no entry at the failed enumeration"
        );
        assert!(missing(&builder, "alpha.txt"), "the file was removed");
        assert!(
            pending.detail.contains("enumeration"),
            "the enumeration failure is the recorded cause"
        );
        Ok(())
    }

    #[test]
    fn seal_sync_failure_is_cleaned_and_reported() -> Result<(), Box<dyn std::error::Error>> {
        faults::set(FaultConfig {
            seal_sync_fails: true,
            ..FaultConfig::empty()
        });
        let temp = tempfile::tempdir()?;
        let mut builder = staging_builder(&temp)?;
        builder.add("alpha.txt", b"one")?;
        let error = builder.sync_directory();
        assert!(
            error.is_err(),
            "injected seal sync failure must be reported"
        );
        let report = builder.discard_partial()?;
        assert_eq!(report.files_removed, 1, "pre-seal entry was removed");
        assert!(report.is_complete(), "cleanup disposition is complete");
        Ok(())
    }

    #[test]
    fn entry_lifecycle_produces_metadata_only_state() -> Result<(), Box<dyn std::error::Error>> {
        faults::reset();
        let temp = tempfile::tempdir()?;
        let mut builder = staging_builder(&temp)?;
        builder.begin_entry("alpha.txt")?;
        builder.write_chunk(b"abcdef")?;
        let entry = builder.finalize_entry(sha256(b"abcdef"), 6)?;
        assert_eq!(entry.stored_bytes, 6, "byte count is metadata");
        assert_eq!(entry.stored_sha256, sha256(b"abcdef"), "digest is metadata");
        assert!(entry.verified, "finalize proved the copy");
        let dir = enumerate_names(&builder.dir)?;
        assert_eq!(dir, vec!["alpha.txt".to_owned()], "bytes live on disk only");
        builder.sync_directory()?;
        let snapshot = builder.seal();
        let proof = snapshot.verify_entry("alpha.txt")?;
        assert_eq!(proof.stored_bytes, 6, "binding re-reads the byte count");
        assert_eq!(
            proof.stored_sha256,
            sha256(b"abcdef"),
            "binding re-reads the digest"
        );
        Ok(())
    }

    #[test]
    fn binding_verification_fault_is_reported_without_bytes()
    -> Result<(), Box<dyn std::error::Error>> {
        faults::reset();
        let temp = tempfile::tempdir()?;
        let mut builder = staging_builder(&temp)?;
        builder.begin_entry("alpha.txt")?;
        builder.write_chunk(b"abcdef")?;
        builder.finalize_entry(sha256(b"abcdef"), 6)?;
        builder.sync_directory()?;
        let snapshot = builder.seal();
        faults::set(FaultConfig {
            binding_verification_fails: true,
            ..FaultConfig::empty()
        });
        let error = snapshot.verify_entry("alpha.txt").unwrap_err();
        faults::reset();
        assert!(
            matches!(error, VerifierError::SnapshotMismatch),
            "binding-time verification failure is a typed mismatch"
        );
        let report = snapshot.discard()?;
        assert!(
            report.is_complete(),
            "cleanup remains available after the fault"
        );
        Ok(())
    }

    #[test]
    fn begin_entry_refuses_an_existing_symlink_with_no_follow()
    -> Result<(), Box<dyn std::error::Error>> {
        faults::reset();
        let temp = tempfile::tempdir()?;
        let mut builder = staging_builder(&temp)?;
        let link = temp.path().join("private-staging").join("alpha.txt");
        std::os::unix::fs::symlink("/tmp/b16-outside", &link)?;
        let error = builder.begin_entry("alpha.txt");
        assert!(
            matches!(error, Err(VerifierError::SymlinkEscape { .. })),
            "no-follow create refuses an existing symlink"
        );
        let pending = pending_result(builder.discard_partial())?;
        assert_pending_code(&pending)?;
        assert_eq!(
            pending.remaining_entries,
            vec!["alpha.txt".to_owned()],
            "the untracked symlink is reported by cleanup"
        );
        Ok(())
    }

    #[test]
    fn sealed_snapshot_discard_proves_an_empty_synced_directory()
    -> Result<(), Box<dyn std::error::Error>> {
        faults::reset();
        let temp = tempfile::tempdir()?;
        let mut builder = staging_builder(&temp)?;
        builder.add("alpha.txt", b"one")?;
        builder.add("beta.txt", b"two")?;
        builder.sync_directory()?;
        let snapshot = builder.seal();
        let path = snapshot.staging_path().to_path_buf();
        let report = snapshot.discard()?;
        assert_eq!(report.files_expected, 2, "both entries were tracked");
        assert_eq!(report.files_removed, 2, "both entries were removed");
        assert_eq!(report.bytes_removed, 6, "both file sizes were counted");
        assert_eq!(report.files_already_absent, 0, "no entry was absent");
        assert!(report.directory_empty, "final enumeration found no entries");
        assert!(
            report.directory_sync_completed,
            "directory sync after deletion completed"
        );
        assert!(report.is_complete(), "cleanup disposition is complete");
        assert_eq!(
            std::fs::read_dir(&path)?.count(),
            0,
            "staging directory is empty"
        );
        Ok(())
    }
}
