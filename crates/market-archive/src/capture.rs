use std::fs::{
    File, //
};
use std::io::{
    BufReader, //
    Read,
};
use std::path::{
    Component, //
    Path,
    PathBuf,
};

use serde::{
    Deserialize, //
    Serialize,
};
use sha2::{
    Digest, //
    Sha256,
};

use crate::{
    ArchiveError, //
    JournalReader,
};

/// Maximum serialized capture manifest size, excluding any framing.
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// The manifest written by a capture session alongside its raw journal.
///
/// Matching this manifest to its journal proves consistency between the two
/// files. It is not an external trust anchor: both files can be replaced
/// together. Provider authenticity and completeness remain separate claims.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureManifest {
    pub schema_version: u32,
    pub provider: String,
    pub feed_protocol: String,
    pub mode: String,
    pub instrument_keys: Vec<String>,
    pub entitlement_ref: String,
    pub started_at_utc: String,
    pub ended_at_utc: String,
    pub duration_ns: u128,
    pub connection_count: u64,
    pub frame_count: u64,
    pub decode_error_count: u64,
    pub terminal_error: Option<String>,
    pub raw_journal: String,
    pub raw_journal_sha256: String,
    pub raw_journal_frame_count: u64,
    pub provider_sequence_available: bool,
    pub capture_sequence_semantics: String,
    pub limitations: Vec<String>,
}

/// Evidence produced by a successful session verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionVerification {
    pub frame_count: u64,
    pub bytes_scanned: u64,
    pub sha256_hex: String,
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

impl CaptureManifest {
    /// Parse a bounded manifest. Never read an unbounded manifest into memory.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ArchiveError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ArchiveError::InvalidManifest(format!(
                "manifest of {} bytes exceeds the {MAX_MANIFEST_BYTES}-byte limit",
                bytes.len()
            )));
        }
        serde_json::from_slice(bytes)
            .map_err(|error| ArchiveError::InvalidManifest(format!("invalid JSON: {error}")))
    }

    /// Validate fields that do not require reading the journal.
    ///
    /// Terminal capture state is a separate input from journal integrity: a
    /// journal can be internally consistent even when capture ended early.
    pub fn validate_basics(&self, expected_provider: &str) -> Result<(), ArchiveError> {
        if self.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(ArchiveError::InvalidManifest(format!(
                "unsupported schema_version {}",
                self.schema_version
            )));
        }
        if !self.provider.eq_ignore_ascii_case(expected_provider) {
            return Err(ArchiveError::InvalidManifest(format!(
                "provider `{}` is not `{expected_provider}`",
                self.provider
            )));
        }
        if let Some(error) = self.terminal_error.as_deref() {
            return Err(ArchiveError::TerminalCaptureError(error.to_owned()));
        }
        if self.raw_journal.trim().is_empty() {
            return Err(ArchiveError::InvalidManifest(
                "raw_journal is empty".to_owned(),
            ));
        }
        if self.raw_journal_sha256.len() != 64
            || !self
                .raw_journal_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(ArchiveError::InvalidManifest(
                "raw_journal_sha256 is not a 64-character hex digest".to_owned(),
            ));
        }
        Ok(())
    }

    /// Resolve the referenced journal to a regular file beneath the manifest.
    fn journal_path(&self, manifest_path: &Path) -> Result<PathBuf, ArchiveError> {
        let referenced = Path::new(&self.raw_journal);
        if referenced.is_absolute()
            || referenced.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(ArchiveError::InvalidManifest(format!(
                "raw_journal `{}` must be a relative path inside the session",
                self.raw_journal
            )));
        }
        let base = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let resolved = base.join(referenced);
        let metadata = std::fs::symlink_metadata(&resolved)?;
        if !metadata.is_file() {
            return Err(ArchiveError::InvalidManifest(format!(
                "raw_journal `{}` is not a regular file",
                self.raw_journal
            )));
        }
        Ok(resolved)
    }
}

