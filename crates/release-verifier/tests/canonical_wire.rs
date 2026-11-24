// SPDX-License-Identifier: Apache-2.0
//! Abstract conformance tests for the canonical NDC v2 Profile 1 wire mapping.
//!
//! Every scalar here is arbitrary structured text. No market observation,
//! price, trade, order, universe, venue fact, right or signing secret for a
//! real publisher appears. The fixed seed `[0x11; 32]` is a published
//! conformance value used only to produce local test signatures.

use base64::Engine;
use ed25519_dalek::Signer;
use ed25519_dalek::SigningKey;
use flate2::Compression;
use flate2::write::GzEncoder;
use release_verifier::canonical_wire;
use release_verifier::canonical_wire::ArtifactRecordOrderProfile;
use release_verifier::canonical_wire::CanonicalError;
use release_verifier::canonical_wire::DerivedOrderKey;
use release_verifier::canonical_wire::SequencedArtifactProfile;
use release_verifier::canonical_wire::SequencedKind;
use release_verifier::jcs::canonical_bytes;
use serde_json::Value;
use serde_json::json;
use std::error::Error;
use std::fs;
use std::io::Write as IoWrite;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

const TEST_SEED: [u8; 32] = [0x11; 32];
const AS_OF: &str = "1700000000000000000";
const KNOWLEDGE: &str = "1600000000000000000";
const DIGEST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const OTHER_DIGEST: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const UTC_BEGIN: &str = "2026-01-01T00:00:00.000000000Z";
const UTC_END: &str = "2026-01-02T00:00:00.000000000Z";
const DOMAIN: &str = "domain-example";
const EPOCH: &str = "epoch-example";
const EVENT_TYPE: &str = "event-type-example";

struct Fixture {
    temp: TempDir,
    release: PathBuf,
    manifest: Value,
    sidecar: Value,
    expanded: Vec<u8>,
    stored: Vec<u8>,
}

impl Fixture {
    fn new(compression: &str) -> Result<Self, Box<dyn Error>> {
        let temp = TempDir::new()?;
        let release = temp
            .path()
            .join("trusted-root")
            .join("conformance-publisher")
            .join("ds-canonical")
            .join("rel-canonical");
        fs::create_dir_all(release.join("artifacts"))?;
        let records = vec![
            record_json("1", "abstract-one"),
            record_json("3", "abstract-three"),
        ];
        let expanded = encode_lines(&records)?;
        let stored = compress(&expanded, compression)?;
        let mut fixture = Self {
            temp,
            release,
            manifest: manifest_json(compression),
            sidecar: Value::Null,
            expanded,
            stored,
        };
        fixture.resign()?;
        fixture.write()?;
        Ok(fixture)
    }

    fn resign(&mut self) -> Result<(), Box<dyn Error>> {
        let data_digest = hex(&sha256(&self.expanded));
        let stored_digest = hex(&sha256(&self.stored));
        self.manifest["digests"]["data_sha256"] = Value::String(data_digest);
        self.manifest["artifacts"][0]["stored_bytes"] = json!(self.stored.len());
        self.manifest["artifacts"][0]["stored_sha256"] = Value::String(stored_digest);
        self.manifest["artifacts"][0]["uncompressed_bytes"] = json!(self.expanded.len());
        self.manifest["artifacts"][0]["uncompressed_sha256"] =
            Value::String(hex(&sha256(&self.expanded)));
        self.manifest["artifacts"][0]["record_count"] = json!(record_count(&self.expanded)?);

        let canonical = canonical_bytes(&self.manifest);
        let digest = sha256(&canonical);
        let digest_text = hex(&digest);
        let mut message = b"NDC-V2-RELEASE-MANIFEST-SHA256-V1\x00".to_vec();
        message.extend_from_slice(&digest);
        let signing_key = SigningKey::from_bytes(&TEST_SEED);
        let signature = signing_key.sign(&message).to_bytes();
        self.sidecar = json!({
            "schema_version": 1,
            "algorithm": "Ed25519",
            "key_id": "conformance-key-1",
            "manifest_sha256": digest_text,
            "signature_b64": base64::engine::general_purpose::STANDARD.encode(signature),
        });
        Ok(())
    }

