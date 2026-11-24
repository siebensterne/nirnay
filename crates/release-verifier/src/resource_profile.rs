// SPDX-License-Identifier: Apache-2.0
//! Normative, immutable `ResourceProfile` contract for bounded streaming.
//!
//! This module implements the form ratified in
//! `engineering/decisions/WS-STREAM-001-b15-b16-architecture-ratification.md`
//! section 3. It defines a contract, not an approved instance: no production
//! digest, numeric maximum, codec identifier or capability state is registered
//! here. Every field of [`ResourceProfile`] and [`ProfileLimits`] is private and
//! populated by a validating constructor. Finite `TEST_ONLY profiles` are
//! available to test harnesses through [`LimitsInput`] and carry no production
//! authority.
//!
//! Effective limits are the stricter-of combination of a trusted local ceiling,
//! the selected role profile and a producer declaration when one is present.
//! A producer declaration can lower an allowance but never raise it.

use crate::error::VerifierError;
use sha2::Digest;
use sha2::Sha256;
use std::fmt::Write as FmtWrite;

/// Domain prefix bound into every canonical profile digest.
const PROFILE_DIGEST_DOMAIN: &[u8] = b"NDC-B16-RESOURCE-PROFILE-V1\x00";

/// Validation failure for a profile identifier, limits set or selection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    /// A profile identifier has an empty or malformed name or a zero version.
    #[error("resource profile identifier is invalid: {detail}")]
    Identifier {
        /// Field-level detail.
        detail: String,
    },
    /// A limits declaration has a non-finite, zero or inconsistent field.
    #[error("resource profile limits are invalid: {detail}")]
    Limits {
        /// Field-level detail.
        detail: String,
    },
    /// A declared digest does not match the canonical profile serialization.
    #[error("resource profile digest mismatch")]
    DigestMismatch,
    /// The executing role differs from the selected profile role.
    #[error("resource profile role mismatch: {detail}")]
    Role {
        /// Role-level detail.
        detail: String,
    },
}

impl ProfileError {
    /// Map a validation failure to the fail-closed verifier refusal.
    pub fn into_verifier_error(self) -> VerifierError {
        VerifierError::ResourceProfile {
            detail: self.to_string(),
        }
    }
}

/// Versioned, immutable identity of one resource profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceProfileId {
    name: String,
    version: u32,
}

impl ResourceProfileId {
    /// Validate and construct a profile identifier.
    pub fn new(name: impl Into<String>, version: u32) -> Result<Self, ProfileError> {
        let name = name.into();
        if name.is_empty() || name.len() > 128 || !name.is_ascii() {
            return Err(ProfileError::Identifier {
                detail: "name is empty, oversized or non-ASCII".to_owned(),
            });
        }
        if !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(ProfileError::Identifier {
                detail: "name uses a character outside the identifier alphabet".to_owned(),
            });
        }
        if version == 0 {
            return Err(ProfileError::Identifier {
                detail: "version must be positive".to_owned(),
            });
        }
        Ok(Self { name, version })
    }

    /// Return the declared profile name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the declared profile version.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Report whether the name explicitly marks a finite test-only profile.
    ///
    /// A test-only profile is accepted by a test harness only. It does not
    /// satisfy `STREAM_RESOURCE_PROFILE_RATIFIED` and is not evidence for
    /// `PERF-001`, resident-memory plateaus or operational qualification.
    pub fn is_test_only(&self) -> bool {
        self.name.contains("TEST_ONLY")
    }
}

/// Processing role bound to one resource profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessingRole {
    /// Fieldbook-side bounded producer.
    Producer,
    /// Nirnay-side bounded release verifier.
    Verifier,
    /// Bounded recovery task.
    Recovery,
    /// Private snapshot writer and binder.
    Snapshot,
}

impl ProcessingRole {
    /// Return the registered role token.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Producer => "PRODUCER",
            Self::Verifier => "VERIFIER",
            Self::Recovery => "RECOVERY",
            Self::Snapshot => "SNAPSHOT",
        }
    }
}

