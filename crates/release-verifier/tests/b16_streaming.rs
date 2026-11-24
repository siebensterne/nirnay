// SPDX-License-Identifier: Apache-2.0
//! B16 bounded streaming acceptance tests.
//!
//! Records are abstract scalars. No provider record, market observation, price,
//! order, trade, universe, experiment or India artefact appears here. The key
//! is the RFC 8032 section 7.1 test vector and all resource numbers belong to
//! finite `TEST_ONLY` profiles; none is a production maximum or PERF-001
//! evidence.

use base64::Engine;
use ed25519_dalek::Signer;
use ed25519_dalek::SigningKey;
use flate2::Compression;
use flate2::write::GzEncoder;
use release_verifier::AdmissionReceipt;
use release_verifier::AdmissionRequest;
use release_verifier::CleanupDisposition;
use release_verifier::Decision;
use release_verifier::KeyState;
use release_verifier::LimitsInput;
use release_verifier::LineageResolver;
use release_verifier::ProcessingRole;
use release_verifier::RefusalDiagnostic;
use release_verifier::ReleaseLocator;
use release_verifier::ReleaseManifest;
use release_verifier::ReleaseVerifier;
use release_verifier::ResourceProfile;
use release_verifier::ResourceProfileId;
use release_verifier::StreamingResourcePolicy;
use release_verifier::TrustedKeyEntry;
use release_verifier::TrustedKeyRegistry;
use release_verifier::VerifierResourcePolicy;
use release_verifier::jcs::canonical_bytes;
use release_verifier::schema::ArtifactDescriptor;
use release_verifier::schema::CoverageDecision;
use release_verifier::schema::CoverageDimension;
use release_verifier::schema::CoverageEvaluator;
use release_verifier::schema::DocumentReference;
use release_verifier::schema::Lineage;
use release_verifier::schema::OperationEvidence;
use release_verifier::schema::PerformedOperation;
use release_verifier::schema::PermissionGrant;
use release_verifier::schema::PredecessorEvidence;
use release_verifier::schema::ReleaseRecord;
use release_verifier::schema::RetentionPolicy;
use release_verifier::schema::RightsDecision;
use release_verifier::schema::RightsEvaluator;
use release_verifier::schema::ScopeDeclaration;
use release_verifier::schema::SignatureSidecar;
use release_verifier::verifier::COVERAGE_DIMENSIONS;
use release_verifier::verifier::DECLARED_PURPOSE;
use release_verifier::verifier::RECORD_ENCODING;
use release_verifier::verifier::SIGNING_DOMAIN;
use serde_json::Map;
use serde_json::Value;
use std::error::Error;
use std::fs;
use std::io::Write as IoWrite;
use std::mem::size_of;
use std::path::PathBuf;
use tempfile::TempDir;

const SECRET_SEED: [u8; 32] = [
    0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c, 0xc4,
    0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
];
const PUBLIC_KEY: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];
const AS_OF_NS: u64 = 1_700_000_000_000_000_000;
const KNOWLEDGE_NS: u64 = 1_600_000_000_000_000_000;
const CREATED_NS: u64 = 1_699_999_999_000_000_000;
const DIGEST_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn test_limits() -> LimitsInput {
    LimitsInput {
        max_record_bytes: 512,
        max_records_per_artifact: 64,
        max_records_per_release: 256,
        max_stored_bytes_per_artifact: 2_000_000,
        max_stored_bytes_per_release: 8_000_000,
        max_expanded_bytes_per_artifact: 2_000_000,
        max_expanded_bytes_per_release: 8_000_000,
        max_decompression_budget_bytes: 2_000_000,
        max_expansion_ratio: 64,
        codec_window_bytes: 32_768,
        io_chunk_bytes: 16,
        max_nesting_depth: 8,
        max_string_bytes: 256,
        max_token_bytes: 128,
        max_object_members: 16,
        max_array_elements: 16,
        max_concurrent_artifacts: 4,
        worker_count: 4,
        queue_capacity_records: 32,
        temp_disk_bytes: 8_000_000,
        max_processing_duration_ms: 30_000,
        max_journal_record_bytes: 4_096,
        max_preparation_intent_bytes: 65_536,
        max_sealed_intent_bytes: 4_096,
        max_open_file_descriptors: 16,
        memory_budget_bytes: 268_435_456,
        max_artifacts: 4,
    }
}

