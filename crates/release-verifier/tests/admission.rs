// SPDX-License-Identifier: Apache-2.0
//! Admission and receipt contract tests for one source event.
//!
//! Rights remain UNKNOWN and dataset acceptance remains BLOCKED. Every case
//! below is a refusal, and a counting sink proves that no downstream consumer
//! is reached. No positive admission path is exposed to integration callers.

use release_verifier::AdmissionError;
use release_verifier::AdmissionInput;
use release_verifier::AdmissionReceipt;
use release_verifier::ContinuityState;
use release_verifier::DatasetAcceptance;
use release_verifier::QualityState;
use release_verifier::RightsVerdict;
use release_verifier::SourceEvent;
use release_verifier::SourceEventSink;
use release_verifier::admit;
use release_verifier::receipt::ArtifactBinding;
use release_verifier::receipt::ReceiptInput;
use release_verifier::schema::AdmissionRequest;
use release_verifier::schema::CoverageDecision;
use release_verifier::schema::Decision;
use release_verifier::schema::RetentionPolicy;
use release_verifier::schema::RightsDecision;
use release_verifier::schema::ScopeDeclaration;
use std::cell::Cell;

#[derive(Debug, Default)]
struct CountingSink {
    calls: Cell<usize>,
}

impl SourceEventSink for CountingSink {
    fn consume(&self, _event: &SourceEvent) {
        self.calls.set(self.calls.get() + 1);
    }
}

