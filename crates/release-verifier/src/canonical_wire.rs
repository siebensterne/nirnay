// SPDX-License-Identifier: Apache-2.0
//! Independent strict types for the canonical NDC v2 Profile 1 wire schema.
//!
//! This module is a consumer-side implementation of the language-independent
//! authority `WS-NDC-2026-11-24-003` and the versioned schema package
//! `ndc-v2-schema-profile-1`. It imports no Fieldbook code and shares no
//! validator or serializer. The types below are strict: unknown members,
//! missing required members, wrong JSON types, implicit coercions, duplicate
//! members and non-canonical numeric forms are refused.
//!
//! Neither these types nor the older internal [`crate::schema`] types are
//! normative. [`into_internal_manifest`] and [`into_internal_record`] are
//! explicit mappings from the canonical wire representation into the existing
//! internal representation. Byte-derived ordering is recomputed from received
//! record bytes in [`derive_key_from_bytes`]; no caller-supplied key is trusted
//! independently of those bytes.

use crate::error::VerifierError;
use crate::files::hex_lower;
use crate::files::sha256;
use crate::jcs::canonical_bytes;
use crate::jcs::parse_strict;
use crate::schema::AdmissionRequest;
use crate::schema::CoverageDimension as InternalCoverageDimension;
use crate::schema::CoverageRecord as InternalCoverageRecord;
use crate::schema::DocumentReference as InternalDocumentReference;
use crate::schema::Lineage as InternalLineage;
use crate::schema::OperationEvidence as InternalOperationEvidence;
use crate::schema::PerformedOperation as InternalPerformedOperation;
use crate::schema::ReleaseIdentity as InternalReleaseIdentity;
use crate::schema::ReleaseManifest;
use crate::schema::ReleaseRecord;
use crate::schema::RetentionPolicy;
use crate::schema::RightsRecord as InternalRightsRecord;
use crate::schema::ScopeDeclaration as InternalScopeDeclaration;
use crate::schema::SupersedesRelation;
use crate::verifier::DECLARED_PURPOSE;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
use serde::de::Error as DeError;
use serde::de::Visitor;
use serde_json::Value;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;

/// Registered digest of the canonical schema package used by this mapping.
pub const SCHEMA_PACKAGE_DIGEST: &str =
    "6e8c1e48e27289b8c3d3e959a06a805e736d0759b4fb244edbb074f65e107bab";

/// Version of the canonical wire schema implemented here.
pub const WIRE_SCHEMA_VERSION: u32 = 1;

/// Largest integer emitted as a JSON number under the ratified numeric rule.
const MAX_EXACT_INTEGER: u64 = 9_007_199_254_740_991;

/// A refusal from canonical wire parsing, semantic validation or key derivation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalError {
    /// A member is present but outside the closed schema member set.
    UnknownMember(String),
    /// A JSON object repeats a member name.
    DuplicateMember(String),
    /// A value, grammar, type or cross-field invariant is invalid.
    Invalid(String),
    /// Ordering metadata does not recompute from the received record bytes.
    Ordering(String),
}

impl fmt::Display for CanonicalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownMember(detail)
            | Self::DuplicateMember(detail)
            | Self::Invalid(detail)
            | Self::Ordering(detail) => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for CanonicalError {}

impl CanonicalError {
    /// Return the stable rejection family for this refusal.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownMember(_) | Self::DuplicateMember(_) | Self::Invalid(_) => {
                "REJECT_SCHEMA_ENVELOPE"
            }
            Self::Ordering(_) => "REJECT_ORDERING",
        }
    }

    /// Map a manifest-level refusal to the verifier's typed stage-one error.
    pub fn into_manifest_error(self) -> VerifierError {
        match self {
            Self::UnknownMember(detail) => VerifierError::UnknownMember { member: detail },
            Self::DuplicateMember(detail) => VerifierError::DuplicateMember { member: detail },
            Self::Invalid(detail) | Self::Ordering(detail) => {
                VerifierError::ManifestSchema { detail }
            }
        }
    }

    /// Map a record-level refusal to the verifier's typed stage-five error.
    pub fn into_record_error(self) -> VerifierError {
        match self {
            Self::Ordering(detail) | Self::Invalid(detail) => VerifierError::RecordOrder { detail },
            Self::UnknownMember(detail) | Self::DuplicateMember(detail) => {
                VerifierError::RecordStream { detail }
            }
        }
    }
}

fn is_decimal_text(text: &str) -> bool {
    !text.is_empty()
        && (text == "0" || text.as_bytes().first().is_some_and(|byte| *byte >= b'1'))
        && text.bytes().all(|byte| byte.is_ascii_digit())
}

fn to_verifier_decode_error(error: serde_json::Error) -> CanonicalError {
    let detail = error.to_string();
    if detail.contains("duplicate field") {
        CanonicalError::DuplicateMember(detail)
    } else if detail.contains("unknown field") {
        CanonicalError::UnknownMember(detail)
    } else {
        CanonicalError::Invalid(detail)
    }
}

/// A decimal string matching `^(0|[1-9][0-9]*)$` within the `u64` range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DecimalU64(u64);

impl DecimalU64 {
    /// Return the parsed integer value.
    pub fn value(&self) -> u64 {
        self.0
    }
}

impl Serialize for DecimalU64 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for DecimalU64 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(DecimalVisitor)
    }
}

struct DecimalVisitor;

impl<'de> Visitor<'de> for DecimalVisitor {
    type Value = DecimalU64;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a canonical decimal string within the u64 range")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        if !is_decimal_text(value) {
            return Err(E::custom("decimal string grammar rejected"));
        }
        let parsed = value
            .parse::<u64>()
            .map_err(|_| E::custom("decimal string exceeds u64 range"))?;
        Ok(DecimalU64(parsed))
    }
}

/// A count or byte size: a JSON integer below 2^53 or a decimal string above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CountOrSize(u64);

impl CountOrSize {
    /// Return the numeric value independent of its wire representation.
    pub fn value(&self) -> u64 {
        self.0
    }
}

impl Serialize for CountOrSize {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.0 <= MAX_EXACT_INTEGER {
            serializer.serialize_u64(self.0)
        } else {
            serializer.serialize_str(&self.0.to_string())
        }
    }
}

impl<'de> Deserialize<'de> for CountOrSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(CountVisitor)
    }
}

struct CountVisitor;

impl<'de> Visitor<'de> for CountVisitor {
    type Value = CountOrSize;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an exact non-negative integer or canonical decimal string")
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        if value > MAX_EXACT_INTEGER {
            return Err(E::custom("count above 2^53 must be a decimal string"));
        }
        Ok(CountOrSize(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        if value < 0 {
            return Err(E::custom("negative count rejected"));
        }
        self.visit_u64(value as u64)
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        if !is_decimal_text(value) {
            return Err(E::custom("decimal string grammar rejected"));
        }
        let parsed = value
            .parse::<u64>()
            .map_err(|_| E::custom("decimal string exceeds u64 range"))?;
        if parsed <= MAX_EXACT_INTEGER {
            return Err(E::custom("small count must be a JSON integer"));
        }
        Ok(CountOrSize(parsed))
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
    where
        E: DeError,
    {
        Err(E::custom("floating point is not an exact count"))
    }
}

/// A non-empty text member.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonEmptyString(String);

impl NonEmptyString {
    /// Return the text value.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Create a validated non-empty text value.
    pub fn new(value: impl Into<String>) -> Result<Self, CanonicalError> {
        let value = value.into();
        if value.is_empty() {
            return Err(CanonicalError::Invalid("empty text member".to_owned()));
        }
        Ok(Self(value))
    }
}

impl Serialize for NonEmptyString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for NonEmptyString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(DeError::custom)
    }
}

/// A ratified identifier: no placeholder, empty or unregistered-shape value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Identifier(String);

const PLACEHOLDERS: [&str; 10] = [
    "UNKNOWN",
    "ALL",
    "NONE",
    "NULL",
    "null",
    "N/A",
    "TBD",
    "PLACEHOLDER",
    "unknown",
    "all",
];

fn is_identifier(text: &str) -> bool {
    if text.is_empty() || text.len() > 128 || PLACEHOLDERS.contains(&text) {
        return false;
    }
    let mut bytes = text.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    bytes.all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
}

