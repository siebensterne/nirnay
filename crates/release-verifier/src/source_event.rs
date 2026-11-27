// SPDX-License-Identifier: Apache-2.0
//! Independent consumer-side validator for the v1 source-event envelope.
//!
//! This is a separate implementation. It imports no Fieldbook code, contacts no
//! provider, collects nothing and contains no market observation. The envelope
//! is abstract structured data: identities, revisions, clocks, digests and
//! states. A refusal is typed, carries a stable reason code and consumes
//! nothing; the sequence ledger is advanced only after a full acceptance.
//!
//! The envelope digest is SHA-256 over compact JSON with all keys sorted and
//! every field except `envelope_digest` present, including explicit nulls.
//! Digests are lowercase 64-hex SHA-256 values. Provider sequence is never
//! invented: an absent value stays null and a present zero is refused.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use thiserror::Error;

/// The only source-event envelope schema version accepted here.
pub const V1_SCHEMA_VERSION: u32 = 1;

/// Verified quality state of one source event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum QualityState {
    /// Captured but not decoded.
    Captured,
    /// Decoded but not validated against reference data.
    Decoded,
    /// Validated with decoder and reference revisions present.
    Validated,
    /// Quarantined; never admissible.
    Quarantined,
}

impl QualityState {
    /// Return the exact envelope string for this state.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Captured => "CAPTURED",
            Self::Decoded => "DECODED",
            Self::Validated => "VALIDATED",
            Self::Quarantined => "QUARANTINED",
        }
    }
}

/// Verified continuity state of one source event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContinuityState {
    /// No continuity gap is known.
    Continuous,
    /// A gap is suspected and the event cannot be relied upon.
    GapSuspected,
    /// A previously suspected gap was recovered.
    Recovered,
    /// Continuity was not established.
    Unknown,
}

impl ContinuityState {
    /// Return the exact envelope string for this state.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Continuous => "CONTINUOUS",
            Self::GapSuspected => "GAP_SUSPECTED",
            Self::Recovered => "RECOVERED",
            Self::Unknown => "UNKNOWN",
        }
    }
}

/// One v1 source-event envelope, owned by the consumer after independent
/// validation. No field is inferred, defaulted or synthesized by this crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceEvent {
    /// Envelope schema version; must equal [`V1_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Opaque source identity.
    pub source_id: String,
    /// Opaque source-product reference.
    pub source_product_ref: String,
    /// Opaque session identity used to scope sequence monotonicity.
    pub session_ref: String,
    /// Positive connection epoch.
    pub connection_epoch: u64,
    /// Strictly increasing capture sequence within the session.
    pub capture_sequence: u64,
    /// Strictly increasing canonical sequence within the session.
    pub canonical_sequence: u64,
    /// Provider sequence when supplied; zero is invalid and null is preserved.
    pub provider_sequence: Option<u64>,
    /// Provider timestamp in nanoseconds when supplied.
    pub source_timestamp_ns: Option<u64>,
    /// Positive receipt time in epoch nanoseconds.
    pub receive_utc_ns: u64,
    /// Positive monotonic receive time in nanoseconds.
    pub receive_monotonic_ns: u64,
    /// Decoder version revision.
    pub decoder_version: String,
    /// Reference-data revision used by the decoder.
    pub reference_data_revision: String,
    /// Reference to the rights evidence reviewed for this source.
    pub rights_evidence_ref: String,
    /// Lowercase hex SHA-256 of the original frame bytes.
    pub raw_frame_digest: String,
    /// Verified quality state.
    pub quality_state: QualityState,
    /// Verified continuity state.
    pub continuity_state: ContinuityState,
    /// Lowercase hex SHA-256 over all other canonical envelope fields.
    pub envelope_digest: String,
}