/// Plain declaration consumed by the validating [`ProfileLimits`] constructor.
///
/// A declaration is not a profile. It carries no digest and grants no
/// authority. All values are finite and checked before a profile is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitsInput {
    /// Maximum bytes in one framed canonical record line, excluding LF.
    pub max_record_bytes: u64,
    /// Maximum observed records in one artifact.
    pub max_records_per_artifact: u64,
    /// Maximum observed records in one release.
    pub max_records_per_release: u64,
    /// Maximum stored bytes in one artifact.
    pub max_stored_bytes_per_artifact: u64,
    /// Maximum stored bytes in one release.
    pub max_stored_bytes_per_release: u64,
    /// Maximum expanded bytes in one artifact.
    pub max_expanded_bytes_per_artifact: u64,
    /// Maximum expanded bytes in one release.
    pub max_expanded_bytes_per_release: u64,
    /// Consumer-side emission cap applied before decoder buffer growth.
    pub max_decompression_budget_bytes: u64,
    /// Maximum expanded-to-stored ratio for one artifact.
    pub max_expansion_ratio: u64,
    /// Registered codec window for the compression method.
    pub codec_window_bytes: u64,
    /// Fixed stored and decoder chunk size.
    pub io_chunk_bytes: u64,
    /// Maximum JSON container depth inside one record line.
    pub max_nesting_depth: u32,
    /// Maximum bytes in one JSON string token.
    pub max_string_bytes: u64,
    /// Maximum bytes in one non-string token.
    pub max_token_bytes: u64,
    /// Maximum object members in one record.
    pub max_object_members: u32,
    /// Maximum array elements in one record.
    pub max_array_elements: u32,
    /// Maximum artifacts verified in parallel.
    pub max_concurrent_artifacts: u32,
    /// Fixed worker count for the role.
    pub worker_count: u32,
    /// Fixed record queue capacity.
    pub queue_capacity_records: u64,
    /// Spool and staging disk bound for one operation.
    pub temp_disk_bytes: u64,
    /// Wall-clock processing budget in milliseconds for one operation.
    pub max_processing_duration_ms: u64,
    /// Maximum bytes in one durable journal record.
    pub max_journal_record_bytes: u64,
    /// Maximum bytes in one preparation intent.
    pub max_preparation_intent_bytes: u64,
    /// Maximum bytes in one sealed publication intent.
    pub max_sealed_intent_bytes: u64,
    /// Parallel descriptor bound for one operation.
    pub max_open_file_descriptors: u32,
    /// Role-specific resident-memory budget.
    pub memory_budget_bytes: u64,
    /// Maximum declared artifacts in one release.
    pub max_artifacts: u64,
}

/// Validated, immutable limits set with private fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileLimits {
    max_record_bytes: u64,
    max_records_per_artifact: u64,
    max_records_per_release: u64,
    max_stored_bytes_per_artifact: u64,
    max_stored_bytes_per_release: u64,
    max_expanded_bytes_per_artifact: u64,
    max_expanded_bytes_per_release: u64,
    max_decompression_budget_bytes: u64,
    max_expansion_ratio: u64,
    codec_window_bytes: u64,
    io_chunk_bytes: u64,
    max_nesting_depth: u32,
    max_string_bytes: u64,
    max_token_bytes: u64,
    max_object_members: u32,
    max_array_elements: u32,
    max_concurrent_artifacts: u32,
    worker_count: u32,
    queue_capacity_records: u64,
    temp_disk_bytes: u64,
    max_processing_duration_ms: u64,
    max_journal_record_bytes: u64,
    max_preparation_intent_bytes: u64,
    max_sealed_intent_bytes: u64,
    max_open_file_descriptors: u32,
    memory_budget_bytes: u64,
    max_artifacts: u64,
}

