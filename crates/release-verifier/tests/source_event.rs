// SPDX-License-Identifier: Apache-2.0
//! Independent tests for the v1 source-event envelope validator.
//!
//! All values are abstract identifiers, clocks and digests. No provider is
//! contacted, no collection runs and no market observation appears here. The
//! fixed digest vector is computed from the documented canonical byte string.

use release_verifier::ContinuityState;
use release_verifier::QualityState;
use release_verifier::SequenceLedger;
use release_verifier::SourceEvent;
use release_verifier::SourceEventError;
use std::fmt::Write as FmtWrite;

const GOLDEN_JSON: &str = concat!(
    r#"{"canonical_sequence":23,"capture_sequence":11,"connection_epoch":7,"#,
    r#""continuity_state":"CONTINUOUS","decoder_version":"decoder-1","#,
    r#""provider_sequence":null,"quality_state":"DECODED","#,
    r#""raw_frame_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","#,
    r#""receive_monotonic_ns":9000000000,"receive_utc_ns":1700000000000000000,"#,
    r#""reference_data_revision":"ref-1","rights_evidence_ref":"rights-1","#,
    r#""schema_version":1,"session_ref":"session-1","source_id":"source-1","#,
    r#""source_product_ref":"product-1","source_timestamp_ns":null}"#,
);
const GOLDEN_DIGEST: &str = "4c1d001f64bafd34b9dc98b8c0ce74de83ad58d4c88d01be7e36b5488e2e7f1f";

fn valid_event() -> SourceEvent {
    SourceEvent {
        schema_version: 1,
        source_id: "source-1".to_owned(),
        source_product_ref: "product-1".to_owned(),
        session_ref: "session-1".to_owned(),
        connection_epoch: 7,
        capture_sequence: 11,
        canonical_sequence: 23,
        provider_sequence: None,
        source_timestamp_ns: None,
        receive_utc_ns: 1_700_000_000_000_000_000,
        receive_monotonic_ns: 9_000_000_000,
        decoder_version: "decoder-1".to_owned(),
        reference_data_revision: "ref-1".to_owned(),
        rights_evidence_ref: "rights-1".to_owned(),
        raw_frame_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .to_owned(),
        quality_state: QualityState::Decoded,
        continuity_state: ContinuityState::Continuous,
        envelope_digest: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            .to_owned(),
    }
}

fn rejection_code<T>(result: Result<T, SourceEventError>) -> &'static str {
    match result {
        Ok(_) => "ACCEPTED",
        Err(error) => error.code(),
    }
}