/// A typed refusal with a stable, externally comparable reason code.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SourceEventError {
    /// The envelope bytes are not one strict JSON object with known fields.
    #[error("SOURCE_EVENT_PARSE: {detail}")]
    Parse {
        /// Parser detail without raw envelope bytes.
        detail: String,
    },
    /// The declared schema version is not one.
    #[error("SOURCE_SCHEMA_VERSION: expected 1, got {actual}")]
    SchemaVersion {
        /// Rejected schema version.
        actual: u32,
    },
    /// A required identity or revision field is empty.
    #[error("SOURCE_FIELD_EMPTY: {field}")]
    EmptyField {
        /// Exact field name.
        field: &'static str,
    },
    /// The connection epoch is zero.
    #[error("CONNECTION_EPOCH_INVALID: connection_epoch must be positive")]
    ConnectionEpoch,
    /// A present provider sequence is zero.
    #[error("PROVIDER_SEQUENCE_ZERO: a present provider_sequence must be positive")]
    ProviderSequenceZero,
    /// The epoch receive time is zero.
    #[error("RECEIVE_UTC_INVALID: receive_utc_ns must be positive")]
    ReceiveUtc,
    /// The monotonic receive time is zero.
    #[error("RECEIVE_MONOTONIC_INVALID: receive_monotonic_ns must be positive")]
    ReceiveMonotonic,
    /// A digest is not lowercase 64-hex.
    #[error("DIGEST_MALFORMED: {field}")]
    DigestMalformed {
        /// Exact field name.
        field: &'static str,
    },
    /// The declared envelope digest does not match the recomputed digest.
    #[error("ENVELOPE_DIGEST_MISMATCH: canonical fields differ from the declared digest")]
    DigestMismatch,
    /// A `VALIDATED` event lacks decoder or reference revision evidence.
    #[error("VALIDATED_REVISION_MISSING: decoder_version or reference_data_revision is empty")]
    ValidatedRevision,
    /// A `VALIDATED` event carries an unresolved continuity state.
    #[error("VALIDATED_CONTINUITY_INVALID: continuity_state {state:?} cannot be VALIDATED")]
    ValidatedContinuity {
        /// Rejected continuity state.
        state: ContinuityState,
    },
    /// A capture sequence did not increase within its session.
    #[error("CAPTURE_SEQUENCE_NOT_INCREASING: session capture sequence is not strictly increasing")]
    CaptureSequence,
    /// A canonical sequence did not increase within its session.
    #[error(
        "CANONICAL_SEQUENCE_NOT_INCREASING: session canonical sequence is not strictly increasing"
    )]
    CanonicalSequence,
    /// Canonical JSON serialization failed.
    #[error("SOURCE_CANONICALIZATION_FAILED: {detail}")]
    Canonicalization {
        /// Serializer detail.
        detail: String,
    },
}

impl SourceEventError {
    /// Return the stable reason code for this refusal.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Parse { .. } => "SOURCE_EVENT_PARSE",
            Self::SchemaVersion { .. } => "SOURCE_SCHEMA_VERSION",
            Self::EmptyField { .. } => "SOURCE_FIELD_EMPTY",
            Self::ConnectionEpoch => "CONNECTION_EPOCH_INVALID",
            Self::ProviderSequenceZero => "PROVIDER_SEQUENCE_ZERO",
            Self::ReceiveUtc => "RECEIVE_UTC_INVALID",
            Self::ReceiveMonotonic => "RECEIVE_MONOTONIC_INVALID",
            Self::DigestMalformed { .. } => "DIGEST_MALFORMED",
            Self::DigestMismatch => "ENVELOPE_DIGEST_MISMATCH",
            Self::ValidatedRevision => "VALIDATED_REVISION_MISSING",
            Self::ValidatedContinuity { .. } => "VALIDATED_CONTINUITY_INVALID",
            Self::CaptureSequence => "CAPTURE_SEQUENCE_NOT_INCREASING",
            Self::CanonicalSequence => "CANONICAL_SEQUENCE_NOT_INCREASING",
            Self::Canonicalization { .. } => "SOURCE_CANONICALIZATION_FAILED",
        }
    }
}