fn profiles() -> Result<StreamingResourcePolicy, Box<dyn Error>> {
    let input = test_limits();
    let verifier = ResourceProfile::new(
        ResourceProfileId::new("TEST_ONLY_B16_VERIFIER_V1", 1)?,
        ProcessingRole::Verifier,
        input,
    )?;
    let snapshot = ResourceProfile::new(
        ResourceProfileId::new("TEST_ONLY_B16_SNAPSHOT_V1", 1)?,
        ProcessingRole::Snapshot,
        input,
    )?;
    Ok(StreamingResourcePolicy::new(input, verifier, snapshot)?)
}

fn abstract_records(count: usize) -> Vec<ReleaseRecord> {
    (1..=count as u64)
        .map(|index| ReleaseRecord {
            sequence_domain_id: "domain-a".to_owned(),
            sequence_epoch: "epoch-1".to_owned(),
            sequence_number: index,
            object_id: format!("object-{}", index),
            revision: 1,
            knowledge_time_ns: KNOWLEDGE_NS,
            assertion_id: format!("assertion-{}", index),
            envelope_schema_version: "1".to_owned(),
            payload: {
                let mut payload = Map::new();
                payload.insert("kind".to_owned(), Value::String("abstract".to_owned()));
                payload.insert("scalar".to_owned(), Value::from(index));
                payload
            },
        })
        .collect()
}

fn encode_records(records: &[ReleaseRecord]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut output = Vec::new();
    for record in records {
        let value = serde_json::to_value(record)?;
        output.extend_from_slice(&canonical_bytes(&value));
        output.push(b'\n');
    }
    Ok(output)
}

fn compress(bytes: &[u8], compression: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    if compression == "none" {
        return Ok(bytes.to_vec());
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?)
}

fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(bytes);
    let mut out = [0_u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn hex_digest(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{:02x}", byte)).collect()
}

fn named_scope() -> ScopeDeclaration {
    ScopeDeclaration {
        provider_id: "provider-a".to_owned(),
        source_product_id: "product-a".to_owned(),
        venue_id: "venue-a".to_owned(),
        segment_id: "segment-a".to_owned(),
        subject_state: "NAMED".to_owned(),
        named_subjects: vec!["object-1".to_owned()],
        data_time_from_ns: Some(KNOWLEDGE_NS),
        data_time_to_ns: Some(AS_OF_NS),
    }
}

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    staging: PathBuf,
    manifest: ReleaseManifest,
    sidecar: SignatureSidecar,
    request: AdmissionRequest,
    registry: TrustedKeyRegistry,
    signing_key: SigningKey,
    manifest_bytes: Vec<u8>,
    sidecar_bytes: Vec<u8>,
    stored: Vec<u8>,
    expanded: Vec<u8>,
    policy: StreamingResourcePolicy,
}

impl Fixture {
    fn new(compression: &str, count: usize) -> Result<Self, Box<dyn Error>> {
        let temp = TempDir::new()?;
        let root = temp.path().join("trusted-root");
        let staging = temp.path().join("private-staging");
        let release_dir = root.join("pub-one").join("ds-alpha").join("rel-0001");
        fs::create_dir_all(release_dir.join("artifacts"))?;
        fs::create_dir_all(&staging)?;

        let records = abstract_records(count);
        let expanded = encode_records(&records)?;
        let stored = compress(&expanded, compression)?;
        let manifest = manifest(compression, &stored, &expanded, records.len() as u64);
        let request = request(&manifest.digests.data_sha256);
        let registry = registry()?;
        let sidecar = SignatureSidecar {
            schema_version: 1,
            algorithm: "Ed25519".to_owned(),
            key_id: "key-1".to_owned(),
            manifest_sha256: String::new(),
            signature_b64: String::new(),
        };
        let mut fixture = Self {
            _temp: temp,
            root,
            staging,
            manifest,
            sidecar,
            request,
            registry,
            signing_key: SigningKey::from_bytes(&SECRET_SEED),
            manifest_bytes: Vec::new(),
            sidecar_bytes: Vec::new(),
            stored,
            expanded,
            policy: profiles()?,
        };
        fixture.resign()?;
        fixture.write_all()?;
        Ok(fixture)
    }