    fn write(&self) -> Result<(), Box<dyn Error>> {
        fs::write(
            self.release.join("manifest.json"),
            serde_json::to_vec(&self.manifest)?,
        )?;
        fs::write(
            self.release.join("signature.json"),
            serde_json::to_vec(&self.sidecar)?,
        )?;
        fs::write(
            self.release.join("artifacts").join("records.jsonl"),
            &self.stored,
        )?;
        Ok(())
    }

    fn mutate_manifest<F>(&mut self, change: F) -> Result<(), Box<dyn Error>>
    where
        F: FnOnce(&mut Value),
    {
        change(&mut self.manifest);
        self.resign()?;
        self.write()?;
        Ok(())
    }

    fn set_records(&mut self, records: &[Value], compression: &str) -> Result<(), Box<dyn Error>> {
        self.expanded = encode_lines(records)?;
        self.stored = compress(&self.expanded, compression)?;
        self.resign()?;
        self.write()?;
        Ok(())
    }

    fn run_binary(&self) -> Result<(std::process::Output, PathBuf, PathBuf), Box<dyn Error>> {
        let out = self.temp.path().join("diagnostic.json");
        let tmpdir = self.temp.path().join("child-tmp");
        fs::create_dir_all(&tmpdir)?;
        let output = Command::new(env!("CARGO_BIN_EXE_ndc-conformance-verifier"))
            .arg("--release")
            .arg(&self.release)
            .arg("--out")
            .arg(&out)
            .env("TMPDIR", &tmpdir)
            .output()?;
        Ok((output, out, tmpdir))
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(bytes);
    let mut out = [0_u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{:02x}", byte)).collect()
}

fn record_json(number: &str, value: &str) -> Value {
    json!({
        "envelope_schema_version": "envelope-1",
        "event_type": EVENT_TYPE,
        "ordering": {
            "kind": "sequenced",
            "sequence_domain_id": DOMAIN,
            "sequence_epoch": EPOCH,
            "sequence_number": number,
        },
        "payload": {
            "fields": [
                {"field_name": "abstract-field", "kind": "string", "value": value}
            ]
        }
    })
}

fn encode_lines(records: &[Value]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut output = Vec::new();
    for record in records {
        output.extend_from_slice(&canonical_bytes(record));
        output.push(b'\n');
    }
    Ok(output)
}

fn record_count(bytes: &[u8]) -> Result<usize, Box<dyn Error>> {
    if bytes.is_empty() || bytes.last() != Some(&b'\n') {
        return Err("record stream lacks an LF boundary".into());
    }
    Ok(bytes[..bytes.len() - 1]
        .split(|byte| *byte == b'\n')
        .count())
}

fn compress(bytes: &[u8], compression: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    if compression == "none" {
        return Ok(bytes.to_vec());
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?)
}

fn attestation() -> Value {
    json!({
        "purpose": "abstract-purpose",
        "usage_right": "abstract-usage-right",
        "usage_mode": "abstract-usage-mode",
        "territory": "abstract-territory",
        "scope_assertions": ["abstract-assertion"],
        "reviewer": "abstract-reviewer",
        "authority": "abstract-review-authority",
    })
}

fn grant() -> Value {
    json!({
        "grant_id": "grant-example",
        "grant_revision": "grant-revision-1",
        "granting_authority": {
            "authority_id": "granting-authority-example",
            "evidence_reference": "authority-evidence-example",
        },
        "action": "internal_analytics",
        "object_class": "object-class-example",
        "decision": "PERMIT",
        "actor": "actor-example",
        "licensee": "licensee-example",
        "recipient_class": "NAMED",
        "recipient_identity": "recipient-example",
        "purpose": "proprietary_own_account_trading_research",
        "scope": {
            "provider_id": "provider-example",
            "source_product_id": "product-example",
            "venue_id": "venue-example",
            "segment_id": "segment-example",
            "dataset_id": "dataset-example",
            "subject_state": "NAMED",
            "named_subjects": ["object-1"],
            "data_time_begin_ns": KNOWLEDGE,
            "data_time_end_ns": AS_OF,
        },
        "validity": {
            "begin_ns": "1",
            "end_ns": AS_OF,
            "clock_domain": "clock-domain-example",
        },
        "delivery": {
            "method": "private-transfer",
            "approved_endpoint_identities": ["endpoint-example"],
        },
        "retention": {
            "max_duration_ns": 1000,
            "purge_rule": "purge-at-expiry",
        },
        "supporting_document_references": [
            {"reference": "doc-example", "digest_sha256": OTHER_DIGEST}
        ],
        "contract_revision": "contract-rev-1",
        "review_authority": "abstract-review-authority",
        "reviewed_on": UTC_BEGIN,
        "reviewed_attestation": attestation(),
    })
}

fn dimension(index: usize) -> Value {
    let identifiers = [
        "instrument_coverage",
        "listing_identity_coverage",
        "contract_coverage",
        "corporate_action_coverage",
        "session_time_coverage",
        "trade_event_coverage",
        "quote_depth_coverage",
        "mbo_coverage",
        "sequence_continuity",
    ];
    json!({
        "dimension_id": identifiers[index],
        "universe_id": format!("universe-example-{}", index),
        "denominator_identity": format!("denominator-example-{}", index),
        "count_basis": "count-basis-example",
        "numerator": 3,
        "denominator": 4,
        "missing_count": 1,
        "rejected_count": 0,
        "as_of_ns": AS_OF,
        "verdict": "UNKNOWN",
        "evidence_reference": format!("evidence-example-{}", index),
        "gaps": [{
            "gap_id": format!("gap-example-{}", index),
            "gap_kind": "MISSING",
            "component": "component-example",
            "count": 1,
            "description": "abstract-gap-description",
        }],
    })
}

fn manifest_json(compression: &str) -> Value {
    let (compression_token, version_token) = if compression == "gzip" {
        ("gzip", "gzip-deflate/flate2-1")
    } else {
        ("none", "none/1")
    };
    json!({
        "schema_version": 1,
        "contract_version": "v2",
        "contract_revision": "contract-rev-1",
        "release_identity": {
            "release_id": "rel-canonical",
            "release_version": "1",
            "as_of_ns": AS_OF,
            "created_at_utc_ns": "1699999999000000000",
            "publication_state": "AVAILABLE",
        },
        "source_scope": {
            "provider_id": "provider-example",
            "source_product_id": "product-example",
            "venue_id": "venue-example",
            "segment_id": "segment-example",
            "dataset_id": "dataset-example",
        },
        "producer_activity": {
            "performed_operations": [{
                "operation_ref": "operation-example",
                "action": "internal_analytics",
                "data_object": {
                    "kind": "object-class-example",
                    "id": "object-example",
                    "version": "1",
                }
            }],
            "operation_evidence": [{
                "operation_ref": "operation-example",
                "event_id": "event-example",
                "run_id": "run-example",
                "recorded_at_ns": KNOWLEDGE,
                "decision_key": "decision-key-example",
                "digest_sha256": DIGEST,
            }],
        },
        "rights": {
            "permission_grants": [grant()],
            "document_references": [{
                "reference": "doc-example",
                "clause": "clause-example",
                "document_sha256": OTHER_DIGEST,
                "contract_revision": "contract-rev-1",
                "review_authority": "abstract-review-authority",
                "reviewed_on": UTC_BEGIN,
            }],
            "reviewed_attestations": [{
                "attestation_id": "attestation-example",
                "grant_id": "grant-example",
                "attestation": attestation(),
            }],
        },
        "coverage": {
            "dimensions": (0..9).map(dimension).collect::<Vec<Value>>(),
        },
        "lineage": {
            "raw_archive_reference": "archive-example",
            "raw_archive_sha256": DIGEST,
            "transformation_record": {"identity": "transform-example", "version": "1"},
            "knowledge_time_ns": KNOWLEDGE,
            "revision": 1,
            "was_derived_from": {
                "source_entity_id": "archive-entity-example",
                "deriving_activity_id": "transform-activity-example",
                "knowledge_time_ns": KNOWLEDGE,
            },
            "was_revision_of": {
                "predecessor_entity_id": "predecessor-entity-example",
                "revision_activity_id": "revision-activity-example",
                "knowledge_time_ns": KNOWLEDGE,
            },
            "had_primary_source": {
                "source_entity_id": "archive-entity-example",
                "attribution_activity_id": "attribution-activity-example",
                "knowledge_time_ns": KNOWLEDGE,
            },
            "generated_at_time": UTC_BEGIN,
            "invalidated_at_time": UTC_END,
            "qualified_attribution": {
                "agent_id": "agent-example",
                "activity_id": "transform-activity-example",
                "role": "role-example",
                "knowledge_time_ns": KNOWLEDGE,
            },
            "supersedes": Value::Null,
        },
        "schema_versions": {
            "normalizer_version": "normalizer-1",
            "envelope_version": "envelope-1",
            "payload_schemas": [{
                "event_type": EVENT_TYPE,
                "schema_id": "payload-example",
                "schema_version": "1",
                "digest_sha256": DIGEST,
            }],
        },
        "digests": {
            "digest_algorithm": "sha256",
            "data_sha256": DIGEST,
        },
        "publication_identity": {
            "publisher_identity": "conformance-publisher",
            "signing_key_id": "conformance-key-1",
            "signature_reference": "signature.json",
            "archive_reference": "archive-ref-1",
        },
        "artifacts": [{
            "logical_partition_id": "partition-example",
            "shard_ordinal": 0,
            "path": "artifacts/records.jsonl",
            "record_order_profile": {
                "kind": "sequenced",
                "sequence_domain_id": DOMAIN,
                "sequence_epoch": EPOCH,
            },
            "encoding": "application/x-ndjson; charset=utf-8",
            "compression": compression_token,
            "compression_version": version_token,
            "stored_bytes": 1,
            "stored_sha256": DIGEST,
            "uncompressed_bytes": 1,
            "uncompressed_sha256": OTHER_DIGEST,
            "record_count": 2,
        }],
    })
}

fn parse_fixture(fixture: &Fixture) -> Result<canonical_wire::ParsedManifest, CanonicalError> {
    canonical_wire::parse_manifest(&fixture.manifest)
}

fn artifact_profile() -> Result<ArtifactRecordOrderProfile, Box<dyn Error>> {
    Ok(ArtifactRecordOrderProfile::Sequenced(
        SequencedArtifactProfile {
            kind: SequencedKind::Sequenced,
            sequence_domain_id: canonical_wire::NonEmptyString::new(DOMAIN)?,
            sequence_epoch: canonical_wire::NonEmptyString::new(EPOCH)?,
        },
    ))
}

fn assert_rejection(result: Result<canonical_wire::ParsedManifest, CanonicalError>) {
    assert!(
        result.is_err(),
        "strict canonical parsing must refuse the mutated value"
    );
}

#[test]
fn canonical_manifest_parses_and_maps() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let parsed = parse_fixture(&fixture)?;
    assert_eq!(
        parsed.manifest.release_identity.as_of_ns.value(),
        1_700_000_000_000_000_000,
        "decimal string instant must decode to its u64 value"
    );
    let internal = canonical_wire::into_internal_manifest(&parsed)?;
    assert_eq!(
        internal.artifacts[0].record_order_profile, "sequence-domain-declared-gaps",
        "sequenced canonical profile must map to the internal gap-visible token"
    );
    assert_eq!(
        internal.coverage.dimensions.len(),
        9,
        "all nine coverage dimensions must map"
    );
    Ok(())
}

#[test]
fn decimal_instant_rejects_integer_form() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        value["release_identity"]["as_of_ns"] = json!(1_700_000_000_000_000_000_u64);
    })?;
    assert_rejection(parse_fixture(&fixture));
    Ok(())
}

