// SPDX-License-Identifier: Apache-2.0
//! Abstract integration tests for the independent NDC release verifier.
//!
//! No market observation, price, order, trade or synthetic financial object
//! appears here. Records are abstract scalars. The signing key is the Ed25519
//! test vector from RFC 8032 section 7.1, not production key material.

use base64::Engine;
use ed25519_dalek::Signer;
use ed25519_dalek::SigningKey;
use flate2::Compression;
use flate2::write::GzEncoder;
use release_verifier::AdmissionReceipt;
use release_verifier::AdmissionRequest;
use release_verifier::Decision;
use release_verifier::KeyState;
use release_verifier::LineageResolver;
use release_verifier::ReleaseLocator;
use release_verifier::ReleaseManifest;
use release_verifier::ReleaseVerifier;
use release_verifier::TrustedKeyEntry;
use release_verifier::TrustedKeyRegistry;
use release_verifier::VerifierError;
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
use release_verifier::schema::SupersedesRelation;
use release_verifier::verifier::COVERAGE_DIMENSIONS;
use release_verifier::verifier::DECLARED_PURPOSE;
use release_verifier::verifier::RECORD_ENCODING;
use release_verifier::verifier::SIGNING_DOMAIN;
use release_verifier::verifier::VERIFICATION_POLICY_VERSION;
use serde_json::Map;
use serde_json::Value;
use std::error::Error;
use std::fs;
use std::io::Write as IoWrite;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::process::Command;
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
const OTHER_DIGEST_HEX: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

struct Fixture {
    temp: TempDir,
    root: PathBuf,
    staging: PathBuf,
    manifest: ReleaseManifest,
    sidecar: SignatureSidecar,
    request: AdmissionRequest,
    registry: TrustedKeyRegistry,
    policy: VerifierResourcePolicy,
    signing_key: SigningKey,
    manifest_bytes: Vec<u8>,
    sidecar_bytes: Vec<u8>,
    stored: Vec<u8>,
    expanded: Vec<u8>,
}