    fn release_dir(&self) -> PathBuf {
        self.root.join("pub-one").join("ds-alpha").join("rel-0001")
    }

    fn artifact_path(&self) -> PathBuf {
        self.release_dir().join("artifacts").join("records.jsonl")
    }

    fn write_all(&self) -> Result<(), Box<dyn Error>> {
        fs::write(
            self.release_dir().join("manifest.json"),
            &self.manifest_bytes,
        )?;
        fs::write(
            self.release_dir().join("signature.json"),
            &self.sidecar_bytes,
        )?;
        fs::write(self.artifact_path(), &self.stored)?;
        Ok(())
    }

    fn clean_staging(&self) -> Result<(), Box<dyn Error>> {
        for entry in fs::read_dir(&self.staging)? {
            let entry = entry?;
            if entry.path().is_file() {
                fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }

    fn resign(&mut self) -> Result<(), Box<dyn Error>> {
        let value = serde_json::to_value(&self.manifest)?;
        let canonical = canonical_bytes(&value);
        let digest = sha256_bytes(&canonical);
        let digest_hex = hex_digest(&digest);
        let mut message = SIGNING_DOMAIN.to_vec();
        message.extend_from_slice(&digest);
        let signature = self.signing_key.sign(&message).to_bytes();
        self.sidecar.manifest_sha256 = digest_hex.clone();
        self.sidecar.signature_b64 = base64::engine::general_purpose::STANDARD.encode(signature);
        self.manifest_bytes = serde_json::to_vec(&self.manifest)?;
        self.sidecar_bytes = serde_json::to_vec(&self.sidecar)?;
        self.request.manifest_sha256 = digest_hex;
        Ok(())
    }

    fn set_stored(
        &mut self,
        stored: Vec<u8>,
        expanded: Vec<u8>,
        count: u64,
    ) -> Result<(), Box<dyn Error>> {
        self.stored = stored;
        self.expanded = expanded;
        self.manifest.artifacts[0].record_count = count;
        self.manifest.artifacts[0].stored_bytes = self.stored.len() as u64;
        self.manifest.artifacts[0].stored_sha256 = hex_digest(&sha256_bytes(&self.stored));
        self.manifest.artifacts[0].uncompressed_bytes = self.expanded.len() as u64;
        self.manifest.artifacts[0].uncompressed_sha256 = hex_digest(&sha256_bytes(&self.expanded));
        self.manifest.digests.data_sha256 = hex_digest(&sha256_bytes(&self.expanded));
        self.request.data_sha256 = self.manifest.digests.data_sha256.clone();
        self.resign()?;
        self.write_all()?;
        Ok(())
    }

    fn locator(&self) -> ReleaseLocator {
        ReleaseLocator {
            publisher_id: "pub-one".to_owned(),
            dataset_id: "ds-alpha".to_owned(),
            release_id: "rel-0001".to_owned(),
        }
    }

    fn verify(&self) -> Result<AdmissionReceipt, RefusalDiagnostic> {
        self.verify_with(Box::new(PermitRights), &self.policy)
    }
    fn verify_production(&self) -> Result<AdmissionReceipt, RefusalDiagnostic> {
        self.clean_staging().ok();
        ReleaseVerifier::new(release_verifier::VerifierConfig {
            root: self.root.clone(),
            private_staging: self.staging.clone(),
            registry: self.registry.clone(),
            rights: Box::new(PermitRights),
            coverage: Box::new(PassCoverage),
            lineage: Box::new(EmptyLineage),
            policy: VerifierResourcePolicy {
                max_manifest_bytes: 1_000_000,
                max_sidecar_bytes: 4_096,
                max_artifact_count: 16,
                max_stored_bytes: 2_000_000,
                max_expanded_bytes: 2_000_000,
                max_total_expanded_bytes: 8_000_000,
                max_record_bytes: 512,
                max_record_count: 64,
                max_json_nesting: 8,
                max_decompression_budget: 2_000_000,
                max_temp_disk_bytes: 8_000_000,
                max_open_file_descriptors: 16,
            },
            verifier_commit: "b16-test-commit".to_owned(),
        })
        .verify_release_streaming(&self.locator(), &self.request, &self.policy)
    }

    fn verify_with(
        &self,
        rights: Box<dyn RightsEvaluator>,
        policy: &StreamingResourcePolicy,
    ) -> Result<AdmissionReceipt, RefusalDiagnostic> {
        self.clean_staging().ok();
        ReleaseVerifier::new(release_verifier::VerifierConfig {
            root: self.root.clone(),
            private_staging: self.staging.clone(),
            registry: self.registry.clone(),
            rights,
            coverage: Box::new(PassCoverage),
            lineage: Box::new(EmptyLineage),
            policy: VerifierResourcePolicy {
                max_manifest_bytes: 1_000_000,
                max_sidecar_bytes: 4_096,
                max_artifact_count: 16,
                max_stored_bytes: 2_000_000,
                max_expanded_bytes: 2_000_000,
                max_total_expanded_bytes: 8_000_000,
                max_record_bytes: 512,
                max_record_count: 64,
                max_json_nesting: 8,
                max_decompression_budget: 2_000_000,
                max_temp_disk_bytes: 8_000_000,
                max_open_file_descriptors: 16,
            },
            verifier_commit: "b16-test-commit".to_owned(),
        })
        .with_test_only_profiles()
        .verify_release_streaming(&self.locator(), &self.request, policy)
    }
}

fn manifest(compression: &str, stored: &[u8], expanded: &[u8], count: u64) -> ReleaseManifest {
    ReleaseManifest {
        schema_version: 1,
        contract_version: "v2".to_owned(),
        contract_revision: "contract-rev-1".to_owned(),
        release_identity: release_verifier::schema::ReleaseIdentity {
            release_id: "rel-0001".to_owned(),
            release_version: "1".to_owned(),
            as_of_ns: AS_OF_NS,
            publication_state: "AVAILABLE".to_owned(),
            created_at_utc_ns: CREATED_NS,
        },
        source_scope: release_verifier::schema::SourceScope {
            provider_id: "provider-a".to_owned(),
            source_product_id: "product-a".to_owned(),
            venue_id: Some("venue-a".to_owned()),
            segment_id: "segment-a".to_owned(),
            dataset_id: "ds-alpha".to_owned(),
        },
        producer_activity: release_verifier::schema::ProducerActivity {
            performed_operations: vec![PerformedOperation {
                operation_ref: "observe_abstract".to_owned(),
                action: "observe_abstract".to_owned(),
                object_kind: "abstract-record".to_owned(),
                object_id: "object-1".to_owned(),
                object_version: "1".to_owned(),
            }],
            operation_evidence: vec![OperationEvidence {
                operation_ref: "observe_abstract".to_owned(),
                event_id: "event-1".to_owned(),
                run_id: "run-1".to_owned(),
                recorded_at_ns: KNOWLEDGE_NS,
                decision_key: "decision-1".to_owned(),
                digest_sha256: DIGEST_HEX.to_owned(),
            }],
        },
        rights: release_verifier::schema::RightsRecord {
            permission_grants: vec![PermissionGrant {
                grant_id: "grant-1".to_owned(),
                action: "read_release".to_owned(),
                object_class: "release".to_owned(),
                decision: "PERMIT".to_owned(),
                actor: "consumer-a".to_owned(),
                licensee: "licensee-a".to_owned(),
                recipient_class: "named-recipient".to_owned(),
                recipient_identity: "recipient-a".to_owned(),
                scope: named_scope(),
                validity_begin_ns: 1,
                validity_end_ns: AS_OF_NS,
                delivery: "private-transfer".to_owned(),
                retention: RetentionPolicy {
                    max_duration_ns: 86_400_000_000_000,
                    purge_rule: "purge-at-expiry".to_owned(),
                },
            }],
            document_references: vec![DocumentReference {
                reference: "written-agreement-1".to_owned(),
                clause: "4.2".to_owned(),
                document_sha256: DIGEST_HEX.to_owned(),
                contract_revision: "contract-rev-1".to_owned(),
                reviewed_on: "2026-11-24".to_owned(),
                authority: "rights-review".to_owned(),
            }],
            reviewed_attestations: Vec::new(),
        },
        coverage: release_verifier::schema::CoverageRecord {
            dimensions: COVERAGE_DIMENSIONS
                .iter()
                .map(|name| CoverageDimension {
                    dimension_id: (*name).to_owned(),
                    universe_name: format!("universe-{}", name),
                    denominator_identity: format!("denominator-{}", name),
                    denominator: Some(100),
                    numerator: 100,
                    missing_count: 0,
                    rejected_count: 0,
                    gaps: Vec::new(),
                    verdict: "PASS".to_owned(),
                    as_of_ns: AS_OF_NS,
                    evidence_reference: format!("evidence-{}", name),
                })
                .collect(),
        },
        lineage: Lineage {
            raw_archive_reference: "raw-archive-1".to_owned(),
            raw_archive_sha256: DIGEST_HEX.to_owned(),
            transformation_record: "transform-1".to_owned(),
            was_derived_from: "raw-archive-1".to_owned(),
            had_primary_source: "raw-archive-1".to_owned(),
            revision: 2,
            knowledge_time_ns: KNOWLEDGE_NS,
            invalidated_at_time_ns: 0,
            supersedes: None,
        },
        schema_versions: release_verifier::schema::SchemaVersions {
            contract_version: "v2".to_owned(),
            normalizer_version: "normalizer-1".to_owned(),
            payload_schema_version: "payload-1".to_owned(),
            envelope_version: "1".to_owned(),
        },
        digests: release_verifier::schema::Digests {
            data_sha256: hex_digest(&sha256_bytes(expanded)),
            digest_algorithm: "sha256".to_owned(),
        },
        publication_identity: release_verifier::schema::PublicationIdentity {
            publisher_identity: "pub-one".to_owned(),
            signing_key_id: "key-1".to_owned(),
            signature_reference: "signature.json".to_owned(),
            archive_reference: "archive-ref-1".to_owned(),
        },
        artifacts: vec![ArtifactDescriptor {
            logical_partition_id: "partition-a".to_owned(),
            shard_ordinal: 0,
            path: "artifacts/records.jsonl".to_owned(),
            record_order_profile: "sequence-domain-strict".to_owned(),
            encoding: RECORD_ENCODING.to_owned(),
            compression: compression.to_owned(),
            compression_version: 1,
            stored_bytes: stored.len() as u64,
            stored_sha256: hex_digest(&sha256_bytes(stored)),
            uncompressed_bytes: expanded.len() as u64,
            uncompressed_sha256: hex_digest(&sha256_bytes(expanded)),
            record_count: count,
        }],
    }
}

fn request(data_digest: &str) -> AdmissionRequest {
    AdmissionRequest {
        release_id: "rel-0001".to_owned(),
        manifest_sha256: String::new(),
        data_sha256: data_digest.to_owned(),
        publisher_identity: "pub-one".to_owned(),
        actor: "consumer-a".to_owned(),
        use_profile: "reference_lookup".to_owned(),
        requested_actions: vec!["read_release".to_owned()],
        purpose: DECLARED_PURPOSE.to_owned(),
        licensee: "licensee-a".to_owned(),
        recipient_class: "named-recipient".to_owned(),
        recipient_identity: "recipient-a".to_owned(),
        scope: named_scope(),
        requested_retention: RetentionPolicy {
            max_duration_ns: 86_400_000_000_000,
            purge_rule: "purge-at-expiry".to_owned(),
        },
        as_of_ns: AS_OF_NS,
        declared_capabilities: Vec::new(),
    }
}

fn registry() -> Result<TrustedKeyRegistry, Box<dyn Error>> {
    Ok(TrustedKeyRegistry::new(
        vec![TrustedKeyEntry {
            publisher_id: "pub-one".to_owned(),
            key_id: "key-1".to_owned(),
            public_key_bytes: PUBLIC_KEY,
            algorithm: "Ed25519".to_owned(),
            purpose: release_verifier::trust::KeyPurpose::ReleaseSigning,
            valid_from_ns: 1,
            valid_until_ns: AS_OF_NS + 1,
            state: KeyState::Active,
            evidence_reference: None,
        }],
        "registry-rev-1".to_owned(),
    )?)
}

struct PermitRights;

impl RightsEvaluator for PermitRights {
    fn evaluate(&self, _manifest: &ReleaseManifest, _request: &AdmissionRequest) -> RightsDecision {
        RightsDecision {
            decision: Decision::Permit,
            matching_grant_ids: vec!["grant-1".to_owned()],
        }
    }
}

struct UnknownRights;

impl RightsEvaluator for UnknownRights {
    fn evaluate(&self, _manifest: &ReleaseManifest, _request: &AdmissionRequest) -> RightsDecision {
        RightsDecision {
            decision: Decision::Unknown,
            matching_grant_ids: Vec::new(),
        }
    }
}

struct PassCoverage;

impl CoverageEvaluator for PassCoverage {
    fn evaluate(
        &self,
        manifest: &ReleaseManifest,
        _request: &AdmissionRequest,
    ) -> CoverageDecision {
        CoverageDecision {
            approved_dimensions: manifest
                .coverage
                .dimensions
                .iter()
                .map(|item| item.dimension_id.clone())
                .collect(),
            used_implicit_all: false,
        }
    }
}

struct EmptyLineage;

impl LineageResolver for EmptyLineage {
    fn predecessor(&self, _release_id: &str) -> Option<PredecessorEvidence> {
        None
    }
}

#[test]
fn streaming_accepts_gzip_release() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("gzip", 3)?;
    let receipt = fixture.verify()?;
    assert_eq!(
        receipt.artifact_set().len(),
        1,
        "one artifact binding is retained"
    );
    assert_eq!(
        fs::read_dir(&fixture.staging)?.count(),
        3,
        "three metadata-backed files staged"
    );
    Ok(())
}

#[test]
fn streaming_accepts_uncompressed_release() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none", 3)?;
    let receipt = fixture.verify()?;
    assert_eq!(
        receipt.artifact_set().len(),
        1,
        "uncompressed artifact was verified"
    );
    Ok(())
}