/// Read a manifest without ever buffering more than the declared limit.
fn read_bounded_manifest(path: &Path) -> Result<Vec<u8>, ArchiveError> {
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        if bytes.len().saturating_add(count) > MAX_MANIFEST_BYTES {
            return Err(ArchiveError::InvalidManifest(format!(
                "manifest exceeds the {MAX_MANIFEST_BYTES}-byte limit"
            )));
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(bytes)
}

/// Resolve the journal referenced by a validated session manifest.
///
/// Use this before decoding so the decoded file cannot differ from the file
/// that was verified. It performs the same bounded parse and basic validation
/// as [`verify_capture_session`], but does not read the journal itself.
pub fn verified_journal_path(
    manifest_path: impl AsRef<Path>,
    expected_provider: &str,
) -> Result<PathBuf, ArchiveError> {
    let manifest_path = manifest_path.as_ref();
    let bytes = read_bounded_manifest(manifest_path)?;
    let manifest = CaptureManifest::from_json(&bytes)?;
    manifest.validate_basics(expected_provider)?;
    manifest.journal_path(manifest_path)
}

/// Hash bytes as they are consumed from the underlying reader.
struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(buffer)?;
        self.hasher.update(&buffer[..count]);
        Ok(count)
    }
}

/// Verify a capture session in one bounded pass over its journal.
///
/// The journal is streamed through the shared [`JournalReader`] while its bytes
/// are hashed, so record validation, frame counting and whole-file hashing all
/// observe the same stream. No frame collection or whole-archive buffering is
/// performed. A terminal capture error is rejected before the journal opens.
pub fn verify_capture_session(
    manifest_path: impl AsRef<Path>,
    expected_provider: &str,
) -> Result<SessionVerification, ArchiveError> {
    let manifest_path = manifest_path.as_ref();
    let manifest_bytes = read_bounded_manifest(manifest_path)?;
    let manifest = CaptureManifest::from_json(&manifest_bytes)?;
    manifest.validate_basics(expected_provider)?;
    let journal_path = manifest.journal_path(manifest_path)?;

    let hashing = HashingReader {
        inner: File::open(journal_path)?,
        hasher: Sha256::new(),
    };
    let mut reader = JournalReader::new(BufReader::new(hashing));
    while reader.next_frame()?.is_some() {}
    let frame_count = reader.frame_count();
    let bytes_scanned = reader.bytes_scanned();
    let hashing = reader.into_inner().into_inner();

    let sha256_hex = to_hex(&hashing.hasher.finalize());
    if !sha256_hex.eq_ignore_ascii_case(&manifest.raw_journal_sha256) {
        return Err(ArchiveError::JournalHashMismatch);
    }
    if frame_count != manifest.raw_journal_frame_count {
        return Err(ArchiveError::JournalFrameCountMismatch {
            expected: manifest.raw_journal_frame_count,
            actual: frame_count,
        });
    }
    Ok(SessionVerification {
        frame_count,
        bytes_scanned,
        sha256_hex,
    })
}

#[cfg(all(test, feature = "market-shaped-tests"))]
mod tests {
    use std::path::{
        Path, //
        PathBuf,
    };

    use serde_json::{
        Value, //
        json,
    };

    use super::{
        CaptureManifest, //
        MAX_MANIFEST_BYTES,
        verify_capture_session,
    };
    use crate::{
        ArchiveError, //
        RawFrame,
    };

    fn frame(sequence: u64) -> RawFrame {
        RawFrame::new(
            "upstox",
            "NSE_EQ|TEST",
            sequence as i64,
            sequence,
            None,
            Some(sequence),
            "synthetic-fixture",
            format!("payload-{sequence}").as_bytes(),
        )
        .unwrap()
    }

    fn journal_bytes(frames: &[u64]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for sequence in frames {
            bytes.extend_from_slice(frame(*sequence).to_json_line().unwrap().as_bytes());
            bytes.push(b'\n');
        }
        bytes
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{
            Digest, //
            Sha256,
        };
        let digest = Sha256::digest(bytes);
        let mut output = String::with_capacity(64);
        for byte in digest {
            output.push_str(&format!("{byte:02x}"));
        }
        output
    }