impl Identifier {
    /// Return the identifier text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Validate and wrap an identifier string.
    pub fn new(value: impl Into<String>) -> Result<Self, CanonicalError> {
        let value = value.into();
        if !is_identifier(&value) {
            return Err(CanonicalError::Invalid(format!(
                "identifier grammar rejected: {}",
                value
            )));
        }
        Ok(Self(value))
    }
}

impl Serialize for Identifier {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Identifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(DeError::custom)
    }
}

/// A release-relative artifact path in the registered component alphabet.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactPath(String);

impl ArtifactPath {
    /// Return the path text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Validate and wrap an artifact path.
    pub fn new(value: impl Into<String>) -> Result<Self, CanonicalError> {
        let value = value.into();
        let Some(component) = value.strip_prefix("artifacts/") else {
            return Err(CanonicalError::Invalid("artifact path prefix".to_owned()));
        };
        if component.is_empty() || component.len() > 128 || component.contains('/') {
            return Err(CanonicalError::Invalid(
                "artifact path component".to_owned(),
            ));
        }
        let mut bytes = component.bytes();
        let Some(first) = bytes.next() else {
            return Err(CanonicalError::Invalid(
                "artifact path component".to_owned(),
            ));
        };
        if !first.is_ascii_alphanumeric()
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(CanonicalError::Invalid("artifact path alphabet".to_owned()));
        }
        Ok(Self(value))
    }
}

impl Serialize for ArtifactPath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ArtifactPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(DeError::custom)
    }
}

/// A lowercase 64-hexadecimal SHA-256 value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256Hex(String);

impl Sha256Hex {
    /// Return the digest text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for Sha256Hex {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Sha256Hex {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(DeError::custom("digest is not lowercase 64-hex"));
        }
        Ok(Self(value))
    }
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let shifted_year = year - i64::from(month <= 2);
    let era = if shifted_year >= 0 {
        shifted_year
    } else {
        (shifted_year - 399) / 400
    };
    let year_of_era = shifted_year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

fn parse_utc_nanos(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() != 30
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'.'
        || bytes[29] != b'Z'
    {
        return None;
    }
    let digits = |range: std::ops::Range<usize>| -> Option<u64> {
        let mut value = 0_u64;
        for index in range {
            let byte = *bytes.get(index)?;
            if !byte.is_ascii_digit() {
                return None;
            }
            value = value.checked_mul(10)?.checked_add(u64::from(byte - b'0'))?;
        }
        Some(value)
    };
    let year = digits(0..4)? as i64;
    let month = digits(5..7)? as i64;
    let day = digits(8..10)? as i64;
    let hour = digits(11..13)?;
    let minute = digits(14..16)?;
    let second = digits(17..19)?;
    let nano = digits(20..29)?;
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
        || nano > 999_999_999
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let seconds =
        (days as u128) * 86_400 + (hour as u128) * 3_600 + (minute as u128) * 60 + second as u128;
    let nanos = seconds
        .checked_mul(1_000_000_000)?
        .checked_add(nano as u128)?;
    u64::try_from(nanos).ok()
}

/// A canonical UTC instant string with exactly nine fractional digits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcInstantString {
    text: String,
    nanos: u64,
}

impl UtcInstantString {
    /// Return the wire text.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Return the decoded instant in nanoseconds since the Unix epoch.
    pub fn nanos(&self) -> u64 {
        self.nanos
    }

    /// Validate and wrap a canonical UTC instant string.
    pub fn new(value: impl Into<String>) -> Result<Self, CanonicalError> {
        let text = value.into();
        let nanos = parse_utc_nanos(&text)
            .ok_or_else(|| CanonicalError::Invalid("invalid canonical UTC instant".to_owned()))?;
        Ok(Self { text, nanos })
    }
}

impl Serialize for UtcInstantString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for UtcInstantString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(DeError::custom)
    }
}

macro_rules! token_enum {
    ($name:ident, $(($variant:ident, $wire:expr)),+ $(,)?) => {
        /// A closed wire token with an exact registered spelling.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $(
                #[doc = concat!("Wire token `", $wire, "`.")]
                #[serde(rename = $wire)]
                $variant,
            )+
        }

        impl $name {
            /// Return the exact wire token.
            pub fn as_str(&self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),+
                }
            }
        }
    };
}

token_enum!(
    ProducerAction,
    (SourceContact, "source_contact"),
    (ReferenceDataRetrieval, "reference_data_retrieval"),
    (HistoricalDataRetrieval, "historical_data_retrieval"),
    (RealTimeCapture, "real_time_capture"),
    (RawStorage, "raw_storage"),
    (Normalization, "normalization"),
    (Replay, "replay"),
    (Backup, "backup"),
    (InternalAnalytics, "internal_analytics"),
    (DerivedDataCreation, "derived_data_creation"),
    (Display, "display"),
    (ExternalRedistribution, "external_redistribution"),
    (Retention, "retention"),
    (Deletion, "deletion"),
);

token_enum!(Decision, (Permit, "PERMIT"), (Deny, "DENY"),);

token_enum!(
    Verdict,
    (Pass, "PASS"),
    (Fail, "FAIL"),
    (Unknown, "UNKNOWN"),
);

token_enum!(
    GapKind,
    (Missing, "MISSING"),
    (Rejected, "REJECTED"),
    (Other, "OTHER"),
);

token_enum!(
    SubjectState,
    (Named, "NAMED"),
    (All, "ALL"),
    (NotApplicable, "NOT_APPLICABLE"),
);

token_enum!(Compression, (None, "none"), (Gzip, "gzip"),);

token_enum!(
    CompressionVersion,
    (NoneV1, "none/1"),
    (GzipFlate2V1, "gzip-deflate/flate2-1"),
);

token_enum!(
    PublicationState,
    (StagingCreated, "STAGING_CREATED"),
    (FilesWritten, "FILES_WRITTEN"),
    (FilesSynced, "FILES_SYNCED"),
    (StagingDirFsynced, "STAGING_DIR_FSYNCED"),
    (AtomicNoReplaceRename, "ATOMIC_NO_REPLACE_RENAME"),
    (ParentDirFsynced, "PARENT_DIR_FSYNCED"),
    (DurableReceipt, "DURABLE_PUBLICATION_RECEIPT"),
    (Available, "AVAILABLE"),
);

token_enum!(DigestAlgorithm, (Sha256, "sha256"));
token_enum!(V2Contract, (V2, "v2"));
token_enum!(SignatureReference, (SignatureJson, "signature.json"));
token_enum!(
    RecordEncoding,
    (NdjsonUtf8, "application/x-ndjson; charset=utf-8")
);
token_enum!(SequencedKind, (Sequenced, "sequenced"));
token_enum!(ReferenceKind, (Reference, "reference"));

token_enum!(
    CoverageDimensionId,
    (Instrument, "instrument_coverage"),
    (ListingIdentity, "listing_identity_coverage"),
    (Contract, "contract_coverage"),
    (CorporateAction, "corporate_action_coverage"),
    (SessionTime, "session_time_coverage"),
    (TradeEvent, "trade_event_coverage"),
    (QuoteDepth, "quote_depth_coverage"),
    (Mbo, "mbo_coverage"),
    (SequenceContinuity, "sequence_continuity"),
);

token_enum!(
    PayloadFieldKind,
    (String, "string"),
    (DecimalString, "decimal_string"),
    (Boolean, "boolean"),
    (BytesHex, "bytes_hex"),
);

/// One structured producer operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DataObject {
    /// Object kind named by the operation.
    pub kind: Identifier,
    /// Object identity named by the operation.
    pub id: Identifier,
    /// Object version named by the operation.
    pub version: NonEmptyString,
}

/// One structured producer operation with its typed action.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PerformedOperation {
    /// Stable reference used to bind evidence to this operation.
    pub operation_ref: Identifier,
    /// Producer action vocabulary value.
    pub action: ProducerAction,
    /// Object acted upon.
    pub data_object: DataObject,
}