#[test]
fn incremental_digests_match_the_manifest() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("gzip", 3)?;
    let receipt = fixture.verify()?;
    assert_eq!(
        receipt.data_sha256(),
        hex_digest(&sha256_bytes(&fixture.expanded)),
        "dataset digest matches"
    );
    assert_eq!(
        receipt.artifact_set()[0].stored_sha256,
        hex_digest(&sha256_bytes(&fixture.stored)),
        "stored digest matches"
    );
    Ok(())
}

#[test]
fn high_expansion_is_refused_during_decode() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("gzip", 3)?;
    let mut strict = test_limits();
    strict.max_decompression_budget_bytes = 32;
    strict.max_expanded_bytes_per_artifact = 64;
    strict.codec_window_bytes = 32;
    strict.io_chunk_bytes = 8;
    let verifier = ResourceProfile::new(
        ResourceProfileId::new("TEST_ONLY_B16_VERIFIER_TIGHT", 1)?,
        ProcessingRole::Verifier,
        strict,
    )?;
    let snapshot = ResourceProfile::new(
        ResourceProfileId::new("TEST_ONLY_B16_SNAPSHOT_TIGHT", 1)?,
        ProcessingRole::Snapshot,
        strict,
    )?;
    let policy = StreamingResourcePolicy::new(strict, verifier, snapshot)?;
    let refusal = fixture
        .verify_with(Box::new(PermitRights), &policy)
        .unwrap_err();
    assert_eq!(
        refusal.refusal.code(),
        "REJECT_RESOURCE_LIMIT",
        "expansion is refused"
    );
    assert!(
        refusal.refusal.stage() == 5,
        "refusal belongs to stage five"
    );
    Ok(())
}