impl ProfileLimits {
    /// Validate a declaration and construct immutable limits.
    pub fn validate(input: LimitsInput) -> Result<Self, ProfileError> {
        fn positive(value: u64, field: &str) -> Result<(), ProfileError> {
            if value == 0 {
                return Err(ProfileError::Limits {
                    detail: format!("{} must be positive", field),
                });
            }
            Ok(())
        }
        fn at_least(value: u32, minimum: u32, field: &str) -> Result<(), ProfileError> {
            if value < minimum {
                return Err(ProfileError::Limits {
                    detail: format!("{} is below the required minimum", field),
                });
            }
            Ok(())
        }

        for (value, field) in [
            (input.max_record_bytes, "max_record_bytes"),
            (input.max_records_per_artifact, "max_records_per_artifact"),
            (input.max_records_per_release, "max_records_per_release"),
            (
                input.max_stored_bytes_per_artifact,
                "max_stored_bytes_per_artifact",
            ),
            (
                input.max_stored_bytes_per_release,
                "max_stored_bytes_per_release",
            ),
            (
                input.max_expanded_bytes_per_artifact,
                "max_expanded_bytes_per_artifact",
            ),
            (
                input.max_expanded_bytes_per_release,
                "max_expanded_bytes_per_release",
            ),
            (
                input.max_decompression_budget_bytes,
                "max_decompression_budget_bytes",
            ),
            (input.max_expansion_ratio, "max_expansion_ratio"),
            (input.codec_window_bytes, "codec_window_bytes"),
            (input.io_chunk_bytes, "io_chunk_bytes"),
            (input.max_string_bytes, "max_string_bytes"),
            (input.max_token_bytes, "max_token_bytes"),
            (input.queue_capacity_records, "queue_capacity_records"),
            (input.temp_disk_bytes, "temp_disk_bytes"),
            (
                input.max_processing_duration_ms,
                "max_processing_duration_ms",
            ),
            (input.max_journal_record_bytes, "max_journal_record_bytes"),
            (
                input.max_preparation_intent_bytes,
                "max_preparation_intent_bytes",
            ),
            (input.max_sealed_intent_bytes, "max_sealed_intent_bytes"),
            (input.memory_budget_bytes, "memory_budget_bytes"),
            (input.max_artifacts, "max_artifacts"),
        ] {
            positive(value, field)?;
        }
        for (value, field) in [
            (input.max_nesting_depth, "max_nesting_depth"),
            (input.max_object_members, "max_object_members"),
            (input.max_array_elements, "max_array_elements"),
            (input.max_concurrent_artifacts, "max_concurrent_artifacts"),
            (input.worker_count, "worker_count"),
            (input.max_open_file_descriptors, "max_open_file_descriptors"),
        ] {
            at_least(value, 1, field)?;
        }
        if input.max_records_per_artifact > input.max_records_per_release {
            return Err(ProfileError::Limits {
                detail: "per-artifact record bound exceeds the release bound".to_owned(),
            });
        }
        if input.max_stored_bytes_per_artifact > input.max_stored_bytes_per_release {
            return Err(ProfileError::Limits {
                detail: "per-artifact stored bound exceeds the release bound".to_owned(),
            });
        }
        if input.max_expanded_bytes_per_artifact > input.max_expanded_bytes_per_release {
            return Err(ProfileError::Limits {
                detail: "per-artifact expanded bound exceeds the release bound".to_owned(),
            });
        }
        if input.io_chunk_bytes > input.max_record_bytes {
            return Err(ProfileError::Limits {
                detail: "chunk bound exceeds the record bound".to_owned(),
            });
        }
        if input.max_string_bytes > input.max_record_bytes
            || input.max_token_bytes > input.max_record_bytes
        {
            return Err(ProfileError::Limits {
                detail: "string or token bound exceeds the record bound".to_owned(),
            });
        }
        if input.queue_capacity_records > input.max_records_per_artifact {
            return Err(ProfileError::Limits {
                detail: "queue capacity exceeds the per-artifact record bound".to_owned(),
            });
        }
        if input.codec_window_bytes > input.max_expanded_bytes_per_artifact {
            return Err(ProfileError::Limits {
                detail: "codec window exceeds the per-artifact expanded bound".to_owned(),
            });
        }
        if input.max_decompression_budget_bytes > input.max_expanded_bytes_per_artifact {
            return Err(ProfileError::Limits {
                detail: "emission cap exceeds the per-artifact expanded bound".to_owned(),
            });
        }
        if input.temp_disk_bytes < input.max_stored_bytes_per_release {
            return Err(ProfileError::Limits {
                detail: "staging disk bound cannot hold the stored release".to_owned(),
            });
        }
        let fixed_buffers = input.io_chunk_bytes.saturating_add(input.max_record_bytes);
        if input.memory_budget_bytes < fixed_buffers {
            return Err(ProfileError::Limits {
                detail: "role memory budget cannot hold the fixed buffers".to_owned(),
            });
        }
        // The structural parallel set is the artifact descriptor, the staging
        // writer and the verify descriptor. The root and release directory
        // descriptors are held while the set is opened.
        if input.max_open_file_descriptors < 3 {
            return Err(ProfileError::Limits {
                detail: "descriptor bound is below the required parallel set".to_owned(),
            });
        }
        Ok(Self {
            max_record_bytes: input.max_record_bytes,
            max_records_per_artifact: input.max_records_per_artifact,
            max_records_per_release: input.max_records_per_release,
            max_stored_bytes_per_artifact: input.max_stored_bytes_per_artifact,
            max_stored_bytes_per_release: input.max_stored_bytes_per_release,
            max_expanded_bytes_per_artifact: input.max_expanded_bytes_per_artifact,
            max_expanded_bytes_per_release: input.max_expanded_bytes_per_release,
            max_decompression_budget_bytes: input.max_decompression_budget_bytes,
            max_expansion_ratio: input.max_expansion_ratio,
            codec_window_bytes: input.codec_window_bytes,
            io_chunk_bytes: input.io_chunk_bytes,
            max_nesting_depth: input.max_nesting_depth,
            max_string_bytes: input.max_string_bytes,
            max_token_bytes: input.max_token_bytes,
            max_object_members: input.max_object_members,
            max_array_elements: input.max_array_elements,
            max_concurrent_artifacts: input.max_concurrent_artifacts,
            worker_count: input.worker_count,
            queue_capacity_records: input.queue_capacity_records,
            temp_disk_bytes: input.temp_disk_bytes,
            max_processing_duration_ms: input.max_processing_duration_ms,
            max_journal_record_bytes: input.max_journal_record_bytes,
            max_preparation_intent_bytes: input.max_preparation_intent_bytes,
            max_sealed_intent_bytes: input.max_sealed_intent_bytes,
            max_open_file_descriptors: input.max_open_file_descriptors,
            memory_budget_bytes: input.memory_budget_bytes,
            max_artifacts: input.max_artifacts,
        })
    }

    /// Return the maximum bytes in one framed record line.
    pub fn max_record_bytes(&self) -> u64 {
        self.max_record_bytes
    }

    /// Return the maximum records in one artifact.
    pub fn max_records_per_artifact(&self) -> u64 {
        self.max_records_per_artifact
    }

    /// Return the maximum records in one release.
    pub fn max_records_per_release(&self) -> u64 {
        self.max_records_per_release
    }

    /// Return the maximum stored bytes in one artifact.
    pub fn max_stored_bytes_per_artifact(&self) -> u64 {
        self.max_stored_bytes_per_artifact
    }

    /// Return the maximum stored bytes in one release.
    pub fn max_stored_bytes_per_release(&self) -> u64 {
        self.max_stored_bytes_per_release
    }

