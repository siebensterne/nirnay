// SPDX-License-Identifier: Apache-2.0
//! Golden-fixture conformance for the v1 source-event envelope.
//!
//! Every fixture is read byte-for-byte from the shared canonical fixture set
//! and checked against its recorded SHA-256 before use. Events are applied in
//! fixture order through the independent validator and a single sequence
//! ledger; each `expect` entry must match exactly one outcome, either
//! acceptance or the normative v1 rejection code.
//!
//! Conformance evidence only: no fixture, digest or test result admits data,
//! grants rights, contacts a provider, collects market data or authorizes live
//! execution. No positive production admission path is exercised here.

use release_verifier::ContinuityState;
use release_verifier::QualityState;
use release_verifier::SequenceLedger;
use release_verifier::SourceEvent;
use release_verifier::SourceEventError;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/contract_fixtures");
const MANIFEST_SHA256: &str = "b9066d915d77eb63b309e5f05eace4288e833754de2d731ebf25538347a3a3fd";
const FIXTURE_COUNT: usize = 30;
const ACCEPTED: &str = "ACCEPTED";

fn fixture_path(name: &str) -> PathBuf {
    Path::new(FIXTURE_DIR).join(name)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(64);
    for byte in digest {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn load_manifest() -> Result<Value, Box<dyn std::error::Error>> {
    let path = fixture_path("manifest.json");
    let bytes = fs::read(&path)?;
    assert_eq!(
        sha256_hex(&bytes),
        MANIFEST_SHA256,
        "manifest bytes match the canonical fixture set"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

fn load_verified_fixture(meta: &Value) -> Result<(String, Value), Box<dyn std::error::Error>> {
    let id = meta["id"]
        .as_str()
        .ok_or("fixture entry has no string id")?
        .to_owned();
    let name = meta["path"]
        .as_str()
        .ok_or("fixture entry has no string path")?;
    let name = Path::new(name)
        .file_name()
        .ok_or("fixture path has no file name")?
        .to_str()
        .ok_or("fixture file name is not UTF-8")?;
    let bytes = fs::read(fixture_path(name))?;
    assert_eq!(
        sha256_hex(&bytes),
        meta["sha256"].as_str().ok_or("fixture has no sha256")?,
        "fixture {id} bytes match the canonical fixture set"
    );
    Ok((id, serde_json::from_slice(&bytes)?))
}

fn outcome<T>(result: Result<T, SourceEventError>) -> &'static str {
    match result {
        Ok(_) => ACCEPTED,
        Err(error) => error.normative_code(),
    }
}

fn check_expectation(
    id: &str,
    index: usize,
    expect: &Value,
    result: Result<[u8; 32], SourceEventError>,
) {
    let accepted = expect.get("accepted");
    let rejected_code = expect.get("rejected_code");
    assert!(
        accepted.is_some() ^ rejected_code.is_some(),
        "fixture {id} entry {index} has exactly one expectation kind"
    );
    if let Some(value) = accepted {
        assert_eq!(
            value,
            &Value::Bool(true),
            "fixture {id} entry {index} expects an explicit accepted:true"
        );
        assert_eq!(
            outcome(result),
            ACCEPTED,
            "fixture {id} entry {index} must be accepted"
        );
    } else if let Some(value) = rejected_code {
        let expected = value.as_str().unwrap_or("not a string rejection code");
        assert_eq!(
            outcome(result),
            expected,
            "fixture {id} entry {index} must carry the exact normative code"
        );
    }
}

fn run_fixture(id: &str, fixture: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let mode = fixture["mode"]
        .as_str()
        .ok_or("fixture has no string mode")?;
    let expects = fixture["expect"]
        .as_array()
        .ok_or("fixture expect is not an array")?;
    let members: Vec<&Value> = match mode {
        "events" => fixture["events"]
            .as_array()
            .ok_or("fixture events is not an array")?
            .iter()
            .collect(),
        "raw" => fixture["raw_events"]
            .as_array()
            .ok_or("fixture raw_events is not an array")?
            .iter()
            .collect(),
        other => return Err(format!("fixture {id} has unknown mode {other:?}").into()),
    };
    assert_eq!(
        members.len(),
        expects.len(),
        "fixture {id} has one expectation per event"
    );

    let mut ledger = SequenceLedger::default();
    for (index, (member, expect)) in members.iter().zip(expects.iter()).enumerate() {
        let bytes = match mode {
            "raw" => member
                .as_str()
                .ok_or("raw fixture member is not a JSON string")?
                .as_bytes()
                .to_vec(),
            _ => serde_json::to_vec(member)?,
        };
        // The public ledger API owns admission; no pre-check wrapper here.
        let result = SourceEvent::parse(&bytes).and_then(|event| ledger.observe(&event));
        check_expectation(id, index, expect, result);
    }
    Ok(())
}

fn event_with_sequences(capture_sequence: u64, canonical_sequence: u64) -> SourceEvent {
    let mut event = SourceEvent {
        schema_version: 1,
        source_id: "CONFORMANCE-SOURCE".to_owned(),
        source_product_ref: "CONFORMANCE-PRODUCT".to_owned(),
        session_ref: "CONFORMANCE-SESSION".to_owned(),
        connection_epoch: 1,
        capture_sequence,
        canonical_sequence,
        provider_sequence: None,
        source_timestamp_ns: None,
        receive_utc_ns: 1_000,
        receive_monotonic_ns: 2_000,
        decoder_version: "CONFORMANCE-DECODER".to_owned(),
        reference_data_revision: "CONFORMANCE-REFERENCE".to_owned(),
        rights_evidence_ref: "CONFORMANCE-RIGHTS-REF-NOT-A-GRANT".to_owned(),
        raw_frame_digest: "a".repeat(64),
        quality_state: QualityState::Captured,
        continuity_state: ContinuityState::Continuous,
        envelope_digest: "0".repeat(64),
    };
    let _ = event.finalize_digest();
    event
}

#[test]
fn canonical_fixture_copies_are_byte_identical() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = load_manifest()?;
    let files = manifest["files"]
        .as_array()
        .ok_or("manifest files is not an array")?;
    assert_eq!(files.len(), FIXTURE_COUNT, "all 30 fixtures are recorded");
    for meta in files {
        let (id, fixture) = load_verified_fixture(meta)?;
        assert_eq!(
            fixture["id"].as_str().ok_or("fixture id missing")?,
            id,
            "fixture id matches its file name"
        );
    }
    Ok(())
}

#[test]
fn every_golden_fixture_matches_its_expectations() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = load_manifest()?;
    let files = manifest["files"]
        .as_array()
        .ok_or("manifest files is not an array")?;
    assert_eq!(files.len(), FIXTURE_COUNT, "all 30 fixtures are recorded");
    for meta in files {
        let (id, fixture) = load_verified_fixture(meta)?;
        run_fixture(&id, &fixture)?;
    }
    Ok(())
}

#[test]
fn sequence_ledger_observe_enforces_admission_on_the_ledger_fixture()
-> Result<(), Box<dyn std::error::Error>> {
    let manifest = load_manifest()?;
    let files = manifest["files"]
        .as_array()
        .ok_or("manifest files is not an array")?;
    let meta = files
        .iter()
        .find(|meta| meta["id"].as_str() == Some("P13-LEDGER-QUARANTINE"))
        .ok_or("ledger fixture is missing from the manifest")?;
    let (id, fixture) = load_verified_fixture(meta)?;
    let events = fixture["events"]
        .as_array()
        .ok_or("ledger fixture events is not an array")?;
    let expects = fixture["expect"]
        .as_array()
        .ok_or("ledger fixture expect is not an array")?;
    assert_eq!(
        events.len(),
        expects.len(),
        "one expectation per ledger event"
    );
    let source_id = events[0]["source_id"]
        .as_str()
        .ok_or("ledger event has no source_id")?;
    let session_ref = events[0]["session_ref"]
        .as_str()
        .ok_or("ledger event has no session_ref")?;
    let mut ledger = SequenceLedger::default();

    for (index, (member, expect)) in events.iter().zip(expects.iter()).enumerate() {
        let event = SourceEvent::parse(&serde_json::to_vec(member)?)?;
        // Direct public-API call: no wrapper pre-checks admissibility.
        let result = ledger.observe(&event);
        match index {
            0 => {
                assert!(
                    event.validate().is_ok(),
                    "the quarantined envelope is structurally valid"
                );
                assert_eq!(
                    outcome(result.clone()),
                    "QUARANTINED_NOT_ADMISSIBLE",
                    "the ledger itself refuses a quarantined event"
                );
                assert_eq!(
                    ledger.watermark(source_id, session_ref),
                    None,
                    "the refusal created no watermark"
                );
                assert_eq!(ledger.session_count(), 0, "the refusal created no scope");
            }
            1 => {
                assert_eq!(
                    outcome(result.clone()),
                    ACCEPTED,
                    "the same sequence is admissible once the event is not quarantined"
                );
                assert_eq!(
                    ledger.watermark(source_id, session_ref),
                    Some((1, 1)),
                    "the accepted event established the watermark"
                );
            }
            2 => {
                assert_eq!(
                    outcome(result.clone()),
                    "QUARANTINED_NOT_ADMISSIBLE",
                    "a later quarantined event is also refused"
                );
                assert_eq!(
                    ledger.watermark(source_id, session_ref),
                    Some((1, 1)),
                    "the refusal advanced no watermark"
                );
            }
            3 => {
                assert_eq!(
                    outcome(result.clone()),
                    ACCEPTED,
                    "sequence two is accepted"
                );
                assert_eq!(
                    ledger.watermark(source_id, session_ref),
                    Some((2, 2)),
                    "the refusal consumed neither sequence"
                );
            }
            _ => {}
        }
        check_expectation(&id, index, expect, result);
    }
    Ok(())
}

#[test]
fn zero_sequences_are_refused_before_a_watermark_exists() {
    let mut ledger = SequenceLedger::default();
    let zero_capture = event_with_sequences(0, 1);
    assert_eq!(
        outcome(ledger.observe(&zero_capture)),
        "NON_MONOTONIC_CAPTURE_SEQUENCE",
        "zero capture sequence is a non-monotonic refusal"
    );
    let zero_canonical = event_with_sequences(1, 0);
    assert_eq!(
        outcome(ledger.observe(&zero_canonical)),
        "NON_MONOTONIC_CANONICAL_SEQUENCE",
        "zero canonical sequence is a non-monotonic refusal"
    );
    assert_eq!(
        ledger.watermark("CONFORMANCE-SOURCE", "CONFORMANCE-SESSION"),
        None,
        "no refusal established a watermark"
    );
    assert_eq!(ledger.session_count(), 0, "no scope was created");
    let valid = event_with_sequences(1, 1);
    assert_eq!(
        outcome(ledger.observe(&valid)),
        ACCEPTED,
        "a strictly positive first event is accepted"
    );
}

#[test]
fn repeated_and_decreasing_sequences_do_not_advance_the_watermark() {
    let mut ledger = SequenceLedger::default();
    assert_eq!(
        outcome(ledger.observe(&event_with_sequences(2, 3))),
        ACCEPTED,
        "first event establishes the watermark"
    );
    assert_eq!(
        outcome(ledger.observe(&event_with_sequences(2, 4))),
        "NON_MONOTONIC_CAPTURE_SEQUENCE",
        "a repeated capture sequence is refused"
    );
    assert_eq!(
        outcome(ledger.observe(&event_with_sequences(3, 2))),
        "NON_MONOTONIC_CANONICAL_SEQUENCE",
        "a decreasing canonical sequence is refused"
    );
    assert_eq!(
        ledger.watermark("CONFORMANCE-SOURCE", "CONFORMANCE-SESSION"),
        Some((2, 3)),
        "refusals consumed no sequence"
    );
    assert_eq!(
        outcome(ledger.observe(&event_with_sequences(3, 4))),
        ACCEPTED,
        "the next strictly increasing event is accepted"
    );
}

#[test]
fn two_sources_sharing_a_session_have_independent_histories() {
    let mut ledger = SequenceLedger::default();
    let mut other = event_with_sequences(1, 1);
    other.source_id = "CONFORMANCE-SOURCE-B".to_owned();
    let _ = other.finalize_digest();

    assert_eq!(
        outcome(ledger.observe(&event_with_sequences(1, 1))),
        ACCEPTED,
        "first source establishes its own watermark"
    );
    assert_eq!(
        outcome(ledger.observe(&other)),
        ACCEPTED,
        "a second source may reuse the same sequences in the same session"
    );
    assert_eq!(
        ledger.watermark("CONFORMANCE-SOURCE", "CONFORMANCE-SESSION"),
        Some((1, 1)),
        "the first source has its own watermark"
    );
    assert_eq!(
        ledger.watermark("CONFORMANCE-SOURCE-B", "CONFORMANCE-SESSION"),
        Some((1, 1)),
        "the second source has an independent watermark"
    );
    assert_eq!(ledger.session_count(), 2, "two scopes are recorded");

    let mut repeated = other.clone();
    repeated.capture_sequence = 1;
    repeated.canonical_sequence = 2;
    let _ = repeated.finalize_digest();
    assert_eq!(
        outcome(ledger.observe(&repeated)),
        "NON_MONOTONIC_CAPTURE_SEQUENCE",
        "one source's refusal is scoped to that source only"
    );
    assert_eq!(
        ledger.watermark("CONFORMANCE-SOURCE-B", "CONFORMANCE-SESSION"),
        Some((1, 1)),
        "the other source's watermark is untouched"
    );
    assert_eq!(
        outcome(ledger.observe(&event_with_sequences(2, 2))),
        ACCEPTED,
        "the first source advances independently"
    );
}

#[test]
fn whitespace_only_required_strings_are_refused() {
    for field in [
        "source_id",
        "source_product_ref",
        "session_ref",
        "decoder_version",
        "reference_data_revision",
        "rights_evidence_ref",
    ] {
        let mut event = event_with_sequences(1, 1);
        let blank = "   ".to_owned();
        match field {
            "source_id" => event.source_id = blank,
            "source_product_ref" => event.source_product_ref = blank,
            "session_ref" => event.session_ref = blank,
            "decoder_version" => event.decoder_version = blank,
            "reference_data_revision" => event.reference_data_revision = blank,
            "rights_evidence_ref" => event.rights_evidence_ref = blank,
            _ => {}
        }
        assert_eq!(
            outcome(event.validate()),
            "EMPTY_REQUIRED_STRING",
            "whitespace-only {field} is an empty required string"
        );
    }
}

#[test]
fn duplicate_member_fixture_is_refused_before_digest_recomputation() {
    let raw = fixture_path("NEG-009.json");
    let bytes = fs::read(raw).unwrap_or_default();
    let fixture: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let text = fixture["raw_events"][0]
        .as_str()
        .unwrap_or("not a raw JSON string");
    assert_eq!(
        outcome(SourceEvent::parse(text.as_bytes())),
        "UNKNOWN_OR_MISSING_MEMBER",
        "a duplicate member is refused before the matching digest is examined"
    );
}

#[test]
fn normative_codes_are_stable_for_identical_conditions() {
    let mut event = event_with_sequences(1, 1);
    event.schema_version = 2;
    assert_eq!(
        outcome(event.validate()),
        "SCHEMA_VERSION_MISMATCH",
        "schema mismatch uses the normative v1 code"
    );
    let mut quarantined = event_with_sequences(1, 1);
    quarantined.quality_state = QualityState::Quarantined;
    let _ = quarantined.finalize_digest();
    assert_eq!(
        outcome(quarantined.validate_admissible()),
        "QUARANTINED_NOT_ADMISSIBLE",
        "a quarantined envelope is never admissible"
    );
    assert_eq!(
        outcome(quarantined.validate()),
        ACCEPTED,
        "quarantine is a structural-valid record, refused only at admission"
    );
}

fn valid_fixture_event() -> Result<Value, Box<dyn std::error::Error>> {
    let bytes = fs::read(fixture_path("VAL-001.json"))?;
    assert_eq!(
        sha256_hex(&bytes),
        "9ced8f95f6ffc3ff3ac9940a85a3bd6d791315b748c1791b467469f3fe4601c7",
        "VAL-001 bytes match the canonical fixture"
    );
    let fixture: Value = serde_json::from_slice(&bytes)?;
    Ok(fixture["events"][0].clone())
}

#[test]
fn out_of_domain_numbers_use_field_specific_normative_codes()
-> Result<(), Box<dyn std::error::Error>> {
    let base = valid_fixture_event()?;
    let negative = Value::from(-1_i64);
    let too_large: Value = serde_json::from_str("18446744073709551616")?;

    for (field, code) in [
        ("connection_epoch", "INVALID_CONNECTION_EPOCH"),
        ("provider_sequence", "INVALID_PROVIDER_SEQUENCE"),
        ("source_timestamp_ns", "INVALID_SOURCE_TIMESTAMP"),
    ] {
        for out_of_domain in [&negative, &too_large] {
            let mut member = base.clone();
            member[field] = out_of_domain.clone();
            let bytes = serde_json::to_vec(&member)?;
            assert_eq!(
                outcome(SourceEvent::parse(&bytes)),
                code,
                "{field} outside the u64-or-null domain maps to {code}"
            );
        }
    }
    Ok(())
}

#[test]
fn zero_and_null_domain_values_keep_their_existing_codes() -> Result<(), Box<dyn std::error::Error>>
{
    let base = valid_fixture_event()?;
    let mut zero_epoch = base.clone();
    zero_epoch["connection_epoch"] = Value::from(0_u64);
    let zero_epoch = SourceEvent::parse(&serde_json::to_vec(&zero_epoch)?)?;
    assert_eq!(
        outcome(zero_epoch.validate()),
        "INVALID_CONNECTION_EPOCH",
        "a zero connection epoch keeps its field-specific code"
    );

    let mut zero_provider = base.clone();
    zero_provider["provider_sequence"] = Value::from(0_u64);
    let zero_provider = SourceEvent::parse(&serde_json::to_vec(&zero_provider)?)?;
    assert_eq!(
        outcome(zero_provider.validate()),
        "INVALID_PROVIDER_SEQUENCE",
        "a present zero provider sequence keeps its field-specific code"
    );

    let mut null_timestamp = base.clone();
    null_timestamp["source_timestamp_ns"] = Value::Null;
    let null_timestamp = SourceEvent::parse(&serde_json::to_vec(&null_timestamp)?)?;
    assert_eq!(
        outcome(null_timestamp.validate()),
        ACCEPTED,
        "an absent source timestamp stays null and is never synthesized"
    );
    Ok(())
}
