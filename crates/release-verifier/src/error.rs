// SPDX-License-Identifier: Apache-2.0
//! Typed refusals for the independent NDC release verifier.
//!
//! One variant covers one failure class. The `code` method maps a variant to the
//! stable D2 rejection code reported in an admission decision.

use std::path::PathBuf;

/// A private snapshot cleanup that did not reach an empty, synced directory.
///
/// The staging path is retained verbatim so an operator can identify the
/// directory that still needs attention. The entry names are the tracked names
/// cleanup had not confirmed as absent or removed when it stopped.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("CLEANUP_PENDING: private snapshot cleanup incomplete at {staging_path:?}: {detail}")]
pub struct CleanupPending {
    /// Exact caller-provided private staging path.
    pub staging_path: PathBuf,
    /// Tracked entry names not confirmed absent or removed.
    pub remaining_entries: Vec<String>,
    /// Operation that left cleanup incomplete, without untrusted file bytes.
    pub detail: String,
}

/// A refusal raised by one of the twelve verification stages.
///
/// The variant identifies the first failing stage and failure class. No variant
/// carries a recovery suggestion: a rejected release is not retried under the
/// same identity.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifierError {
    /// The manifest bytes are not a single strict JSON object.
    #[error("REJECT_SCHEMA_ENVELOPE: stage 1 manifest parse: {detail}")]
    ManifestParse {
        /// Parser detail without raw release bytes.
        detail: String,
    },
    /// A JSON object in the manifest or sidecar repeats a member name.
    #[error("REJECT_SCHEMA_ENVELOPE: stage 1 duplicate JSON member: {member}")]
    DuplicateMember {
        /// Repeating member name.
        member: String,
    },
    /// A manifest or sidecar object carries a member outside its schema.
    #[error("REJECT_SCHEMA_ENVELOPE: stage 1 unknown member: {member}")]
    UnknownMember {
        /// Unknown member name.
        member: String,
    },
    /// A required manifest member is absent or has the wrong value.
    #[error("REJECT_SCHEMA_ENVELOPE: stage 1 invalid manifest: {detail}")]
    ManifestSchema {
        /// Field-level detail.
        detail: String,
    },
    /// Recomputed RFC 8785 bytes do not match the sidecar digest.
    #[error("REJECT_SIGNATURE_INTEGRITY: stage 2 manifest digest mismatch")]
    ManifestDigestMismatch,
    /// The sidecar digest is not a lowercase 64-hex SHA-256 value.
    #[error("REJECT_SIGNATURE_INTEGRITY: stage 2 malformed manifest digest")]
    MalformedManifestDigest,
    /// The sidecar does not match its exact five-member schema.
    #[error("REJECT_SIGNATURE_INTEGRITY: stage 3 sidecar schema: {detail}")]
    SidecarSchema {
        /// Field-level detail.
        detail: String,
    },
    /// The sidecar signature is not standard padded base64 of 64 bytes.
    #[error("REJECT_SIGNATURE_INTEGRITY: stage 3 malformed signature encoding")]
    MalformedSignature,
    /// The detached Ed25519 signature does not verify over the domain digest.
    #[error("REJECT_SIGNATURE_INTEGRITY: stage 3 signature verification failed")]
    SignatureInvalid,
    /// The publisher key is absent from the out-of-band registry.
    #[error("REJECT_PUBLISHER_IDENTITY: stage 4 unknown publisher key")]
    UnknownKey,
    /// The publisher key is not in a state that permits verification.
    #[error("REJECT_PUBLISHER_IDENTITY: stage 4 publisher key state {state} is not active")]
    KeyState {
        /// Registry state text.
        state: String,
    },
    /// The sidecar key id, manifest signing key id and registry entry disagree.
    #[error("REJECT_PUBLISHER_IDENTITY: stage 4 publisher key binding mismatch")]
    KeyBinding,
    /// The registry entry declares an algorithm other than Ed25519.
    #[error("REJECT_PUBLISHER_IDENTITY: stage 4 key algorithm mismatch")]
    KeyAlgorithm,
    /// The registry entry declares a purpose other than release signing.
    #[error("REJECT_PUBLISHER_IDENTITY: stage 4 key purpose mismatch")]
    KeyPurpose,
    /// The publisher-supplied validity interval does not cover the release time.
    #[error("REJECT_PUBLISHER_IDENTITY: stage 4 key validity does not cover as_of_ns")]
    KeyValidity,
    /// The trusted registry itself is malformed.
    #[error("REJECT_PUBLISHER_IDENTITY: stage 4 registry invalid: {detail}")]
    RegistryInvalid {
        /// Registry-level detail.
        detail: String,
    },
    /// A declared artifact path violates the path-safety rules.
    #[error("REJECT_TRUSTED_PATH: stage 5 unsafe artifact path: {detail}")]
    UnsafePath {
        /// Path-level detail.
        detail: String,
    },
    /// A path component is a symlink or resolves outside the release directory.
    #[error("REJECT_TRUSTED_PATH: stage 5 symlink component: {detail}")]
    SymlinkEscape {
        /// Component detail.
        detail: String,
    },
    /// A published file has more than one hard link.
    #[error("REJECT_TRUSTED_PATH: stage 5 hard-linked published file: {detail}")]
    HardLink {
        /// File detail.
        detail: String,
    },
    /// A required path names a directory, socket, device or other special file.
    #[error("REJECT_TRUSTED_PATH: stage 5 special file where regular file required")]
    SpecialFile,
    /// The published directory file set differs from the declared set.
    #[error("REJECT_ARTIFACT_DIGESTS: stage 5 file set mismatch: {detail}")]
    FileSet {
        /// Missing, extra or duplicate entry detail.
        detail: String,
    },
    /// Actual bytes, declared bytes and post-read metadata disagree.
    #[error("REJECT_ARTIFACT_DIGESTS: stage 5 byte-count mismatch: {detail}")]
    ByteCount {
        /// Size detail.
        detail: String,
    },
    /// A stored, uncompressed or dataset digest differs from the manifest.
    #[error("REJECT_ARTIFACT_DIGESTS: stage 5 digest mismatch: {detail}")]
    DigestMismatch {
        /// Digest family detail.
        detail: String,
    },
    /// A declared digest is not lowercase 64-hex.
    #[error("REJECT_ARTIFACT_DIGESTS: stage 5 malformed digest: {detail}")]
    MalformedDigest {
        /// Field detail.
        detail: String,
    },
    /// A record stream is not canonical UTF-8 JSONL with LF boundaries.
    #[error("REJECT_ARTIFACT_DIGESTS: stage 5 non-canonical record stream: {detail}")]
    RecordStream {
        /// Record-level detail.
        detail: String,
    },
    /// Declared record count differs from observed boundaries.
    #[error("REJECT_ARTIFACT_DIGESTS: stage 5 record count mismatch")]
    RecordCount,
    /// Records are reordered, duplicated or contain a disallowed gap.
    #[error("REJECT_ARTIFACT_DIGESTS: stage 5 record order policy violation: {detail}")]
    RecordOrder {
        /// Ordering detail.
        detail: String,
    },
    /// Decompression fails or the declared compression profile is unknown.
    #[error("REJECT_ARTIFACT_DIGESTS: stage 5 decompression failure: {detail}")]
    Decompression {
        /// Codec detail.
        detail: String,
    },
    /// A declared or actual quantity exceeds the effective resource profile.
    #[error("REJECT_RESOURCE_LIMIT: verifier resource policy exceeded: {limit}")]
    ResourceLimit {
        /// Effective limit field name.
        limit: String,
    },
    /// Profile selection, digest, role or ceiling validation refused the run.
    #[error("REJECT_RESOURCE_LIMIT: stage 5 resource profile refused: {detail}")]
    ResourceProfile {
        /// Profile-level detail without raw limits or caller declarations.
        detail: String,
    },
    /// A record envelope version differs from the manifest at detection.
    #[error("REJECT_SCHEMA_ENVELOPE: stage 5 record envelope version mismatch: {detail}")]
    EnvelopeVersion {
        /// Envelope-level detail.
        detail: String,
    },
    /// A record event type missed the ratified payload schema registry.
    #[error("REJECT_PAYLOAD_REGISTRY: stage 5 payload registry miss: {detail}")]
    PayloadRegistry {
        /// Registry-level detail without raw record bytes.
        detail: String,
    },
    /// The private snapshot copy does not reproduce the opened descriptor bytes.
    #[error("REJECT_TRUSTED_PATH: stage 5 private snapshot verification failed")]
    SnapshotMismatch,
    /// A filesystem operation failed.
    #[error("REJECT_IO: trusted filesystem operation failed: {detail}")]
    Io {
        /// Operation and error text without untrusted file bytes.
        detail: String,
    },
    /// Private snapshot cleanup did not prove an empty, synced directory.
    #[error(transparent)]
    CleanupPending(#[from] CleanupPending),
    /// Injected rights evaluation returned UNKNOWN or DENY.
    #[error("REJECT_RIGHTS_EVIDENCE: stage 6 rights decision is not PERMIT")]
    RightsNotPermitted,
    /// Coverage is absent, lacks a denominator or relies on implicit ALL.
    #[error("REJECT_COVERAGE: stage 7 coverage decision fails closed: {detail}")]
    Coverage {
        /// Coverage detail.
        detail: String,
    },
    /// Producer activity evidence references are incomplete or inconsistent.
    #[error("REJECT_RIGHTS_EVIDENCE: stage 8 producer activity evidence missing: {detail}")]
    ProducerActivity {
        /// Evidence detail.
        detail: String,
    },
    /// A declared envelope or payload schema version is missing or unknown.
    #[error("REJECT_SCHEMA_ENVELOPE: stage 9 schema version violation: {detail}")]
    Envelope {
        /// Envelope detail.
        detail: String,
    },
    /// Correction lineage has a cycle, self-reference or missing predecessor.
    #[error("REJECT_SOURCE_LINEAGE: stage 10 correction lineage invalid: {detail}")]
    Lineage {
        /// Lineage detail.
        detail: String,
    },
    /// Knowledge time is after the as-of instant or invalidates before knowledge.
    #[error("REJECT_PIT_CONSISTENCY: stage 10 point-in-time violation: {detail}")]
    PointInTime {
        /// Time detail.
        detail: String,
    },
    /// A requested consumer action is not covered by the declared profile.
    #[error("REJECT_USE_SCOPE: stage 11 consumer use violation: {detail}")]
    UseScope {
        /// Use detail.
        detail: String,
    },
    /// A requested capability is absent from the admission declaration.
    #[error("REJECT_USE_SCOPE: stage 11 declared capability missing: {detail}")]
    Capability {
        /// Capability detail.
        detail: String,
    },
    /// Retention is unbounded or lacks an explicit purge rule.
    #[error("REJECT_RETENTION: stage 11 retention declaration fails closed")]
    Retention,
    /// More than one grant matches a requested action.
    #[error("REJECT_AMBIGUOUS_GRANTS: stage 11 non-unique matching grant")]
    AmbiguousGrants,
    /// Receipt construction could not bind every required field.
    #[error("REJECT_RECEIPT: stage 12 receipt binding failed: {detail}")]
    Receipt {
        /// Receipt detail.
        detail: String,
    },
}

