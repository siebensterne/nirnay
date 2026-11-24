// SPDX-License-Identifier: Apache-2.0
//! Strict schemas for the manifest, sidecar, records and admission request.
//!
//! Every schema type denies unknown members. Unknown, ambiguous or missing
//! members therefore stop verification at stage one instead of being treated as
//! absent.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

/// The complete NDC v2 release manifest after Ratification Profile 1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    /// Positive integer layout version; profile 1 uses one.
    pub schema_version: u32,
    /// Approved NDC major contract version.
    pub contract_version: String,
    /// Reviewed contract revision bound to the approved requirement digests.
    pub contract_revision: String,
    /// Identity and publication state of this release.
    pub release_identity: ReleaseIdentity,
    /// Declared source scope of the release.
    pub source_scope: SourceScope,
    /// Producer operations and their evidence references.
    pub producer_activity: ProducerActivity,
    /// Written rights evidence carried by the release.
    pub rights: RightsRecord,
    /// Per-dimension coverage reports.
    pub coverage: CoverageRecord,
    /// Raw source and correction lineage.
    pub lineage: Lineage,
    /// Declared schema versions for payloads and envelopes.
    pub schema_versions: SchemaVersions,
    /// Dataset digest and digest algorithm.
    pub digests: Digests,
    /// Publisher identity and detached signature reference.
    pub publication_identity: PublicationIdentity,
    /// Sole inventory of published artifacts.
    pub artifacts: Vec<ArtifactDescriptor>,
}

/// Release identity and publication declaration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReleaseIdentity {
    /// New immutable identity string for this release.
    pub release_id: String,
    /// Release version string.
    pub release_version: String,
    /// Positive epoch-nanosecond as-of instant.
    pub as_of_ns: u64,
    /// Publication state name; only `AVAILABLE` is admissible.
    pub publication_state: String,
    /// Positive epoch-nanosecond creation instant.
    pub created_at_utc_ns: u64,
}

/// Source scope declared by the producer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceScope {
    /// Provider identity.
    pub provider_id: String,
    /// Source product identity.
    pub source_product_id: String,
    /// Venue identity; an absent venue is a denial.
    pub venue_id: Option<String>,
    /// Segment identity.
    pub segment_id: String,
    /// Dataset identity.
    pub dataset_id: String,
}

/// Producer operations and evidence links.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProducerActivity {
    /// Operations recorded by the producer.
    pub performed_operations: Vec<PerformedOperation>,
    /// Evidence records for those operations.
    pub operation_evidence: Vec<OperationEvidence>,
}

/// One recorded producer operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PerformedOperation {
    /// Stable reference used to bind evidence to this operation.
    pub operation_ref: String,
    /// Operation action value.
    pub action: String,
    /// Kind of object acted upon.
    pub object_kind: String,
    /// Identity of the object acted upon.
    pub object_id: String,
    /// Version of the object acted upon.
    pub object_version: String,
}

/// Evidence record for one producer operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OperationEvidence {
    /// Identity of the operation this evidence supports.
    pub operation_ref: String,
    /// Event identity in the producer record.
    pub event_id: String,
    /// Producer run identity.
    pub run_id: String,
    /// Positive epoch-nanosecond recording instant.
    pub recorded_at_ns: u64,
    /// Decision key under which the operation ran.
    pub decision_key: String,
    /// Lowercase hex SHA-256 of the evidence object.
    pub digest_sha256: String,
}

/// Written rights evidence carried by a release.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RightsRecord {
    /// Permission grants from the written agreement.
    pub permission_grants: Vec<PermissionGrant>,
    /// References to the written documents.
    pub document_references: Vec<DocumentReference>,
    /// Reviewed attestations.
    pub reviewed_attestations: Vec<ReviewedAttestation>,
}