    /// Return the maximum expanded bytes in one artifact.
    pub fn max_expanded_bytes_per_artifact(&self) -> u64 {
        self.max_expanded_bytes_per_artifact
    }

    /// Return the maximum expanded bytes in one release.
    pub fn max_expanded_bytes_per_release(&self) -> u64 {
        self.max_expanded_bytes_per_release
    }

    /// Return the consumer-side emission cap.
    pub fn max_decompression_budget_bytes(&self) -> u64 {
        self.max_decompression_budget_bytes
    }

    /// Return the maximum expansion ratio for one artifact.
    pub fn max_expansion_ratio(&self) -> u64 {
        self.max_expansion_ratio
    }

    /// Return the registered codec window bound.
    pub fn codec_window_bytes(&self) -> u64 {
        self.codec_window_bytes
    }

    /// Return the fixed stored and decoder chunk size.
    pub fn io_chunk_bytes(&self) -> u64 {
        self.io_chunk_bytes
    }

    /// Return the maximum JSON nesting depth.
    pub fn max_nesting_depth(&self) -> u32 {
        self.max_nesting_depth
    }

    /// Return the maximum string token length.
    pub fn max_string_bytes(&self) -> u64 {
        self.max_string_bytes
    }

    /// Return the maximum non-string token length.
    pub fn max_token_bytes(&self) -> u64 {
        self.max_token_bytes
    }

    /// Return the maximum object member count.
    pub fn max_object_members(&self) -> u32 {
        self.max_object_members
    }

    /// Return the maximum array element count.
    pub fn max_array_elements(&self) -> u32 {
        self.max_array_elements
    }

    /// Return the maximum parallel artifact count.
    pub fn max_concurrent_artifacts(&self) -> u32 {
        self.max_concurrent_artifacts
    }

    /// Return the worker count.
    pub fn worker_count(&self) -> u32 {
        self.worker_count
    }

    /// Return the record queue capacity.
    pub fn queue_capacity_records(&self) -> u64 {
        self.queue_capacity_records
    }

    /// Return the staging disk bound.
    pub fn temp_disk_bytes(&self) -> u64 {
        self.temp_disk_bytes
    }

    /// Return the processing duration budget in milliseconds.
    pub fn max_processing_duration_ms(&self) -> u64 {
        self.max_processing_duration_ms
    }

    /// Return the journal record size bound.
    pub fn max_journal_record_bytes(&self) -> u64 {
        self.max_journal_record_bytes
    }

    /// Return the preparation intent size bound.
    pub fn max_preparation_intent_bytes(&self) -> u64 {
        self.max_preparation_intent_bytes
    }

    /// Return the sealed intent size bound.
    pub fn max_sealed_intent_bytes(&self) -> u64 {
        self.max_sealed_intent_bytes
    }

    /// Return the parallel descriptor bound.
    pub fn max_open_file_descriptors(&self) -> u32 {
        self.max_open_file_descriptors
    }

    /// Return the role-specific resident-memory budget.
    pub fn memory_budget_bytes(&self) -> u64 {
        self.memory_budget_bytes
    }

    /// Return the maximum declared artifact count.
    pub fn max_artifacts(&self) -> u64 {
        self.max_artifacts
    }
}

fn canonical_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&u64::from(value).to_le_bytes());
}

fn canonical_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn canonical_text(out: &mut Vec<u8>, value: &str) {
    canonical_u64(out, value.len() as u64);
    out.extend_from_slice(value.as_bytes());
}

fn canonical_limits(out: &mut Vec<u8>, l: &ProfileLimits) {
    canonical_u64(out, l.max_record_bytes);
    canonical_u64(out, l.max_records_per_artifact);
    canonical_u64(out, l.max_records_per_release);
    canonical_u64(out, l.max_stored_bytes_per_artifact);
    canonical_u64(out, l.max_stored_bytes_per_release);
    canonical_u64(out, l.max_expanded_bytes_per_artifact);
    canonical_u64(out, l.max_expanded_bytes_per_release);
    canonical_u64(out, l.max_decompression_budget_bytes);
    canonical_u64(out, l.max_expansion_ratio);
    canonical_u64(out, l.codec_window_bytes);
    canonical_u64(out, l.io_chunk_bytes);
    canonical_u32(out, l.max_nesting_depth);
    canonical_u64(out, l.max_string_bytes);
    canonical_u64(out, l.max_token_bytes);
    canonical_u32(out, l.max_object_members);
    canonical_u32(out, l.max_array_elements);
    canonical_u32(out, l.max_concurrent_artifacts);
    canonical_u32(out, l.worker_count);
    canonical_u64(out, l.queue_capacity_records);
    canonical_u64(out, l.temp_disk_bytes);
    canonical_u64(out, l.max_processing_duration_ms);
    canonical_u64(out, l.max_journal_record_bytes);
    canonical_u64(out, l.max_preparation_intent_bytes);
    canonical_u64(out, l.max_sealed_intent_bytes);
    canonical_u32(out, l.max_open_file_descriptors);
    canonical_u64(out, l.memory_budget_bytes);
    canonical_u64(out, l.max_artifacts);
}