impl VerifierError {
    /// Return the stable rejection code for this refusal class.
    pub fn code(&self) -> &'static str {
        match self {
            Self::ManifestParse { .. }
            | Self::DuplicateMember { .. }
            | Self::UnknownMember { .. }
            | Self::ManifestSchema { .. }
            | Self::Envelope { .. }
            | Self::EnvelopeVersion { .. } => "REJECT_SCHEMA_ENVELOPE",
            Self::PayloadRegistry { .. } => "REJECT_PAYLOAD_REGISTRY",
            Self::ManifestDigestMismatch
            | Self::MalformedManifestDigest
            | Self::SidecarSchema { .. }
            | Self::MalformedSignature
            | Self::SignatureInvalid => "REJECT_SIGNATURE_INTEGRITY",
            Self::UnknownKey
            | Self::KeyState { .. }
            | Self::KeyBinding
            | Self::KeyAlgorithm
            | Self::KeyPurpose
            | Self::KeyValidity
            | Self::RegistryInvalid { .. } => "REJECT_PUBLISHER_IDENTITY",
            Self::UnsafePath { .. }
            | Self::SymlinkEscape { .. }
            | Self::HardLink { .. }
            | Self::SpecialFile
            | Self::SnapshotMismatch => "REJECT_TRUSTED_PATH",
            Self::FileSet { .. }
            | Self::ByteCount { .. }
            | Self::DigestMismatch { .. }
            | Self::MalformedDigest { .. }
            | Self::RecordStream { .. }
            | Self::RecordCount
            | Self::RecordOrder { .. }
            | Self::Decompression { .. } => "REJECT_ARTIFACT_DIGESTS",
            Self::ResourceLimit { .. } | Self::ResourceProfile { .. } => "REJECT_RESOURCE_LIMIT",
            Self::Io { .. } => "REJECT_IO",
            Self::CleanupPending(..) => "CLEANUP_PENDING",
            Self::RightsNotPermitted | Self::ProducerActivity { .. } => "REJECT_RIGHTS_EVIDENCE",
            Self::Coverage { .. } => "REJECT_COVERAGE",
            Self::Lineage { .. } => "REJECT_SOURCE_LINEAGE",
            Self::PointInTime { .. } => "REJECT_PIT_CONSISTENCY",
            Self::UseScope { .. } | Self::Capability { .. } => "REJECT_USE_SCOPE",
            Self::Retention => "REJECT_RETENTION",
            Self::AmbiguousGrants => "REJECT_AMBIGUOUS_GRANTS",
            Self::Receipt { .. } => "REJECT_RECEIPT",
        }
    }

    /// Return the one-based stage number that raised this refusal.
    ///
    /// Cross-cutting policy and filesystem refusals report the stage that first
    /// encountered them.
    pub fn stage(&self) -> u8 {
        match self {
            Self::ManifestParse { .. }
            | Self::DuplicateMember { .. }
            | Self::UnknownMember { .. }
            | Self::ManifestSchema { .. } => 1,
            Self::ManifestDigestMismatch | Self::MalformedManifestDigest => 2,
            Self::SidecarSchema { .. } | Self::MalformedSignature | Self::SignatureInvalid => 3,
            Self::UnknownKey
            | Self::KeyState { .. }
            | Self::KeyBinding
            | Self::KeyAlgorithm
            | Self::KeyPurpose
            | Self::KeyValidity
            | Self::RegistryInvalid { .. } => 4,
            Self::UnsafePath { .. }
            | Self::SymlinkEscape { .. }
            | Self::HardLink { .. }
            | Self::SpecialFile
            | Self::FileSet { .. }
            | Self::ByteCount { .. }
            | Self::DigestMismatch { .. }
            | Self::MalformedDigest { .. }
            | Self::RecordStream { .. }
            | Self::RecordCount
            | Self::RecordOrder { .. }
            | Self::Decompression { .. }
            | Self::SnapshotMismatch
            | Self::Io { .. }
            | Self::ResourceLimit { .. }
            | Self::ResourceProfile { .. }
            | Self::EnvelopeVersion { .. }
            | Self::PayloadRegistry { .. }
            | Self::CleanupPending(..) => 5,
            Self::RightsNotPermitted => 6,
            Self::Coverage { .. } => 7,
            Self::ProducerActivity { .. } => 8,
            Self::Envelope { .. } => 9,
            Self::Lineage { .. } | Self::PointInTime { .. } => 10,
            Self::UseScope { .. }
            | Self::Capability { .. }
            | Self::Retention
            | Self::AmbiguousGrants => 11,
            Self::Receipt { .. } => 12,
        }
    }
}