/// Typed evidence bound to one performed operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OperationEvidence {
    /// Reference of the performed operation supported by this record.
    pub operation_ref: Identifier,
    /// Producer event identity.
    pub event_id: Identifier,
    /// Producer run identity.
    pub run_id: Identifier,
    /// Decimal-string recording instant.
    pub recorded_at_ns: DecimalU64,
    /// Decision key under which the operation ran.
    pub decision_key: NonEmptyString,
    /// Lowercase hex digest of the evidence object.
    pub digest_sha256: Sha256Hex,
}

/// The two-member producer activity section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProducerActivity {
    /// Structured performed operations.
    pub performed_operations: Vec<PerformedOperation>,
    /// Typed operation evidence.
    pub operation_evidence: Vec<OperationEvidence>,
}

/// The authority that granted a written right.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GrantingAuthority {
    /// Identity of the granting authority.
    pub authority_id: Identifier,
    /// Evidence reference for that authority.
    pub evidence_reference: NonEmptyString,
}

/// Typed scope declaration carried by a grant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScopeDeclaration {
    /// Provider scope.
    pub provider_id: Identifier,
    /// Source product scope.
    pub source_product_id: Identifier,
    /// Venue scope.
    pub venue_id: Identifier,
    /// Segment scope.
    pub segment_id: Identifier,
    /// Dataset scope.
    pub dataset_id: Identifier,
    /// Subject state for this scope.
    pub subject_state: SubjectState,
    /// Named subjects; an empty NAMED set is not ALL.
    pub named_subjects: Vec<Identifier>,
    /// Decimal-string data-time begin.
    pub data_time_begin_ns: DecimalU64,
    /// Decimal-string data-time end.
    pub data_time_end_ns: DecimalU64,
}

/// Named validity interval and clock domain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Validity {
    /// Decimal-string validity begin.
    pub begin_ns: DecimalU64,
    /// Decimal-string validity end.
    pub end_ns: DecimalU64,
    /// Clock domain for the interval.
    pub clock_domain: NonEmptyString,
}

/// Approved delivery method and endpoint identities.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    /// Delivery method.
    pub method: NonEmptyString,
    /// Approved endpoint identities.
    pub approved_endpoint_identities: Vec<Identifier>,
}

/// Bounded retention with an explicit purge rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Retention {
    /// Bounded maximum duration.
    pub max_duration_ns: CountOrSize,
    /// Explicit purge rule.
    pub purge_rule: NonEmptyString,
}

/// Typed reference to a supporting written document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DocumentReference {
    /// Document reference.
    pub reference: NonEmptyString,
    /// Reviewed clause.
    pub clause: NonEmptyString,
    /// Lowercase hex document digest.
    pub document_sha256: Sha256Hex,
    /// Contract revision reviewed with the document.
    pub contract_revision: NonEmptyString,
    /// Review authority.
    pub review_authority: NonEmptyString,
    /// Canonical UTC review date.
    pub reviewed_on: UtcInstantString,
}

/// Typed digest reference to a supporting document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DigestReference {
    /// Document reference.
    pub reference: NonEmptyString,
    /// Lowercase hex document digest.
    pub digest_sha256: Sha256Hex,
}

/// Reviewed attestation; review is not permission.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewedAttestation {
    /// Verbatim reviewed purpose.
    pub purpose: NonEmptyString,
    /// Reviewed usage right.
    pub usage_right: NonEmptyString,
    /// Reviewed usage mode.
    pub usage_mode: NonEmptyString,
    /// Reviewed territory.
    pub territory: NonEmptyString,
    /// Reviewed scope assertions.
    pub scope_assertions: Vec<NonEmptyString>,
    /// Reviewer identity.
    pub reviewer: NonEmptyString,
    /// Review authority.
    pub authority: NonEmptyString,
}

/// Registered attestation for a grant identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegisteredAttestation {
    /// Attestation identity.
    pub attestation_id: Identifier,
    /// Grant identity bound to this attestation.
    pub grant_id: Identifier,
    /// The reviewed attestation.
    pub attestation: ReviewedAttestation,
}

/// One full typed source-rights grant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PermissionGrant {
    /// Immutable grant identity.
    pub grant_id: Identifier,
    /// Grant revision.
    pub grant_revision: NonEmptyString,
    /// Typed granting authority.
    pub granting_authority: GrantingAuthority,
    /// Producer action covered by the grant.
    pub action: ProducerAction,
    /// Object class covered by the grant.
    pub object_class: Identifier,
    /// Exact written decision.
    pub decision: Decision,
    /// Named actor.
    pub actor: NonEmptyString,
    /// Named licensee.
    pub licensee: NonEmptyString,
    /// Recipient class.
    pub recipient_class: NonEmptyString,
    /// Recipient identity.
    pub recipient_identity: NonEmptyString,
    /// Reviewed purpose.
    pub purpose: NonEmptyString,
    /// Typed grant scope.
    pub scope: ScopeDeclaration,
    /// Validity interval.
    pub validity: Validity,
    /// Delivery declaration.
    pub delivery: Delivery,
    /// Retention declaration.
    pub retention: Retention,
    /// Supporting document references with digests.
    pub supporting_document_references: Vec<DigestReference>,
    /// Reviewed contract revision.
    pub contract_revision: NonEmptyString,
    /// Review authority.
    pub review_authority: NonEmptyString,
    /// Canonical UTC review date.
    pub reviewed_on: UtcInstantString,
    /// Attestation embedded in the grant.
    pub reviewed_attestation: ReviewedAttestation,
}

/// Independent signed rights-evidence section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RightsEvidence {
    /// Full typed grants.
    pub permission_grants: Vec<PermissionGrant>,
    /// Typed document references.
    pub document_references: Vec<DocumentReference>,
    /// Registered attestations.
    pub reviewed_attestations: Vec<RegisteredAttestation>,
}

/// A typed coverage gap.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Gap {
    /// Gap identity.
    pub gap_id: Identifier,
    /// Gap class.
    pub gap_kind: GapKind,
    /// Denominator component described by this gap.
    pub component: NonEmptyString,
    /// Gap count.
    pub count: CountOrSize,
    /// Gap description.
    pub description: NonEmptyString,
}

/// One of the nine separately declared coverage dimensions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CoverageDimension {
    /// Registered dimension identity.
    pub dimension_id: CoverageDimensionId,
    /// Declared universe identity.
    pub universe_id: Identifier,
    /// Declared denominator identity.
    pub denominator_identity: NonEmptyString,
    /// Count basis for this dimension.
    pub count_basis: NonEmptyString,
    /// Accepted count.
    pub numerator: CountOrSize,
    /// Required non-zero denominator.
    pub denominator: CountOrSize,
    /// Missing count.
    pub missing_count: CountOrSize,
    /// Rejected count.
    pub rejected_count: CountOrSize,
    /// Decimal-string as-of instant.
    pub as_of_ns: DecimalU64,
    /// Wire verdict; a producer PASS does not promote the verdict.
    pub verdict: Verdict,
    /// Evidence reference.
    pub evidence_reference: NonEmptyString,
    /// Typed gaps; filling them silently is refused.
    pub gaps: Vec<Gap>,
}

/// The nine-dimension coverage section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CoverageEvidence {
    /// Separate typed dimensions.
    pub dimensions: Vec<CoverageDimension>,
}

/// Transformation identity and version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TransformationRecord {
    /// Transformation identity.
    pub identity: Identifier,
    /// Transformation version.
    pub version: NonEmptyString,
}

/// PROV `wasDerivedFrom` relation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProvWasDerivedFrom {
    /// Source entity.
    pub source_entity_id: Identifier,
    /// Deriving activity.
    pub deriving_activity_id: Identifier,
    /// Decimal-string knowledge time.
    pub knowledge_time_ns: DecimalU64,
}

/// PROV `wasRevisionOf` relation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProvWasRevisionOf {
    /// Predecessor entity.
    pub predecessor_entity_id: Identifier,
    /// Revision activity.
    pub revision_activity_id: Identifier,
    /// Decimal-string knowledge time.
    pub knowledge_time_ns: DecimalU64,
}

/// PROV `hadPrimarySource` relation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProvHadPrimarySource {
    /// Primary source entity.
    pub source_entity_id: Identifier,
    /// Attribution activity.
    pub attribution_activity_id: Identifier,
    /// Decimal-string knowledge time.
    pub knowledge_time_ns: DecimalU64,
}