#[test]
fn decimal_instant_rejects_leading_zero_and_u64_overflow() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        value["release_identity"]["as_of_ns"] = Value::String("01800".to_owned());
    })?;
    assert_rejection(parse_fixture(&fixture));
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        value["release_identity"]["as_of_ns"] = Value::String("18446744073709551616".to_owned());
    })?;
    assert_rejection(parse_fixture(&fixture));
    Ok(())
}

#[test]
fn utc_instant_requires_nine_fractional_digits_and_valid_calendar() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        value["lineage"]["generated_at_time"] = Value::String("2026-01-01T00:00:00Z".to_owned());
    })?;
    assert_rejection(parse_fixture(&fixture));
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        value["lineage"]["generated_at_time"] =
            Value::String("2026-02-30T00:00:00.000000000Z".to_owned());
    })?;
    assert_rejection(parse_fixture(&fixture));
    Ok(())
}

#[test]
fn required_venue_segment_and_dataset_reject_placeholders() -> Result<(), Box<dyn Error>> {
    for member in [
        "venue_id",
        "segment_id",
        "dataset_id",
        "provider_id",
        "source_product_id",
    ] {
        let mut fixture = Fixture::new("none")?;
        fixture.mutate_manifest(|value| {
            value["source_scope"][member] = Value::String("UNKNOWN".to_owned());
        })?;
        assert_rejection(parse_fixture(&fixture));
    }
    Ok(())
}

