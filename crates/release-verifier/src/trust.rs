// SPDX-License-Identifier: Apache-2.0
//! Out-of-band trust inputs and caller-supplied resource maxima.
//!
//! The registry is injected by the caller. No release supplies its own key,
//! and this crate creates no signing key and no production trust material.

use crate::error::VerifierError;

/// State of a trusted publisher key in the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    /// The key may sign new releases and verify historical ones.
    Active,
    /// The key cannot sign new releases; historical use needs separate evidence.
    Retired,
    /// The key is revoked and fails closed.
    Revoked,
    /// The key is compromised and fails closed.
    Compromised,
}

impl KeyState {
    /// Return the registry state token for this value.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Retired => "RETIRED",
            Self::Revoked => "REVOKED",
            Self::Compromised => "COMPROMISED",
        }
    }
}

/// Purpose for which a trusted key may be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPurpose {
    /// Signing and verifying NDC v2 release manifests.
    ReleaseSigning,
}

/// One out-of-band trusted publisher key entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedKeyEntry {
    /// Publisher identity bound to the key.
    pub publisher_id: String,
    /// Immutable key identity; never reused.
    pub key_id: String,
    /// Raw Ed25519 public key bytes.
    pub public_key_bytes: [u8; 32],
    /// Algorithm token; only `Ed25519` is accepted.
    pub algorithm: String,
    /// Permitted key purpose.
    pub purpose: KeyPurpose,
    /// Positive epoch-nanosecond validity begin.
    pub valid_from_ns: u64,
    /// Positive epoch-nanosecond validity end.
    pub valid_until_ns: u64,
    /// Current registry state.
    pub state: KeyState,
    /// Reference to revocation or retirement evidence.
    pub evidence_reference: Option<String>,
}

/// An immutable snapshot of the trusted publisher registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedKeyRegistry {
    entries: Vec<TrustedKeyEntry>,
    revision: String,
}

impl TrustedKeyRegistry {
    /// Build a registry from caller-supplied entries and a revision string.
    pub fn new(entries: Vec<TrustedKeyEntry>, revision: String) -> Result<Self, VerifierError> {
        if revision.is_empty() {
            return Err(VerifierError::RegistryInvalid {
                detail: "registry revision is empty".to_owned(),
            });
        }
        let mut identities = std::collections::HashSet::new();
        for entry in &entries {
            if entry.publisher_id.is_empty() || entry.key_id.is_empty() {
                return Err(VerifierError::RegistryInvalid {
                    detail: "registry entry has an empty identity".to_owned(),
                });
            }
            if !identities.insert(entry.key_id.clone()) {
                return Err(VerifierError::RegistryInvalid {
                    detail: "registry repeats a key id".to_owned(),
                });
            }
            if entry.valid_from_ns == 0
                || entry.valid_until_ns == 0
                || entry.valid_from_ns >= entry.valid_until_ns
            {
                return Err(VerifierError::RegistryInvalid {
                    detail: "registry validity interval is not positive".to_owned(),
                });
            }
        }
        Ok(Self { entries, revision })
    }

    /// Return the audited registry revision bound into the receipt.
    pub fn revision(&self) -> &str {
        &self.revision
    }

    /// Return the caller-supplied entries in registry order.
    pub fn entries(&self) -> &[TrustedKeyEntry] {
        &self.entries
    }

    /// Look up an entry by publisher and key identity without trust checks.
    pub fn lookup(
        &self,
        publisher_id: &str,
        key_id: &str,
    ) -> Result<&TrustedKeyEntry, VerifierError> {
        self.entries
            .iter()
            .find(|entry| entry.key_id == key_id && entry.publisher_id == publisher_id)
            .ok_or(VerifierError::UnknownKey)
    }

    /// Resolve a publisher key and enforce state, purpose and validity.
    pub fn resolve(
        &self,
        publisher_id: &str,
        key_id: &str,
        as_of_ns: u64,
    ) -> Result<&TrustedKeyEntry, VerifierError> {
        let entry = self.lookup(publisher_id, key_id)?;
        self.check_policy(entry, as_of_ns)?;
        Ok(entry)
    }

    /// Enforce state, algorithm, purpose and validity for a resolved entry.
    pub fn check_policy(
        &self,
        entry: &TrustedKeyEntry,
        as_of_ns: u64,
    ) -> Result<(), VerifierError> {
        if entry.state != KeyState::Active {
            return Err(VerifierError::KeyState {
                state: entry.state.as_str().to_owned(),
            });
        }
        if entry.algorithm != "Ed25519" {
            return Err(VerifierError::KeyAlgorithm);
        }
        if entry.purpose != KeyPurpose::ReleaseSigning {
            return Err(VerifierError::KeyPurpose);
        }
        if as_of_ns < entry.valid_from_ns || as_of_ns > entry.valid_until_ns {
            return Err(VerifierError::KeyValidity);
        }
        Ok(())
    }
}

/// Caller-supplied maxima for bounded streaming and private staging.
///
/// No field has a default value in this crate. Numeric maxima come from the
/// caller after workload qualification; the verifier invents no numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifierResourcePolicy {
    /// Maximum accepted manifest size in bytes.
    pub max_manifest_bytes: u64,
    /// Maximum accepted sidecar size in bytes.
    pub max_sidecar_bytes: u64,
    /// Maximum number of declared artifacts.
    pub max_artifact_count: u64,
    /// Maximum stored size of one artifact in bytes.
    pub max_stored_bytes: u64,
    /// Maximum expanded size of one artifact in bytes.
    pub max_expanded_bytes: u64,
    /// Maximum sum of expanded artifact bytes in a release.
    pub max_total_expanded_bytes: u64,
    /// Maximum length of one JSONL record in bytes.
    pub max_record_bytes: u64,
    /// Maximum number of records in one artifact.
    pub max_record_count: u64,
    /// Maximum JSON container nesting depth.
    pub max_json_nesting: u32,
    /// Maximum bytes emitted by one decompression operation.
    pub max_decompression_budget: u64,
    /// Maximum bytes written into the private staging directory.
    pub max_temp_disk_bytes: u64,
    /// Maximum number of simultaneously opened descriptors.
    pub max_open_file_descriptors: u32,
}