/// Qualified PROV attribution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QualifiedAttribution {
    /// Agent identity.
    pub agent_id: Identifier,
    /// Activity identity.
    pub activity_id: Identifier,
    /// Agent role.
    pub role: NonEmptyString,
    /// Decimal-string knowledge time.
    pub knowledge_time_ns: DecimalU64,
}

/// Seven-member immediate-predecessor relation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Supersedes {
    /// Immediate predecessor release identity.
    pub supersedes_release_id: Identifier,
    /// Immediate predecessor manifest digest.
    pub supersedes_manifest_sha256: Sha256Hex,
    /// Immediate predecessor dataset digest.
    pub supersedes_data_sha256: Sha256Hex,
    /// Immediate predecessor revision, strictly below the current revision.
    pub revision: u64,
    /// Correction reason.
    pub correction_reason: NonEmptyString,
    /// Decimal-string correction knowledge time.
    pub knowledge_time_ns: DecimalU64,
    /// Canonical UTC invalidation instant of the predecessor.
    pub invalidated_at_time: UtcInstantString,
}

/// Complete typed PROV lineage for the release.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Lineage {
    /// Raw archive reference.
    pub raw_archive_reference: NonEmptyString,
    /// Lowercase hex raw archive digest.
    pub raw_archive_sha256: Sha256Hex,
    /// Transformation record.
    pub transformation_record: TransformationRecord,
    /// Decimal-string knowledge time.
    pub knowledge_time_ns: DecimalU64,
    /// Current release revision.
    pub revision: u64,
    /// PROV derivation relation.
    pub was_derived_from: ProvWasDerivedFrom,
    /// PROV revision relation.
    pub was_revision_of: ProvWasRevisionOf,
    /// PROV primary-source relation.
    pub had_primary_source: ProvHadPrimarySource,
    /// Canonical UTC generation instant.
    pub generated_at_time: UtcInstantString,
    /// Canonical UTC invalidation instant.
    pub invalidated_at_time: UtcInstantString,
    /// Qualified attribution.
    pub qualified_attribution: QualifiedAttribution,
    /// Immediate predecessor, or null for an initial release.
    pub supersedes: Option<Supersedes>,
}

/// One per-event-type versioned payload schema entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PayloadSchemaEntry {
    /// Event type registered by this entry.
    pub event_type: Identifier,
    /// Payload schema identity.
    pub schema_id: Identifier,
    /// Payload schema version.
    pub schema_version: NonEmptyString,
    /// Registered digest of that schema's exact bytes.
    pub digest_sha256: Sha256Hex,
}

/// Declared normalizer, envelope and per-event payload registry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SchemaVersions {
    /// Normalizer version.
    pub normalizer_version: NonEmptyString,
    /// Event envelope version.
    pub envelope_version: NonEmptyString,
    /// Versioned per-event-type registry.
    pub payload_schemas: Vec<PayloadSchemaEntry>,
}

/// Dataset-level digest declaration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Digests {
    /// Digest algorithm token.
    pub digest_algorithm: DigestAlgorithm,
    /// Dataset digest over ordered uncompressed streams.
    pub data_sha256: Sha256Hex,
}

/// Publisher identity and signature reference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PublicationIdentity {
    /// Publisher identity resolved by the trusted registry.
    pub publisher_identity: Identifier,
    /// Immutable signing key identity.
    pub signing_key_id: Identifier,
    /// Detached sidecar file name.
    pub signature_reference: SignatureReference,
    /// Durable publication archive reference.
    pub archive_reference: NonEmptyString,
}

/// Sequenced artifact-level order profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SequencedArtifactProfile {
    /// Discriminator.
    pub kind: SequencedKind,
    /// Non-empty sequence domain.
    pub sequence_domain_id: NonEmptyString,
    /// Non-empty sequence epoch.
    pub sequence_epoch: NonEmptyString,
}

/// Reference artifact-level order profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReferenceArtifactProfile {
    /// Discriminator.
    pub kind: ReferenceKind,
}

/// Tagged artifact-level record order profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ArtifactRecordOrderProfile {
    /// Sequenced profile with domain and epoch.
    Sequenced(SequencedArtifactProfile),
    /// Reference profile with neither domain nor epoch.
    Reference(ReferenceArtifactProfile),
}

impl ArtifactRecordOrderProfile {
    /// Return the profile discriminator.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Sequenced(_) => "sequenced",
            Self::Reference(_) => "reference",
        }
    }
}

/// One published artifact descriptor with exactly twelve members.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactDescriptor {
    /// Logical partition identity.
    pub logical_partition_id: Identifier,
    /// Shard ordinal within the logical partition.
    pub shard_ordinal: u32,
    /// Release-relative artifact path.
    pub path: ArtifactPath,
    /// Tagged artifact order profile.
    pub record_order_profile: ArtifactRecordOrderProfile,
    /// Record stream encoding.
    pub encoding: RecordEncoding,
    /// Compression method.
    pub compression: Compression,
    /// Registered string compression identifier.
    pub compression_version: CompressionVersion,
    /// Exact stored byte count.
    pub stored_bytes: CountOrSize,
    /// Lowercase hex digest of stored bytes.
    pub stored_sha256: Sha256Hex,
    /// Exact decoded byte count.
    pub uncompressed_bytes: CountOrSize,
    /// Lowercase hex digest of decoded bytes.
    pub uncompressed_sha256: Sha256Hex,
    /// Declared logical record count.
    pub record_count: CountOrSize,
}

/// Immutable release identity and publication declaration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReleaseIdentity {
    /// Release identity.
    pub release_id: Identifier,
    /// Release version.
    pub release_version: NonEmptyString,
    /// Decimal-string as-of instant.
    pub as_of_ns: DecimalU64,
    /// Decimal-string creation instant.
    pub created_at_utc_ns: DecimalU64,
    /// Declared publication state.
    pub publication_state: PublicationState,
}

/// Required non-empty source scope for Profile 1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceScope {
    /// Provider identity.
    pub provider_id: Identifier,
    /// Source product identity.
    pub source_product_id: Identifier,
    /// Named venue identity.
    pub venue_id: Identifier,
    /// Named segment identity.
    pub segment_id: Identifier,
    /// Dataset identity.
    pub dataset_id: Identifier,
}

/// The complete canonical manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalManifest {
    /// Wire schema version.
    pub schema_version: u32,
    /// Single authoritative contract version location.
    pub contract_version: V2Contract,
    /// Reviewed contract revision.
    pub contract_revision: NonEmptyString,
    /// Release identity.
    pub release_identity: ReleaseIdentity,
    /// Required source scope.
    pub source_scope: SourceScope,
    /// Producer activity with typed operations and evidence.
    pub producer_activity: ProducerActivity,
    /// Full typed source-rights evidence.
    pub rights: RightsEvidence,
    /// Nine-dimension typed coverage.
    pub coverage: CoverageEvidence,
    /// Complete PROV lineage.
    pub lineage: Lineage,
    /// Schema versions and per-event payload registry.
    pub schema_versions: SchemaVersions,
    /// Dataset digest declaration.
    pub digests: Digests,
    /// Publisher identity.
    pub publication_identity: PublicationIdentity,
    /// Sole artifact inventory.
    pub artifacts: Vec<ArtifactDescriptor>,
}

/// Sequenced per-record ordering mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SequencedRecordOrdering {
    /// Discriminator.
    pub kind: SequencedKind,
    /// Sequence domain, matched to the artifact profile.
    pub sequence_domain_id: NonEmptyString,
    /// Sequence epoch, matched to the artifact profile.
    pub sequence_epoch: NonEmptyString,
    /// Decimal-string sequence number.
    pub sequence_number: DecimalU64,
}

/// Reference per-record ordering mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReferenceRecordOrdering {
    /// Discriminator.
    pub kind: ReferenceKind,
    /// Referenced object identity.
    pub object_id: Identifier,
    /// Decimal-string object revision.
    pub revision: DecimalU64,
    /// Decimal-string knowledge time.
    pub knowledge_time_ns: DecimalU64,
    /// Assertion identity.
    pub assertion_id: Identifier,
}