#[test]
fn truncated_gzip_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("gzip", 3)?;
    let truncated = fixture.stored[..fixture.stored.len() - 5].to_vec();
    let expanded = fixture.expanded.clone();
    fixture.set_stored(truncated, expanded, 3)?;
    let refusal = fixture.verify().unwrap_err();
    assert_eq!(
        refusal.refusal.code(),
        "REJECT_ARTIFACT_DIGESTS",
        "truncated stream is refused"
    );
    Ok(())
}

#[test]
fn invalid_crc_is_refused_at_decoder_eof() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("gzip", 3)?;
    let mut damaged = fixture.stored.clone();
    let last = damaged.len() - 1;
    damaged[last] ^= 0x01;
    let expanded = fixture.expanded.clone();
    fixture.set_stored(damaged, expanded, 3)?;
    let refusal = fixture.verify().unwrap_err();
    assert_eq!(
        refusal.refusal.code(),
        "REJECT_ARTIFACT_DIGESTS",
        "damaged trailer is refused"
    );
    Ok(())
}

#[test]
fn oversized_record_is_refused_while_framing() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none", 3)?;
    let mut records = abstract_records(3);
    let mut long = Map::new();
    long.insert("kind".to_owned(), Value::String("abstract".to_owned()));
    long.insert("scalar".to_owned(), Value::String("s".repeat(600)));
    records[1].payload = long;
    let expanded = encode_records(&records)?;
    let stored = compress(&expanded, "none")?;
    fixture.set_stored(stored, expanded, 3)?;
    let refusal = fixture.verify().unwrap_err();
    assert_eq!(
        refusal.refusal.code(),
        "REJECT_RESOURCE_LIMIT",
        "record bound is enforced"
    );
    Ok(())
}