impl Fixture {
    fn new(compression: &str) -> Result<Self, Box<dyn Error>> {
        let temp = TempDir::new()?;
        let root = temp.path().join("trusted-root");
        let staging = temp.path().join("private-staging");
        let release_dir = root.join("pub-one").join("ds-alpha").join("rel-0001");
        fs::create_dir_all(release_dir.join("artifacts"))?;
        fs::create_dir_all(&staging)?;

        let mut records = Vec::new();
        for index in 1..=3_u64 {
            let mut payload = Map::new();
            payload.insert("kind".to_owned(), Value::String("abstract".to_owned()));
            payload.insert("scalar".to_owned(), Value::from(index));
            records.push(ReleaseRecord {
                sequence_domain_id: "domain-a".to_owned(),
                sequence_epoch: "epoch-1".to_owned(),
                sequence_number: index,
                object_id: format!("object-{}", index),
                revision: 1,
                knowledge_time_ns: KNOWLEDGE_NS,
                assertion_id: format!("assertion-{}", index),
                envelope_schema_version: "1".to_owned(),
                payload,
            });
        }
        let expanded = encode_records(&records)?;
        let stored = compress(&expanded, compression)?;
        let expanded_digest = hex_digest(&sha256_bytes(&expanded));
        let stored_digest = hex_digest(&sha256_bytes(&stored));

        let manifest = ReleaseManifest {
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
                reviewed_attestations: vec![release_verifier::schema::ReviewedAttestation {
                    purpose: DECLARED_PURPOSE.to_owned(),
                    usage_right: "non-display-research".to_owned(),
                    usage_mode: "internal".to_owned(),
                    territory: "named-territory".to_owned(),
                    scope_assertions: vec!["abstract-object-1".to_owned()],
                    reviewer: "reviewer-a".to_owned(),
                    authority: "rights-review".to_owned(),
                }],
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
                data_sha256: String::new(),
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
                stored_sha256: stored_digest,
                uncompressed_bytes: expanded.len() as u64,
                uncompressed_sha256: expanded_digest,
                record_count: records.len() as u64,
            }],
        };

        let request = AdmissionRequest {
            release_id: "rel-0001".to_owned(),
            manifest_sha256: String::new(),
            data_sha256: String::new(),
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
        };

        let registry = TrustedKeyRegistry::new(
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
        )?;
        let policy = VerifierResourcePolicy {
            max_manifest_bytes: 1_000_000,
            max_sidecar_bytes: 4_096,
            max_artifact_count: 16,
            max_stored_bytes: 1_000_000,
            max_expanded_bytes: 1_000_000,
            max_total_expanded_bytes: 1_000_000,
            max_record_bytes: 4_096,
            max_record_count: 1_000,
            max_json_nesting: 16,
            max_decompression_budget: 1_000_000,
            max_temp_disk_bytes: 1_000_000,
            max_open_file_descriptors: 64,
        };
        let signing_key = SigningKey::from_bytes(&SECRET_SEED);
        let sidecar = SignatureSidecar {
            schema_version: 1,
            algorithm: "Ed25519".to_owned(),
            key_id: "key-1".to_owned(),
            manifest_sha256: String::new(),
            signature_b64: String::new(),
        };
        let mut fixture = Self {
            temp,
            root,
            staging,
            manifest,
            sidecar,
            request,
            registry,
            policy,
            signing_key,
            manifest_bytes: Vec::new(),
            sidecar_bytes: Vec::new(),
            stored,
            expanded,
        };
        fixture.rebuild_digests()?;
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

    fn rebuild_digests(&mut self) -> Result<(), Box<dyn Error>> {
        let data_digest = hex_digest(&sha256_bytes(&self.expanded));
        self.manifest.digests.data_sha256 = data_digest.clone();
        let descriptor = self
            .manifest
            .artifacts
            .first_mut()
            .ok_or_else(|| std::io::Error::other("no artifact"))?;
        descriptor.stored_bytes = self.stored.len() as u64;
        descriptor.stored_sha256 = hex_digest(&sha256_bytes(&self.stored));
        descriptor.uncompressed_bytes = self.expanded.len() as u64;
        descriptor.uncompressed_sha256 = hex_digest(&sha256_bytes(&self.expanded));
        self.request.data_sha256 = data_digest;
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

    fn set_records(&mut self, records: Vec<ReleaseRecord>) -> Result<(), Box<dyn Error>> {
        let compression = self.manifest.artifacts[0].compression.clone();
        self.expanded = encode_records(&records)?;
        self.stored = compress(&self.expanded, &compression)?;
        self.manifest.artifacts[0].record_count = records.len() as u64;
        self.rebuild_digests()?;
        self.resign()?;
        self.write_all()?;
        Ok(())
    }

    fn registry_entry(&self) -> &TrustedKeyEntry {
        &self.registry.entries()[0]
    }

    fn set_registry(&mut self, entries: Vec<TrustedKeyEntry>) -> Result<(), Box<dyn Error>> {
        self.registry = TrustedKeyRegistry::new(entries, "registry-rev-1".to_owned())?;
        Ok(())
    }

    fn locator(&self) -> ReleaseLocator {
        ReleaseLocator {
            publisher_id: "pub-one".to_owned(),
            dataset_id: "ds-alpha".to_owned(),
            release_id: "rel-0001".to_owned(),
        }
    }

    fn verify(&self) -> Result<AdmissionReceipt, VerifierError> {
        self.verify_with(
            Box::new(PermitRights),
            Box::new(PassCoverage),
            Box::new(EmptyLineage),
        )
    }

    fn verify_with(
        &self,
        rights: Box<dyn RightsEvaluator>,
        coverage: Box<dyn CoverageEvaluator>,
        lineage: Box<dyn LineageResolver>,
    ) -> Result<AdmissionReceipt, VerifierError> {
        self.clean_staging().ok();
        let verifier = ReleaseVerifier::new(release_verifier::VerifierConfig {
            root: self.root.clone(),
            private_staging: self.staging.clone(),
            registry: self.registry.clone(),
            rights,
            coverage,
            lineage,
            policy: self.policy,
            verifier_commit: "test-verifier-commit".to_owned(),
        });
        verifier.verify_release(&self.locator(), &self.request)
    }
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

struct ImplicitAllCoverage;

impl CoverageEvaluator for ImplicitAllCoverage {
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
            used_implicit_all: true,
        }
    }
}

struct MutatingCoverage {
    artifact_path: PathBuf,
}

