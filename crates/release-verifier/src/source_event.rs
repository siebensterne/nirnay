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
//!
//! Rejections expose two stable codes. [`SourceEventError::code`] keeps the
//! original diagnostic strings for existing callers;
//! [`SourceEventError::normative_code`] is the wire-facing v1 code from
//! `contracts/source-event-envelope-v1.json:rejection_reason_codes` and is the
//! code used by conformance evidence. Both are total, constant mappings from
//! the internal variants: no rejection can fall through to an ad hoc string.

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
    /// The declared schema version is not the literal v1 integer one.
    #[error("SOURCE_SCHEMA_VERSION: expected the literal 1")]
    SchemaVersionOutOfDomain,
    /// A required identity or revision field is empty.
    #[error("SOURCE_FIELD_EMPTY: {field}")]
    EmptyField {
        /// Exact field name.
        field: &'static str,
    },
    /// A quality state is not one of the four exact v1 members.
    #[error("INVALID_QUALITY_STATE: quality_state is not a v1 member")]
    InvalidQualityState,
    /// A continuity state is not one of the four exact v1 members.
    #[error("INVALID_CONTINUITY_STATE: continuity_state is not a v1 member")]
    InvalidContinuityState,
    /// The connection epoch is zero.
    #[error("CONNECTION_EPOCH_INVALID: connection_epoch must be positive")]
    ConnectionEpoch,
    /// A present provider sequence is zero or outside the u64-or-null domain.
    #[error("PROVIDER_SEQUENCE_ZERO: a present provider_sequence must be positive and in u64")]
    ProviderSequenceZero,
    /// A present source timestamp is outside the u64-or-null domain.
    #[error("INVALID_SOURCE_TIMESTAMP: source_timestamp_ns is neither null nor a u64")]
    SourceTimestamp,
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
    /// A capture sequence is zero or did not increase within its scope.
    #[error("CAPTURE_SEQUENCE_NOT_INCREASING: capture sequence is zero or not strictly increasing")]
    CaptureSequence,
    /// A canonical sequence is zero or did not increase within its scope.
    #[error(
        "CANONICAL_SEQUENCE_NOT_INCREASING: canonical sequence is zero or not strictly increasing"
    )]
    CanonicalSequence,
    /// A `QUARANTINED` envelope was presented as admissible.
    #[error("QUARANTINED_NOT_ADMISSIBLE: quality_state QUARANTINED is never admissible")]
    Quarantined,
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
            Self::SchemaVersion { .. } | Self::SchemaVersionOutOfDomain => "SOURCE_SCHEMA_VERSION",
            Self::EmptyField { .. } => "SOURCE_FIELD_EMPTY",
            Self::InvalidQualityState => "INVALID_QUALITY_STATE",
            Self::InvalidContinuityState => "INVALID_CONTINUITY_STATE",
            Self::ConnectionEpoch => "CONNECTION_EPOCH_INVALID",
            Self::ProviderSequenceZero => "PROVIDER_SEQUENCE_ZERO",
            Self::SourceTimestamp => "INVALID_SOURCE_TIMESTAMP",
            Self::ReceiveUtc => "RECEIVE_UTC_INVALID",
            Self::ReceiveMonotonic => "RECEIVE_MONOTONIC_INVALID",
            Self::DigestMalformed { .. } => "DIGEST_MALFORMED",
            Self::DigestMismatch => "ENVELOPE_DIGEST_MISMATCH",
            Self::ValidatedRevision => "VALIDATED_REVISION_MISSING",
            Self::ValidatedContinuity { .. } => "VALIDATED_CONTINUITY_INVALID",
            Self::CaptureSequence => "CAPTURE_SEQUENCE_NOT_INCREASING",
            Self::CanonicalSequence => "CANONICAL_SEQUENCE_NOT_INCREASING",
            Self::Quarantined => "QUARANTINED_NOT_ADMITTED",
            Self::Canonicalization { .. } => "SOURCE_CANONICALIZATION_FAILED",
        }
    }

    /// Return the normative v1 rejection code for this refusal.
    ///
    /// The returned strings are taken verbatim from
    /// `contracts/source-event-envelope-v1.json:rejection_reason_codes`. The
    /// mapping is `const` and total, so two independent implementations can
    /// compare external codes for identical conditions without interpreting
    /// display wording. A parse or canonicalization failure (duplicate,
    /// unknown or missing members) maps to `UNKNOWN_OR_MISSING_MEMBER`.
    pub const fn normative_code(&self) -> &'static str {
        match self {
            Self::Parse { .. } | Self::Canonicalization { .. } => "UNKNOWN_OR_MISSING_MEMBER",
            Self::SchemaVersion { .. } | Self::SchemaVersionOutOfDomain => {
                "SCHEMA_VERSION_MISMATCH"
            }
            Self::EmptyField { .. } => "EMPTY_REQUIRED_STRING",
            Self::InvalidQualityState => "INVALID_QUALITY_STATE",
            Self::InvalidContinuityState => "INVALID_CONTINUITY_STATE",
            Self::ConnectionEpoch => "INVALID_CONNECTION_EPOCH",
            Self::ProviderSequenceZero => "INVALID_PROVIDER_SEQUENCE",
            Self::SourceTimestamp => "INVALID_SOURCE_TIMESTAMP",
            Self::ReceiveUtc | Self::ReceiveMonotonic => "INVALID_RECEIVE_TIME",
            Self::DigestMalformed { .. } => "INVALID_DIGEST_FORMAT",
            Self::DigestMismatch => "ENVELOPE_DIGEST_MISMATCH",
            Self::ValidatedRevision => "VALIDATED_MISSING_REVISIONS",
            Self::ValidatedContinuity { .. } => "VALIDATED_CONTINUITY_UNRESOLVED",
            Self::CaptureSequence => "NON_MONOTONIC_CAPTURE_SEQUENCE",
            Self::CanonicalSequence => "NON_MONOTONIC_CANONICAL_SEQUENCE",
            Self::Quarantined => "QUARANTINED_NOT_ADMISSIBLE",
        }
    }
}