/// Tagged per-record ordering: exactly one mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum RecordOrdering {
    /// Sequenced mode.
    Sequenced(SequencedRecordOrdering),
    /// Reference mode.
    Reference(ReferenceRecordOrdering),
}

impl RecordOrdering {
    /// Return the ordering discriminator.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Sequenced(_) => "sequenced",
            Self::Reference(_) => "reference",
        }
    }
}

/// One typed abstract payload field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PayloadField {
    /// Field name from the registered payload schema.
    pub field_name: NonEmptyString,
    /// Scalar kind.
    pub kind: PayloadFieldKind,
    /// Scalar text interpreted by `kind`.
    pub value: String,
}

/// Strict abstract payload container.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    /// Typed abstract fields.
    pub fields: Vec<PayloadField>,
}

/// One canonical record envelope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalRecord {
    /// Envelope schema version.
    pub envelope_schema_version: NonEmptyString,
    /// Event type registered in the payload schema registry.
    pub event_type: Identifier,
    /// Tagged ordering union.
    pub ordering: RecordOrdering,
    /// Abstract typed payload.
    pub payload: Payload,
}

/// An ordering key recomputed from canonical record bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DerivedOrderKey {
    /// Sequenced key derived from domain, epoch and decimal sequence number.
    Sequenced(String, String, u64),
    /// Reference key derived from object, revision, knowledge time and assertion.
    Reference(String, u64, u64, String),
}

impl DerivedOrderKey {
    /// Return the key discriminator.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Sequenced(_, _, _) => "sequenced",
            Self::Reference(_, _, _, _) => "reference",
        }
    }
}

/// Parsed canonical manifest with the byte-ordering inputs extracted from it.
#[derive(Debug, Clone)]
pub struct ParsedManifest {
    /// Typed canonical manifest.
    pub manifest: CanonicalManifest,
    /// Artifact order profiles keyed by declared artifact path.
    pub profiles: HashMap<String, ArtifactRecordOrderProfile>,
    /// Registered event types keyed from the payload schema registry.
    pub event_types: HashSet<String>,
}

fn classify_json_error(error: serde_json::Error) -> CanonicalError {
    to_verifier_decode_error(error)
}

/// Parse a strict JSON value as a canonical manifest and validate its semantics.
pub fn parse_manifest(value: &Value) -> Result<ParsedManifest, CanonicalError> {
    let manifest: CanonicalManifest =
        serde_json::from_value(value.clone()).map_err(classify_json_error)?;
    if manifest.schema_version != WIRE_SCHEMA_VERSION {
        return Err(CanonicalError::Invalid(
            "canonical schema version is not registered".to_owned(),
        ));
    }
    validate_manifest_semantics(&manifest)?;
    let mut profiles = HashMap::new();
    for descriptor in &manifest.artifacts {
        profiles.insert(
            descriptor.path.as_str().to_owned(),
            descriptor.record_order_profile.clone(),
        );
    }
    let event_types = manifest
        .schema_versions
        .payload_schemas
        .iter()
        .map(|entry| entry.event_type.as_str().to_owned())
        .collect();
    Ok(ParsedManifest {
        manifest,
        profiles,
        event_types,
    })
}

/// Validate the cross-field invariants of a parsed canonical manifest.
pub fn validate_manifest_semantics(manifest: &CanonicalManifest) -> Result<(), CanonicalError> {
    let invalid = |detail: String| CanonicalError::Invalid(detail);
    for descriptor in &manifest.artifacts {
        let binding_ok = matches!(
            (descriptor.compression, descriptor.compression_version),
            (Compression::None, CompressionVersion::NoneV1)
                | (Compression::Gzip, CompressionVersion::GzipFlate2V1)
        );
        if !binding_ok {
            return Err(invalid(
                "compression declaration is not a registered pair".to_owned(),
            ));
        }
    }
    let mut artifact_keys = HashSet::new();
    for descriptor in &manifest.artifacts {
        let key = (
            descriptor.logical_partition_id.as_str().to_owned(),
            descriptor.shard_ordinal,
        );
        if !artifact_keys.insert(key) {
            return Err(invalid(
                "duplicate logical partition and shard ordinal".to_owned(),
            ));
        }
    }

    let mut operation_refs = HashSet::new();
    for operation in &manifest.producer_activity.performed_operations {
        if !operation_refs.insert(operation.operation_ref.as_str()) {
            return Err(invalid(
                "duplicate performed operation reference".to_owned(),
            ));
        }
    }
    let mut evidence_refs = HashSet::new();
    for evidence in &manifest.producer_activity.operation_evidence {
        let reference = evidence.operation_ref.as_str();
        if !evidence_refs.insert(reference) {
            return Err(invalid("duplicate operation evidence reference".to_owned()));
        }
        if !operation_refs.contains(reference) {
            return Err(invalid(
                "operation evidence is not bound to a performed operation".to_owned(),
            ));
        }
        if evidence.recorded_at_ns.value() > manifest.release_identity.as_of_ns.value() {
            return Err(invalid(
                "operation evidence recording instant is after as_of".to_owned(),
            ));
        }
    }
    if evidence_refs.len() != operation_refs.len() {
        return Err(invalid(
            "a performed operation has no evidence record".to_owned(),
        ));
    }

    let mut grant_ids = HashSet::new();
    for grant in &manifest.rights.permission_grants {
        if !grant_ids.insert(grant.grant_id.as_str()) {
            return Err(invalid("duplicate grant identity".to_owned()));
        }
        if grant.validity.begin_ns.value() > grant.validity.end_ns.value() {
            return Err(invalid("grant validity interval is inverted".to_owned()));
        }
        if grant.scope.data_time_begin_ns.value() > grant.scope.data_time_end_ns.value() {
            return Err(invalid(
                "grant scope data-time interval is inverted".to_owned(),
            ));
        }
        for reference in &grant.supporting_document_references {
            let found = manifest.rights.document_references.iter().any(|document| {
                document.reference.as_str() == reference.reference.as_str()
                    && document.document_sha256 == reference.digest_sha256
            });
            if !found {
                return Err(invalid("grant document reference is unresolved".to_owned()));
            }
        }
    }
    for attestation in &manifest.rights.reviewed_attestations {
        let grant = manifest
            .rights
            .permission_grants
            .iter()
            .find(|grant| grant.grant_id == attestation.grant_id)
            .ok_or_else(|| invalid("registered attestation names an absent grant".to_owned()))?;
        if grant.reviewed_attestation != attestation.attestation {
            return Err(invalid(
                "registered attestation differs from the grant attestation".to_owned(),
            ));
        }
    }

    if manifest.coverage.dimensions.len() < 9 {
        return Err(invalid(
            "coverage has fewer than nine dimensions".to_owned(),
        ));
    }
    let mut dimensions = HashSet::new();
    for dimension in &manifest.coverage.dimensions {
        if !dimensions.insert(&dimension.dimension_id) {
            return Err(invalid("duplicate coverage dimension".to_owned()));
        }
        let denominator = dimension.denominator.value();
        if denominator == 0 {
            return Err(invalid(
                "coverage denominator is required and non-zero".to_owned(),
            ));
        }
        let parts = dimension
            .numerator
            .value()
            .checked_add(dimension.missing_count.value())
            .and_then(|value| value.checked_add(dimension.rejected_count.value()))
            .ok_or_else(|| invalid("coverage arithmetic overflow".to_owned()))?;
        if parts != denominator {
            return Err(invalid(
                "coverage components do not decompose the denominator".to_owned(),
            ));
        }
        if dimension.as_of_ns.value() != manifest.release_identity.as_of_ns.value() {
            return Err(invalid(
                "coverage as-of differs from the release as-of".to_owned(),
            ));
        }
    }

    let lineage = &manifest.lineage;
    if lineage.knowledge_time_ns.value() > manifest.release_identity.as_of_ns.value() {
        return Err(invalid("lineage knowledge time is after as_of".to_owned()));
    }
    if lineage.invalidated_at_time.nanos() < lineage.knowledge_time_ns.value() {
        return Err(invalid(
            "lineage invalidation precedes knowledge time".to_owned(),
        ));
    }
    if let Some(supersedes) = &lineage.supersedes {
        if supersedes.revision >= lineage.revision {
            return Err(invalid(
                "supersedes revision is not an immediate predecessor".to_owned(),
            ));
        }
        if supersedes.supersedes_release_id.as_str()
            == manifest.release_identity.release_id.as_str()
        {
            return Err(invalid("release supplies its own predecessor".to_owned()));
        }
        if supersedes.knowledge_time_ns.value() > lineage.knowledge_time_ns.value() {
            return Err(invalid(
                "predecessor knowledge time is after the current revision".to_owned(),
            ));
        }
        if supersedes.invalidated_at_time.nanos() < supersedes.knowledge_time_ns.value() {
            return Err(invalid(
                "predecessor invalidation precedes its knowledge time".to_owned(),
            ));
        }
    }

    let mut event_types = HashSet::new();
    let mut schema_ids = HashSet::new();
    for entry in &manifest.schema_versions.payload_schemas {
        if !event_types.insert(entry.event_type.as_str()) {
            return Err(invalid("duplicate payload registry event type".to_owned()));
        }
        if !schema_ids.insert(entry.schema_id.as_str()) {
            return Err(invalid(
                "duplicate payload registry schema identity".to_owned(),
            ));
        }
    }
    Ok(())
}