fn valid_event() -> SourceEvent {
    SourceEvent {
        schema_version: 1,
        source_id: "source-1".to_owned(),
        source_product_ref: "product-1".to_owned(),
        session_ref: "session-1".to_owned(),
        connection_epoch: 1,
        capture_sequence: 10,
        canonical_sequence: 20,
        provider_sequence: None,
        source_timestamp_ns: None,
        receive_utc_ns: 1_000,
        receive_monotonic_ns: 2_000,
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

fn receipt(use_profile: &str) -> AdmissionReceipt {
    let input = ReceiptInput {
        release_id: "release-1".to_owned(),
        manifest_sha256: "a".repeat(64),
        data_sha256: "b".repeat(64),
        publisher_id: "publisher-1".to_owned(),
        key_id: "key-1".to_owned(),
        trust_registry_revision: "trust-1".to_owned(),
        contract_version: "v1".to_owned(),
        contract_revision: "r1".to_owned(),
        verification_policy_version: "policy-1".to_owned(),
        artifact_set: vec![ArtifactBinding {
            path: "artifact.json".to_owned(),
            logical_partition_id: "partition-1".to_owned(),
            shard_ordinal: 0,
            stored_sha256: "c".repeat(64),
            uncompressed_sha256: "d".repeat(64),
            stored_bytes: 1,
            uncompressed_bytes: 1,
            record_count: 1,
        }],
        rights_decision: RightsDecision {
            decision: Decision::Unknown,
            matching_grant_ids: Vec::new(),
        },
        coverage_decision: CoverageDecision {
            approved_dimensions: Vec::new(),
            used_implicit_all: false,
        },
        admission_request: AdmissionRequest {
            release_id: "release-1".to_owned(),
            manifest_sha256: "a".repeat(64),
            data_sha256: "b".repeat(64),
            publisher_identity: "publisher-1".to_owned(),
            actor: "consumer-1".to_owned(),
            use_profile: use_profile.to_owned(),
            requested_actions: vec!["consume".to_owned()],
            purpose: "contract-test".to_owned(),
            licensee: "licensee-1".to_owned(),
            recipient_class: "test".to_owned(),
            recipient_identity: "recipient-1".to_owned(),
            scope: ScopeDeclaration {
                provider_id: "provider-1".to_owned(),
                source_product_id: "product-1".to_owned(),
                venue_id: "venue-1".to_owned(),
                segment_id: "segment-1".to_owned(),
                subject_state: "NOT_APPLICABLE".to_owned(),
                named_subjects: Vec::new(),
                data_time_from_ns: None,
                data_time_to_ns: None,
            },
            requested_retention: RetentionPolicy {
                max_duration_ns: 1,
                purge_rule: "purge-after-contract-test".to_owned(),
            },
            as_of_ns: 1,
            declared_capabilities: Vec::new(),
        },
        consumer_use_profile: use_profile.to_owned(),
        verifier_commit: "test-commit".to_owned(),
    };
    match AdmissionReceipt::new(input) {
        Ok(built) => built,
        Err(_) => panic!("test receipt must be constructible"),
    }
}

fn input_for(event: SourceEvent, expected_receipt: &AdmissionReceipt) -> AdmissionInput<'_> {
    let digest = event.envelope_sha256().unwrap_or([0_u8; 32]);
    AdmissionInput {
        envelope: Some(event),
        receipt: expected_receipt,
        expected_receipt_digest: *expected_receipt.receipt_digest(),
        expected_envelope_digest: digest,
        expected_reference_revision: "ref-1",
        rights: RightsVerdict::Unknown,
        dataset_acceptance: DatasetAcceptance::Blocked,
    }
}

fn refusal(input: AdmissionInput<'_>, sink: &CountingSink) -> Option<AdmissionError> {
    admit(input, sink).err()
}

#[test]
fn missing_envelope_is_refused_without_consumption() {
    let receipt = receipt("test-profile");
    let sink = CountingSink::default();
    let mut input = input_for(valid_event(), &receipt);
    input.envelope = None;
    assert_eq!(
        refusal(input, &sink),
        Some(AdmissionError::EnvelopeMissing),
        "a missing envelope is refused"
    );
    assert_eq!(sink.calls.get(), 0, "missing envelope reaches no consumer");
}

#[test]
fn altered_envelope_is_refused_without_consumption() {
    let receipt = receipt("test-profile");
    let sink = CountingSink::default();
    let mut event = valid_event();
    let digest = event.finalize_digest().unwrap_or([0_u8; 32]);
    event.source_id = "tampered-source".to_owned();
    let mut input = input_for(event, &receipt);
    input.expected_envelope_digest = digest;
    assert_eq!(
        refusal(input, &sink),
        Some(AdmissionError::EnvelopeInvalid {
            code: "ENVELOPE_DIGEST_MISMATCH",
        }),
        "an altered envelope is refused by recomputation"
    );
    assert_eq!(sink.calls.get(), 0, "altered envelope reaches no consumer");
}

#[test]
fn envelope_digest_binding_mismatch_is_refused() {
    let receipt = receipt("test-profile");
    let sink = CountingSink::default();
    let mut event = valid_event();
    event.finalize_digest().unwrap_or([0_u8; 32]);
    let mut input = input_for(event, &receipt);
    input.expected_envelope_digest = [0_u8; 32];
    assert_eq!(
        refusal(input, &sink),
        Some(AdmissionError::EnvelopeDigestMismatch),
        "a trusted digest different from the envelope binding is refused"
    );
    assert_eq!(sink.calls.get(), 0, "unbound envelope reaches no consumer");
}

#[test]
fn tampered_receipt_is_refused() {
    let trusted = receipt("test-profile");
    let tampered = receipt("tampered-profile");
    let sink = CountingSink::default();
    let mut event = valid_event();
    event.finalize_digest().unwrap_or([0_u8; 32]);
    let mut input = input_for(event, &trusted);
    input.receipt = &tampered;
    assert_eq!(
        refusal(input, &sink),
        Some(AdmissionError::ReceiptDigestMismatch),
        "a receipt whose digest differs from the trusted binding is refused"
    );
    assert_eq!(sink.calls.get(), 0, "tampered receipt reaches no consumer");
}

#[test]
fn rights_unknown_is_refused() {
    let receipt = receipt("test-profile");
    let sink = CountingSink::default();
    let mut event = valid_event();
    event.finalize_digest().unwrap_or([0_u8; 32]);
    for rights in [RightsVerdict::Unknown, RightsVerdict::Denied] {
        let mut input = input_for(event.clone(), &receipt);
        input.rights = rights;
        assert_eq!(
            refusal(input, &sink),
            Some(AdmissionError::RightsNotAdmitted),
            "rights {rights:?} cannot admit an event"
        );
    }
    assert_eq!(sink.calls.get(), 0, "unadmitted rights reach no consumer");
}

#[test]
fn stale_reference_revision_is_refused() {
    let receipt = receipt("test-profile");
    let sink = CountingSink::default();
    let mut event = valid_event();
    event.finalize_digest().unwrap_or([0_u8; 32]);
    let mut input = input_for(event, &receipt);
    input.expected_reference_revision = "ref-2";
    assert_eq!(
        refusal(input, &sink),
        Some(AdmissionError::ReferenceRevisionStale),
        "a non-accepted reference revision is refused"
    );
    assert_eq!(sink.calls.get(), 0, "stale reference reaches no consumer");
}

#[test]
fn gap_suspected_and_unknown_continuity_are_refused() {
    let receipt = receipt("test-profile");
    let sink = CountingSink::default();
    for (state, expected) in [
        (
            ContinuityState::GapSuspected,
            AdmissionError::ContinuityGapSuspected,
        ),
        (ContinuityState::Unknown, AdmissionError::ContinuityUnknown),
    ] {
        let mut event = valid_event();
        event.continuity_state = state;
        event.finalize_digest().unwrap_or([0_u8; 32]);
        assert_eq!(
            refusal(input_for(event, &receipt), &sink),
            Some(expected),
            "continuity {state:?} cannot admit an event"
        );
    }
    assert_eq!(
        sink.calls.get(),
        0,
        "unresolved continuity reaches no consumer"
    );
}

#[test]
fn quarantine_is_refused() {
    let receipt = receipt("test-profile");
    let sink = CountingSink::default();
    let mut event = valid_event();
    event.quality_state = QualityState::Quarantined;
    event.finalize_digest().unwrap_or([0_u8; 32]);
    assert_eq!(
        refusal(input_for(event, &receipt), &sink),
        Some(AdmissionError::QualityQuarantined),
        "a quarantined event cannot be admitted"
    );
    assert_eq!(sink.calls.get(), 0, "quarantined event reaches no consumer");
}

#[test]
fn dataset_acceptance_blocked_is_refused_after_rights_check() {
    let receipt = receipt("test-profile");
    let sink = CountingSink::default();
    let mut event = valid_event();
    event.finalize_digest().unwrap_or([0_u8; 32]);
    let input = input_for(event, &receipt);
    assert_eq!(
        refusal(input, &sink),
        Some(AdmissionError::RightsNotAdmitted),
        "current rights are UNKNOWN and stop the real-world path first"
    );
    assert_eq!(
        DatasetAcceptance::Blocked,
        DatasetAcceptance::Blocked,
        "only the blocked acceptance state is available to integration callers"
    );
    assert_eq!(sink.calls.get(), 0, "blocked dataset reaches no consumer");
}