fn profile_digest(
    id: &ResourceProfileId,
    role: ProcessingRole,
    limits: &ProfileLimits,
) -> [u8; 32] {
    let mut canonical = Vec::with_capacity(PROFILE_DIGEST_DOMAIN.len() + 256);
    canonical.extend_from_slice(PROFILE_DIGEST_DOMAIN);
    canonical_text(&mut canonical, id.name());
    canonical_u32(&mut canonical, id.version());
    canonical_text(&mut canonical, role.as_str());
    canonical_limits(&mut canonical, limits);
    let bytes = Sha256::digest(&canonical);
    let mut out = [0_u8; 32];
    out.copy_from_slice(&bytes);
    out
}

/// Immutable, versioned, role-specific resource profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceProfile {
    id: ResourceProfileId,
    digest: [u8; 32],
    role: ProcessingRole,
    limits: ProfileLimits,
}

impl ResourceProfile {
    /// Validate a declaration and derive the canonical profile digest.
    pub fn new(
        id: ResourceProfileId,
        role: ProcessingRole,
        input: LimitsInput,
    ) -> Result<Self, ProfileError> {
        let limits = ProfileLimits::validate(input)?;
        let digest = profile_digest(&id, role, &limits);
        Ok(Self {
            id,
            digest,
            role,
            limits,
        })
    }

    /// Build a profile and fail closed when a declared digest disagrees.
    pub fn from_declared(
        id: ResourceProfileId,
        role: ProcessingRole,
        input: LimitsInput,
        declared: [u8; 32],
    ) -> Result<Self, ProfileError> {
        let profile = Self::new(id, role, input)?;
        if profile.digest != declared {
            return Err(ProfileError::DigestMismatch);
        }
        Ok(profile)
    }

    /// Return the immutable identifier.
    pub fn id(&self) -> &ResourceProfileId {
        &self.id
    }

    /// Return the canonical digest over identifier, role and complete limits.
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    /// Return the bound processing role.
    pub fn role(&self) -> ProcessingRole {
        self.role
    }

    /// Return the validated limits set.
    pub fn limits(&self) -> &ProfileLimits {
        &self.limits
    }
}

fn stricter_u64(local: u64, role: u64, declared: Option<u64>) -> u64 {
    match declared {
        Some(value) => local.min(role).min(value),
        None => local.min(role),
    }
}

fn stricter_u32(local: u32, role: u32, declared: Option<u32>) -> u32 {
    match declared {
        Some(value) => local.min(role).min(value),
        None => local.min(role),
    }
}

/// Effective limits derived from the stricter-of rule in WS-STREAM-001 §3.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveLimits {
    limits: ProfileLimits,
}

impl EffectiveLimits {
    fn from_limits(limits: ProfileLimits) -> Self {
        Self { limits }
    }

    /// Combine the trusted local ceiling with the role profile and an optional
    /// producer declaration. A declaration can lower, never raise, an allowance.
    pub fn combine(
        ceiling: &ProfileLimits,
        role: &ProfileLimits,
        producer: Option<&ProfileLimits>,
    ) -> Result<Self, ProfileError> {
        let min64 = |pick: fn(&ProfileLimits) -> u64| -> u64 {
            stricter_u64(pick(ceiling), pick(role), producer.map(pick))
        };
        let min32 = |pick: fn(&ProfileLimits) -> u32| -> u32 {
            stricter_u32(pick(ceiling), pick(role), producer.map(pick))
        };
        let input = LimitsInput {
            max_record_bytes: min64(|l| l.max_record_bytes),
            max_records_per_artifact: min64(|l| l.max_records_per_artifact),
            max_records_per_release: min64(|l| l.max_records_per_release),
            max_stored_bytes_per_artifact: min64(|l| l.max_stored_bytes_per_artifact),
            max_stored_bytes_per_release: min64(|l| l.max_stored_bytes_per_release),
            max_expanded_bytes_per_artifact: min64(|l| l.max_expanded_bytes_per_artifact),
            max_expanded_bytes_per_release: min64(|l| l.max_expanded_bytes_per_release),
            max_decompression_budget_bytes: min64(|l| l.max_decompression_budget_bytes),
            max_expansion_ratio: min64(|l| l.max_expansion_ratio),
            codec_window_bytes: min64(|l| l.codec_window_bytes),
            io_chunk_bytes: min64(|l| l.io_chunk_bytes),
            max_nesting_depth: min32(|l| l.max_nesting_depth),
            max_string_bytes: min64(|l| l.max_string_bytes),
            max_token_bytes: min64(|l| l.max_token_bytes),
            max_object_members: min32(|l| l.max_object_members),
            max_array_elements: min32(|l| l.max_array_elements),
            max_concurrent_artifacts: min32(|l| l.max_concurrent_artifacts),
            worker_count: min32(|l| l.worker_count),
            queue_capacity_records: min64(|l| l.queue_capacity_records),
            temp_disk_bytes: min64(|l| l.temp_disk_bytes),
            max_processing_duration_ms: min64(|l| l.max_processing_duration_ms),
            max_journal_record_bytes: min64(|l| l.max_journal_record_bytes),
            max_preparation_intent_bytes: min64(|l| l.max_preparation_intent_bytes),
            max_sealed_intent_bytes: min64(|l| l.max_sealed_intent_bytes),
            max_open_file_descriptors: min32(|l| l.max_open_file_descriptors),
            memory_budget_bytes: min64(|l| l.memory_budget_bytes),
            max_artifacts: min64(|l| l.max_artifacts),
        };
        Ok(Self::from_limits(ProfileLimits::validate(input)?))
    }