fn internal_scope(scope: &ScopeDeclaration) -> InternalScopeDeclaration {
    InternalScopeDeclaration {
        provider_id: scope.provider_id.as_str().to_owned(),
        source_product_id: scope.source_product_id.as_str().to_owned(),
        venue_id: scope.venue_id.as_str().to_owned(),
        segment_id: scope.segment_id.as_str().to_owned(),
        subject_state: scope.subject_state.as_str().to_owned(),
        named_subjects: scope
            .named_subjects
            .iter()
            .map(|subject| subject.as_str().to_owned())
            .collect(),
        data_time_from_ns: Some(scope.data_time_begin_ns.value()),
        data_time_to_ns: Some(scope.data_time_end_ns.value()),
    }
}

fn internal_coverage_dimension_id(dimension: CoverageDimensionId) -> &'static str {
    match dimension {
        CoverageDimensionId::Instrument => "instrument",
        CoverageDimensionId::ListingIdentity => "listing_identity",
        CoverageDimensionId::Contract => "contract",
        CoverageDimensionId::CorporateAction => "corporate_action",
        CoverageDimensionId::SessionTime => "calendar_session",
        CoverageDimensionId::TradeEvent => "trades",
        CoverageDimensionId::QuoteDepth => "quotes_depth",
        CoverageDimensionId::Mbo => "mbo",
        CoverageDimensionId::SequenceContinuity => "sequence_continuity",
    }
}

fn payload_registry_version(entries: &[PayloadSchemaEntry]) -> Result<String, VerifierError> {
    let value = serde_json::to_value(entries).map_err(|error| VerifierError::ManifestSchema {
        detail: error.to_string(),
    })?;
    let digest = sha256(&canonical_bytes(&value));
    Ok(format!("payload-registry-v1:{}", hex_lower(&digest)))
}

/// Map a validated canonical manifest into the verifier's internal manifest type.
pub fn into_internal_manifest(wire: &ParsedManifest) -> Result<ReleaseManifest, VerifierError> {
    let manifest = &wire.manifest;
    let mapping_error = |detail: String| VerifierError::ManifestSchema { detail };
    let performed_operations = manifest
        .producer_activity
        .performed_operations
        .iter()
        .map(|operation| InternalPerformedOperation {
            operation_ref: operation.operation_ref.as_str().to_owned(),
            action: operation.action.as_str().to_owned(),
            object_kind: operation.data_object.kind.as_str().to_owned(),
            object_id: operation.data_object.id.as_str().to_owned(),
            object_version: operation.data_object.version.as_str().to_owned(),
        })
        .collect();
    let operation_evidence = manifest
        .producer_activity
        .operation_evidence
        .iter()
        .map(|evidence| InternalOperationEvidence {
            operation_ref: evidence.operation_ref.as_str().to_owned(),
            event_id: evidence.event_id.as_str().to_owned(),
            run_id: evidence.run_id.as_str().to_owned(),
            recorded_at_ns: evidence.recorded_at_ns.value(),
            decision_key: evidence.decision_key.as_str().to_owned(),
            digest_sha256: evidence.digest_sha256.as_str().to_owned(),
        })
        .collect();
    let grants = manifest
        .rights
        .permission_grants
        .iter()
        .map(|grant| crate::schema::PermissionGrant {
            grant_id: grant.grant_id.as_str().to_owned(),
            action: grant.action.as_str().to_owned(),
            object_class: grant.object_class.as_str().to_owned(),
            decision: grant.decision.as_str().to_owned(),
            actor: grant.actor.as_str().to_owned(),
            licensee: grant.licensee.as_str().to_owned(),
            recipient_class: grant.recipient_class.as_str().to_owned(),
            recipient_identity: grant.recipient_identity.as_str().to_owned(),
            scope: internal_scope(&grant.scope),
            validity_begin_ns: grant.validity.begin_ns.value(),
            validity_end_ns: grant.validity.end_ns.value(),
            delivery: grant.delivery.method.as_str().to_owned(),
            retention: RetentionPolicy {
                max_duration_ns: grant.retention.max_duration_ns.value(),
                purge_rule: grant.retention.purge_rule.as_str().to_owned(),
            },
        })
        .collect();
    let documents = manifest
        .rights
        .document_references
        .iter()
        .map(|document| InternalDocumentReference {
            reference: document.reference.as_str().to_owned(),
            clause: document.clause.as_str().to_owned(),
            document_sha256: document.document_sha256.as_str().to_owned(),
            contract_revision: document.contract_revision.as_str().to_owned(),
            reviewed_on: document.reviewed_on.as_str().to_owned(),
            authority: document.review_authority.as_str().to_owned(),
        })
        .collect();
    let attestations = manifest
        .rights
        .reviewed_attestations
        .iter()
        .map(|registered| crate::schema::ReviewedAttestation {
            purpose: registered.attestation.purpose.as_str().to_owned(),
            usage_right: registered.attestation.usage_right.as_str().to_owned(),
            usage_mode: registered.attestation.usage_mode.as_str().to_owned(),
            territory: registered.attestation.territory.as_str().to_owned(),
            scope_assertions: registered
                .attestation
                .scope_assertions
                .iter()
                .map(|assertion| assertion.as_str().to_owned())
                .collect(),
            reviewer: registered.attestation.reviewer.as_str().to_owned(),
            authority: registered.attestation.authority.as_str().to_owned(),
        })
        .collect();
    let dimensions = manifest
        .coverage
        .dimensions
        .iter()
        .map(|dimension| InternalCoverageDimension {
            dimension_id: internal_coverage_dimension_id(dimension.dimension_id).to_owned(),
            universe_name: dimension.universe_id.as_str().to_owned(),
            denominator_identity: dimension.denominator_identity.as_str().to_owned(),
            denominator: Some(dimension.denominator.value()),
            numerator: dimension.numerator.value(),
            missing_count: dimension.missing_count.value(),
            rejected_count: dimension.rejected_count.value(),
            gaps: dimension
                .gaps
                .iter()
                .map(|gap| gap.description.as_str().to_owned())
                .collect(),
            verdict: dimension.verdict.as_str().to_owned(),
            as_of_ns: dimension.as_of_ns.value(),
            evidence_reference: dimension.evidence_reference.as_str().to_owned(),
        })
        .collect();
    let lineage = &manifest.lineage;
    let supersedes = lineage
        .supersedes
        .as_ref()
        .map(|relation| SupersedesRelation {
            supersedes_release_id: relation.supersedes_release_id.as_str().to_owned(),
            supersedes_manifest_sha256: relation.supersedes_manifest_sha256.as_str().to_owned(),
            supersedes_data_sha256: relation.supersedes_data_sha256.as_str().to_owned(),
            revision: relation.revision,
            correction_reason: relation.correction_reason.as_str().to_owned(),
            knowledge_time_ns: relation.knowledge_time_ns.value(),
            invalidated_at_time_ns: relation.invalidated_at_time.nanos(),
        });
    let artifacts: Vec<crate::schema::ArtifactDescriptor> = manifest
        .artifacts
        .iter()
        .map(|descriptor| crate::schema::ArtifactDescriptor {
            logical_partition_id: descriptor.logical_partition_id.as_str().to_owned(),
            shard_ordinal: descriptor.shard_ordinal,
            path: descriptor.path.as_str().to_owned(),
            record_order_profile: match descriptor.record_order_profile {
                ArtifactRecordOrderProfile::Sequenced(_) => "sequence-domain-declared-gaps",
                ArtifactRecordOrderProfile::Reference(_) => "reference-key",
            }
            .to_owned(),
            encoding: descriptor.encoding.as_str().to_owned(),
            compression: descriptor.compression.as_str().to_owned(),
            compression_version: 1,
            stored_bytes: descriptor.stored_bytes.value(),
            stored_sha256: descriptor.stored_sha256.as_str().to_owned(),
            uncompressed_bytes: descriptor.uncompressed_bytes.value(),
            uncompressed_sha256: descriptor.uncompressed_sha256.as_str().to_owned(),
            record_count: descriptor.record_count.value(),
        })
        .collect();
    if artifacts.is_empty() {
        return Err(mapping_error(
            "canonical manifest has no artifacts".to_owned(),
        ));
    }
    Ok(ReleaseManifest {
        schema_version: manifest.schema_version,
        contract_version: manifest.contract_version.as_str().to_owned(),
        contract_revision: manifest.contract_revision.as_str().to_owned(),
        release_identity: InternalReleaseIdentity {
            release_id: manifest.release_identity.release_id.as_str().to_owned(),
            release_version: manifest
                .release_identity
                .release_version
                .as_str()
                .to_owned(),
            as_of_ns: manifest.release_identity.as_of_ns.value(),
            created_at_utc_ns: manifest.release_identity.created_at_utc_ns.value(),
            publication_state: manifest
                .release_identity
                .publication_state
                .as_str()
                .to_owned(),
        },
        source_scope: crate::schema::SourceScope {
            provider_id: manifest.source_scope.provider_id.as_str().to_owned(),
            source_product_id: manifest.source_scope.source_product_id.as_str().to_owned(),
            venue_id: Some(manifest.source_scope.venue_id.as_str().to_owned()),
            segment_id: manifest.source_scope.segment_id.as_str().to_owned(),
            dataset_id: manifest.source_scope.dataset_id.as_str().to_owned(),
        },
        producer_activity: crate::schema::ProducerActivity {
            performed_operations,
            operation_evidence,
        },
        rights: InternalRightsRecord {
            permission_grants: grants,
            document_references: documents,
            reviewed_attestations: attestations,
        },
        coverage: InternalCoverageRecord { dimensions },
        lineage: InternalLineage {
            raw_archive_reference: lineage.raw_archive_reference.as_str().to_owned(),
            raw_archive_sha256: lineage.raw_archive_sha256.as_str().to_owned(),
            transformation_record: format!(
                "{}:{}",
                lineage.transformation_record.identity.as_str(),
                lineage.transformation_record.version.as_str()
            ),
            was_derived_from: lineage
                .was_derived_from
                .source_entity_id
                .as_str()
                .to_owned(),
            had_primary_source: lineage
                .had_primary_source
                .source_entity_id
                .as_str()
                .to_owned(),
            revision: lineage.revision,
            knowledge_time_ns: lineage.knowledge_time_ns.value(),
            invalidated_at_time_ns: lineage.invalidated_at_time.nanos(),
            supersedes,
        },
        schema_versions: crate::schema::SchemaVersions {
            contract_version: manifest.contract_version.as_str().to_owned(),
            normalizer_version: manifest
                .schema_versions
                .normalizer_version
                .as_str()
                .to_owned(),
            payload_schema_version: payload_registry_version(
                &manifest.schema_versions.payload_schemas,
            )?,
            envelope_version: manifest
                .schema_versions
                .envelope_version
                .as_str()
                .to_owned(),
        },
        digests: crate::schema::Digests {
            data_sha256: manifest.digests.data_sha256.as_str().to_owned(),
            digest_algorithm: manifest.digests.digest_algorithm.as_str().to_owned(),
        },
        publication_identity: crate::schema::PublicationIdentity {
            publisher_identity: manifest
                .publication_identity
                .publisher_identity
                .as_str()
                .to_owned(),
            signing_key_id: manifest
                .publication_identity
                .signing_key_id
                .as_str()
                .to_owned(),
            signature_reference: manifest
                .publication_identity
                .signature_reference
                .as_str()
                .to_owned(),
            archive_reference: manifest
                .publication_identity
                .archive_reference
                .as_str()
                .to_owned(),
        },
        artifacts,
    })
}