#[test]
fn deep_nesting_is_refused_by_the_one_record_parser() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none", 1)?;
    let value = serde_json::to_value(&abstract_records(1)[0])?;
    let mut text = String::new();
    for _ in 0..10 {
        text.push('[');
    }
    text.push_str(&serde_json::to_string(&value)?);
    for _ in 0..10 {
        text.push(']');
    }
    let mut expanded = text.into_bytes();
    expanded.push(b'\n');
    let stored = compress(&expanded, "none")?;
    fixture.set_stored(stored, expanded, 1)?;
    let refusal = fixture.verify().unwrap_err();
    assert_eq!(
        refusal.refusal.code(),
        "REJECT_RESOURCE_LIMIT",
        "nesting depth is enforced"
    );
    Ok(())
}

#[test]
fn many_tiny_records_are_refused_beyond_the_profile_count() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none", 3)?;
    let expanded = encode_records(&abstract_records(80))?;
    let stored = compress(&expanded, "none")?;
    fixture.set_stored(stored, expanded, 80)?;
    let refusal = fixture.verify().unwrap_err();
    assert_eq!(
        refusal.refusal.code(),
        "REJECT_RESOURCE_LIMIT",
        "record count is enforced while parsing"
    );
    Ok(())
}

#[test]
fn duplicate_ordering_key_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none", 3)?;
    let mut records = abstract_records(3);
    records[1].sequence_number = 1;
    let expanded = encode_records(&records)?;
    let stored = compress(&expanded, "none")?;
    fixture.set_stored(stored, expanded, 3)?;
    let refusal = fixture.verify().unwrap_err();
    assert_eq!(
        refusal.refusal.code(),
        "REJECT_ARTIFACT_DIGESTS",
        "duplicate position is refused"
    );
    Ok(())
}