/// One permission grant in the written agreement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PermissionGrant {
    /// Stable grant identity.
    pub grant_id: String,
    /// Authorized consumer action.
    pub action: String,
    /// Object class covered by the grant.
    pub object_class: String,
    /// `PERMIT` or `DENY`.
    pub decision: String,
    /// Named actor bound to the grant.
    pub actor: String,
    /// Named licensee.
    pub licensee: String,
    /// Exact recipient class.
    pub recipient_class: String,
    /// Named recipient identity.
    pub recipient_identity: String,
    /// Scope dimensions covered by the grant.
    pub scope: ScopeDeclaration,
    /// Positive epoch-nanosecond validity begin.
    pub validity_begin_ns: u64,
    /// Positive epoch-nanosecond validity end.
    pub validity_end_ns: u64,
    /// Delivery statement from the agreement.
    pub delivery: String,
    /// Bounded typed retention policy.
    pub retention: RetentionPolicy,
}

/// A scope declaration shared by grants and admission requests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScopeDeclaration {
    /// Provider dimension.
    pub provider_id: String,
    /// Source product dimension.
    pub source_product_id: String,
    /// Venue dimension.
    pub venue_id: String,
    /// Segment dimension.
    pub segment_id: String,
    /// `NOT_APPLICABLE`, `NAMED` or `ALL`.
    pub subject_state: String,
    /// Named subjects; an empty `NAMED` set is not `ALL`.
    pub named_subjects: Vec<String>,
    /// Inclusive data-time begin in epoch nanoseconds.
    pub data_time_from_ns: Option<u64>,
    /// Inclusive data-time end in epoch nanoseconds.
    pub data_time_to_ns: Option<u64>,
}

/// A bounded retention declaration with an explicit purge rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RetentionPolicy {
    /// Positive maximum retention duration in nanoseconds.
    pub max_duration_ns: u64,
    /// Explicit purge rule text.
    pub purge_rule: String,
}

/// Reference to a written rights document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DocumentReference {
    /// Document reference string.
    pub reference: String,
    /// Clause within the document.
    pub clause: String,
    /// Lowercase hex SHA-256 of the document bytes.
    pub document_sha256: String,
    /// Contract revision reviewed with this document.
    pub contract_revision: String,
    /// Review date as an ISO string.
    pub reviewed_on: String,
    /// Reviewing authority.
    pub authority: String,
}

/// One reviewed rights attestation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewedAttestation {
    /// Exact reviewed purpose string.
    pub purpose: String,
    /// Usage right value.
    pub usage_right: String,
    /// Usage mode value.
    pub usage_mode: String,
    /// Territory value.
    pub territory: String,
    /// Scope assertion strings.
    pub scope_assertions: Vec<String>,
    /// Reviewer identity.
    pub reviewer: String,
    /// Reviewing authority.
    pub authority: String,
}

/// Per-dimension coverage reports.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CoverageRecord {
    /// One report per declared coverage dimension.
    pub dimensions: Vec<CoverageDimension>,
}

/// One coverage dimension report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CoverageDimension {
    /// Stable dimension identity.
    pub dimension_id: String,
    /// Named universe for this dimension.
    pub universe_name: String,
    /// Identity of the denominator.
    pub denominator_identity: String,
    /// Declared denominator; absence fails closed.
    pub denominator: Option<u64>,
    /// Matching numerator.
    pub numerator: u64,
    /// Missing count declared against this dimension.
    pub missing_count: u64,
    /// Rejected count declared against this dimension.
    pub rejected_count: u64,
    /// Visible gap descriptors; gaps are not interpolated.
    pub gaps: Vec<String>,
    /// `PASS` or `FAIL`.
    pub verdict: String,
    /// Positive epoch-nanosecond as-of instant.
    pub as_of_ns: u64,
    /// Evidence reference for this dimension.
    pub evidence_reference: String,
}