impl CoverageEvaluator for MutatingCoverage {
    fn evaluate(
        &self,
        manifest: &ReleaseManifest,
        _request: &AdmissionRequest,
    ) -> CoverageDecision {
        let _ = fs::write(&self.artifact_path, b"mutated-after-seal");
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

struct CycleLineage {
    current_id: String,
}

impl LineageResolver for CycleLineage {
    fn predecessor(&self, release_id: &str) -> Option<PredecessorEvidence> {
        Some(PredecessorEvidence {
            release_id: release_id.to_owned(),
            manifest_sha256: DIGEST_HEX.to_owned(),
            data_sha256: DIGEST_HEX.to_owned(),
            revision: 1,
            knowledge_time_ns: KNOWLEDGE_NS,
            supersedes_release_id: Some(self.current_id.clone()),
        })
    }
}

fn assert_refusal(result: Result<AdmissionReceipt, VerifierError>, code: &str, context: &str) {
    let actual = result.err().map(|error| error.code().to_owned());
    let matches = actual.as_deref() == Some(code);
    assert!(
        matches,
        "{context}: expected {code} refusal, got {actual:?}"
    );
}

fn shifted_records() -> Result<Vec<ReleaseRecord>, Box<dyn Error>> {
    let mut records = Vec::new();
    for (index, number) in [1_u64, 3, 5].iter().enumerate() {
        let mut payload = Map::new();
        payload.insert("kind".to_owned(), Value::String("abstract".to_owned()));
        payload.insert("scalar".to_owned(), Value::from(*number));
        records.push(ReleaseRecord {
            sequence_domain_id: "domain-a".to_owned(),
            sequence_epoch: "epoch-1".to_owned(),
            sequence_number: *number,
            object_id: format!("object-{}", index),
            revision: 1,
            knowledge_time_ns: KNOWLEDGE_NS,
            assertion_id: format!("assertion-{}", index),
            envelope_schema_version: "1".to_owned(),
            payload,
        });
    }
    Ok(records)
}

#[test]
fn happy_path_uncompressed_receipt_is_bound() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let receipt = fixture.verify().map_err(|error| -> Box<dyn Error> {
        format!("happy path must admit: {:?}", error).into()
    })?;
    assert_eq!(
        receipt.release_id(),
        "rel-0001",
        "receipt release id binding"
    );
    assert_eq!(
        receipt.publisher_id(),
        "pub-one",
        "receipt publisher binding"
    );
    assert_eq!(receipt.key_id(), "key-1", "receipt key binding");
    assert_eq!(
        receipt.trust_registry_revision(),
        "registry-rev-1",
        "receipt registry revision binding"
    );
    assert_eq!(
        receipt.artifact_set().len(),
        1,
        "receipt artifact set binding"
    );
    assert_eq!(
        receipt.verification_policy_version(),
        VERIFICATION_POLICY_VERSION,
        "receipt policy version binding"
    );
    Ok(())
}

#[test]
fn happy_path_gzip_release_is_admitted() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("gzip")?;
    let receipt = fixture.verify().map_err(|error| -> Box<dyn Error> {
        format!("gzip release must admit: {:?}", error).into()
    })?;
    assert_eq!(
        receipt.data_sha256(),
        fixture.manifest.digests.data_sha256,
        "gzip dataset digest is compression independent"
    );
    Ok(())
}

#[test]
fn manifest_mutation_after_signing_is_refused() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let mut value = serde_json::to_value(&fixture.manifest)?;
    value["contract_revision"] = Value::String("contract-rev-mutated".to_owned());
    fs::write(
        fixture.release_dir().join("manifest.json"),
        serde_json::to_vec(&value)?,
    )?;
    assert_refusal(
        fixture.verify(),
        "REJECT_SIGNATURE_INTEGRITY",
        "mutated manifest must fail digest comparison",
    );
    Ok(())
}

#[test]
fn sidecar_digest_mutation_is_refused() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let mut value = serde_json::to_value(&fixture.sidecar)?;
    value["manifest_sha256"] = Value::String(OTHER_DIGEST_HEX.to_owned());
    fs::write(
        fixture.release_dir().join("signature.json"),
        serde_json::to_vec(&value)?,
    )?;
    assert_refusal(
        fixture.verify(),
        "REJECT_SIGNATURE_INTEGRITY",
        "mutated sidecar digest must fail",
    );
    Ok(())
}