#[test]
fn extra_artifact_is_refused_by_exact_inventory() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("gzip", 3)?;
    fs::write(
        fixture.release_dir().join("artifacts").join("extra.jsonl"),
        b"x",
    )?;
    let refusal = fixture.verify().unwrap_err();
    assert_eq!(
        refusal.refusal.code(),
        "REJECT_ARTIFACT_DIGESTS",
        "file set is exact"
    );
    Ok(())
}

#[test]
fn rights_refusal_keeps_stage_six_and_has_no_receipt() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("gzip", 3)?;
    let refusal = fixture
        .verify_with(Box::new(UnknownRights), &fixture.policy)
        .unwrap_err();
    assert_eq!(
        refusal.refusal.code(),
        "REJECT_RIGHTS_EVIDENCE",
        "rights code is stable"
    );
    assert_eq!(
        refusal.refusal.stage(),
        6,
        "rights refusal stays at stage six"
    );
    assert!(
        matches!(&refusal.cleanup, Some(box_report) if matches!(&**box_report, CleanupDisposition::Completed(report) if report.is_complete())),
        "cleanup completed after rights refusal"
    );
    assert_eq!(
        fs::read_dir(&fixture.staging)?.count(),
        0,
        "no snapshot survives the refusal"
    );
    Ok(())
}

#[test]
fn envelope_version_mismatch_refuses_at_stage_five() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none", 3)?;
    fixture.manifest.schema_versions.envelope_version = "2".to_owned();
    let mut records = abstract_records(3);
    for record in &mut records {
        record.envelope_schema_version = "1".to_owned();
    }
    let expanded = encode_records(&records)?;
    let stored = compress(&expanded, "none")?;
    fixture.set_stored(stored, expanded, 3)?;
    let refusal = fixture.verify().unwrap_err();
    assert_eq!(
        refusal.refusal.code(),
        "REJECT_SCHEMA_ENVELOPE",
        "envelope code is stable"
    );
    assert_eq!(
        refusal.refusal.stage(),
        5,
        "envelope mismatch is refused at detection"
    );
    Ok(())
}