#[test]
fn missing_required_scope_member_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        value["source_scope"]
            .as_object_mut()
            .and_then(|object| object.remove("venue_id"));
    })?;
    assert_rejection(parse_fixture(&fixture));
    Ok(())
}

#[test]
fn unknown_manifest_member_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        value["limits"] = json!({"max_records": 10});
    })?;
    let rejection = canonical_wire::parse_manifest(&fixture.manifest);
    assert!(
        matches!(rejection, Err(CanonicalError::UnknownMember(_))),
        "unknown member must be classified as UnknownMember"
    );
    Ok(())
}

#[test]
fn nested_contract_version_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        value["schema_versions"]["contract_version"] = Value::String("v2".to_owned());
    })?;
    assert_rejection(parse_fixture(&fixture));
    Ok(())
}

#[test]
fn bare_string_artifact_profile_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        value["artifacts"][0]["record_order_profile"] = Value::String("sequenced".to_owned());
    })?;
    assert_rejection(parse_fixture(&fixture));
    Ok(())
}

#[test]
fn reference_profile_with_domain_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        value["artifacts"][0]["record_order_profile"] = json!({
            "kind": "reference",
            "sequence_domain_id": DOMAIN,
        });
    })?;
    assert_rejection(parse_fixture(&fixture));
    Ok(())
}