#[test]
fn signature_byte_mutation_is_refused() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let mut value = serde_json::to_value(&fixture.sidecar)?;
    let mut signature = base64::engine::general_purpose::STANDARD
        .decode(fixture.sidecar.signature_b64.as_bytes())?;
    signature[0] ^= 1;
    value["signature_b64"] =
        Value::String(base64::engine::general_purpose::STANDARD.encode(signature));
    fs::write(
        fixture.release_dir().join("signature.json"),
        serde_json::to_vec(&value)?,
    )?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::SignatureInvalid)),
        "mutated signature bytes must fail Ed25519 verification"
    );
    Ok(())
}

#[test]
fn wrong_sidecar_key_id_breaks_publisher_binding() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    let first_key = fixture.registry_entry().clone();
    let second_key = TrustedKeyEntry {
        key_id: "key-2".to_owned(),
        ..first_key.clone()
    };
    fixture.set_registry(vec![first_key, second_key])?;
    fixture.sidecar.key_id = "key-2".to_owned();
    fixture.sidecar_bytes = serde_json::to_vec(&fixture.sidecar)?;
    fixture.write_all()?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::KeyBinding)),
        "sidecar key id must bind to manifest signing key id"
    );
    Ok(())
}

#[test]
fn unknown_key_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    let base = fixture.registry_entry().clone();
    let other = TrustedKeyEntry {
        key_id: "key-9".to_owned(),
        ..base
    };
    fixture.set_registry(vec![other])?;
    assert_refusal(
        fixture.verify(),
        "REJECT_PUBLISHER_IDENTITY",
        "unknown trusted key must fail closed",
    );
    Ok(())
}

#[test]
fn revoked_key_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    let base = fixture.registry_entry().clone();
    let revoked = TrustedKeyEntry {
        state: KeyState::Revoked,
        ..base
    };
    fixture.set_registry(vec![revoked])?;
    assert_refusal(
        fixture.verify(),
        "REJECT_PUBLISHER_IDENTITY",
        "revoked key must fail closed",
    );
    Ok(())
}

#[test]
fn compromised_key_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    let base = fixture.registry_entry().clone();
    let compromised = TrustedKeyEntry {
        state: KeyState::Compromised,
        ..base
    };
    fixture.set_registry(vec![compromised])?;
    assert_refusal(
        fixture.verify(),
        "REJECT_PUBLISHER_IDENTITY",
        "compromised key must fail closed",
    );
    Ok(())
}

#[test]
fn duplicate_json_member_is_refused() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let original = String::from_utf8(fixture.manifest_bytes.clone())?;
    let injected = original.replacen('{', "{\"schema_version\":1,", 1);
    fs::write(fixture.release_dir().join("manifest.json"), injected)?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::ManifestParse { .. })),
        "duplicate JSON member must fail strict parsing"
    );
    Ok(())
}

#[test]
fn invalid_utf8_manifest_is_refused() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let mut bytes = b"{\"schema_version\":1,\"text\":\"".to_vec();
    bytes.push(0xFF);
    bytes.extend_from_slice(b"\"}");
    fs::write(fixture.release_dir().join("manifest.json"), bytes)?;
    assert_refusal(
        fixture.verify(),
        "REJECT_SCHEMA_ENVELOPE",
        "invalid UTF-8 must fail manifest parsing",
    );
    Ok(())
}

#[test]
fn path_traversal_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.manifest.artifacts[0].path = "../escape.jsonl".to_owned();
    fixture.rebuild_digests()?;
    fixture.resign()?;
    fixture.write_all()?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::UnsafePath { .. })),
        "traversal artifact path must be refused"
    );
    Ok(())
}

#[test]
fn symlink_escape_is_refused() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let outside = fixture.temp.path().join("outside-target");
    fs::write(&outside, b"outside")?;
    fs::remove_file(fixture.artifact_path())?;
    symlink(&outside, fixture.artifact_path())?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::SymlinkEscape { .. })),
        "symlink artifact must be refused before open"
    );
    Ok(())
}

#[test]
fn hard_linked_published_file_is_refused() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let sibling = fixture.temp.path().join("sibling-link");
    fs::hard_link(fixture.artifact_path(), sibling)?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::HardLink { .. })),
        "published file with nlink greater than one must be refused"
    );
    Ok(())
}