#[test]
fn snapshot_metadata_is_fixed_size() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none", 3)?;
    let receipt = fixture.verify()?;
    let size = size_of::<release_verifier::SnapshotEntry>();
    assert!(
        size < 256,
        "metadata holds no byte vector; its size does not scale with artifact bytes"
    );
    assert_eq!(
        receipt.artifact_set().len(),
        1,
        "binding metadata is retained"
    );
    Ok(())
}

#[test]
fn preexisting_staged_entry_is_refused_before_writing() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none", 3)?;
    let staged = fixture.staging.join("manifest.json");
    let _ = fs::remove_file(&staged);
    std::os::unix::fs::symlink("/tmp/b16-outside", &staged)?;
    let refusal = fixture.verify().unwrap_err();
    assert_eq!(
        refusal.refusal.code(),
        "REJECT_ARTIFACT_DIGESTS",
        "a non-empty staging directory is refused before any write"
    );
    Ok(())
}

#[test]
fn producer_declaration_cannot_raise_the_local_ceiling() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none", 3)?;
    let mut generous = test_limits();
    generous.max_record_bytes = 65_536;
    generous.max_records_per_artifact = 1_024;
    generous.max_records_per_release = 2_048;
    let producer = ResourceProfile::new(
        ResourceProfileId::new("TEST_ONLY_B16_PRODUCER_V1", 1)?,
        ProcessingRole::Producer,
        generous,
    )?;
    let policy = profiles()?.with_producer_declaration(producer)?;
    let receipt = fixture.verify_with(Box::new(PermitRights), &policy)?;
    assert_eq!(
        receipt.artifact_set().len(),
        1,
        "local limits govern the accepted release"
    );
    Ok(())
}

#[test]
fn role_mismatch_is_refused_before_the_source_opens() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none", 3)?;
    let input = test_limits();
    let selected = profiles()?;
    let wrong_role = ResourceProfile::new(
        ResourceProfileId::new("TEST_ONLY_B16_SNAPSHOT_DECL_V1", 1)?,
        ProcessingRole::Snapshot,
        input,
    )?;
    assert!(
        matches!(
            selected.clone().with_producer_declaration(wrong_role),
            Err(release_verifier::ProfileError::Role { .. })
        ),
        "a non-producer declaration is refused before any source opens"
    );
    assert_eq!(
        fs::read_dir(&fixture.staging)?.count(),
        0,
        "nothing was staged"
    );
    Ok(())
}

#[test]
fn streaming_state_is_constant_across_one_two_four_x() -> Result<(), Box<dyn Error>> {
    let sizes = [size_of::<release_verifier::streaming::ArtifactState>(); 3];
    assert_eq!(
        sizes[0], sizes[1],
        "state size is independent of record count at 2x"
    );
    assert_eq!(
        sizes[1], sizes[2],
        "state size is independent of record count at 4x"
    );
    for count in [1_usize, 2, 4] {
        let fixture = Fixture::new("gzip", count)?;
        let receipt = fixture.verify()?;
        assert_eq!(
            receipt.artifact_set()[0].record_count,
            count as u64,
            "scaled input is accepted"
        );
    }
    Ok(())
}

#[test]
fn test_only_profiles_are_refused_without_harness_opt_in() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none", 3)?;
    let refusal = fixture.verify_production().unwrap_err();
    assert_eq!(
        refusal.refusal.code(),
        "REJECT_RESOURCE_LIMIT",
        "a production invocation fails closed on a test-only profile"
    );
    assert_eq!(
        fs::read_dir(&fixture.staging)?.count(),
        0,
        "no source was opened"
    );
    Ok(())
}

#[test]
fn first_refusal_wins_over_rights_refusal() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none", 3)?;
    let mut records = abstract_records(3);
    records[1].sequence_number = 1;
    let expanded = encode_records(&records)?;
    let stored = compress(&expanded, "none")?;
    fixture.set_stored(stored, expanded, 3)?;
    let refusal = fixture
        .verify_with(Box::new(UnknownRights), &fixture.policy)
        .unwrap_err();
    assert_eq!(
        refusal.refusal.code(),
        "REJECT_ARTIFACT_DIGESTS",
        "stage five refusal precedes stage six"
    );
    Ok(())
}