impl SourceEvent {
    /// Parse one strict JSON envelope, rejecting duplicate and unknown fields.
    ///
    /// Invalid integer values and non-member enum values are classified before
    /// typed deserialization: otherwise serde collapses a negative, out-of-range
    /// or non-string value into a generic parse error and the normative field
    /// code would be unreachable. Missing members are left to the strict
    /// deserializer and stay `UNKNOWN_OR_MISSING_MEMBER`. Duplicate members are
    /// rejected by the strict parser before this classification runs.
    pub fn parse(bytes: &[u8]) -> Result<Self, SourceEventError> {
        let value = crate::jcs::parse_strict(bytes).map_err(|error| SourceEventError::Parse {
            detail: error.to_string(),
        })?;
        if let Some(members) = value.as_object() {
            reject_missing_members(members)?;
            classify_integers(members)?;
            classify_enums(members)?;
        }
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
            if value.trim().is_empty() {
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
        // Zero is never a valid starting watermark. Refuse it here, before a
        // ledger can establish any high-water mark for this scope.
        if self.capture_sequence == 0 {
            return Err(SourceEventError::CaptureSequence);
        }
        if self.canonical_sequence == 0 {
            return Err(SourceEventError::CanonicalSequence);
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
            if self.decoder_version.trim().is_empty()
                || self.reference_data_revision.trim().is_empty()
            {
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

    /// Validate the envelope and refuse a quarantined event.
    ///
    /// This is a refusal-only, envelope-level check used by conformance
    /// evidence. It grants no right, accepts no dataset and reaches no
    /// downstream consumer; a non-quarantined result is not an admission.
    /// The v1 code for the quarantine refusal is
    /// `QUARANTINED_NOT_ADMISSIBLE` via [`SourceEventError::normative_code`].
    pub fn validate_admissible(&self) -> Result<[u8; 32], SourceEventError> {
        let digest = self.validate()?;
        if self.quality_state == QualityState::Quarantined {
            return Err(SourceEventError::Quarantined);
        }
        Ok(digest)
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

/// Every v1 envelope member, including the two u64-or-null members.
const ENVELOPE_MEMBERS: [&str; 18] = [
    "schema_version",
    "source_id",
    "source_product_ref",
    "session_ref",
    "connection_epoch",
    "capture_sequence",
    "canonical_sequence",
    "provider_sequence",
    "source_timestamp_ns",
    "receive_utc_ns",
    "receive_monotonic_ns",
    "decoder_version",
    "reference_data_revision",
    "rights_evidence_ref",
    "raw_frame_digest",
    "quality_state",
    "continuity_state",
    "envelope_digest",
];

/// Refuse an absent envelope member before typed deserialization.
///
/// Serde treats a missing `Option<u64>` member as `None`, which would let an
/// absent optional member reach digest checking with a synthesized null. A
/// missing member is a structural defect, never a field-code rejection.
fn reject_missing_members(members: &Map<String, Value>) -> Result<(), SourceEventError> {
    if let Some(field) = ENVELOPE_MEMBERS
        .iter()
        .find(|field| !members.contains_key(**field))
    {
        return Err(SourceEventError::Parse {
            detail: format!("missing field: {field}"),
        });
    }
    Ok(())
}

/// Classify every integer member outside its declared domain before typed
/// deserialization, using that field's normative rejection code.
///
/// A missing member is already refused as `UNKNOWN_OR_MISSING_MEMBER` by
/// `reject_missing_members`. `null` is in domain only for the two optional
/// u64-or-null members; all other JSON types and out-of-u64 numbers are
/// field-code rejections.
fn classify_integers(members: &Map<String, Value>) -> Result<(), SourceEventError> {
    if let Some(value) = members.get("schema_version")
        && !matches!(
            value,
            Value::Number(number) if number.as_u64() == Some(u64::from(V1_SCHEMA_VERSION))
        )
    {
        return Err(SourceEventError::SchemaVersionOutOfDomain);
    }
    for (field, error) in [
        ("connection_epoch", SourceEventError::ConnectionEpoch),
        ("capture_sequence", SourceEventError::CaptureSequence),
        ("canonical_sequence", SourceEventError::CanonicalSequence),
        ("receive_utc_ns", SourceEventError::ReceiveUtc),
        ("receive_monotonic_ns", SourceEventError::ReceiveMonotonic),
        ("provider_sequence", SourceEventError::ProviderSequenceZero),
        ("source_timestamp_ns", SourceEventError::SourceTimestamp),
    ] {
        if let Some(value) = members.get(field) {
            let in_domain = match value {
                Value::Number(number) => number.as_u64().is_some(),
                Value::Null => matches!(field, "provider_sequence" | "source_timestamp_ns"),
                _ => false,
            };
            if !in_domain {
                return Err(error);
            }
        }
    }
    Ok(())
}

/// Classify present enum members that are not exact v1 members.
///
/// Unknown strings, numbers, nulls and objects receive their own typed codes;
/// missing or duplicate members are still `UNKNOWN_OR_MISSING_MEMBER`.
fn classify_enums(members: &Map<String, Value>) -> Result<(), SourceEventError> {
    if let Some(value) = members.get("quality_state")
        && !value
            .as_str()
            .is_some_and(|text| parse_quality_state(text).is_some())
    {
        return Err(SourceEventError::InvalidQualityState);
    }
    if let Some(value) = members.get("continuity_state")
        && !value
            .as_str()
            .is_some_and(|text| parse_continuity_state(text).is_some())
    {
        return Err(SourceEventError::InvalidContinuityState);
    }
    Ok(())
}

fn parse_quality_state(text: &str) -> Option<QualityState> {
    [
        QualityState::Captured,
        QualityState::Decoded,
        QualityState::Validated,
        QualityState::Quarantined,
    ]
    .into_iter()
    .find(|state| state.as_str() == text)
}

fn parse_continuity_state(text: &str) -> Option<ContinuityState> {
    [
        ContinuityState::Continuous,
        ContinuityState::GapSuspected,
        ContinuityState::Recovered,
        ContinuityState::Unknown,
    ]
    .into_iter()
    .find(|state| state.as_str() == text)
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

/// Sequence watermarks scoped by `(source_id, session_ref)`.
///
/// Two sources sharing one session own independent histories: neither can
/// consume the other's sequence. A rejected event never changes the ledger,
/// so a refusal cannot consume a sequence or hide a later gap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SequenceLedger {
    scopes: HashMap<(String, String), SessionWatermark>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SessionWatermark {
    capture_sequence: u64,
    canonical_sequence: u64,
}

impl SequenceLedger {
    /// Check admission and advance its scope watermark only on success.
    ///
    /// Structural validity is not enough: a `QUARANTINED` event is refused with
    /// `QUARANTINED_NOT_ADMISSIBLE` before any watermark is read or written.
    pub fn observe(&mut self, event: &SourceEvent) -> Result<[u8; 32], SourceEventError> {
        let digest = event.validate_admissible()?;
        let scope = (event.source_id.clone(), event.session_ref.clone());
        if let Some(previous) = self.scopes.get(&scope) {
            if event.capture_sequence <= previous.capture_sequence {
                return Err(SourceEventError::CaptureSequence);
            }
            if event.canonical_sequence <= previous.canonical_sequence {
                return Err(SourceEventError::CanonicalSequence);
            }
        }
        self.scopes.insert(
            scope,
            SessionWatermark {
                capture_sequence: event.capture_sequence,
                canonical_sequence: event.canonical_sequence,
            },
        );
        Ok(digest)
    }

    /// Return the highest accepted `(capture, canonical)` pair for one scope.
    ///
    /// A `None` result means no event has been accepted for that source and
    /// session; a refusal never creates or advances a watermark.
    pub fn watermark(&self, source_id: &str, session_ref: &str) -> Option<(u64, u64)> {
        self.scopes
            .get(&(source_id.to_owned(), session_ref.to_owned()))
            .map(|watermark| (watermark.capture_sequence, watermark.canonical_sequence))
    }

    /// Return the number of `(source_id, session_ref)` scopes with an accepted
    /// watermark.
    pub fn session_count(&self) -> usize {
        self.scopes.len()
    }
}