#[test]
fn special_file_is_refused() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    fs::remove_file(fixture.artifact_path())?;
    let status = Command::new("mkfifo")
        .arg(fixture.artifact_path())
        .status()?;
    assert!(status.success(), "mkfifo must create the special file");
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::SpecialFile)),
        "fifo at artifact path must be refused"
    );
    Ok(())
}

#[test]
fn missing_artifact_is_refused() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    fs::remove_file(fixture.artifact_path())?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::FileSet { .. })),
        "missing declared artifact must be refused"
    );
    Ok(())
}

#[test]
fn extra_file_is_refused() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    fs::write(
        fixture.release_dir().join("artifacts").join("extra.jsonl"),
        b"extra",
    )?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::FileSet { .. })),
        "undeclared artifact must be refused"
    );
    Ok(())
}

#[test]
fn duplicate_artifact_path_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    let mut duplicate = fixture.manifest.artifacts[0].clone();
    duplicate.logical_partition_id = "partition-b".to_owned();
    fixture.manifest.artifacts.push(duplicate);
    fixture.rebuild_digests()?;
    fixture.resign()?;
    fixture.write_all()?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::FileSet { .. })),
        "duplicate normalized artifact path must be refused"
    );
    Ok(())
}

#[test]
fn reordered_records_are_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    let mut records = Vec::new();
    for number in [1_u64, 3, 2] {
        let mut payload = Map::new();
        payload.insert("kind".to_owned(), Value::String("abstract".to_owned()));
        payload.insert("scalar".to_owned(), Value::from(number));
        records.push(ReleaseRecord {
            sequence_domain_id: "domain-a".to_owned(),
            sequence_epoch: "epoch-1".to_owned(),
            sequence_number: number,
            object_id: format!("object-{}", number),
            revision: 1,
            knowledge_time_ns: KNOWLEDGE_NS,
            assertion_id: format!("assertion-{}", number),
            envelope_schema_version: "1".to_owned(),
            payload,
        });
    }
    fixture.set_records(records)?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::RecordOrder { .. })),
        "reordered records must be refused"
    );
    Ok(())
}

#[test]
fn duplicate_sequence_position_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    let mut records = Vec::new();
    for number in [1_u64, 1, 2] {
        let mut payload = Map::new();
        payload.insert("kind".to_owned(), Value::String("abstract".to_owned()));
        payload.insert("scalar".to_owned(), Value::from(number));
        records.push(ReleaseRecord {
            sequence_domain_id: "domain-a".to_owned(),
            sequence_epoch: "epoch-1".to_owned(),
            sequence_number: number,
            object_id: format!("object-{}", number),
            revision: 1,
            knowledge_time_ns: KNOWLEDGE_NS,
            assertion_id: format!("assertion-{}", number),
            envelope_schema_version: "1".to_owned(),
            payload,
        });
    }
    fixture.set_records(records)?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::RecordOrder { .. })),
        "duplicate sequence position must be refused"
    );
    Ok(())
}

#[test]
fn declared_gap_policy_admits_visible_gaps() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.manifest.artifacts[0].record_order_profile = "sequence-domain-declared-gaps".to_owned();
    fixture.set_records(shifted_records()?)?;
    let receipt = fixture.verify().map_err(|error| -> Box<dyn Error> {
        format!("declared gaps must be admitted: {:?}", error).into()
    })?;
    assert_eq!(
        receipt.artifact_set().len(),
        1,
        "gap policy receipt is bound"
    );
    Ok(())
}

#[test]
fn strict_gap_policy_refuses_gaps() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.set_records(shifted_records()?)?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::RecordOrder { .. })),
        "strict gap policy must refuse a sequence gap"
    );
    Ok(())
}

