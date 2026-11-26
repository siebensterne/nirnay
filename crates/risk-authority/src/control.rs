//! Kill-switch and reconciliation control state with owner-local append-only
//! persistence.
//!
//! No new service or database is introduced: the file-backed implementation
//! uses fsynced JSON-lines files under the owner's runtime directory. Fail-closed
//! default: with no readable event the switch is engaged and reconciliation is
//! blocked. A revision is an exact sequence: a duplicate, gap, regression or
//! ambiguous record is a hard corruption error, never a silently tolerated
//! line.

use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use serde::{
    Deserialize,
    Serialize,
    de::DeserializeOwned, //
};
use thiserror::Error;

/// One authored kill-switch decision. Append-only; never rewritten.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KillSwitchState {
    pub enabled: bool,
    pub actor_id: String,
    pub reason: String,
    pub revision: u64,
    pub at_ns: i64,
}

/// One authored reconciliation-latch decision. The reason and the evidence
/// reference are durable facts, not incidental log text: a restart must be able
/// to show why a block was raised or cleared and which evidence supported it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationState {
    pub blocked: bool,
    pub reason: String,
    pub evidence_ref: Option<String>,
    pub revision: u64,
    pub at_ns: i64,
}

impl ReconciliationState {
    #[must_use]
    pub fn new(
        blocked: bool,
        reason: String,
        evidence_ref: Option<String>,
        revision: u64,
        at_ns: i64,
    ) -> Self {
        Self {
            blocked,
            reason,
            evidence_ref,
            revision,
            at_ns,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ControlError {
    #[error("control store io failure: {0}")]
    Io(String),
    #[error("control store serialization failure: {0}")]
    Serialize(String),
    #[error("control store record is corrupt: {0}")]
    Corrupt(String),
    #[error("control store cannot author a reconciliation decision: {0}")]
    Unsupported(String),
}

/// Minimal persistence contract. Implementations must make `append` durable
/// before returning, and `load` must fail closed on unreadable or non-sequential
/// data.
pub trait ControlStore {
    fn load_kill_switch(&self) -> Result<Option<KillSwitchState>, ControlError>;
    fn append_kill_switch(&self, state: &KillSwitchState) -> Result<(), ControlError>;
    fn load_reconciliation(&self) -> Result<Option<ReconciliationState>, ControlError> {
        Ok(None)
    }
    fn append_reconciliation(&self, _state: &ReconciliationState) -> Result<(), ControlError> {
        Err(ControlError::Unsupported(
            "store does not persist reconciliation decisions".to_owned(),
        ))
    }
    /// Whether the store is backed by durable records. Startup recovery must
    /// refuse a store that cannot survive a process restart.
    fn is_durable(&self) -> bool {
        true
    }
}

/// Explicit simulation/test store. It is never a production default: recovery
/// refuses a non-durable control store, and simulation constructors are named
/// as such.
#[derive(Clone, Debug, Default)]
pub struct NoopControlStore;

impl ControlStore for NoopControlStore {
    fn load_kill_switch(&self) -> Result<Option<KillSwitchState>, ControlError> {
        Ok(None)
    }

    fn append_kill_switch(&self, _state: &KillSwitchState) -> Result<(), ControlError> {
        Ok(())
    }

    fn load_reconciliation(&self) -> Result<Option<ReconciliationState>, ControlError> {
        Ok(None)
    }

    fn append_reconciliation(&self, _state: &ReconciliationState) -> Result<(), ControlError> {
        Ok(())
    }

    fn is_durable(&self) -> bool {
        false
    }
}

/// Append-only file store. Every record is one JSON line followed by fsync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileControlStore {
    path: PathBuf,
}

impl FileControlStore {
    #[must_use]
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The independent stream for reconciliation decisions. Keeping it beside
    /// the kill-switch stream preserves one owner directory without mixing two
    /// different revision sequences in one file.
    #[must_use]
    pub fn reconciliation_path(&self) -> PathBuf {
        let mut path = self.path.clone();
        let name = path
            .file_name()
            .and_then(|component| component.to_str())
            .unwrap_or("control");
        path.set_file_name(format!("{name}.reconciliation.jsonl"));
        path
    }

    fn load_stream<T>(path: &Path) -> Result<Option<T>, ControlError>
    where
        T: DeserializeOwned + Serialize,
    {
        if !path.exists() {
            return Ok(None);
        }
        let file = File::open(path).map_err(|error| ControlError::Io(error.to_string()))?;
        let mut latest = None;
        let mut expected: Option<u64> = None;
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|error| ControlError::Io(error.to_string()))?;
            if line.trim().is_empty() {
                continue;
            }
            let record: T = serde_json::from_str(&line)
                .map_err(|error| ControlError::Corrupt(format!("line {}: {error}", index + 1)))?;
            let revision = Self::revision_of(&record, path)?;
            if let Some(prior) = expected
                && revision != prior + 1
            {
                return Err(ControlError::Corrupt(format!(
                    "line {}: revision {revision} is not the exact successor of {prior}",
                    index + 1
                )));
            }
            expected = Some(revision);
            latest = Some(record);
        }
        Ok(latest)
    }

    fn revision_of<T>(record: &T, path: &Path) -> Result<u64, ControlError>
    where
        T: Serialize,
    {
        let value = serde_json::to_value(record)
            .map_err(|error| ControlError::Serialize(error.to_string()))?;
        let revision = value
            .get("revision")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                ControlError::Corrupt(format!(
                    "{}: record has no unsigned revision",
                    path.display()
                ))
            })?;
        if revision == 0 {
            return Err(ControlError::Corrupt(format!(
                "{}: revision zero is reserved for an absent record",
                path.display()
            )));
        }
        Ok(revision)
    }

    fn append_stream<T>(&self, path: &Path, record: &T, revision: u64) -> Result<(), ControlError>
    where
        T: Serialize + DeserializeOwned,
    {
        let prior = match Self::load_stream::<T>(path)? {
            Some(value) => Self::revision_of(&value, path)?,
            None => 0,
        };
        if revision != prior + 1 {
            return Err(ControlError::Corrupt(format!(
                "revision {revision} is not the exact successor of {prior}"
            )));
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| ControlError::Io(error.to_string()))?;
        let line = serde_json::to_string(record)
            .map_err(|error| ControlError::Serialize(error.to_string()))?;
        file.write_all(line.as_bytes())
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_all())
            .map_err(|error| ControlError::Io(error.to_string()))
    }
}

impl ControlStore for FileControlStore {
    fn load_kill_switch(&self) -> Result<Option<KillSwitchState>, ControlError> {
        Self::load_stream(&self.path)
    }

    fn append_kill_switch(&self, state: &KillSwitchState) -> Result<(), ControlError> {
        self.append_stream(&self.path, state, state.revision)
    }

    fn load_reconciliation(&self) -> Result<Option<ReconciliationState>, ControlError> {
        Self::load_stream(&self.reconciliation_path())
    }

    fn append_reconciliation(&self, state: &ReconciliationState) -> Result<(), ControlError> {
        self.append_stream(&self.reconciliation_path(), state, state.revision)
    }

    fn is_durable(&self) -> bool {
        true
    }
}