impl SourceEvent {
    /// Parse one strict JSON envelope, rejecting duplicate and unknown fields.
    pub fn parse(bytes: &[u8]) -> Result<Self, SourceEventError> {
        let value = crate::jcs::parse_strict(bytes).map_err(|error| SourceEventError::Parse {
            detail: error.to_string(),
        })?;
        serde_json::from_value(value).map_err(|error| SourceEventError::Parse {
            detail: error.to_string(),
        })
    }

    /// Validate every field rule and return the recomputed envelope digest.
    ///
    /// Nothing is changed by this method. A quarantined event is structurally
    /// valid here but is refused by the admission gate.
    pub fn validate(&self) -> Result<[u8; 32], SourceEventError> {
        if self.schema_version != V1_SCHEMA_VERSION {
            return Err(SourceEventError::SchemaVersion {
                actual: self.schema_version,
            });
        }
        for (field, value) in [
            ("source_id", self.source_id.as_str()),
            ("source_product_ref", self.source_product_ref.as_str()),
            ("session_ref", self.session_ref.as_str()),
            ("decoder_version", self.decoder_version.as_str()),
            (
                "reference_data_revision",
                self.reference_data_revision.as_str(),
            ),
            ("rights_evidence_ref", self.rights_evidence_ref.as_str()),
        ] {
            if value.is_empty() {
                return Err(SourceEventError::EmptyField { field });
            }
        }
        if self.connection_epoch == 0 {
            return Err(SourceEventError::ConnectionEpoch);
        }
        if self.provider_sequence == Some(0) {
            return Err(SourceEventError::ProviderSequenceZero);
        }
        if self.receive_utc_ns == 0 {
            return Err(SourceEventError::ReceiveUtc);
        }
        if self.receive_monotonic_ns == 0 {
            return Err(SourceEventError::ReceiveMonotonic);
        }
        if !is_lowercase_hex64(&self.raw_frame_digest) {
            return Err(SourceEventError::DigestMalformed {
                field: "raw_frame_digest",
            });
        }
        if !is_lowercase_hex64(&self.envelope_digest) {
            return Err(SourceEventError::DigestMalformed {
                field: "envelope_digest",
            });
        }
        if self.quality_state == QualityState::Validated {
            if self.decoder_version.is_empty() || self.reference_data_revision.is_empty() {
                return Err(SourceEventError::ValidatedRevision);
            }
            if matches!(
                self.continuity_state,
                ContinuityState::GapSuspected | ContinuityState::Unknown
            ) {
                return Err(SourceEventError::ValidatedContinuity {
                    state: self.continuity_state,
                });
            }
        }
        let computed = self.envelope_sha256()?;
        let declared =
            decode_hex32(&self.envelope_digest).ok_or(SourceEventError::DigestMalformed {
                field: "envelope_digest",
            })?;
        if computed != declared {
            return Err(SourceEventError::DigestMismatch);
        }
        Ok(computed)
    }