    fn session(directory: &Path, frames: &[u64]) -> (PathBuf, PathBuf, Value) {
        let manifest_path = directory.join("manifest.json");
        let journal_path = directory.join("raw-frames.jsonl");
        let bytes = journal_bytes(frames);
        std::fs::write(&journal_path, &bytes).unwrap();
        let manifest = json!({
            "schema_version": 1,
            "provider": "Upstox",
            "feed_protocol": "Market Data Feed V3; protobuf binary",
            "mode": "LTPC",
            "instrument_keys": ["NSE_EQ|TEST"],
            "entitlement_ref": "synthetic-fixture",
            "started_at_utc": "2026-11-18T00:00:00.000000000Z",
            "ended_at_utc": "2026-11-18T00:01:00.000000000Z",
            "duration_ns": 60_000_000_000_u128,
            "connection_count": 1,
            "frame_count": frames.len(),
            "decode_error_count": 0,
            "terminal_error": null,
            "raw_journal": "raw-frames.jsonl",
            "raw_journal_sha256": sha256_hex(&bytes),
            "raw_journal_frame_count": frames.len(),
            "provider_sequence_available": false,
            "capture_sequence_semantics": "local receive-order counter; not a provider sequence",
            "limitations": ["synthetic fixture"],
        });
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        (manifest_path, journal_path, manifest)
    }

    fn write_manifest(path: &Path, manifest: &Value) {
        std::fs::write(path, serde_json::to_string_pretty(manifest).unwrap()).unwrap();
    }

    #[test]
    fn complete_session_verifies() {
        let directory = tempfile::tempdir().unwrap();
        let (manifest, _, _) = session(directory.path(), &[1, 2, 3]);
        let result = verify_capture_session(&manifest, "Upstox").unwrap();
        assert_eq!(result.frame_count, 3, "all frames counted");
        assert_eq!(
            result.bytes_scanned as usize,
            journal_bytes(&[1, 2, 3]).len(),
            "all bytes hashed"
        );
    }

    #[test]
    fn modified_journal_fails_hash_check() {
        let directory = tempfile::tempdir().unwrap();
        let (manifest, journal, _) = session(directory.path(), &[1, 2]);
        let text = std::fs::read_to_string(&journal).unwrap();
        // Whitespace keeps every record valid; only the file digest changes.
        let mutated = text.replacen("\"source\":\"upstox\"", "\"source\": \"upstox\"", 1);
        assert_ne!(mutated, text, "fixture must actually change");
        std::fs::write(&journal, mutated).unwrap();
        assert!(
            matches!(
                verify_capture_session(&manifest, "Upstox"),
                Err(ArchiveError::JournalHashMismatch)
            ),
            "a mutated journal is rejected"
        );
    }

    #[test]
    fn removed_record_fails_verification() {
        let directory = tempfile::tempdir().unwrap();
        let (manifest, journal, _) = session(directory.path(), &[1, 2, 3]);
        let bytes = journal_bytes(&[1, 3]);
        std::fs::write(&journal, bytes).unwrap();
        assert!(
            verify_capture_session(&manifest, "Upstox").is_err(),
            "removal is rejected"
        );
    }

    #[test]
    fn reordered_records_fails_hash_check() {
        let directory = tempfile::tempdir().unwrap();
        let (manifest, journal, _) = session(directory.path(), &[1, 2]);
        std::fs::write(&journal, journal_bytes(&[2, 1])).unwrap();
        assert!(
            matches!(
                verify_capture_session(&manifest, "Upstox"),
                Err(ArchiveError::JournalHashMismatch)
            ),
            "reordering changes the whole-file hash"
        );
    }

