// SPDX-License-Identifier: Apache-2.0
//! Consumer admission gate for one source event and its NDC receipt.
//!
//! A receipt records checks; it is not a right and not a dataset acceptance.
//! This gate admits only when the trusted receipt digest matches, the envelope
//! digest matches its trusted binding, continuity is resolved, quality is not
//! quarantined, rights are explicitly `ADMITTED` and dataset acceptance is
//! explicitly `ACCEPTED`. Current rights are `UNKNOWN` and dataset acceptance
//! is `BLOCKED`, so the production path always refuses with a typed reason.
//! The explicit positive states exist only in this crate's test builds; no
//! downstream consumer and no external caller can enable them.
//!
//! Every refusal is final for that event. No downstream consumer is invoked on
//! any refusal path and no partial state is consumed.

use crate::receipt::AdmissionReceipt;
use crate::source_event::ContinuityState;
use crate::source_event::QualityState;
use crate::source_event::SourceEvent;
use thiserror::Error;

/// Rights verdict supplied by the authoritative rights decision.
///
/// Only `Unknown` and `Denied` exist in production builds. The `Admitted`
/// variant is compiled solely for in-crate contract tests and cannot be
/// constructed by a downstream consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightsVerdict {
    /// Explicit admitted decision, available only to in-crate tests.
    #[cfg(test)]
    Admitted,
    /// No rights decision was established.
    Unknown,
    /// Rights were explicitly denied.
    Denied,
}

/// Accepted state of the dataset under the current programme decision.
///
/// Only `Blocked` exists in production builds. The `Accepted` variant is
/// compiled solely for in-crate contract tests and cannot be constructed by a
/// downstream consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetAcceptance {
    /// Explicit acceptance, available only to in-crate tests.
    #[cfg(test)]
    Accepted,
    /// Dataset acceptance is blocked.
    Blocked,
}

/// All trusted inputs for one admission decision.
///
/// The expected digests and revision are supplied out of band by the consumer;
/// they are never taken from the event being examined.
#[derive(Debug, Clone)]
pub struct AdmissionInput<'a> {
    /// Candidate envelope, absent when the expected event never arrived.
    pub envelope: Option<SourceEvent>,
    /// Verified NDC admission receipt.
    pub receipt: &'a AdmissionReceipt,
    /// Digest of that receipt from the trusted binding.
    pub expected_receipt_digest: [u8; 32],
    /// Envelope digest from the trusted binding.
    pub expected_envelope_digest: [u8; 32],
    /// Reference revision accepted by the consumer policy.
    pub expected_reference_revision: &'a str,
    /// Externally supplied rights decision.
    pub rights: RightsVerdict,
    /// Externally supplied dataset acceptance state.
    pub dataset_acceptance: DatasetAcceptance,
}

/// A downstream consumer that can receive an admitted event only.
pub trait SourceEventSink {
    /// Receive one fully admitted event. Called at most once per event.
    fn consume(&self, event: &SourceEvent);
}

/// A typed refusal from the admission gate with a stable reason code.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AdmissionError {
    /// The expected envelope is missing.
    #[error("ENVELOPE_MISSING: no source event envelope was supplied")]
    EnvelopeMissing,
    /// The envelope failed independent structural validation.
    #[error("ENVELOPE_INVALID: {code}")]
    EnvelopeInvalid {
        /// Stable code from the source-event validator.
        code: &'static str,
    },
    /// The recomputed envelope digest differs from the trusted binding.
    #[error("ENVELOPE_DIGEST_MISMATCH: envelope digest binding does not match")]
    EnvelopeDigestMismatch,
    /// The receipt digest differs from the trusted binding.
    #[error("RECEIPT_DIGEST_MISMATCH: NDC receipt digest binding does not match")]
    ReceiptDigestMismatch,
    /// The event uses a reference revision other than the accepted one.
    #[error("REFERENCE_REVISION_STALE: reference_data_revision is not the accepted revision")]
    ReferenceRevisionStale,
    /// Continuity is suspected to have a gap.
    #[error("CONTINUITY_GAP_SUSPECTED: continuity_state is GAP_SUSPECTED")]
    ContinuityGapSuspected,
    /// Continuity was never established.
    #[error("CONTINUITY_UNKNOWN: continuity_state is UNKNOWN")]
    ContinuityUnknown,
    /// The event is quarantined.
    #[error("QUALITY_QUARANTINED: quality_state is QUARANTINED")]
    QualityQuarantined,
    /// Rights are not an explicit ADMITTED decision.
    #[error("RIGHTS_NOT_ADMITTED: rights decision is not explicitly ADMITTED")]
    RightsNotAdmitted,
    /// Dataset acceptance is blocked.
    #[error("DATASET_NOT_ACCEPTED: dataset acceptance is BLOCKED")]
    DatasetNotAccepted,
}

impl AdmissionError {
    /// Return the stable reason code for this refusal.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::EnvelopeMissing => "ENVELOPE_MISSING",
            Self::EnvelopeInvalid { .. } => "ENVELOPE_INVALID",
            Self::EnvelopeDigestMismatch => "ENVELOPE_DIGEST_MISMATCH",
            Self::ReceiptDigestMismatch => "RECEIPT_DIGEST_MISMATCH",
            Self::ReferenceRevisionStale => "REFERENCE_REVISION_STALE",
            Self::ContinuityGapSuspected => "CONTINUITY_GAP_SUSPECTED",
            Self::ContinuityUnknown => "CONTINUITY_UNKNOWN",
            Self::QualityQuarantined => "QUALITY_QUARANTINED",
            Self::RightsNotAdmitted => "RIGHTS_NOT_ADMITTED",
            Self::DatasetNotAccepted => "DATASET_NOT_ACCEPTED",
        }
    }
}