#[test]
fn correction_cycle_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.manifest.lineage.supersedes = Some(SupersedesRelation {
        supersedes_release_id: "rel-0000".to_owned(),
        supersedes_manifest_sha256: DIGEST_HEX.to_owned(),
        supersedes_data_sha256: DIGEST_HEX.to_owned(),
        revision: 1,
        correction_reason: "abstract-correction".to_owned(),
        knowledge_time_ns: KNOWLEDGE_NS,
        invalidated_at_time_ns: KNOWLEDGE_NS,
    });
    fixture.resign()?;
    fixture.write_all()?;
    let result = fixture.verify_with(
        Box::new(PermitRights),
        Box::new(PassCoverage),
        Box::new(CycleLineage {
            current_id: "rel-0001".to_owned(),
        }),
    );
    assert!(
        matches!(result, Err(VerifierError::Lineage { .. })),
        "correction cycle must be refused"
    );
    Ok(())
}

#[test]
fn correction_self_reference_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.manifest.lineage.supersedes = Some(SupersedesRelation {
        supersedes_release_id: "rel-0001".to_owned(),
        supersedes_manifest_sha256: DIGEST_HEX.to_owned(),
        supersedes_data_sha256: DIGEST_HEX.to_owned(),
        revision: 1,
        correction_reason: "abstract-correction".to_owned(),
        knowledge_time_ns: KNOWLEDGE_NS,
        invalidated_at_time_ns: KNOWLEDGE_NS,
    });
    fixture.resign()?;
    fixture.write_all()?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::Lineage { .. })),
        "self-superseding release must be refused"
    );
    Ok(())
}

#[test]
fn missing_predecessor_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.manifest.lineage.supersedes = Some(SupersedesRelation {
        supersedes_release_id: "rel-0000".to_owned(),
        supersedes_manifest_sha256: DIGEST_HEX.to_owned(),
        supersedes_data_sha256: DIGEST_HEX.to_owned(),
        revision: 1,
        correction_reason: "abstract-correction".to_owned(),
        knowledge_time_ns: KNOWLEDGE_NS,
        invalidated_at_time_ns: KNOWLEDGE_NS,
    });
    fixture.resign()?;
    fixture.write_all()?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::Lineage { .. })),
        "missing trusted predecessor must be refused"
    );
    Ok(())
}

#[test]
fn future_knowledge_time_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.manifest.lineage.knowledge_time_ns = AS_OF_NS + 1;
    fixture.resign()?;
    fixture.write_all()?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::PointInTime { .. })),
        "knowledge time after as_of instant must be refused"
    );
    Ok(())
}

#[test]
fn rights_unknown_is_refused() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let result = fixture.verify_with(
        Box::new(UnknownRights),
        Box::new(PassCoverage),
        Box::new(EmptyLineage),
    );
    assert!(
        matches!(result, Err(VerifierError::RightsNotPermitted)),
        "rights UNKNOWN must refuse admission"
    );
    Ok(())
}

#[test]
fn absent_denominator_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.manifest.coverage.dimensions[0].denominator = None;
    fixture.resign()?;
    fixture.write_all()?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::Coverage { .. })),
        "absent coverage denominator must refuse admission"
    );
    Ok(())
}

#[test]
fn implicit_all_coverage_is_refused() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let result = fixture.verify_with(
        Box::new(PermitRights),
        Box::new(ImplicitAllCoverage),
        Box::new(EmptyLineage),
    );
    assert!(
        matches!(result, Err(VerifierError::Coverage { .. })),
        "implicit ALL coverage must refuse admission"
    );
    Ok(())
}

#[test]
fn refusal_leaves_no_private_snapshot() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let result = fixture.verify_with(
        Box::new(PermitRights),
        Box::new(ImplicitAllCoverage),
        Box::new(EmptyLineage),
    );
    assert!(
        result.is_err(),
        "coverage refusal must stop before receipt issuance"
    );
    let staged: Vec<_> = fs::read_dir(&fixture.staging)?.collect();
    assert!(
        staged.is_empty(),
        "refusal must discard staged snapshot files"
    );
    Ok(())
}

#[test]
fn private_snapshot_contains_verified_bytes() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let receipt = fixture.verify().map_err(|error| -> Box<dyn Error> {
        format!("snapshot happy path must admit: {:?}", error).into()
    })?;
    let manifest_copy = fs::read(fixture.staging.join("manifest.json"))?;
    let sidecar_copy = fs::read(fixture.staging.join("signature.json"))?;
    let artifact_copy = fs::read(fixture.staging.join("records.jsonl"))?;
    assert_eq!(
        manifest_copy, fixture.manifest_bytes,
        "manifest snapshot bytes match"
    );
    assert_eq!(
        sidecar_copy, fixture.sidecar_bytes,
        "sidecar snapshot bytes match"
    );
    assert_eq!(
        artifact_copy, fixture.stored,
        "artifact snapshot bytes match"
    );
    assert_eq!(
        receipt.artifact_set()[0].stored_sha256,
        hex_digest(&sha256_bytes(&artifact_copy)),
        "receipt binds verified snapshot bytes"
    );
    Ok(())
}