#[test]
fn record_with_both_ordering_modes_is_refused() -> Result<(), Box<dyn Error>> {
    let mut record = record_json("7", "abstract-value");
    record["ordering"]["object_id"] = Value::String("object-example".to_owned());
    record["ordering"]["revision"] = Value::String("2".to_owned());
    record["ordering"]["knowledge_time_ns"] = Value::String("1000".to_owned());
    record["ordering"]["assertion_id"] = Value::String("assertion-example".to_owned());
    let line = canonical_bytes(&record);
    let error = canonical_wire::derive_key_from_bytes(&line, &artifact_profile()?)
        .expect_err("a both-mode record must be refused");
    assert_eq!(
        error.code(),
        "REJECT_SCHEMA_ENVELOPE",
        "both modes are a shape refusal"
    );
    Ok(())
}

#[test]
fn integer_sequence_number_is_refused() -> Result<(), Box<dyn Error>> {
    let mut record = record_json("7", "abstract-value");
    record["ordering"]["sequence_number"] = json!(7);
    let line = canonical_bytes(&record);
    assert!(
        canonical_wire::derive_key_from_bytes(&line, &artifact_profile()?).is_err(),
        "sequence numbers must be decimal strings"
    );
    Ok(())
}

#[test]
fn byte_derived_key_recomputes_from_record_bytes() -> Result<(), Box<dyn Error>> {
    let line = canonical_bytes(&record_json("7", "abstract-value"));
    let key = canonical_wire::derive_key_from_bytes(&line, &artifact_profile()?)?;
    assert_eq!(
        key,
        DerivedOrderKey::Sequenced(DOMAIN.to_owned(), EPOCH.to_owned(), 7),
        "the key must be derived from the parsed canonical bytes"
    );
    Ok(())
}

#[test]
fn caller_supplied_key_mismatch_is_refused() -> Result<(), Box<dyn Error>> {
    let line = canonical_bytes(&record_json("7", "abstract-value"));
    let derived = canonical_wire::derive_key_from_bytes(&line, &artifact_profile()?)?;
    let supplied = DerivedOrderKey::Sequenced(DOMAIN.to_owned(), EPOCH.to_owned(), 9);
    let error = canonical_wire::check_supplied_key(&supplied, &derived)
        .expect_err("a mismatched supplied key must be refused");
    assert_eq!(
        error.code(),
        "REJECT_ORDERING",
        "key mismatch is an ordering refusal"
    );
    Ok(())
}