    #[test]
    fn manifest_count_mismatch_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let (manifest, _, mut value) = session(directory.path(), &[1, 2]);
        value["raw_journal_frame_count"] = json!(99);
        write_manifest(&manifest, &value);
        assert!(
            matches!(
                verify_capture_session(&manifest, "Upstox"),
                Err(ArchiveError::JournalFrameCountMismatch { .. })
            ),
            "a declared count that disagrees with the journal is rejected"
        );
    }

    #[test]
    fn manifest_hash_mismatch_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let (manifest, _, mut value) = session(directory.path(), &[1]);
        value["raw_journal_sha256"] = json!("00".repeat(32));
        write_manifest(&manifest, &value);
        assert!(
            matches!(
                verify_capture_session(&manifest, "Upstox"),
                Err(ArchiveError::JournalHashMismatch)
            ),
            "a wrong declared digest is rejected"
        );
    }

    #[test]
    fn terminal_capture_error_is_rejected_before_reading_journal() {
        let directory = tempfile::tempdir().unwrap();
        let (manifest, journal, mut value) = session(directory.path(), &[1]);
        value["terminal_error"] = json!("socket closed unexpectedly");
        write_manifest(&manifest, &value);
        assert!(
            matches!(
                verify_capture_session(&manifest, "Upstox"),
                Err(ArchiveError::TerminalCaptureError(_))
            ),
            "an incomplete capture is not research-grade"
        );
        assert!(
            journal.exists(),
            "the journal is left untouched for diagnosis"
        );
        value["terminal_error"] = json!("   ");
        write_manifest(&manifest, &value);
        assert!(
            matches!(
                verify_capture_session(&manifest, "Upstox"),
                Err(ArchiveError::TerminalCaptureError(_))
            ),
            "a whitespace-only terminal error still marks the session incomplete"
        );
    }

    #[test]
    fn torn_tail_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let (manifest, journal, mut value) = session(directory.path(), &[1]);
        let mut bytes = std::fs::read(&journal).unwrap();
        bytes.pop();
        std::fs::write(&journal, &bytes).unwrap();
        value["raw_journal_sha256"] = json!(sha256_hex(&bytes));
        write_manifest(&manifest, &value);
        assert!(
            matches!(
                verify_capture_session(&manifest, "Upstox"),
                Err(ArchiveError::TornTail)
            ),
            "a journal without a final newline fails closed"
        );
    }

    #[test]
    fn schema_and_provider_are_validated() {
        let directory = tempfile::tempdir().unwrap();
        let (manifest, _, mut value) = session(directory.path(), &[1]);
        value["schema_version"] = json!(2);
        write_manifest(&manifest, &value);
        assert!(
            matches!(
                verify_capture_session(&manifest, "Upstox"),
                Err(ArchiveError::InvalidManifest(_))
            ),
            "an unsupported schema is rejected"
        );
        let directory = tempfile::tempdir().unwrap();
        let (manifest, _, _) = session(directory.path(), &[1]);
        assert!(
            matches!(
                verify_capture_session(&manifest, "FYERS"),
                Err(ArchiveError::InvalidManifest(_))
            ),
            "a mismatched provider is rejected"
        );
    }

    #[test]
    fn unsafe_journal_paths_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let (manifest, _, mut value) = session(directory.path(), &[1]);
        value["raw_journal"] = json!("../other/raw-frames.jsonl");
        write_manifest(&manifest, &value);
        assert!(
            matches!(
                verify_capture_session(&manifest, "Upstox"),
                Err(ArchiveError::InvalidManifest(_))
            ),
            "parent traversal is rejected"
        );
        let outside = directory.path().parent().unwrap().join("outside.jsonl");
        std::fs::write(&outside, b"{}").unwrap();
        let link = directory.path().join("linked.jsonl");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        value["raw_journal"] = json!("linked.jsonl");
        write_manifest(&manifest, &value);
        assert!(
            matches!(
                verify_capture_session(&manifest, "Upstox"),
                Err(ArchiveError::InvalidManifest(_))
            ),
            "a symlinked journal is rejected"
        );
        let _ = std::fs::remove_file(outside);
    }

    #[test]
    fn manifest_parsing_is_bounded_and_strict() {
        let oversized = vec![b' '; MAX_MANIFEST_BYTES + 1];
        assert!(
            CaptureManifest::from_json(&oversized).is_err(),
            "an oversized manifest is rejected"
        );
        assert!(
            CaptureManifest::from_json(b"{\"schema_version\":1}").is_err(),
            "a partial manifest is rejected"
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("huge-manifest.json");
        std::fs::write(&path, vec![b' '; MAX_MANIFEST_BYTES + 1]).unwrap();
        assert!(
            matches!(
                verify_capture_session(&path, "Upstox"),
                Err(ArchiveError::InvalidManifest(_))
            ),
            "an oversized manifest file is rejected before being buffered"
        );
    }
}