/// Raw source and correction lineage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Lineage {
    /// Immutable raw archive reference.
    pub raw_archive_reference: String,
    /// Lowercase hex SHA-256 of the raw archive.
    pub raw_archive_sha256: String,
    /// Transformation record reference.
    pub transformation_record: String,
    /// PROV `wasDerivedFrom` reference.
    pub was_derived_from: String,
    /// PROV `hadPrimarySource` reference.
    pub had_primary_source: String,
    /// Release revision number.
    pub revision: u64,
    /// Positive epoch-nanosecond knowledge time.
    pub knowledge_time_ns: u64,
    /// Positive epoch-nanosecond invalidation time.
    pub invalidated_at_time_ns: u64,
    /// The single immediate predecessor for a corrected release.
    pub supersedes: Option<SupersedesRelation>,
}

/// One immediate predecessor relationship for a corrected release.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SupersedesRelation {
    /// Predecessor release identity.
    pub supersedes_release_id: String,
    /// Predecessor manifest digest.
    pub supersedes_manifest_sha256: String,
    /// Predecessor dataset digest.
    pub supersedes_data_sha256: String,
    /// Predecessor revision number.
    pub revision: u64,
    /// Correction reason text.
    pub correction_reason: String,
    /// Positive epoch-nanosecond knowledge time of the correction.
    pub knowledge_time_ns: u64,
    /// Positive epoch-nanosecond invalidation time of the predecessor.
    pub invalidated_at_time_ns: u64,
}

/// Declared versions for payloads and event envelopes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SchemaVersions {
    /// Contract version repeated for binding.
    pub contract_version: String,
    /// Normalizer version.
    pub normalizer_version: String,
    /// Payload schema version.
    pub payload_schema_version: String,
    /// Event envelope schema version.
    pub envelope_version: String,
}

/// Dataset-level digests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Digests {
    /// SHA-256 over the ordered uncompressed artifact streams.
    pub data_sha256: String,
    /// Digest algorithm identifier.
    pub digest_algorithm: String,
}

/// Publisher identity and signature reference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PublicationIdentity {
    /// Publisher identity from the trusted registry.
    pub publisher_identity: String,
    /// Immutable trusted key identity.
    pub signing_key_id: String,
    /// Detached sidecar file name.
    pub signature_reference: String,
    /// Durable publication archive reference.
    pub archive_reference: String,
}

/// One published artifact descriptor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactDescriptor {
    /// Logical partition identity.
    pub logical_partition_id: String,
    /// Shard ordinal within the logical partition.
    pub shard_ordinal: u32,
    /// Repository-relative POSIX artifact path.
    pub path: String,
    /// Record order profile identifier.
    pub record_order_profile: String,
    /// Content encoding identifier.
    pub encoding: String,
    /// Compression identifier.
    pub compression: String,
    /// Compression format version.
    pub compression_version: u32,
    /// Exact stored byte count.
    pub stored_bytes: u64,
    /// Lowercase hex SHA-256 over stored bytes.
    pub stored_sha256: String,
    /// Exact uncompressed byte count.
    pub uncompressed_bytes: u64,
    /// Lowercase hex SHA-256 over uncompressed bytes.
    pub uncompressed_sha256: String,
    /// Number of canonical JSONL records.
    pub record_count: u64,
}

/// Detached sidecar with exactly five members.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignatureSidecar {
    /// Sidecar schema version.
    pub schema_version: u32,
    /// Signature algorithm identifier.
    pub algorithm: String,
    /// Trusted key identity.
    pub key_id: String,
    /// Lowercase hex SHA-256 over canonical manifest bytes.
    pub manifest_sha256: String,
    /// Standard padded base64 of the 64-byte Ed25519 signature.
    pub signature_b64: String,
}