#[test]
fn non_canonical_record_bytes_are_refused() -> Result<(), Box<dyn Error>> {
    let record = record_json("7", "abstract-value");
    let canonical = String::from_utf8(canonical_bytes(&record))?;
    let altered = canonical.replace("\"kind\":\"sequenced\"", "\"kind\" : \"sequenced\"");
    let error = canonical_wire::derive_key_from_bytes(altered.as_bytes(), &artifact_profile()?)
        .expect_err("a re-serialized line must not be an ordering source");
    assert_eq!(
        error.code(),
        "REJECT_ORDERING",
        "canonical bytes are the sole key source"
    );
    Ok(())
}

#[test]
fn derived_order_sequence_rejects_reordering_but_allows_visible_gaps() -> Result<(), Box<dyn Error>>
{
    let first = DerivedOrderKey::Sequenced(DOMAIN.to_owned(), EPOCH.to_owned(), 1);
    let third = DerivedOrderKey::Sequenced(DOMAIN.to_owned(), EPOCH.to_owned(), 3);
    canonical_wire::validate_order_sequence(&[first.clone(), third.clone()])?;
    assert!(
        canonical_wire::validate_order_sequence(&[third, first]).is_err(),
        "descending byte-derived keys must be refused"
    );
    Ok(())
}

#[test]
fn operation_evidence_binding_is_typed_and_unique() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        value["producer_activity"]["operation_evidence"][0]["operation_ref"] =
            Value::String("unbound-operation".to_owned());
    })?;
    assert_rejection(parse_fixture(&fixture));
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        let evidence = value["producer_activity"]["operation_evidence"][0].clone();
        if let Some(array) = value["producer_activity"]["operation_evidence"].as_array_mut() {
            array.push(evidence);
        }
    })?;
    assert_rejection(parse_fixture(&fixture));
    Ok(())
}

#[test]
fn attestation_mismatch_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        value["rights"]["reviewed_attestations"][0]["attestation"]["reviewer"] =
            Value::String("different-reviewer".to_owned());
    })?;
    assert_rejection(parse_fixture(&fixture));
    Ok(())
}

#[test]
fn coverage_requires_nonzero_denominator_and_exhaustive_parts() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        value["coverage"]["dimensions"][0]["denominator"] = json!(0);
    })?;
    assert_rejection(parse_fixture(&fixture));
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        value["coverage"]["dimensions"][0]["numerator"] = json!(4);
        value["coverage"]["dimensions"][0]["missing_count"] = json!(1);
    })?;
    assert_rejection(parse_fixture(&fixture));
    Ok(())
}

#[test]
fn lineage_revision_regression_and_self_reference_are_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        value["lineage"]["revision"] = json!(1);
        value["lineage"]["supersedes"] = json!({
            "supersedes_release_id": "predecessor-release",
            "supersedes_manifest_sha256": DIGEST,
            "supersedes_data_sha256": OTHER_DIGEST,
            "revision": 2,
            "correction_reason": "abstract-correction",
            "knowledge_time_ns": KNOWLEDGE,
            "invalidated_at_time": UTC_END,
        });
    })?;
    assert_rejection(parse_fixture(&fixture));
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        value["lineage"]["revision"] = json!(2);
        value["lineage"]["supersedes"] = json!({
            "supersedes_release_id": "rel-canonical",
            "supersedes_manifest_sha256": DIGEST,
            "supersedes_data_sha256": OTHER_DIGEST,
            "revision": 1,
            "correction_reason": "abstract-correction",
            "knowledge_time_ns": KNOWLEDGE,
            "invalidated_at_time": UTC_END,
        });
    })?;
    assert_rejection(parse_fixture(&fixture));
    Ok(())
}

#[test]
fn unregistered_compression_pair_is_refused() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        value["artifacts"][0]["compression"] = Value::String("gzip".to_owned());
        value["artifacts"][0]["compression_version"] = Value::String("none/1".to_owned());
    })?;
    assert_rejection(parse_fixture(&fixture));
    Ok(())
}