/// Decide whether one source event may reach a downstream consumer.
///
/// Checks run in a fixed order and the first refusal stops processing. The
/// sink is reached only after every gate passes; a refusal leaves no partial
/// consumption behind.
pub fn admit(input: AdmissionInput<'_>, sink: &dyn SourceEventSink) -> Result<(), AdmissionError> {
    let AdmissionInput {
        envelope,
        receipt,
        expected_receipt_digest,
        expected_envelope_digest,
        expected_reference_revision,
        rights,
        dataset_acceptance,
    } = input;
    let Some(envelope) = envelope else {
        return Err(AdmissionError::EnvelopeMissing);
    };
    let computed = envelope
        .validate()
        .map_err(|error| AdmissionError::EnvelopeInvalid { code: error.code() })?;
    if computed != expected_envelope_digest {
        return Err(AdmissionError::EnvelopeDigestMismatch);
    }
    if *receipt.receipt_digest() != expected_receipt_digest {
        return Err(AdmissionError::ReceiptDigestMismatch);
    }
    if envelope.reference_data_revision != expected_reference_revision {
        return Err(AdmissionError::ReferenceRevisionStale);
    }
    match envelope.continuity_state {
        ContinuityState::GapSuspected => return Err(AdmissionError::ContinuityGapSuspected),
        ContinuityState::Unknown => return Err(AdmissionError::ContinuityUnknown),
        ContinuityState::Continuous | ContinuityState::Recovered => {}
    }
    if envelope.quality_state == QualityState::Quarantined {
        return Err(AdmissionError::QualityQuarantined);
    }
    let rights_admitted = match rights {
        #[cfg(test)]
        RightsVerdict::Admitted => true,
        RightsVerdict::Unknown | RightsVerdict::Denied => false,
    };
    if !rights_admitted {
        return Err(AdmissionError::RightsNotAdmitted);
    }
    let dataset_accepted = match dataset_acceptance {
        #[cfg(test)]
        DatasetAcceptance::Accepted => true,
        DatasetAcceptance::Blocked => false,
    };
    if !dataset_accepted {
        return Err(AdmissionError::DatasetNotAccepted);
    }
    sink.consume(&envelope);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::AdmissionError;
    use super::AdmissionInput;
    use super::DatasetAcceptance;
    use super::RightsVerdict;
    use super::SourceEventSink;
    use super::admit;
    use crate::receipt::AdmissionReceipt;
    use crate::receipt::ArtifactBinding;
    use crate::receipt::ReceiptInput;
    use crate::schema::AdmissionRequest;
    use crate::schema::CoverageDecision;
    use crate::schema::Decision;
    use crate::schema::RetentionPolicy;
    use crate::schema::RightsDecision;
    use crate::schema::ScopeDeclaration;
    use crate::source_event::ContinuityState;
    use crate::source_event::QualityState;
    use crate::source_event::SourceEvent;
    use std::cell::Cell;

    struct RecordingSink {
        calls: Cell<usize>,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self {
                calls: Cell::new(0),
            }
        }
    }

    impl SourceEventSink for RecordingSink {
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
            raw_frame_digest: "e".repeat(64),
            quality_state: QualityState::Decoded,
            continuity_state: ContinuityState::Continuous,
            envelope_digest: "f".repeat(64),
        }
    }

    fn test_receipt() -> AdmissionReceipt {
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
                use_profile: "test-profile".to_owned(),
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
            consumer_use_profile: "test-profile".to_owned(),
            verifier_commit: "test-commit".to_owned(),
        };
        match AdmissionReceipt::new(input) {
            Ok(receipt) => receipt,
            Err(_) => panic!("test receipt must be constructible"),
        }
    }

    fn tested_input(event: SourceEvent, receipt: &AdmissionReceipt) -> AdmissionInput<'_> {
        let digest = event.envelope_sha256().unwrap_or([0_u8; 32]);
        AdmissionInput {
            envelope: Some(event),
            receipt,
            expected_receipt_digest: *receipt.receipt_digest(),
            expected_envelope_digest: digest,
            expected_reference_revision: "ref-1",
            rights: RightsVerdict::Unknown,
            dataset_acceptance: DatasetAcceptance::Blocked,
        }
    }

    #[test]
    fn test_only_positive_path_consumes_once() {
        let mut event = valid_event();
        let digest = event.finalize_digest().unwrap_or([0_u8; 32]);
        let receipt = test_receipt();
        let sink = RecordingSink::new();
        let mut input = tested_input(event, &receipt);
        input.expected_envelope_digest = digest;
        input.rights = RightsVerdict::Admitted;
        input.dataset_acceptance = DatasetAcceptance::Accepted;
        let result = admit(input, &sink);
        assert_eq!(result, Ok(()), "fully satisfied gate admits once");
        assert_eq!(sink.calls.get(), 1, "admitted event reaches the sink once");
    }

    #[test]
    fn test_only_positive_path_still_refuses_quarantine() {
        let mut event = valid_event();
        event.quality_state = QualityState::Quarantined;
        event.finalize_digest().unwrap_or([0_u8; 32]);
        let receipt = test_receipt();
        let sink = RecordingSink::new();
        let mut input = tested_input(event, &receipt);
        input.rights = RightsVerdict::Admitted;
        input.dataset_acceptance = DatasetAcceptance::Accepted;
        assert_eq!(
            admit(input, &sink),
            Err(AdmissionError::QualityQuarantined),
            "quarantine overrides the test-only positive rights state"
        );
        assert_eq!(sink.calls.get(), 0, "quarantined event reaches no consumer");
    }
}