/// One canonical JSONL record with retained abstract payload members.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseRecord {
    /// Sequence domain identity.
    pub sequence_domain_id: String,
    /// Epoch or session-reset identity within the domain.
    pub sequence_epoch: String,
    /// Sequence number within the domain and epoch.
    pub sequence_number: u64,
    /// Referenced object identity.
    pub object_id: String,
    /// Object revision.
    pub revision: u64,
    /// Positive epoch-nanosecond knowledge time.
    pub knowledge_time_ns: u64,
    /// Assertion identity for reference ordering.
    pub assertion_id: String,
    /// Declared event envelope schema version.
    pub envelope_schema_version: String,
    /// Abstract payload members retained for canonical byte checks.
    #[serde(flatten)]
    pub payload: Map<String, Value>,
}

/// A consumer request submitted independently of the release.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdmissionRequest {
    /// Release identity expected by the consumer.
    pub release_id: String,
    /// Expected manifest digest.
    pub manifest_sha256: String,
    /// Expected dataset digest.
    pub data_sha256: String,
    /// Expected publisher identity.
    pub publisher_identity: String,
    /// Named consumer actor.
    pub actor: String,
    /// Declared versioned use profile.
    pub use_profile: String,
    /// Explicit requested actions.
    pub requested_actions: Vec<String>,
    /// Exact reviewed purpose.
    pub purpose: String,
    /// Named licensee.
    pub licensee: String,
    /// Exact recipient class.
    pub recipient_class: String,
    /// Named recipient identity.
    pub recipient_identity: String,
    /// Requested source and subject scope.
    pub scope: ScopeDeclaration,
    /// Requested bounded retention.
    pub requested_retention: RetentionPolicy,
    /// Positive epoch-nanosecond point-in-time instant.
    pub as_of_ns: u64,
    /// Declared capability claims; these are not permissions.
    pub declared_capabilities: Vec<CapabilityClaim>,
}

/// One capability claim in an admission request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CapabilityClaim {
    /// Capability identity.
    pub capability_id: String,
    /// Object class for the claim.
    pub object_class: String,
}

/// Outcome of an injected rights evaluation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Decision {
    /// At least one unique matching grant permits the request.
    Permit,
    /// A matching grant denies the request.
    Deny,
    /// No grant can be established; the verifier fails closed.
    Unknown,
}

/// Result of an injected rights evaluation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RightsDecision {
    /// Scalar decision for the request.
    pub decision: Decision,
    /// Identities of grants that matched the request.
    pub matching_grant_ids: Vec<String>,
}

/// Evaluates rights independently of signature and digest checks.
pub trait RightsEvaluator {
    /// Return a rights decision for a release and an admission request.
    fn evaluate(&self, manifest: &ReleaseManifest, request: &AdmissionRequest) -> RightsDecision;
}

/// Result of an injected coverage evaluation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoverageDecision {
    /// Dimension identities for which a PASS was established.
    pub approved_dimensions: Vec<String>,
    /// Whether the evaluator relied on an implicit `ALL` universe.
    pub used_implicit_all: bool,
}

/// Evaluates coverage independently of the byte-level digest checks.
pub trait CoverageEvaluator {
    /// Return a coverage decision for a release and an admission request.
    fn evaluate(&self, manifest: &ReleaseManifest, request: &AdmissionRequest) -> CoverageDecision;
}

/// Trusted evidence for a predecessor release used by lineage checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredecessorEvidence {
    /// Predecessor release identity.
    pub release_id: String,
    /// Predecessor manifest digest.
    pub manifest_sha256: String,
    /// Predecessor dataset digest.
    pub data_sha256: String,
    /// Predecessor revision number.
    pub revision: u64,
    /// Predecessor knowledge time in epoch nanoseconds.
    pub knowledge_time_ns: u64,
    /// Further predecessor identity, when the predecessor is a correction.
    pub supersedes_release_id: Option<String>,
}

/// Resolves predecessor evidence from an out-of-band trusted archive.
pub trait LineageResolver {
    /// Return trusted evidence for a predecessor release identity.
    fn predecessor(&self, release_id: &str) -> Option<PredecessorEvidence>;
}