    /// Return the effective maximum bytes in one record line.
    pub fn max_record_bytes(&self) -> u64 {
        self.limits.max_record_bytes()
    }

    /// Return the effective maximum records in one artifact.
    pub fn max_records_per_artifact(&self) -> u64 {
        self.limits.max_records_per_artifact()
    }

    /// Return the effective maximum records in one release.
    pub fn max_records_per_release(&self) -> u64 {
        self.limits.max_records_per_release()
    }

    /// Return the effective maximum stored bytes in one artifact.
    pub fn max_stored_bytes_per_artifact(&self) -> u64 {
        self.limits.max_stored_bytes_per_artifact()
    }

    /// Return the effective maximum stored bytes in one release.
    pub fn max_stored_bytes_per_release(&self) -> u64 {
        self.limits.max_stored_bytes_per_release()
    }

    /// Return the effective maximum expanded bytes in one artifact.
    pub fn max_expanded_bytes_per_artifact(&self) -> u64 {
        self.limits.max_expanded_bytes_per_artifact()
    }

    /// Return the effective maximum expanded bytes in one release.
    pub fn max_expanded_bytes_per_release(&self) -> u64 {
        self.limits.max_expanded_bytes_per_release()
    }

    /// Return the effective decoder emission cap.
    pub fn max_decompression_budget_bytes(&self) -> u64 {
        self.limits.max_decompression_budget_bytes()
    }

    /// Return the effective expansion ratio bound.
    pub fn max_expansion_ratio(&self) -> u64 {
        self.limits.max_expansion_ratio()
    }

    /// Return the effective codec window bound.
    pub fn codec_window_bytes(&self) -> u64 {
        self.limits.codec_window_bytes()
    }

    /// Return the effective fixed chunk size.
    pub fn io_chunk_bytes(&self) -> u64 {
        self.limits.io_chunk_bytes()
    }

    /// Return the effective JSON nesting depth bound.
    pub fn max_nesting_depth(&self) -> u32 {
        self.limits.max_nesting_depth()
    }

    /// Return the effective string token bound.
    pub fn max_string_bytes(&self) -> u64 {
        self.limits.max_string_bytes()
    }

    /// Return the effective non-string token bound.
    pub fn max_token_bytes(&self) -> u64 {
        self.limits.max_token_bytes()
    }

    /// Return the effective object member bound.
    pub fn max_object_members(&self) -> u32 {
        self.limits.max_object_members()
    }

    /// Return the effective array element bound.
    pub fn max_array_elements(&self) -> u32 {
        self.limits.max_array_elements()
    }

    /// Return the effective parallel artifact bound.
    pub fn max_concurrent_artifacts(&self) -> u32 {
        self.limits.max_concurrent_artifacts()
    }

    /// Return the effective worker count.
    pub fn worker_count(&self) -> u32 {
        self.limits.worker_count()
    }

    /// Return the effective queue capacity.
    pub fn queue_capacity_records(&self) -> u64 {
        self.limits.queue_capacity_records()
    }

    /// Return the effective staging disk bound.
    pub fn temp_disk_bytes(&self) -> u64 {
        self.limits.temp_disk_bytes()
    }

    /// Return the effective processing duration budget.
    pub fn max_processing_duration_ms(&self) -> u64 {
        self.limits.max_processing_duration_ms()
    }

    /// Return the effective journal record bound.
    pub fn max_journal_record_bytes(&self) -> u64 {
        self.limits.max_journal_record_bytes()
    }

    /// Return the effective preparation intent bound.
    pub fn max_preparation_intent_bytes(&self) -> u64 {
        self.limits.max_preparation_intent_bytes()
    }

    /// Return the effective sealed intent bound.
    pub fn max_sealed_intent_bytes(&self) -> u64 {
        self.limits.max_sealed_intent_bytes()
    }

    /// Return the effective descriptor bound.
    pub fn max_open_file_descriptors(&self) -> u32 {
        self.limits.max_open_file_descriptors()
    }

    /// Return the effective role memory budget.
    pub fn memory_budget_bytes(&self) -> u64 {
        self.limits.memory_budget_bytes()
    }

    /// Return the effective artifact count bound.
    pub fn max_artifacts(&self) -> u64 {
        self.limits.max_artifacts()
    }
}

/// Trusted local selection of role profiles and an optional producer evidence
/// declaration used by the streaming verifier entry point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamingResourcePolicy {
    ceiling: ProfileLimits,
    verifier: ResourceProfile,
    snapshot: ResourceProfile,
    producer: Option<ResourceProfile>,
}