#[test]
fn mutation_of_published_path_after_seal_is_ignored() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let result = fixture.verify_with(
        Box::new(PermitRights),
        Box::new(MutatingCoverage {
            artifact_path: fixture.artifact_path(),
        }),
        Box::new(EmptyLineage),
    );
    assert!(
        result.is_ok(),
        "downstream stages must consume sealed snapshot, not published path"
    );
    Ok(())
}

#[test]
fn resource_limit_is_enforced() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.policy.max_manifest_bytes = 1;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::ResourceLimit { .. })),
        "declared manifest size above policy maximum must refuse"
    );
    Ok(())
}

#[test]
fn receipt_digest_binds_request_and_release() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let first = fixture.verify().map_err(|error| -> Box<dyn Error> {
        format!("first receipt must be issued: {:?}", error).into()
    })?;
    fixture.clean_staging()?;
    let mut second_request = fixture.request.clone();
    second_request.actor = "consumer-b".to_owned();
    let verifier = ReleaseVerifier::new(release_verifier::VerifierConfig {
        root: fixture.root.clone(),
        private_staging: fixture.staging.clone(),
        registry: fixture.registry.clone(),
        rights: Box::new(PermitRights),
        coverage: Box::new(PassCoverage),
        lineage: Box::new(EmptyLineage),
        policy: fixture.policy,
        verifier_commit: "test-verifier-commit".to_owned(),
    });
    let second = verifier
        .verify_release(&fixture.locator(), &second_request)
        .map_err(|error| -> Box<dyn Error> {
            format!("second receipt must be issued: {:?}", error).into()
        })?;
    assert_ne!(
        first.receipt_digest(),
        second.receipt_digest(),
        "receipt digest must bind the admission request"
    );
    assert_eq!(
        first.result(),
        release_verifier::AdmissionResult::Admitted,
        "receipt result is terminal"
    );
    Ok(())
}

#[test]
fn manifest_self_digest_field_is_refused() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let mut value: Value = serde_json::from_slice(&fixture.manifest_bytes)?;
    value["digests"]["manifest_sha256"] = Value::String(DIGEST_HEX.to_owned());
    fs::write(
        fixture.release_dir().join("manifest.json"),
        serde_json::to_vec(&value)?,
    )?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::UnknownMember { .. })),
        "manifest must not carry its own digest"
    );
    Ok(())
}

#[test]
fn manifest_limits_field_is_refused() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let mut value: Value = serde_json::from_slice(&fixture.manifest_bytes)?;
    value["limits"] = Value::Object(Map::new());
    fs::write(
        fixture.release_dir().join("manifest.json"),
        serde_json::to_vec(&value)?,
    )?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::UnknownMember { .. })),
        "declared manifest limits must be refused"
    );
    Ok(())
}

#[test]
fn sidecar_extra_field_is_refused() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let mut value: Value = serde_json::from_slice(&fixture.sidecar_bytes)?;
    value["domain"] = Value::String("NDC-V2-RELEASE-MANIFEST-SHA256-V1".to_owned());
    fs::write(
        fixture.release_dir().join("signature.json"),
        serde_json::to_vec(&value)?,
    )?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::UnknownMember { .. })),
        "sidecar has exactly five members"
    );
    Ok(())
}

#[test]
fn non_canonical_record_line_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    let original = String::from_utf8(fixture.expanded.clone())?;
    fixture.expanded = original
        .replace("\"kind\":\"abstract\"", "\"kind\" : \"abstract\"")
        .into_bytes();
    fixture.stored = fixture.expanded.clone();
    fixture.rebuild_digests()?;
    fixture.resign()?;
    fixture.write_all()?;
    let result = fixture.verify();
    assert!(
        matches!(result, Err(VerifierError::RecordStream { .. })),
        "non-canonical record bytes must be refused"
    );
    Ok(())
}