    /// Return compact canonical JSON for every field except `envelope_digest`.
    ///
    /// Keys are sorted and explicit nulls are retained so the digest binding
    /// cannot depend on member order or on option representation.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, SourceEventError> {
        let bytes = serde_json::to_vec(&self.canonical_members()).map_err(|error| {
            SourceEventError::Canonicalization {
                detail: error.to_string(),
            }
        })?;
        Ok(bytes)
    }

    /// Return SHA-256 over the canonical bytes of all other fields.
    pub fn envelope_sha256(&self) -> Result<[u8; 32], SourceEventError> {
        let digest = Sha256::digest(self.canonical_bytes()?);
        let mut out = [0_u8; 32];
        out.copy_from_slice(&digest);
        Ok(out)
    }

    /// Bind `envelope_digest` to the current values of the other fields.
    ///
    /// This computes a digest only. It performs no validation and grants no
    /// right; a consumer always recomputes the digest before admission.
    pub fn finalize_digest(&mut self) -> Result<[u8; 32], SourceEventError> {
        let digest = self.envelope_sha256()?;
        self.envelope_digest = encode_hex32(&digest);
        Ok(digest)
    }

    fn canonical_members(&self) -> Map<String, Value> {
        let mut members: BTreeMap<String, Value> = BTreeMap::new();
        members.insert(
            "schema_version".to_owned(),
            Value::from(self.schema_version),
        );
        members.insert(
            "source_id".to_owned(),
            Value::String(self.source_id.clone()),
        );
        members.insert(
            "source_product_ref".to_owned(),
            Value::String(self.source_product_ref.clone()),
        );
        members.insert(
            "session_ref".to_owned(),
            Value::String(self.session_ref.clone()),
        );
        members.insert(
            "connection_epoch".to_owned(),
            Value::from(self.connection_epoch),
        );
        members.insert(
            "capture_sequence".to_owned(),
            Value::from(self.capture_sequence),
        );
        members.insert(
            "canonical_sequence".to_owned(),
            Value::from(self.canonical_sequence),
        );
        members.insert(
            "provider_sequence".to_owned(),
            option_u64_value(self.provider_sequence),
        );
        members.insert(
            "source_timestamp_ns".to_owned(),
            option_u64_value(self.source_timestamp_ns),
        );
        members.insert(
            "receive_utc_ns".to_owned(),
            Value::from(self.receive_utc_ns),
        );
        members.insert(
            "receive_monotonic_ns".to_owned(),
            Value::from(self.receive_monotonic_ns),
        );
        members.insert(
            "decoder_version".to_owned(),
            Value::String(self.decoder_version.clone()),
        );
        members.insert(
            "reference_data_revision".to_owned(),
            Value::String(self.reference_data_revision.clone()),
        );
        members.insert(
            "rights_evidence_ref".to_owned(),
            Value::String(self.rights_evidence_ref.clone()),
        );
        members.insert(
            "raw_frame_digest".to_owned(),
            Value::String(self.raw_frame_digest.clone()),
        );
        members.insert(
            "quality_state".to_owned(),
            Value::String(self.quality_state.as_str().to_owned()),
        );
        members.insert(
            "continuity_state".to_owned(),
            Value::String(self.continuity_state.as_str().to_owned()),
        );
        members.into_iter().collect()
    }
}

fn option_u64_value(value: Option<u64>) -> Value {
    match value {
        Some(number) => Value::from(number),
        None => Value::Null,
    }
}

fn is_lowercase_hex64(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn decode_hex32(text: &str) -> Option<[u8; 32]> {
    if !is_lowercase_hex64(text) {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = [0_u8; 32];
    for (index, chunk) in bytes.as_chunks::<2>().0.iter().enumerate() {
        let high = hex_nibble(chunk[0])?;
        let low = hex_nibble(chunk[1])?;
        out[index] = (high << 4) | low;
    }
    Some(out)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn encode_hex32(digest: &[u8; 32]) -> String {
    let mut text = String::with_capacity(64);
    for byte in digest {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// Per-session sequence watermarks for strictly increasing sequences.
///
/// A rejected event never changes the ledger, so a refusal cannot consume a
/// sequence or hide a later gap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SequenceLedger {
    sessions: HashMap<String, SessionWatermark>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SessionWatermark {
    capture_sequence: u64,
    canonical_sequence: u64,
}

impl SequenceLedger {
    /// Validate an event and advance its session watermark only on success.
    pub fn observe(&mut self, event: &SourceEvent) -> Result<[u8; 32], SourceEventError> {
        let digest = event.validate()?;
        if let Some(previous) = self.sessions.get(&event.session_ref) {
            if event.capture_sequence <= previous.capture_sequence {
                return Err(SourceEventError::CaptureSequence);
            }
            if event.canonical_sequence <= previous.canonical_sequence {
                return Err(SourceEventError::CanonicalSequence);
            }
        }
        self.sessions.insert(
            event.session_ref.clone(),
            SessionWatermark {
                capture_sequence: event.capture_sequence,
                canonical_sequence: event.canonical_sequence,
            },
        );
        Ok(digest)
    }

    /// Return the number of sessions with an accepted watermark.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }
}