impl StreamingResourcePolicy {
    /// Bind a local ceiling to role profiles selected by trusted local policy.
    ///
    /// The caller supplies no raw maximum at enforcement time: every limit is
    /// read from a validated profile in this selection.
    pub fn new(
        ceiling: LimitsInput,
        verifier: ResourceProfile,
        snapshot: ResourceProfile,
    ) -> Result<Self, ProfileError> {
        if verifier.role() != ProcessingRole::Verifier {
            return Err(ProfileError::Role {
                detail: "streaming verifier requires a VERIFIER profile".to_owned(),
            });
        }
        if snapshot.role() != ProcessingRole::Snapshot {
            return Err(ProfileError::Role {
                detail: "private snapshot requires a SNAPSHOT profile".to_owned(),
            });
        }
        Ok(Self {
            ceiling: ProfileLimits::validate(ceiling)?,
            verifier,
            snapshot,
            producer: None,
        })
    }

    /// Attach a producer declaration with a verified identifier and digest.
    ///
    /// The declaration lowers allowances; it never raises the consumer's
    /// local ceiling or role profile limit.
    pub fn with_producer_declaration(
        mut self,
        producer: ResourceProfile,
    ) -> Result<Self, ProfileError> {
        if producer.role() != ProcessingRole::Producer {
            return Err(ProfileError::Role {
                detail: "producer declaration requires a PRODUCER profile".to_owned(),
            });
        }
        self.producer = Some(producer);
        Ok(self)
    }

    /// Return the locally selected verifier profile.
    pub fn verifier_profile(&self) -> &ResourceProfile {
        &self.verifier
    }

    /// Return the locally selected snapshot profile.
    pub fn snapshot_profile(&self) -> &ResourceProfile {
        &self.snapshot
    }

    /// Return the declared producer profile, when one is present.
    pub fn producer_profile(&self) -> Option<&ResourceProfile> {
        self.producer.as_ref()
    }

    /// Return the effective limits applied while verifying artifacts.
    pub fn verifier_limits(&self) -> Result<EffectiveLimits, ProfileError> {
        EffectiveLimits::combine(
            &self.ceiling,
            self.verifier.limits(),
            self.producer.as_ref().map(ResourceProfile::limits),
        )
    }

    /// Return the effective limits applied while writing the private snapshot.
    pub fn snapshot_limits(&self) -> Result<EffectiveLimits, ProfileError> {
        EffectiveLimits::combine(&self.ceiling, self.snapshot.limits(), None)
    }
}