/// Map a canonical manifest into the internal admission request shape.
pub fn into_internal_request(
    wire: &ParsedManifest,
    manifest_sha256: &str,
) -> Result<AdmissionRequest, VerifierError> {
    let manifest = &wire.manifest;
    let grant =
        manifest
            .rights
            .permission_grants
            .first()
            .ok_or_else(|| VerifierError::ManifestSchema {
                detail: "no permission grant is available to bind the request".to_owned(),
            })?;
    Ok(AdmissionRequest {
        release_id: manifest.release_identity.release_id.as_str().to_owned(),
        manifest_sha256: manifest_sha256.to_owned(),
        data_sha256: manifest.digests.data_sha256.as_str().to_owned(),
        publisher_identity: manifest
            .publication_identity
            .publisher_identity
            .as_str()
            .to_owned(),
        actor: grant.actor.as_str().to_owned(),
        use_profile: "reference_lookup".to_owned(),
        requested_actions: vec!["read_release".to_owned()],
        purpose: DECLARED_PURPOSE.to_owned(),
        licensee: grant.licensee.as_str().to_owned(),
        recipient_class: grant.recipient_class.as_str().to_owned(),
        recipient_identity: grant.recipient_identity.as_str().to_owned(),
        scope: internal_scope(&grant.scope),
        requested_retention: RetentionPolicy {
            max_duration_ns: grant.retention.max_duration_ns.value(),
            purge_rule: grant.retention.purge_rule.as_str().to_owned(),
        },
        as_of_ns: manifest.release_identity.as_of_ns.value(),
        declared_capabilities: Vec::new(),
    })
}

/// Parse one strict JSON value as a canonical record envelope.
pub fn parse_record(value: &Value) -> Result<CanonicalRecord, CanonicalError> {
    let record: CanonicalRecord =
        serde_json::from_value(value.clone()).map_err(classify_json_error)?;
    for field in &record.payload.fields {
        match field.kind {
            PayloadFieldKind::String => {}
            PayloadFieldKind::DecimalString => {
                if !is_decimal_text(&field.value) {
                    return Err(CanonicalError::Invalid(
                        "payload decimal field grammar".to_owned(),
                    ));
                }
            }
            PayloadFieldKind::Boolean => {
                if field.value != "true" && field.value != "false" {
                    return Err(CanonicalError::Invalid(
                        "payload boolean field grammar".to_owned(),
                    ));
                }
            }
            PayloadFieldKind::BytesHex => {
                if field.value.is_empty()
                    || !field
                        .value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(CanonicalError::Invalid(
                        "payload bytes_hex field grammar".to_owned(),
                    ));
                }
            }
        }
    }
    Ok(record)
}

fn ordering_value<'a>(value: &'a Value, member: &str) -> Result<&'a Value, CanonicalError> {
    value
        .get(member)
        .ok_or_else(|| CanonicalError::Invalid(format!("ordering member {} is absent", member)))
}

fn raw_identifier(value: &Value, member: &str) -> Result<String, CanonicalError> {
    let text = ordering_value(value, member)?.as_str().ok_or_else(|| {
        CanonicalError::Invalid(format!("ordering member {} is not text", member))
    })?;
    if !is_identifier(text) {
        return Err(CanonicalError::Invalid(format!(
            "ordering member {} is not a valid identifier",
            member
        )));
    }
    Ok(text.to_owned())
}