fn hex_digest(digest: &[u8; 32]) -> String {
    let mut text = String::with_capacity(64);
    for byte in digest {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[test]
fn canonical_bytes_and_digest_match_the_fixed_vector() {
    let event = valid_event();
    let bytes = event.canonical_bytes().unwrap_or_default();
    let text = String::from_utf8(bytes).unwrap_or_default();
    assert_eq!(
        text, GOLDEN_JSON,
        "canonical keys are sorted with explicit nulls"
    );
    assert_eq!(
        hex_digest(&event.envelope_sha256().unwrap_or([0_u8; 32])),
        GOLDEN_DIGEST,
        "canonical JSON digest matches the independent vector"
    );
}

#[test]
fn valid_envelope_validates_without_inventing_optional_values() {
    let mut event = valid_event();
    let digest = event.finalize_digest().unwrap_or([0_u8; 32]);
    let computed = event.validate().unwrap_or([1_u8; 32]);
    assert_eq!(
        computed, digest,
        "recomputed digest matches the finalized binding"
    );
    assert_eq!(
        event.provider_sequence, None,
        "absent provider sequence stays null"
    );
    assert_eq!(
        event.source_timestamp_ns, None,
        "absent source timestamp stays null"
    );
}

#[test]
fn wrong_schema_version_is_refused() {
    let mut event = valid_event();
    event.finalize_digest().unwrap_or([0_u8; 32]);
    event.schema_version = 2;
    assert_eq!(
        rejection_code(event.validate()),
        "SOURCE_SCHEMA_VERSION",
        "a non-v1 schema version is refused"
    );
}

#[test]
fn every_empty_required_field_is_refused() {
    for field in [
        "source_id",
        "source_product_ref",
        "session_ref",
        "decoder_version",
        "reference_data_revision",
        "rights_evidence_ref",
    ] {
        let mut event = valid_event();
        match field {
            "source_id" => event.source_id.clear(),
            "source_product_ref" => event.source_product_ref.clear(),
            "session_ref" => event.session_ref.clear(),
            "decoder_version" => event.decoder_version.clear(),
            "reference_data_revision" => event.reference_data_revision.clear(),
            "rights_evidence_ref" => event.rights_evidence_ref.clear(),
            _ => {}
        }
        assert_eq!(
            rejection_code(event.validate()),
            "SOURCE_FIELD_EMPTY",
            "empty {field} is refused"
        );
    }
}

#[test]
fn zero_clocks_and_zero_provider_sequence_are_refused() {
    let mut event = valid_event();
    event.connection_epoch = 0;
    assert_eq!(
        rejection_code(event.validate()),
        "CONNECTION_EPOCH_INVALID",
        "zero connection epoch is refused"
    );
    let mut event = valid_event();
    event.receive_utc_ns = 0;
    assert_eq!(
        rejection_code(event.validate()),
        "RECEIVE_UTC_INVALID",
        "zero epoch receive time is refused"
    );
    let mut event = valid_event();
    event.receive_monotonic_ns = 0;
    assert_eq!(
        rejection_code(event.validate()),
        "RECEIVE_MONOTONIC_INVALID",
        "zero monotonic receive time is refused"
    );
    let mut event = valid_event();
    event.provider_sequence = Some(0);
    assert_eq!(
        rejection_code(event.validate()),
        "PROVIDER_SEQUENCE_ZERO",
        "a present zero provider sequence is refused"
    );
}

#[test]
fn malformed_digests_are_refused() {
    let mut event = valid_event();
    event.raw_frame_digest = "A".repeat(64);
    assert_eq!(
        rejection_code(event.validate()),
        "DIGEST_MALFORMED",
        "uppercase hex digest is refused"
    );
    let mut event = valid_event();
    event.envelope_digest = "ab".to_owned();
    assert_eq!(
        rejection_code(event.validate()),
        "DIGEST_MALFORMED",
        "short envelope digest is refused"
    );
}

#[test]
fn altered_field_is_a_digest_mismatch() {
    let mut event = valid_event();
    event.finalize_digest().unwrap_or([0_u8; 32]);
    event.source_id = "source-2".to_owned();
    assert_eq!(
        rejection_code(event.validate()),
        "ENVELOPE_DIGEST_MISMATCH",
        "a changed field invalidates the declared digest"
    );
}

#[test]
fn validated_requires_resolved_continuity() {
    for state in [ContinuityState::GapSuspected, ContinuityState::Unknown] {
        let mut event = valid_event();
        event.quality_state = QualityState::Validated;
        event.continuity_state = state;
        event.finalize_digest().unwrap_or([0_u8; 32]);
        assert_eq!(
            rejection_code(event.validate()),
            "VALIDATED_CONTINUITY_INVALID",
            "VALIDATED cannot carry {state:?} continuity"
        );
    }
    let mut event = valid_event();
    event.quality_state = QualityState::Validated;
    event.finalize_digest().unwrap_or([0_u8; 32]);
    assert_eq!(
        rejection_code(event.validate()),
        "ACCEPTED",
        "VALIDATED with resolved continuity and revisions is structurally valid"
    );
    let mut event = valid_event();
    event.quality_state = QualityState::Quarantined;
    event.finalize_digest().unwrap_or([0_u8; 32]);
    assert_eq!(
        rejection_code(event.validate()),
        "ACCEPTED",
        "QUARANTINED is structurally valid but is refused at admission"
    );
}

#[test]
fn sequence_ledger_is_per_session_and_never_consumes_a_refusal() {
    let mut ledger = SequenceLedger::default();
    let mut first = valid_event();
    first.finalize_digest().unwrap_or([0_u8; 32]);
    assert_eq!(
        rejection_code(ledger.observe(&first)),
        "ACCEPTED",
        "first session event is accepted"
    );
    let mut repeated = first.clone();
    assert_eq!(
        rejection_code(ledger.observe(&repeated)),
        "CAPTURE_SEQUENCE_NOT_INCREASING",
        "equal capture sequence is refused"
    );
    repeated.capture_sequence = 10;
    repeated.finalize_digest().unwrap_or([0_u8; 32]);
    assert_eq!(
        rejection_code(ledger.observe(&repeated)),
        "CAPTURE_SEQUENCE_NOT_INCREASING",
        "decreasing capture sequence is refused"
    );
    let mut canonical_regression = first.clone();
    canonical_regression.capture_sequence = 12;
    canonical_regression.canonical_sequence = 22;
    canonical_regression.finalize_digest().unwrap_or([0_u8; 32]);
    assert_eq!(
        rejection_code(ledger.observe(&canonical_regression)),
        "CANONICAL_SEQUENCE_NOT_INCREASING",
        "decreasing canonical sequence is refused"
    );
    assert_eq!(
        ledger.session_count(),
        1,
        "refusals did not create sessions"
    );
    let mut other_session = valid_event();
    other_session.session_ref = "session-2".to_owned();
    other_session.finalize_digest().unwrap_or([0_u8; 32]);
    assert_eq!(
        rejection_code(ledger.observe(&other_session)),
        "ACCEPTED",
        "sequences are scoped per session"
    );
    assert_eq!(
        ledger.session_count(),
        2,
        "second session was recorded once"
    );
}

#[test]
fn parse_refuses_duplicate_and_unknown_members() -> Result<(), Box<dyn std::error::Error>> {
    let mut event = valid_event();
    event.finalize_digest()?;
    let text = serde_json::to_string(&event)?;
    let duplicate = text.replacen("\"source_id\"", "\"source_id\":\"other\",\"source_id\"", 1);
    let unknown = text.replacen('{', "{\"extra\":1,", 1);
    assert_eq!(
        rejection_code(SourceEvent::parse(duplicate.as_bytes())),
        "SOURCE_EVENT_PARSE",
        "a duplicate member is refused"
    );
    assert_eq!(
        rejection_code(SourceEvent::parse(unknown.as_bytes())),
        "SOURCE_EVENT_PARSE",
        "an unknown member is refused"
    );
    let parsed = SourceEvent::parse(text.as_bytes())?;
    assert_eq!(parsed, event, "round-trip preserves every envelope field");
    Ok(())
}