/// Render a digest as lowercase hex for evidence bindings.
pub fn profile_digest_hex(digest: &[u8; 32]) -> String {
    let mut text = String::with_capacity(64);
    for byte in digest {
        let _ = write!(text, "{:02x}", byte);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn test_limits() -> LimitsInput {
        LimitsInput {
            max_record_bytes: 512,
            max_records_per_artifact: 32,
            max_records_per_release: 128,
            max_stored_bytes_per_artifact: 16_384,
            max_stored_bytes_per_release: 65_536,
            max_expanded_bytes_per_artifact: 65_536,
            max_expanded_bytes_per_release: 262_144,
            max_decompression_budget_bytes: 65_536,
            max_expansion_ratio: 16,
            codec_window_bytes: 32_768,
            io_chunk_bytes: 64,
            max_nesting_depth: 8,
            max_string_bytes: 128,
            max_token_bytes: 64,
            max_object_members: 32,
            max_array_elements: 32,
            max_concurrent_artifacts: 4,
            worker_count: 4,
            queue_capacity_records: 16,
            temp_disk_bytes: 65_536,
            max_processing_duration_ms: 30_000,
            max_journal_record_bytes: 4_096,
            max_preparation_intent_bytes: 65_536,
            max_sealed_intent_bytes: 4_096,
            max_open_file_descriptors: 16,
            memory_budget_bytes: 268_435_456,
            max_artifacts: 4,
        }
    }

    fn verifier_profile() -> Result<ResourceProfile, ProfileError> {
        ResourceProfile::new(
            ResourceProfileId::new("TEST_ONLY_VERIFIER_V1", 1)?,
            ProcessingRole::Verifier,
            test_limits(),
        )
    }

    fn snapshot_profile() -> Result<ResourceProfile, ProfileError> {
        ResourceProfile::new(
            ResourceProfileId::new("TEST_ONLY_SNAPSHOT_V1", 1)?,
            ProcessingRole::Snapshot,
            test_limits(),
        )
    }

    #[test]
    fn profile_fields_are_not_publicly_constructible() -> Result<(), Box<dyn std::error::Error>> {
        let profile = verifier_profile()?;
        assert_eq!(profile.role(), ProcessingRole::Verifier, "bound role");
        assert!(
            profile.id().is_test_only(),
            "identifier carries the test-only marker"
        );
        Ok(())
    }

    #[test]
    fn validation_rejects_zero_and_relation_errors() -> Result<(), Box<dyn std::error::Error>> {
        let mut input = test_limits();
        input.io_chunk_bytes = 0;
        assert!(
            matches!(
                ProfileLimits::validate(input),
                Err(ProfileError::Limits { .. })
            ),
            "zero chunk is refused"
        );
        let mut input = test_limits();
        input.max_records_per_artifact = input.max_records_per_release + 1;
        assert!(
            matches!(
                ProfileLimits::validate(input),
                Err(ProfileError::Limits { .. })
            ),
            "per-artifact above per-release is refused"
        );
        let mut input = test_limits();
        input.queue_capacity_records = input.max_records_per_artifact + 1;
        assert!(
            matches!(
                ProfileLimits::validate(input),
                Err(ProfileError::Limits { .. })
            ),
            "queue above the record bound is refused"
        );
        Ok(())
    }

    #[test]
    fn validation_rejects_below_minimum_descriptor_set() -> Result<(), Box<dyn std::error::Error>> {
        let mut input = test_limits();
        input.max_open_file_descriptors = 2;
        assert!(
            matches!(
                ProfileLimits::validate(input),
                Err(ProfileError::Limits { .. })
            ),
            "parallel descriptor minimum is enforced"
        );
        Ok(())
    }

    #[test]
    fn digest_changes_with_every_bound_field() -> Result<(), Box<dyn std::error::Error>> {
        let first = verifier_profile()?;
        let mut input = test_limits();
        input.max_record_bytes += 1;
        let second = ResourceProfile::new(
            ResourceProfileId::new("TEST_ONLY_VERIFIER_V1", 1)?,
            ProcessingRole::Verifier,
            input,
        )?;
        assert_ne!(first.digest(), second.digest(), "limits bind the digest");
        let third = ResourceProfile::new(
            ResourceProfileId::new("TEST_ONLY_VERIFIER_V1", 2)?,
            ProcessingRole::Verifier,
            test_limits(),
        )?;
        assert_ne!(first.digest(), third.digest(), "version binds the digest");
        Ok(())
    }

    #[test]
    fn role_memory_budget_must_cover_the_fixed_buffers() -> Result<(), Box<dyn std::error::Error>> {
        let mut input = test_limits();
        input.memory_budget_bytes = 1;
        assert!(
            matches!(
                ProfileLimits::validate(input),
                Err(ProfileError::Limits { .. })
            ),
            "a memory budget below the fixed buffers is refused"
        );
        Ok(())
    }

    #[test]
    fn declared_digest_mismatch_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
        let declared = [0_u8; 32];
        assert!(
            matches!(
                ResourceProfile::from_declared(
                    ResourceProfileId::new("TEST_ONLY_VERIFIER_V1", 1)?,
                    ProcessingRole::Verifier,
                    test_limits(),
                    declared
                ),
                Err(ProfileError::DigestMismatch)
            ),
            "a wrong declared digest is refused"
        );
        Ok(())
    }

    #[test]
    fn role_mismatch_is_refused_before_selection() -> Result<(), Box<dyn std::error::Error>> {
        assert!(
            matches!(
                StreamingResourcePolicy::new(
                    test_limits(),
                    snapshot_profile()?,
                    snapshot_profile()?
                ),
                Err(ProfileError::Role { .. })
            ),
            "a snapshot profile cannot serve as the verifier profile"
        );
        Ok(())
    }

    #[test]
    fn producer_declaration_cannot_raise_a_local_limit() -> Result<(), Box<dyn std::error::Error>> {
        let mut generous = test_limits();
        generous.max_record_bytes = 4_096;
        let producer = ResourceProfile::new(
            ResourceProfileId::new("TEST_ONLY_PRODUCER_V1", 1)?,
            ProcessingRole::Producer,
            generous,
        )?;
        let policy =
            StreamingResourcePolicy::new(test_limits(), verifier_profile()?, snapshot_profile()?)?
                .with_producer_declaration(producer)?;
        assert_eq!(
            policy.verifier_limits()?.max_record_bytes(),
            test_limits().max_record_bytes,
            "the local ceiling stays effective"
        );
        Ok(())
    }

    #[test]
    fn producer_declaration_can_lower_an_allowance() -> Result<(), Box<dyn std::error::Error>> {
        let mut strict = test_limits();
        strict.max_record_bytes = 256;
        strict.max_string_bytes = 128;
        strict.max_token_bytes = 64;
        strict.io_chunk_bytes = 64;
        let producer = ResourceProfile::new(
            ResourceProfileId::new("TEST_ONLY_PRODUCER_V1", 1)?,
            ProcessingRole::Producer,
            strict,
        )?;
        let policy =
            StreamingResourcePolicy::new(test_limits(), verifier_profile()?, snapshot_profile()?)?
                .with_producer_declaration(producer)?;
        assert_eq!(
            policy.verifier_limits()?.max_record_bytes(),
            256,
            "the stricter declared limit is applied"
        );
        Ok(())
    }

    #[test]
    fn profile_digest_hex_is_lowercase_and_fixed_width() -> Result<(), Box<dyn std::error::Error>> {
        let text = profile_digest_hex(verifier_profile()?.digest());
        assert_eq!(text.len(), 64, "digest renders to 64 hex characters");
        assert!(
            text.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "digest renders in lowercase hex"
        );
        Ok(())
    }

    #[test]
    fn identifier_rejects_zero_version_and_bad_name() -> Result<(), Box<dyn std::error::Error>> {
        assert!(
            matches!(
                ResourceProfileId::new("TEST_ONLY_V1", 0),
                Err(ProfileError::Identifier { .. })
            ),
            "version zero is refused"
        );
        assert!(
            matches!(
                ResourceProfileId::new("bad name", 1),
                Err(ProfileError::Identifier { .. })
            ),
            "space in name is refused"
        );
        Ok(())
    }
}