#[test]
fn payload_registry_rejects_duplicate_event_type() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.mutate_manifest(|value| {
        let entry = value["schema_versions"]["payload_schemas"][0].clone();
        if let Some(array) = value["schema_versions"]["payload_schemas"].as_array_mut() {
            array.push(entry);
        }
    })?;
    assert_rejection(parse_fixture(&fixture));
    Ok(())
}

#[test]
fn executable_diagnostic_records_rights_refusal_without_receipt_or_snapshot()
-> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("none")?;
    let (output, out, tmpdir) = fixture.run_binary()?;
    if !output.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    }
    assert!(
        output.status.success(),
        "conforming rights refusal requires exit success"
    );
    let report: Value = serde_json::from_slice(&fs::read(&out)?)?;
    assert_eq!(
        report["outcome"], "RIGHTS_ADMISSION_REFUSED",
        "expected outcome"
    );
    assert_eq!(
        report["refusal"]["stage"], 6,
        "refusal must occur at rights admission"
    );
    assert_eq!(
        report["refusal"]["code"], "REJECT_RIGHTS_EVIDENCE",
        "typed refusal code"
    );
    assert_eq!(
        report["stages"][0]["status"], "PASS",
        "structural stage passes"
    );
    assert_eq!(
        report["stages"][4]["status"], "PASS",
        "artifact and ordering stage passes"
    );
    assert_eq!(
        report["stages"][6]["status"], "NOT_RUN",
        "later stages do not run"
    );
    assert!(
        report["admission_receipt"].is_null(),
        "no admission receipt is produced"
    );
    assert_eq!(
        report["private_snapshot_survives"], false,
        "private snapshot is discarded"
    );
    assert_eq!(
        report["cleanup"]["cleanup_succeeded"], true,
        "observed cleanup completed"
    );
    assert_eq!(
        report["cleanup"]["cleanup_pending"], false,
        "no cleanup was left pending"
    );
    assert_eq!(
        report["cleanup"]["directory_empty"], true,
        "final enumeration was empty"
    );
    assert_eq!(
        report["cleanup"]["directory_sync_completed"], true,
        "directory sync followed deletion"
    );
    assert_eq!(report["receipt_written"], false, "no receipt was written");
    assert_eq!(report["rights_state"], "UNKNOWN", "rights remain unknown");
    assert_eq!(report["dataset_accepted"], false, "no dataset acceptance");
    let survivors: Vec<PathBuf> = fs::read_dir(&tmpdir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("ndc-conformance-private-"))
        })
        .collect();
    assert!(
        survivors.is_empty(),
        "no private staging directory survives verification"
    );
    let receipts: Vec<PathBuf> = fs::read_dir(fixture.temp.path())?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.to_string_lossy().contains("receipt"))
        .collect();
    assert!(receipts.is_empty(), "no receipt file is written");
    Ok(())
}

#[test]
fn gzip_artifact_reaches_the_same_rights_refusal() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new("gzip")?;
    let (output, out, _) = fixture.run_binary()?;
    if !output.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    }
    assert!(
        output.status.success(),
        "conforming rights refusal requires exit success"
    );
    let report: Value = serde_json::from_slice(&fs::read(&out)?)?;
    assert_eq!(
        report["outcome"], "RIGHTS_ADMISSION_REFUSED",
        "gzip artifact outcome"
    );
    assert_eq!(
        report["stages"][4]["status"], "PASS",
        "gzip artifact stage passes"
    );
    Ok(())
}

#[test]
fn executable_refuses_reordered_record_bytes() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new("none")?;
    fixture.set_records(
        &[
            record_json("3", "abstract-three"),
            record_json("1", "abstract-one"),
        ],
        "none",
    )?;
    let (output, out, _) = fixture.run_binary()?;
    assert!(
        !output.status.success(),
        "descending record order must fail the executable"
    );
    let report: Value = serde_json::from_slice(&fs::read(&out)?)?;
    assert_eq!(
        report["refusal"]["stage"], 5,
        "ordering refusal is inside stage five"
    );
    assert_eq!(
        report["refusal"]["code"], "REJECT_ARTIFACT_DIGESTS",
        "ordering refusal code"
    );
    Ok(())
}