fn raw_non_empty(value: &Value, member: &str) -> Result<String, CanonicalError> {
    let text = ordering_value(value, member)?.as_str().ok_or_else(|| {
        CanonicalError::Invalid(format!("ordering member {} is not text", member))
    })?;
    if text.is_empty() {
        return Err(CanonicalError::Invalid(format!(
            "ordering member {} is empty",
            member
        )));
    }
    Ok(text.to_owned())
}

fn raw_decimal(value: &Value, member: &str) -> Result<u64, CanonicalError> {
    let text = ordering_value(value, member)?.as_str().ok_or_else(|| {
        CanonicalError::Invalid(format!("ordering member {} is not text", member))
    })?;
    if !is_decimal_text(text) {
        return Err(CanonicalError::Invalid(format!(
            "ordering member {} is not a canonical decimal string",
            member
        )));
    }
    text.parse::<u64>()
        .map_err(|_| CanonicalError::Invalid(format!("ordering member {} exceeds u64", member)))
}

/// Derive an ordering key independently from received canonical record bytes.
///
/// The byte sequence is parsed twice: once into the strict typed envelope and
/// once as raw JSON members from which the key is recomputed. A re-serialized
/// variant of a received line is refused because its bytes differ.
pub fn derive_key_from_bytes(
    bytes: &[u8],
    profile: &ArtifactRecordOrderProfile,
) -> Result<DerivedOrderKey, CanonicalError> {
    let value = parse_strict(bytes).map_err(|error| {
        let detail = error.to_string();
        if detail.contains("duplicate JSON member") {
            CanonicalError::DuplicateMember(detail)
        } else {
            CanonicalError::Invalid("record bytes are not strict JSON".to_owned())
        }
    })?;
    if canonical_bytes(&value) != bytes {
        return Err(CanonicalError::Ordering(
            "key source bytes are not canonical JCS bytes".to_owned(),
        ));
    }
    let typed = parse_record(&value)?;
    let raw_ordering = value
        .get("ordering")
        .ok_or_else(|| CanonicalError::Invalid("record has no ordering member".to_owned()))?;
    let kind = ordering_value(raw_ordering, "kind")?
        .as_str()
        .ok_or_else(|| CanonicalError::Invalid("ordering kind is not text".to_owned()))?;
    let (derived, typed_derived) = match profile {
        ArtifactRecordOrderProfile::Sequenced(artifact) => {
            if kind != "sequenced" {
                return Err(CanonicalError::Ordering(
                    "record ordering mode differs from artifact profile".to_owned(),
                ));
            }
            let domain = raw_non_empty(raw_ordering, "sequence_domain_id")?;
            let epoch = raw_non_empty(raw_ordering, "sequence_epoch")?;
            let number = raw_decimal(raw_ordering, "sequence_number")?;
            if domain != artifact.sequence_domain_id.as_str()
                || epoch != artifact.sequence_epoch.as_str()
            {
                return Err(CanonicalError::Ordering(
                    "record domain or epoch differs from artifact profile".to_owned(),
                ));
            }
            let typed_key = match &typed.ordering {
                RecordOrdering::Sequenced(ordering) => DerivedOrderKey::Sequenced(
                    ordering.sequence_domain_id.as_str().to_owned(),
                    ordering.sequence_epoch.as_str().to_owned(),
                    ordering.sequence_number.value(),
                ),
                RecordOrdering::Reference(_) => {
                    return Err(CanonicalError::Ordering("record mode mismatch".to_owned()));
                }
            };
            (DerivedOrderKey::Sequenced(domain, epoch, number), typed_key)
        }
        ArtifactRecordOrderProfile::Reference(_) => {
            if kind != "reference" {
                return Err(CanonicalError::Ordering(
                    "record ordering mode differs from artifact profile".to_owned(),
                ));
            }
            let object_id = raw_identifier(raw_ordering, "object_id")?;
            let revision = raw_decimal(raw_ordering, "revision")?;
            let knowledge = raw_decimal(raw_ordering, "knowledge_time_ns")?;
            let assertion = raw_identifier(raw_ordering, "assertion_id")?;
            let typed_key = match &typed.ordering {
                RecordOrdering::Reference(ordering) => DerivedOrderKey::Reference(
                    ordering.object_id.as_str().to_owned(),
                    ordering.revision.value(),
                    ordering.knowledge_time_ns.value(),
                    ordering.assertion_id.as_str().to_owned(),
                ),
                RecordOrdering::Sequenced(_) => {
                    return Err(CanonicalError::Ordering("record mode mismatch".to_owned()));
                }
            };
            (
                DerivedOrderKey::Reference(object_id, revision, knowledge, assertion),
                typed_key,
            )
        }
    };
    if derived != typed_derived {
        return Err(CanonicalError::Ordering(
            "ordering metadata does not recompute from record bytes".to_owned(),
        ));
    }
    Ok(derived)
}

/// Reject a caller-supplied key that disagrees with the byte-derived key.
pub fn check_supplied_key(
    supplied: &DerivedOrderKey,
    derived: &DerivedOrderKey,
) -> Result<(), CanonicalError> {
    if supplied != derived {
        return Err(CanonicalError::Ordering(
            "caller-supplied ordering key disagrees with record bytes".to_owned(),
        ));
    }
    Ok(())
}

/// Verify that byte-derived keys are strictly ordered with visible gaps only.
pub fn validate_order_sequence(keys: &[DerivedOrderKey]) -> Result<(), CanonicalError> {
    let mut previous: Option<&DerivedOrderKey> = None;
    for key in keys {
        if let Some(earlier) = previous {
            if earlier.kind() != key.kind() {
                return Err(CanonicalError::Ordering(
                    "mixed ordering modes in one artifact".to_owned(),
                ));
            }
            if earlier >= key {
                return Err(CanonicalError::Ordering(
                    "byte-derived keys are not strictly ascending".to_owned(),
                ));
            }
        }
        previous = Some(key);
    }
    Ok(())
}

/// Map a canonical record into the verifier's internal record type.
pub fn into_internal_record(record: &CanonicalRecord) -> ReleaseRecord {
    let (
        sequence_domain_id,
        sequence_epoch,
        sequence_number,
        object_id,
        revision,
        knowledge_time_ns,
        assertion_id,
    ) = match &record.ordering {
        RecordOrdering::Sequenced(ordering) => (
            ordering.sequence_domain_id.as_str().to_owned(),
            ordering.sequence_epoch.as_str().to_owned(),
            ordering.sequence_number.value(),
            String::new(),
            0_u64,
            0_u64,
            String::new(),
        ),
        RecordOrdering::Reference(ordering) => (
            String::new(),
            String::new(),
            0_u64,
            ordering.object_id.as_str().to_owned(),
            ordering.revision.value(),
            ordering.knowledge_time_ns.value(),
            ordering.assertion_id.as_str().to_owned(),
        ),
    };
    let mut payload = serde_json::Map::new();
    for field in &record.payload.fields {
        payload.insert(
            field.field_name.as_str().to_owned(),
            Value::String(field.value.clone()),
        );
    }
    ReleaseRecord {
        sequence_domain_id,
        sequence_epoch,
        sequence_number,
        object_id,
        revision,
        knowledge_time_ns,
        assertion_id,
        envelope_schema_version: record.envelope_schema_version.as_str().to_owned(),
        payload,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_text_rejects_signs_and_leading_zeroes() {
        assert!(is_decimal_text("0"), "zero is the canonical decimal zero");
        assert!(!is_decimal_text("00"), "leading zeroes are refused");
        assert!(!is_decimal_text("+1"), "a sign is refused");
        assert!(!is_decimal_text("1.0"), "a fraction is refused");
    }

    #[test]
    fn utc_string_rejects_wrong_precision() {
        assert!(
            parse_utc_nanos("2026-01-01T00:00:00.000000000Z").is_some(),
            "nine fractional digits are required"
        );
        assert!(
            parse_utc_nanos("2026-01-01T00:00:00.000000Z").is_none(),
            "six fractional digits are refused"
        );
    }

    #[test]
    fn utc_string_rejects_leap_second() {
        assert!(
            parse_utc_nanos("2026-01-01T00:00:60.000000000Z").is_none(),
            "leap seconds are outside the UTC profile"
        );
    }
}
