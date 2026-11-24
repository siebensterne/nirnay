// SPDX-License-Identifier: Apache-2.0
//! Trusted release-root access and private snapshot sealing.
//!
//! Directory descent uses descriptor-relative `openat` with `O_NOFOLLOW`. The
//! configured root itself is opened with `O_NOFOLLOW | O_DIRECTORY` through the
//! standard library's safe `OpenOptionsExt`. Files are hashed from the opened
//! descriptor and copied into a caller-provided private staging directory
//! before any downstream stage runs. No stage after sealing reopens a
//! published path.

use crate::error::VerifierError;
use crate::trust::VerifierResourcePolicy;
use rustix::fd::AsFd;
use rustix::fd::OwnedFd;
use rustix::fs::Dir;
use rustix::fs::Mode;
use rustix::fs::OFlags;
use rustix::fs::openat;
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

fn file_facts(file: &File) -> Result<FileFacts, VerifierError> {
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

/// One sealed snapshot entry retained in memory for downstream stages.
#[derive(Debug, Clone)]
pub struct SnapshotEntry {
    /// Private staging file name.
    pub name: String,
    /// Exact bytes copied from the verified descriptor.
    pub bytes: Vec<u8>,
    /// SHA-256 of the copied bytes.
    pub sha256: [u8; 32],
}

/// An immutable private snapshot of an accepted release.
#[derive(Debug)]
pub struct PrivateSnapshot {
    dir: File,
    entries: Vec<SnapshotEntry>,
    staging_path: PathBuf,
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

    /// Remove staged files after a refusal so no partial release is retained.
    pub fn discard(&self) {
        discard_files(&self.staging_path, &self.entries);
    }
}

fn discard_files(path: &Path, entries: &[SnapshotEntry]) {
    for entry in entries {
        let _ = std::fs::remove_file(path.join(&entry.name));
    }
}

/// Builder that copies verified bytes into private staging and seals them.
#[derive(Debug)]
pub struct SnapshotBuilder {
    dir: File,
    staging_path: PathBuf,
    entries: Vec<SnapshotEntry>,
    total_bytes: u64,
    budget: FdBudget,
    policy: VerifierResourcePolicy,
}

impl SnapshotBuilder {
    /// Open an empty, caller-provided private staging directory.
    pub fn new(
        staging_path: &Path,
        budget: &mut FdBudget,
        policy: &VerifierResourcePolicy,
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
            policy: *policy,
        })
    }

    /// Copy one verified byte sequence into staging and re-hash the copy.
    pub fn add(&mut self, name: &str, bytes: &[u8]) -> Result<(), VerifierError> {
        if !valid_component(name) {
            return Err(VerifierError::UnsafePath {
                detail: "snapshot entry name outside the component alphabet".to_owned(),
            });
        }
        let added = bytes.len() as u64;
        if self.total_bytes.saturating_add(added) > self.policy.max_temp_disk_bytes {
            return Err(VerifierError::ResourceLimit {
                limit: "max_temp_disk_bytes".to_owned(),
            });
        }
        self.budget.spend()?;
        let flags =
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let opened =
            openat(&self.dir, name, flags, Mode::from_raw_mode(0o600)).map_err(|error| {
                VerifierError::Io {
                    detail: format!("create private snapshot file: {}", error),
                }
            })?;
        let mut file = File::from(opened);
        file.write_all(bytes).map_err(|error| VerifierError::Io {
            detail: format!("write private snapshot file: {}", error),
        })?;
        file.sync_all().map_err(|error| VerifierError::Io {
            detail: format!("fsync private snapshot file: {}", error),
        })?;
        drop(file);
        let digest = verify_copy(&self.dir, name, bytes, &mut self.budget)?;
        self.entries.push(SnapshotEntry {
            name: name.to_owned(),
            bytes: bytes.to_vec(),
            sha256: digest,
        });
        self.total_bytes = self.total_bytes.saturating_add(added);
        Ok(())
    }

    /// Sync the private directory and return an immutable sealed snapshot.
    pub fn seal(self) -> Result<PrivateSnapshot, VerifierError> {
        self.dir.sync_all().map_err(|error| VerifierError::Io {
            detail: format!("fsync private snapshot directory: {}", error),
        })?;
        Ok(PrivateSnapshot {
            dir: self.dir,
            entries: self.entries,
            staging_path: self.staging_path,
        })
    }

    /// Remove files copied so far after a refusal during staging.
    pub fn discard_partial(&self) {
        discard_files(&self.staging_path, &self.entries);
    }
}

fn verify_copy(
    dir: &File,
    name: &str,
    expected: &[u8],
    budget: &mut FdBudget,
) -> Result<[u8; 32], VerifierError> {
    budget.spend()?;
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let opened = openat(dir, name, flags, Mode::empty()).map_err(|error| VerifierError::Io {
        detail: format!("reopen private snapshot copy: {}", error),
    })?;
    let file = File::from(opened);
    let facts = file_facts(&file)?;
    let copied = read_bounded(
        &file,
        &facts,
        expected.len() as u64,
        expected.len() as u64,
        "snapshot",
    )?;
    if copied.as_slice() != expected {
        return Err(VerifierError::SnapshotMismatch);
    }
    Ok(sha256(&copied))
}
