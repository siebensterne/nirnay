#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! Corporate financial facts with explicit provenance and point-in-time semantics.
//!
//! This crate is pure by design: it performs no filesystem, network, database,
//! clock, broker or exchange operations. Untrusted wire records are deserialized
//! into typed values and must pass [`FinancialFact::validate`] before use.
//!
//! A reported value is not interchangeable with a derived value. A revision is
//! append-only and names the revision it supersedes. Historical selection requires
//! both a known publication timestamp and a committed ingestion timestamp.

use std::collections::{
    BTreeMap, //
    BTreeSet,
    VecDeque,
}; //

use jiff::Timestamp;
use jiff::civil::Date;
use serde::{
    Deserialize, //
    Serialize,
}; //
use sha2::{
    Digest, //
    Sha256,
}; //
use thiserror::Error;

pub const FINANCIAL_FACT_SCHEMA_VERSION: u32 = 1;
pub const MAX_DECIMAL_SCALE: u8 = 18;
const COMPANY_IDENTITY_KEY_DOMAIN: &[u8] = b"nirnay:company-identity-key:v1\0";
const FINANCIAL_FACT_KEY_DOMAIN: &[u8] = b"nirnay:financial-fact-key:v1\0";
const DERIVATION_HASH_DOMAIN: &[u8] = b"nirnay:derivation-hash:v1\0";

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DomainError {
    #[error("{field} must not be empty")]
    Empty { field: &'static str },
    #[error("{field} has invalid identifier syntax")]
    InvalidIdentifier { field: &'static str },
    #[error("invalid company CIN: {0}")]
    InvalidCin(String),
    #[error("unsupported financial fact schema version {0}")]
    UnsupportedSchema(u32),
    #[error("invalid date or reporting period: {0}")]
    InvalidPeriod(String),
    #[error("invalid exact decimal: {0}")]
    InvalidDecimal(String),
    #[error("invalid unit: {0}")]
    InvalidUnit(String),
    #[error("invalid concept QName: {0}")]
    InvalidConcept(String),
    #[error("invalid source evidence: {0}")]
    InvalidEvidence(String),
    #[error("fact key mismatch: expected {expected}, found {actual}")]
    FactKeyMismatch { expected: String, actual: String },
    #[error("revision {revision} for fact {fact_key} has invalid predecessor")]
    InvalidRevision { fact_key: String, revision: u32 },
    #[error("duplicate fact revision id {0}")]
    DuplicateRevisionId(String),
    #[error("derived fact {fact_id} references missing input {input_id}")]
    MissingDerivedInput { fact_id: String, input_id: String },
    #[error("derived fact dependency cycle includes {0}")]
    DerivedCycle(String),
    #[error("derived fact {fact_id} was ingested before input {input_id}")]
    DerivedInputNotKnown { fact_id: String, input_id: String },
}

fn validate_identifier(value: &str, field: &'static str) -> Result<(), DomainError> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || !(bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(byte))
    {
        return Err(DomainError::InvalidIdentifier { field });
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_json_pointer(pointer: &str) -> bool {
    let bytes = pointer.as_bytes();
    if bytes.is_empty() {
        return true;
    }
    if bytes[0] != b'/' {
        return false;
    }
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'~' {
            let Some(next) = bytes.get(index + 1) else {
                return false;
            };
            if !matches!(*next, b'0' | b'1') {
                return false;
            }
            index += 1;
        }
        index += 1;
    }
    true
}

/// Opaque Nirnay identity. It is not derived from a company name, CIN or ticker.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CompanyId(String);

impl CompanyId {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        validate_identifier(&value, "company_id")?;
        Ok(Self(value))
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        validate_identifier(&self.0, "company_id")
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Indian Corporate Identity Number in its 21-character form.
///
/// Validation checks the published positional structure, not MCA existence or
/// current legal status. Those require source-backed company-master evidence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Cin(String);

impl TryFrom<String> for Cin {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let bytes = value.as_bytes();
        let valid = bytes.len() == 21
            && matches!(bytes[0], b'L' | b'U')
            && bytes[1..6].iter().all(u8::is_ascii_digit)
            && bytes[6..8].iter().all(u8::is_ascii_uppercase)
            && bytes[8..12].iter().all(u8::is_ascii_digit)
            && bytes[12..15].iter().all(u8::is_ascii_uppercase)
            && bytes[15..21].iter().all(u8::is_ascii_digit);
        if !valid {
            return Err(DomainError::InvalidCin(value));
        }
        Ok(Self(value))
    }
}

impl From<Cin> for String {
    fn from(value: Cin) -> Self {
        value.0
    }
}

impl Cin {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        value.into().try_into()
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// XBRL expanded name: absolute namespace URI plus an ASCII NCName subset.
/// Non-ASCII taxonomy local names are rejected in schema v1 by design.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpandedName {
    pub namespace_uri: String,
    pub local_name: String,
}

impl ExpandedName {
    pub fn validate(&self) -> Result<(), DomainError> {
        let Some((scheme, remainder)) = self.namespace_uri.split_once(':') else {
            return Err(DomainError::InvalidConcept(
                "namespace URI must be absolute".to_owned(),
            ));
        };
        let mut scheme_chars = scheme.chars();
        let valid_scheme = scheme_chars
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic())
            && scheme_chars.all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '+' | '.' | '-')
            });
        if !valid_scheme
            || remainder.is_empty()
            || self
                .namespace_uri
                .chars()
                .any(|character| character.is_whitespace() || character.is_control())
        {
            return Err(DomainError::InvalidConcept(
                "namespace URI must be an absolute URI without whitespace".to_owned(),
            ));
        }
        let mut local_chars = self.local_name.chars();
        let valid_first = local_chars
            .next()
            .is_some_and(|first| first == '_' || first.is_ascii_alphabetic());
        let valid_rest = local_chars.all(|character| {
            character == '_'
                || character == '-'
                || character == '.'
                || character.is_ascii_alphanumeric()
        });
        if !valid_first || !valid_rest {
            return Err(DomainError::InvalidConcept(
                "local name is outside the supported ASCII NCName subset".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Financial reporting concept, represented independently of vendor taxonomy labels.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinancialConcept {
    pub name: ExpandedName,
}

impl FinancialConcept {
    pub fn validate(&self) -> Result<(), DomainError> {
        self.name.validate()
    }
}

/// One XBRL dimensional member. A vector is used instead of a map because
/// JSON object keys cannot safely represent expanded XML names.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DimensionMember {
    pub axis: ExpandedName,
    pub member: ExpandedName,
}

impl DimensionMember {
    pub fn validate(&self) -> Result<(), DomainError> {
        self.axis.validate()?;
        self.member.validate()
    }
}

/// Exact base-10 number. `coefficient` is a canonical signed integer string;
/// `scale` is the number of decimal places. No binary floating point is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactDecimal {
    pub coefficient: String,
    pub scale: u8,
}

impl ExactDecimal {
    pub fn new(coefficient: impl Into<String>, scale: u8) -> Result<Self, DomainError> {
        let value = Self {
            coefficient: coefficient.into(),
            scale,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        let text = self.coefficient.as_str();
        let digits = text.strip_prefix('-').unwrap_or(text);
        let canonical = !digits.is_empty()
            && digits.bytes().all(|byte| byte.is_ascii_digit())
            && (digits == "0" || !digits.starts_with('0'))
            && text != "-0";
        if !canonical || self.scale > MAX_DECIMAL_SCALE || text.parse::<i128>().is_err() {
            return Err(DomainError::InvalidDecimal(format!(
                "coefficient must be canonical i128 and scale must be <= {MAX_DECIMAL_SCALE}"
            )));
        }
        Ok(())
    }

    pub fn checked_add(&self, other: &Self) -> Result<Self, DomainError> {
        self.validate()?;
        other.validate()?;
        let scale = self.scale.max(other.scale);
        let left = self
            .coefficient
            .parse::<i128>()
            .map_err(|_| DomainError::InvalidDecimal("left coefficient outside i128".to_owned()))?;
        let right = other.coefficient.parse::<i128>().map_err(|_| {
            DomainError::InvalidDecimal("right coefficient outside i128".to_owned())
        })?;
        let left_factor = checked_power10(scale - self.scale)?;
        let right_factor = checked_power10(scale - other.scale)?;
        let sum = left
            .checked_mul(left_factor)
            .and_then(|value| {
                right
                    .checked_mul(right_factor)
                    .and_then(|right_value| value.checked_add(right_value))
            })
            .ok_or_else(|| DomainError::InvalidDecimal("addition overflow".to_owned()))?;
        Self::new(sum.to_string(), scale)
    }

    pub fn checked_sub(&self, other: &Self) -> Result<Self, DomainError> {
        self.validate()?;
        other.validate()?;
        let scale = self.scale.max(other.scale);
        let left = self
            .coefficient
            .parse::<i128>()
            .map_err(|_| DomainError::InvalidDecimal("left coefficient outside i128".to_owned()))?;
        let right = other.coefficient.parse::<i128>().map_err(|_| {
            DomainError::InvalidDecimal("right coefficient outside i128".to_owned())
        })?;
        let left_factor = checked_power10(scale - self.scale)?;
        let right_factor = checked_power10(scale - other.scale)?;
        let difference = left
            .checked_mul(left_factor)
            .and_then(|value| {
                right
                    .checked_mul(right_factor)
                    .and_then(|right_value| value.checked_sub(right_value))
            })
            .ok_or_else(|| DomainError::InvalidDecimal("subtraction overflow".to_owned()))?;
        Self::new(difference.to_string(), scale)
    }
}

fn checked_power10(exponent: u8) -> Result<i128, DomainError> {
    10_i128
        .checked_pow(u32::from(exponent))
        .ok_or_else(|| DomainError::InvalidDecimal("decimal scale overflow".to_owned()))
}

/// Currency code syntax. Membership in ISO 4217 is established by a separately
/// versioned currency reference dataset, not guessed by this value type.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CurrencyCode(String);

impl CurrencyCode {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        if value.len() != 3 || !value.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(DomainError::InvalidUnit(value));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Units are explicit; magnitudes are never inferred from a column heading.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FinancialUnit {
    Currency {
        currency: CurrencyCode,
        magnitude: Magnitude,
    },
    Shares,
    PerShare {
        currency: CurrencyCode,
    },
    Ratio,
    Percent,
    Count,
    Other {
        namespace_uri: String,
        local_name: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Magnitude {
    Unit,
    Thousand,
    Lakh,
    Million,
    Crore,
    Billion,
}

impl FinancialUnit {
    pub fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::Currency { currency, .. } | Self::PerShare { currency } => {
                CurrencyCode::new(currency.as_str())?;
            }
            Self::Other {
                namespace_uri,
                local_name,
            } => {
                ExpandedName {
                    namespace_uri: namespace_uri.clone(),
                    local_name: local_name.clone(),
                }
                .validate()?;
            }
            Self::Shares | Self::Ratio | Self::Percent | Self::Count => {}
        }
        Ok(())
    }
}

/// Financial period is either an instant or a duration. Quarters, half-years
/// and financial years are represented as durations, not labels.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReportingPeriod {
    Instant { as_of: Date },
    Duration { start: Date, end: Date },
}

impl ReportingPeriod {
    pub fn validate(&self) -> Result<(), DomainError> {
        if let Self::Duration { start, end } = self
            && start > end
        {
            return Err(DomainError::InvalidPeriod(
                "duration start is after end".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatementScope {
    Standalone,
    Consolidated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AccountingBasis {
    IndAs,
    IndianGaap,
    Ifrs,
    UsGaap,
    Other { name: String },
}

impl AccountingBasis {
    pub fn validate(&self) -> Result<(), DomainError> {
        if let Self::Other { name } = self
            && name.trim().is_empty()
        {
            return Err(DomainError::Empty {
                field: "accounting_basis.name",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceLocator {
    Xbrl {
        fact_id: String,
        context_ref: String,
        unit_ref: String,
    },
    PdfTable {
        page: u32,
        table: Option<u32>,
        row_label: String,
        column_label: String,
    },
    JsonPointer {
        pointer: String,
    },
}

impl EvidenceLocator {
    pub fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::Xbrl {
                fact_id,
                context_ref,
                unit_ref,
            } => {
                for (field, value) in [
                    ("xbrl.fact_id", fact_id),
                    ("xbrl.context_ref", context_ref),
                    ("xbrl.unit_ref", unit_ref),
                ] {
                    if value.trim().is_empty() {
                        return Err(DomainError::Empty { field });
                    }
                }
            }
            Self::PdfTable {
                page,
                table,
                row_label,
                column_label,
            } => {
                if *page == 0
                    || table.is_some_and(|number| number == 0)
                    || row_label.trim().is_empty()
                    || column_label.trim().is_empty()
                {
                    return Err(DomainError::InvalidEvidence(
                        "PDF evidence needs positive page/table numbers and row/column labels"
                            .to_owned(),
                    ));
                }
            }
            Self::JsonPointer { pointer } => {
                if !valid_json_pointer(pointer) {
                    return Err(DomainError::InvalidEvidence(
                        "JSON Pointer must follow RFC 6901 escaping".to_owned(),
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FactOrigin {
    Reported {
        source_id: String,
        filing_id: String,
        document_sha256: String,
        locator: EvidenceLocator,
    },
    Derived {
        method_id: String,
        method_version: String,
        input_fact_ids: Vec<String>,
        derivation_sha256: String,
    },
}

/// SHA-256 over the canonical derivation commitment: domain separator, method
/// identity and version, sorted unique input ids, canonical output value and
/// the output fact id.
pub fn expected_derivation_sha256(
    method_id: &str,
    method_version: &str,
    input_fact_ids: &[String],
    value: &ExactDecimal,
    fact_id: &str,
) -> Result<String, DomainError> {
    let mut sorted_inputs = input_fact_ids.to_vec();
    sorted_inputs.sort();
    sorted_inputs.dedup();
    let canonical = serde_json::to_vec(&DerivationHashInput {
        method_id,
        method_version,
        input_fact_ids: &sorted_inputs,
        value,
        fact_id,
    })
    .map_err(|_| DomainError::InvalidEvidence("derivation hash serialization failed".to_owned()))?;
    let mut digest = Sha256::new();
    digest.update(DERIVATION_HASH_DOMAIN);
    digest.update(&canonical);
    Ok(format!("{:x}", digest.finalize()))
}

#[derive(Serialize)]
struct DerivationHashInput<'a> {
    method_id: &'a str,
    method_version: &'a str,
    input_fact_ids: &'a [String],
    value: &'a ExactDecimal,
    fact_id: &'a str,
}

impl FactOrigin {
    pub fn validate(&self, fact_id: &str, value: &ExactDecimal) -> Result<(), DomainError> {
        match self {
            Self::Reported {
                source_id,
                filing_id,
                document_sha256,
                locator,
            } => {
                validate_identifier(source_id, "source_id")?;
                validate_identifier(filing_id, "filing_id")?;
                if !valid_sha256(document_sha256) {
                    return Err(DomainError::InvalidEvidence(
                        "document_sha256 must be lowercase SHA-256".to_owned(),
                    ));
                }
                locator.validate()?;
            }
            Self::Derived {
                method_id,
                method_version,
                input_fact_ids,
                derivation_sha256,
            } => {
                validate_identifier(method_id, "method_id")?;
                if method_version.trim().is_empty()
                    || method_version.chars().any(char::is_whitespace)
                {
                    return Err(DomainError::InvalidEvidence(
                        "derived method version must be non-empty and contain no whitespace"
                            .to_owned(),
                    ));
                }
                if input_fact_ids.is_empty() || !valid_sha256(derivation_sha256) {
                    return Err(DomainError::InvalidEvidence(
                        "derived facts require inputs and a lowercase derivation SHA-256"
                            .to_owned(),
                    ));
                }
                // The digest commits to the method, its version, the sorted
                // input identity set and the exact output value. It cannot be
                // a self-declared arbitrary digest.
                let expected_hash = expected_derivation_sha256(
                    method_id,
                    method_version,
                    input_fact_ids,
                    value,
                    fact_id,
                )?;
                if expected_hash != *derivation_sha256 {
                    return Err(DomainError::InvalidEvidence(format!(
                        "derivation SHA-256 for {fact_id} does not match method, inputs and value"
                    )));
                }
                let mut seen = BTreeSet::new();
                for input in input_fact_ids {
                    validate_identifier(input, "input_fact_id")?;
                    if input == fact_id || !seen.insert(input) {
                        return Err(DomainError::InvalidEvidence(
                            "derived input IDs must be unique and cannot reference the output"
                                .to_owned(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

/// One source-backed, effective-dated company identity observation. A correction
/// appends a new revision; it never mutates the prior record. `valid_*` describes
/// when the legal name/CIN applied, while `recorded_at` describes when Nirnay
/// learned the observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanyIdentityRecord {
    pub schema_version: u32,
    pub record_id: String,
    pub company_id: CompanyId,
    pub identity_key: String,
    pub revision: u32,
    pub supersedes: Option<String>,
    pub cin: Option<Cin>,
    pub legal_name: String,
    pub valid_from: Date,
    pub valid_to: Option<Date>,
    pub recorded_at: Timestamp,
    pub source_id: String,
    pub source_document_sha256: String,
    pub locator: EvidenceLocator,
}

impl CompanyIdentityRecord {
    pub fn expected_identity_key(&self) -> Result<String, DomainError> {
        #[derive(Serialize)]
        struct IdentityKey<'a> {
            company_id: &'a CompanyId,
            valid_from: Date,
        }
        let bytes = serde_json::to_vec(&IdentityKey {
            company_id: &self.company_id,
            valid_from: self.valid_from,
        })
        .map_err(|_| {
            DomainError::InvalidEvidence("company identity serialization failed".to_owned())
        })?;
        let mut digest = Sha256::new();
        digest.update(COMPANY_IDENTITY_KEY_DOMAIN);
        digest.update(&bytes);
        Ok(format!("{:x}", digest.finalize()))
    }

    pub fn refresh_identity_key(&mut self) -> Result<(), DomainError> {
        self.identity_key = self.expected_identity_key()?;
        Ok(())
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if self.schema_version != 1 {
            return Err(DomainError::UnsupportedSchema(self.schema_version));
        }
        self.company_id.validate()?;
        validate_identifier(&self.record_id, "record_id")?;
        validate_identifier(&self.identity_key, "identity_key")?;
        if self.legal_name.trim().is_empty() {
            return Err(DomainError::Empty {
                field: "legal_name",
            });
        }
        validate_identifier(&self.source_id, "source_id")?;
        if self.valid_to.is_some_and(|end| end < self.valid_from) {
            return Err(DomainError::InvalidPeriod(
                "company identity valid_to precedes valid_from".to_owned(),
            ));
        }
        if self.revision == 0 || (self.revision == 1) != self.supersedes.is_none() {
            return Err(DomainError::InvalidRevision {
                fact_key: self.identity_key.clone(),
                revision: self.revision,
            });
        }
        if let Some(parent) = &self.supersedes {
            validate_identifier(parent, "supersedes")?;
            if parent == &self.record_id {
                return Err(DomainError::InvalidRevision {
                    fact_key: self.identity_key.clone(),
                    revision: self.revision,
                });
            }
        }
        if !valid_sha256(&self.source_document_sha256) {
            return Err(DomainError::InvalidEvidence(
                "source_document_sha256 must be lowercase SHA-256".to_owned(),
            ));
        }
        self.locator.validate()?;
        let expected = self.expected_identity_key()?;
        if self.identity_key != expected {
            return Err(DomainError::FactKeyMismatch {
                expected,
                actual: self.identity_key.clone(),
            });
        }
        Ok(())
    }
}

/// Validate one effective-state slice: no overlapping periods for one company
/// and no CIN claimed by two different companies over overlapping periods.
/// CIN claims are checked by sort-and-sweep, not quadratic pairwise scans.
fn validate_effective_identities(
    mut effective: Vec<&CompanyIdentityRecord>,
) -> Result<(), DomainError> {
    effective.sort_by(|left, right| {
        left.company_id
            .cmp(&right.company_id)
            .then_with(|| left.valid_from.cmp(&right.valid_from))
    });
    for pair in effective.windows(2) {
        if pair[0].company_id == pair[1].company_id
            && pair[0].valid_to.is_none_or(|end| end >= pair[1].valid_from)
        {
            return Err(DomainError::InvalidPeriod(format!(
                "overlapping company identity periods for {}",
                pair[0].company_id.as_str()
            )));
        }
    }

    let mut cin_claims: Vec<&CompanyIdentityRecord> = effective
        .into_iter()
        .filter(|record| record.cin.is_some())
        .collect();
    cin_claims.sort_by(|left, right| {
        left.cin
            .cmp(&right.cin)
            .then_with(|| left.valid_from.cmp(&right.valid_from))
    });
    for window in cin_claims.windows(2) {
        let (left, right) = (window[0], window[1]);
        if left.cin != right.cin {
            continue;
        }
        let overlaps = left.valid_to.is_none_or(|end| end >= right.valid_from)
            && right.valid_to.is_none_or(|end| end >= left.valid_from);
        if overlaps && left.company_id != right.company_id {
            return Err(DomainError::InvalidCin(format!(
                "CIN {} is claimed by overlapping company identities",
                left.cin.as_ref().map_or("", Cin::as_str)
            )));
        }
    }
    Ok(())
}

/// Validate company identity history. Effective intervals for one company may
/// not overlap; source corrections must form a contiguous append-only revision
/// chain for each `(company_id, valid_from)` identity key.
pub fn validate_company_identity_set(records: &[CompanyIdentityRecord]) -> Result<(), DomainError> {
    let mut by_id: BTreeMap<&str, &CompanyIdentityRecord> = BTreeMap::new();
    let mut by_key: BTreeMap<&str, Vec<&CompanyIdentityRecord>> = BTreeMap::new();
    for record in records {
        record.validate()?;
        if by_id.insert(&record.record_id, record).is_some() {
            return Err(DomainError::DuplicateRevisionId(record.record_id.clone()));
        }
        by_key.entry(&record.identity_key).or_default().push(record);
    }
    for (identity_key, revisions) in &mut by_key {
        revisions.sort_by_key(|record| record.revision);
        for (index, record) in revisions.iter().enumerate() {
            let expected_revision =
                u32::try_from(index + 1).map_err(|_| DomainError::InvalidRevision {
                    fact_key: (*identity_key).to_owned(),
                    revision: record.revision,
                })?;
            let expected_parent = index
                .checked_sub(1)
                .map(|parent| revisions[parent].record_id.as_str());
            if record.revision != expected_revision
                || record.supersedes.as_deref() != expected_parent
                || index
                    .checked_sub(1)
                    .is_some_and(|parent| record.recorded_at < revisions[parent].recorded_at)
            {
                return Err(DomainError::InvalidRevision {
                    fact_key: (*identity_key).to_owned(),
                    revision: record.revision,
                });
            }
        }
    }

    let effective: Vec<&CompanyIdentityRecord> = by_key
        .values()
        .filter_map(|revisions| revisions.last().copied())
        .collect();
    validate_effective_identities(effective)
}

/// Fields that define one logical fact across amendments/restatements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct FactIdentity<'a> {
    company_id: &'a CompanyId,
    concept: &'a FinancialConcept,
    period: &'a ReportingPeriod,
    statement_scope: StatementScope,
    accounting_basis: &'a AccountingBasis,
    dimensions: &'a [DimensionMember],
    unit: &'a FinancialUnit,
}

/// One reported or derived financial fact revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinancialFact {
    pub schema_version: u32,
    pub fact_id: String,
    pub fact_key: String,
    pub revision: u32,
    pub supersedes: Option<String>,
    pub company_id: CompanyId,
    pub concept: FinancialConcept,
    pub period: ReportingPeriod,
    pub statement_scope: StatementScope,
    pub accounting_basis: AccountingBasis,
    pub dimensions: Vec<DimensionMember>,
    pub unit: FinancialUnit,
    pub value: ExactDecimal,
    pub published_at: Option<Timestamp>,
    pub observed_at: Timestamp,
    pub ingested_at: Timestamp,
    pub origin: FactOrigin,
}

impl FinancialFact {
    /// Compute a domain-separated v1 identity over the logical fact, excluding
    /// value, source revision and knowledge timestamps. Dimension ordering is
    /// validated before this key is trusted; serde struct field order is fixed.
    pub fn expected_fact_key(&self) -> Result<String, DomainError> {
        let identity = FactIdentity {
            company_id: &self.company_id,
            concept: &self.concept,
            period: &self.period,
            statement_scope: self.statement_scope,
            accounting_basis: &self.accounting_basis,
            dimensions: &self.dimensions,
            unit: &self.unit,
        };
        let bytes = serde_json::to_vec(&identity).map_err(|_| {
            DomainError::InvalidEvidence("fact identity serialization failed".to_owned())
        })?;
        let mut digest = Sha256::new();
        digest.update(FINANCIAL_FACT_KEY_DOMAIN);
        digest.update(&bytes);
        Ok(format!("{:x}", digest.finalize()))
    }

    /// Set the canonical logical identity after constructing a new fact.
    /// Existing persisted keys should be validated, not silently rewritten.
    pub fn refresh_fact_key(&mut self) -> Result<(), DomainError> {
        self.fact_key = self.expected_fact_key()?;
        Ok(())
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if self.schema_version != FINANCIAL_FACT_SCHEMA_VERSION {
            return Err(DomainError::UnsupportedSchema(self.schema_version));
        }
        self.company_id.validate()?;
        validate_identifier(&self.fact_id, "fact_id")?;
        validate_identifier(&self.fact_key, "fact_key")?;
        if self.revision == 0 || (self.revision == 1) != self.supersedes.is_none() {
            return Err(DomainError::InvalidRevision {
                fact_key: self.fact_key.clone(),
                revision: self.revision,
            });
        }
        if let Some(parent) = &self.supersedes {
            validate_identifier(parent, "supersedes")?;
            if parent == &self.fact_id {
                return Err(DomainError::InvalidRevision {
                    fact_key: self.fact_key.clone(),
                    revision: self.revision,
                });
            }
        }
        self.concept.validate()?;
        self.period.validate()?;
        self.accounting_basis.validate()?;
        self.unit.validate()?;
        self.value.validate()?;
        if self.observed_at > self.ingested_at {
            return Err(DomainError::InvalidEvidence(
                "ingested_at cannot precede observed_at".to_owned(),
            ));
        }
        if self
            .published_at
            .is_some_and(|published| published > self.observed_at)
        {
            return Err(DomainError::InvalidEvidence(
                "observed_at cannot precede published_at".to_owned(),
            ));
        }
        for dimension in &self.dimensions {
            dimension.validate()?;
        }
        for pair in self.dimensions.windows(2) {
            if pair[0].axis >= pair[1].axis {
                return Err(DomainError::InvalidConcept(
                    "dimensions must be ordered by unique axis QName".to_owned(),
                ));
            }
        }
        self.origin.validate(&self.fact_id, &self.value)?;
        let expected = self.expected_fact_key()?;
        if self.fact_key != expected {
            return Err(DomainError::FactKeyMismatch {
                expected,
                actual: self.fact_key.clone(),
            });
        }
        Ok(())
    }
}

/// Validate a complete, closed fact set, including revision and derivation lineage.
/// The input order is irrelevant; the result is deterministic.
pub fn validate_fact_set(facts: &[FinancialFact]) -> Result<(), DomainError> {
    let mut by_id: BTreeMap<&str, &FinancialFact> = BTreeMap::new();
    let mut by_key: BTreeMap<&str, Vec<&FinancialFact>> = BTreeMap::new();
    for fact in facts {
        fact.validate()?;
        if by_id.insert(&fact.fact_id, fact).is_some() {
            return Err(DomainError::DuplicateRevisionId(fact.fact_id.clone()));
        }
        by_key.entry(&fact.fact_key).or_default().push(fact);
    }

    for (fact_key, revisions) in &mut by_key {
        revisions.sort_by_key(|fact| fact.revision);
        for (index, fact) in revisions.iter().enumerate() {
            let expected_revision =
                u32::try_from(index + 1).map_err(|_| DomainError::InvalidRevision {
                    fact_key: (*fact_key).to_owned(),
                    revision: fact.revision,
                })?;
            let expected_parent = index
                .checked_sub(1)
                .map(|parent_index| revisions[parent_index].fact_id.as_str());
            if fact.revision != expected_revision || fact.supersedes.as_deref() != expected_parent {
                return Err(DomainError::InvalidRevision {
                    fact_key: (*fact_key).to_owned(),
                    revision: fact.revision,
                });
            }
            if let Some(parent_index) = index.checked_sub(1) {
                let parent = revisions[parent_index];
                if fact.ingested_at < parent.ingested_at
                    || matches!(
                        (parent.published_at, fact.published_at),
                        (Some(left), Some(right)) if right < left
                    )
                {
                    return Err(DomainError::InvalidRevision {
                        fact_key: (*fact_key).to_owned(),
                        revision: fact.revision,
                    });
                }
            }
        }
    }

    let mut graph: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for fact in facts {
        if let FactOrigin::Derived { input_fact_ids, .. } = &fact.origin {
            for input_id in input_fact_ids {
                let input = by_id.get(input_id.as_str()).ok_or_else(|| {
                    DomainError::MissingDerivedInput {
                        fact_id: fact.fact_id.clone(),
                        input_id: input_id.clone(),
                    }
                })?;
                let publication_is_future = matches!(
                    (input.published_at, fact.published_at),
                    (Some(input_time), Some(output_time)) if input_time > output_time
                );
                if input.ingested_at > fact.observed_at || publication_is_future {
                    return Err(DomainError::DerivedInputNotKnown {
                        fact_id: fact.fact_id.clone(),
                        input_id: input_id.clone(),
                    });
                }
                graph.entry(&fact.fact_id).or_default().push(&input.fact_id);
            }
        }
    }

    // Kahn's algorithm avoids recursive stack growth on adversarially deep
    // derivation chains. Edges point from an input fact to its derived consumer.
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut indegree: BTreeMap<&str, usize> = facts
        .iter()
        .map(|fact| (fact.fact_id.as_str(), 0_usize))
        .collect();
    for (consumer, inputs) in graph {
        for input in inputs {
            dependents.entry(input).or_default().push(consumer);
            let count = indegree
                .get_mut(consumer)
                .ok_or_else(|| DomainError::DerivedCycle((*consumer).to_owned()))?;
            *count = count
                .checked_add(1)
                .ok_or_else(|| DomainError::DerivedCycle((*consumer).to_owned()))?;
        }
    }
    let mut ready: BTreeSet<&str> = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
        .collect();
    let mut visited_count = 0_usize;
    while let Some(fact_id) = ready.pop_first() {
        visited_count = visited_count.saturating_add(1);
        if let Some(children) = dependents.get(fact_id) {
            for child in children {
                let degree = indegree
                    .get_mut(*child)
                    .ok_or_else(|| DomainError::DerivedCycle((*child).to_owned()))?;
                *degree = degree
                    .checked_sub(1)
                    .ok_or_else(|| DomainError::DerivedCycle((*child).to_owned()))?;
                if *degree == 0 {
                    ready.insert(*child);
                }
            }
        }
    }
    if visited_count != facts.len() {
        let cycle_member = indegree
            .iter()
            .find_map(|(id, degree)| (*degree > 0).then_some(*id))
            .unwrap_or("unknown");
        return Err(DomainError::DerivedCycle(cycle_member.to_owned()));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointInTimeSummary {
    pub selected: usize,
    pub unknown_publication_time: usize,
    pub not_yet_available: usize,
}

/// Return the latest revisions that the system could have known by `as_of`.
/// Unknown publication timestamps are excluded rather than inferred from the
/// accounting period or filing date. Ingestion time is an availability bound.
pub fn select_point_in_time(
    facts: &[FinancialFact],
    as_of: Timestamp,
) -> Result<(Vec<&FinancialFact>, PointInTimeSummary), DomainError> {
    validate_fact_set(facts)?;
    let mut latest: BTreeMap<&str, &FinancialFact> = BTreeMap::new();
    let mut known_ids: BTreeSet<&str> = BTreeSet::new();
    let mut unknown_publication_time = 0_usize;
    let mut not_yet_available = 0_usize;
    for fact in facts {
        let publication_available = match (&fact.origin, fact.published_at) {
            (FactOrigin::Reported { .. }, None) => {
                unknown_publication_time = unknown_publication_time.saturating_add(1);
                continue;
            }
            (_, Some(published_at)) => published_at <= as_of,
            (FactOrigin::Derived { .. }, None) => true,
        };
        if publication_available && fact.observed_at <= as_of && fact.ingested_at <= as_of {
            known_ids.insert(fact.fact_id.as_str());
        } else {
            not_yet_available = not_yet_available.saturating_add(1);
        }
    }
    // Eligibility is transitive: a fact whose predecessor or derivation input
    // was excluded cannot itself be selected. Propagate exclusions through the
    // dependency graph so a later revision cannot inherit a stale "known" flag.
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for fact in facts {
        let mut needs = Vec::new();
        if let Some(parent) = fact.supersedes.as_deref() {
            needs.push(parent);
        }
        if let FactOrigin::Derived { input_fact_ids, .. } = &fact.origin {
            needs.extend(input_fact_ids.iter().map(String::as_str));
        }
        for dependency in needs {
            dependents
                .entry(dependency)
                .or_default()
                .push(&fact.fact_id);
        }
    }
    let mut queue: VecDeque<&str> = facts
        .iter()
        .filter(|fact| !known_ids.contains(fact.fact_id.as_str()))
        .map(|fact| fact.fact_id.as_str())
        .collect();
    while let Some(excluded) = queue.pop_front() {
        if let Some(consumers) = dependents.get(excluded) {
            for consumer in consumers {
                if known_ids.remove(consumer) {
                    not_yet_available = not_yet_available.saturating_add(1);
                    queue.push_back(consumer);
                }
            }
        }
    }
    for fact in facts {
        if !known_ids.contains(fact.fact_id.as_str()) {
            continue;
        }
        let superseded_parent_unknown = fact
            .supersedes
            .as_deref()
            .is_some_and(|parent| !known_ids.contains(parent));
        let derived_input_unknown = match &fact.origin {
            FactOrigin::Reported { .. } => false,
            FactOrigin::Derived { input_fact_ids, .. } => input_fact_ids
                .iter()
                .any(|input_id| !known_ids.contains(input_id.as_str())),
        };
        if superseded_parent_unknown || derived_input_unknown {
            not_yet_available = not_yet_available.saturating_add(1);
            continue;
        }
        match latest.get(fact.fact_key.as_str()) {
            Some(existing) if existing.revision >= fact.revision => {}
            _ => {
                latest.insert(fact.fact_key.as_str(), fact);
            }
        }
    }
    let mut selected: Vec<&FinancialFact> = latest.into_values().collect();
    selected.sort_by(|left, right| {
        left.company_id
            .cmp(&right.company_id)
            .then_with(|| left.fact_key.cmp(&right.fact_key))
    });
    let summary = PointInTimeSummary {
        selected: selected.len(),
        unknown_publication_time,
        not_yet_available,
    };
    Ok((selected, summary))
}

/// Select company identity observations by effective date and knowledge time.
/// `valid_to` is inclusive. Revisions recorded after `known_at` are invisible.
pub fn select_company_identity_as_of(
    records: &[CompanyIdentityRecord],
    valid_at: Date,
    known_at: Timestamp,
) -> Result<Vec<&CompanyIdentityRecord>, DomainError> {
    validate_company_identity_set(records)?;
    let mut latest: BTreeMap<&str, &CompanyIdentityRecord> = BTreeMap::new();
    for record in records {
        if record.recorded_at > known_at {
            continue;
        }
        match latest.get(record.identity_key.as_str()) {
            Some(existing) if existing.revision >= record.revision => {}
            _ => {
                latest.insert(record.identity_key.as_str(), record);
            }
        }
    }
    let mut selected: Vec<&CompanyIdentityRecord> = latest
        .into_values()
        .filter(|record| {
            record.valid_from <= valid_at
                && record.valid_to.is_none_or(|valid_to| valid_at <= valid_to)
        })
        .collect();
    selected.sort_by(|left, right| left.company_id.cmp(&right.company_id));
    // A later correction must not make a previously ambiguous historical
    // state appear valid: revalidate the reconstructed effective state with
    // the same rules used for the current set.
    validate_effective_identities(selected.clone())?;
    selected.sort_by(|left, right| left.company_id.cmp(&right.company_id));
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(seconds: i64) -> Timestamp {
        Timestamp::new(seconds, 0).expect("test timestamp")
    }

    fn reported_fact(fact_id: &str, value: &str, published: Option<Timestamp>) -> FinancialFact {
        let mut fact = FinancialFact {
            schema_version: 1,
            fact_id: fact_id.to_owned(),
            fact_key: "0".repeat(64),
            revision: 1,
            supersedes: None,
            company_id: CompanyId::new("co-example").expect("company"),
            concept: FinancialConcept {
                name: ExpandedName {
                    namespace_uri: "https://xbrl.example/ind-as".to_owned(),
                    local_name: "RevenueFromOperations".to_owned(),
                },
            },
            period: ReportingPeriod::Duration {
                start: "2025-04-01".parse().expect("date"),
                end: "2026-03-31".parse().expect("date"),
            },
            statement_scope: StatementScope::Consolidated,
            accounting_basis: AccountingBasis::IndAs,
            dimensions: Vec::new(),
            unit: FinancialUnit::Currency {
                currency: CurrencyCode::new("INR").expect("currency"),
                magnitude: Magnitude::Crore,
            },
            value: ExactDecimal::new(value, 2).expect("decimal"),
            published_at: published,
            observed_at: ts(10),
            ingested_at: ts(11),
            origin: FactOrigin::Reported {
                source_id: "nse.corporate_filings".to_owned(),
                filing_id: "filing-2026-annual".to_owned(),
                document_sha256: "a".repeat(64),
                locator: EvidenceLocator::PdfTable {
                    page: 120,
                    table: Some(4),
                    row_label: "Revenue from operations".to_owned(),
                    column_label: "2025-26".to_owned(),
                },
            },
        };
        fact.fact_key = fact.expected_fact_key().expect("fact key");
        fact
    }

    fn derived_fact(
        fact_id: &str,
        concept: &str,
        input_fact_ids: Vec<String>,
        published_at: Option<Timestamp>,
        observed_at: Timestamp,
        ingested_at: Timestamp,
    ) -> FinancialFact {
        let mut fact = reported_fact(fact_id, "10000", published_at);
        fact.concept.name.local_name = concept.to_owned();
        fact.observed_at = observed_at;
        fact.ingested_at = ingested_at;
        fact.origin = FactOrigin::Derived {
            method_id: "quarterly-cash-flow-sum".to_owned(),
            method_version: "1.0.0".to_owned(),
            input_fact_ids: input_fact_ids.clone(),
            derivation_sha256: expected_derivation_sha256(
                "quarterly-cash-flow-sum",
                "1.0.0",
                &input_fact_ids,
                &fact.value,
                &fact.fact_id,
            )
            .expect("derivation hash"),
        };
        fact.refresh_fact_key().expect("derived fact key");
        fact
    }

    fn company_identity() -> CompanyIdentityRecord {
        let mut record = CompanyIdentityRecord {
            schema_version: 1,
            record_id: "company-record-v1".to_owned(),
            company_id: CompanyId::new("co-example").expect("company"),
            identity_key: String::new(),
            revision: 1,
            supersedes: None,
            cin: Some(Cin::new("U72900MH2000PLC123456").expect("cin")),
            legal_name: "Example Systems Limited".to_owned(),
            valid_from: "2000-01-01".parse().expect("date"),
            valid_to: None,
            recorded_at: ts(10),
            source_id: "mca21.company_master".to_owned(),
            source_document_sha256: "c".repeat(64),
            locator: EvidenceLocator::JsonPointer {
                pointer: "/company/name".to_owned(),
            },
        };
        record.refresh_identity_key().expect("identity key");
        record
    }

    #[test]
    fn decimal_addition_aligns_scales_without_float() {
        let left = ExactDecimal::new("12345", 2).expect("left");
        let right = ExactDecimal::new("25", 1).expect("right");
        assert_eq!(
            left.checked_add(&right).expect("sum"),
            ExactDecimal::new("12595", 2).expect("result"),
            "decimal addition must align scales exactly",
        );
    }

    #[test]
    fn decimal_subtraction_aligns_scales_and_preserves_sign() {
        let left = ExactDecimal::new("1000", 2).expect("left");
        let right = ExactDecimal::new("125", 1).expect("right");
        assert_eq!(
            left.checked_sub(&right).expect("difference"),
            ExactDecimal::new("-250", 2).expect("result"),
            "decimal subtraction must align scales and preserve negative results",
        );
    }

    #[test]
    fn decimal_addition_rejects_overflow() {
        let left = ExactDecimal::new(i128::MAX.to_string(), 0).expect("left");
        let right = ExactDecimal::new("1", 0).expect("right");
        assert!(
            left.checked_add(&right).is_err(),
            "addition overflow must fail closed"
        );
    }

    #[test]
    fn decimal_rejects_noncanonical_and_excess_scale() {
        assert!(
            ExactDecimal::new("001", 0).is_err(),
            "leading zero is non-canonical"
        );
        assert!(
            ExactDecimal::new("-0", 0).is_err(),
            "negative zero is non-canonical"
        );
        assert!(
            ExactDecimal::new("1", 19).is_err(),
            "scale limit must be enforced"
        );
    }

    #[test]
    fn fact_key_does_not_depend_on_value_or_revision() {
        let first = reported_fact("fact-v1", "10000", Some(ts(10)));
        let mut revision = first.clone();
        revision.fact_id = "fact-v2".to_owned();
        revision.revision = 2;
        revision.supersedes = Some("fact-v1".to_owned());
        revision.value = ExactDecimal::new("9500", 2).expect("value");
        assert_eq!(
            first.expected_fact_key().expect("key"),
            revision.expected_fact_key().expect("key"),
            "value and revision changes must not change logical fact identity",
        );
    }

    #[test]
    fn point_in_time_excludes_unknown_and_future_facts() {
        let mut facts = vec![
            reported_fact("fact-known", "10000", Some(ts(10))),
            reported_fact("fact-unknown", "20000", None),
            reported_fact("fact-future", "30000", Some(ts(20))),
        ];
        for (index, fact) in facts.iter_mut().enumerate() {
            let available = fact.published_at.unwrap_or(ts(10));
            fact.company_id = CompanyId::new(format!("co-pit-{index}")).expect("company");
            fact.fact_key = fact.expected_fact_key().expect("fact key");
            fact.observed_at = available;
            fact.ingested_at = available + jiff::SignedDuration::from_secs(1);
        }
        let (selected, summary) = select_point_in_time(&facts, ts(15)).expect("valid PIT facts");
        assert_eq!(selected.len(), 1, "one known fact should be selected");
        assert_eq!(
            summary.unknown_publication_time, 1,
            "unknown publication times are counted"
        );
        assert_eq!(summary.not_yet_available, 1, "future facts are counted");
    }

    #[test]
    fn point_in_time_selects_the_revision_known_at_the_requested_time() {
        let first = reported_fact("fact-revenue-v1", "10000", Some(ts(10)));
        let mut second = first.clone();
        second.fact_id = "fact-revenue-v2".to_owned();
        second.revision = 2;
        second.supersedes = Some(first.fact_id.clone());
        second.value = ExactDecimal::new("9500", 2).expect("restated value");
        second.published_at = Some(ts(20));
        second.observed_at = ts(21);
        second.ingested_at = ts(22);

        let historical = [second.clone(), first.clone()];
        let before = select_point_in_time(&historical, ts(15)).expect("historical revision");
        assert_eq!(before.0.len(), 1, "one historical revision is available");
        assert_eq!(
            before.0[0].fact_id, "fact-revenue-v1",
            "the later restatement is invisible"
        );
        assert_eq!(
            before.1.not_yet_available, 1,
            "the later restatement is counted"
        );

        let current = [first, second];
        let after = select_point_in_time(&current, ts(25)).expect("restated view");
        assert_eq!(after.0.len(), 1, "one current logical fact is selected");
        assert_eq!(
            after.0[0].fact_id, "fact-revenue-v2",
            "the latest known revision is selected"
        );
    }

    #[test]
    fn derived_facts_use_computation_time_when_no_publication_time_exists() {
        let input = reported_fact("fact-pit-input", "10000", Some(ts(10)));
        let output = derived_fact(
            "fact-pit-derived",
            "DerivedCashFlow",
            vec![input.fact_id.clone()],
            None,
            ts(13),
            ts(14),
        );
        let inputs = [input, output];
        let (selected, summary) = select_point_in_time(&inputs, ts(15)).expect("derived PIT view");
        assert_eq!(
            selected.len(),
            2,
            "the report and derived fact are available"
        );
        assert!(
            selected
                .iter()
                .any(|fact| fact.fact_id == "fact-pit-derived"),
            "an internally computed fact does not require public filing publication",
        );
        assert_eq!(
            summary.unknown_publication_time, 0,
            "unknown public publication time applies to reported facts only",
        );
    }

    #[test]
    fn derived_facts_require_known_inputs_and_reject_cycles() {
        let input = reported_fact("fact-input", "10000", Some(ts(10)));
        let output = derived_fact(
            "fact-output",
            "DerivedCashFlow",
            vec![input.fact_id.clone()],
            Some(ts(12)),
            ts(13),
            ts(14),
        );
        validate_fact_set(&[output, input]).expect("valid derived lineage");

        let missing = derived_fact(
            "fact-missing-input",
            "DerivedMissing",
            vec!["fact-does-not-exist".to_owned()],
            Some(ts(12)),
            ts(13),
            ts(14),
        );
        assert!(
            matches!(
                validate_fact_set(&[missing]),
                Err(DomainError::MissingDerivedInput { .. })
            ),
            "a derived fact cannot reference a missing input",
        );

        let mut late_input = reported_fact("fact-late-input", "10000", Some(ts(12)));
        late_input.observed_at = ts(12);
        late_input.ingested_at = ts(13);
        let early_output = derived_fact(
            "fact-early-output",
            "DerivedEarly",
            vec![late_input.fact_id.clone()],
            Some(ts(10)),
            ts(12),
            ts(14),
        );
        assert!(
            matches!(
                validate_fact_set(&[early_output, late_input]),
                Err(DomainError::DerivedInputNotKnown { .. })
            ),
            "a derived fact cannot use an input ingested after the derivation",
        );

        let first = derived_fact(
            "fact-cycle-a",
            "DerivedA",
            vec!["fact-cycle-b".to_owned()],
            Some(ts(10)),
            ts(20),
            ts(20),
        );
        let second = derived_fact(
            "fact-cycle-b",
            "DerivedB",
            vec!["fact-cycle-a".to_owned()],
            Some(ts(10)),
            ts(20),
            ts(20),
        );
        let error = validate_fact_set(&[first, second]).expect_err("cycles must be rejected");
        assert!(
            matches!(error, DomainError::DerivedCycle(_)),
            "cyclic derived-fact dependencies must be rejected, got {error:?}"
        );
    }

    #[test]
    fn concept_names_and_evidence_locators_reject_invalid_syntax() {
        let invalid_name = ExpandedName {
            namespace_uri: "not-a-uri".to_owned(),
            local_name: "RevenueFromOperations".to_owned(),
        };
        assert!(
            invalid_name.validate().is_err(),
            "a relative namespace name must not pass as an expanded QName",
        );
        let invalid_local_name = ExpandedName {
            namespace_uri: "https://xbrl.example/taxonomy".to_owned(),
            local_name: "Revenue$Operations".to_owned(),
        };
        assert!(
            invalid_local_name.validate().is_err(),
            "local names must stay within the declared ASCII NCName subset",
        );
        let invalid_pointer = EvidenceLocator::JsonPointer {
            pointer: "/company/name~2value".to_owned(),
        };
        assert!(
            invalid_pointer.validate().is_err(),
            "JSON Pointer tilde escapes must be RFC 6901 escapes",
        );
    }

    #[test]
    fn cin_checks_structure_but_does_not_claim_mca_existence() {
        assert!(
            Cin::new("U72900MH2000PLC123456").is_ok(),
            "well-formed CIN shape is accepted"
        );
        assert!(
            Cin::new("not-a-cin").is_err(),
            "malformed CIN shape is rejected"
        );
    }

    #[test]
    fn company_identity_is_effective_dated_and_source_backed() {
        let record = company_identity();
        record.validate().expect("valid identity");
        validate_company_identity_set(&[record]).expect("valid history");
    }

    #[test]
    fn company_identity_revisions_preserve_historical_knowledge() {
        let first = company_identity();
        let mut second = first.clone();
        second.record_id = "company-record-v2".to_owned();
        second.revision = 2;
        second.supersedes = Some(first.record_id.clone());
        second.legal_name = "Example Systems Holdings Limited".to_owned();
        second.recorded_at = ts(20);
        validate_company_identity_set(&[second.clone(), first.clone()])
            .expect("complete identity revision chain");

        let valid_at = "2020-01-01".parse().expect("valid date");
        let historical = [second.clone(), first.clone()];
        let before = select_company_identity_as_of(&historical, valid_at, ts(15))
            .expect("historical company identity");
        assert_eq!(
            before.len(),
            1,
            "one identity is selected before correction"
        );
        assert_eq!(
            before[0].record_id, "company-record-v1",
            "the correction is not backdated"
        );

        let current = [first, second];
        let after = select_company_identity_as_of(&current, valid_at, ts(25))
            .expect("corrected company identity");
        assert_eq!(after.len(), 1, "one identity is selected after correction");
        assert_eq!(
            after[0].record_id, "company-record-v2",
            "the latest known correction is selected",
        );
    }

    #[test]
    fn company_identity_rejects_overlapping_effective_intervals() {
        let first = company_identity();
        let mut second = company_identity();
        second.record_id = "company-record-later".to_owned();
        second.valid_from = "2010-01-01".parse().expect("date");
        second.refresh_identity_key().expect("identity key");
        assert!(
            matches!(
                validate_company_identity_set(&[first, second]),
                Err(DomainError::InvalidPeriod(_))
            ),
            "one company cannot have overlapping legal identity intervals",
        );
    }

    #[test]
    fn company_identity_rejects_overlapping_duplicate_cin_claims() {
        let first = company_identity();
        let mut second = company_identity();
        second.record_id = "company-record-other".to_owned();
        second.company_id = CompanyId::new("co-other").expect("other company");
        second.refresh_identity_key().expect("identity key");
        assert!(
            matches!(
                validate_company_identity_set(&[first, second]),
                Err(DomainError::InvalidCin(_))
            ),
            "one CIN cannot identify two companies at the same effective time",
        );
    }

    #[test]
    fn reported_fact_requires_document_evidence() {
        let mut fact = reported_fact("fact-1", "100", Some(ts(10)));
        if let FactOrigin::Reported {
            document_sha256, ..
        } = &mut fact.origin
        {
            *document_sha256 = "not-a-hash".to_owned();
        }
        assert!(
            matches!(fact.validate(), Err(DomainError::InvalidEvidence(_))),
            "reported fact must reference a valid document digest",
        );
    }
    #[test]
    fn point_in_time_propagates_revision_ineligibility() {
        let mut first = reported_fact("pit-chain-1", "100", None);
        first.observed_at = ts(5);
        first.ingested_at = ts(6);
        let mut second = reported_fact("pit-chain-2", "90", Some(ts(20)));
        second.fact_id = "pit-chain-2".to_owned();
        second.revision = 2;
        second.supersedes = Some(first.fact_id.clone());
        second.fact_key = first.fact_key.clone();
        second.observed_at = ts(21);
        second.ingested_at = ts(22);
        let mut third = reported_fact("pit-chain-3", "80", Some(ts(30)));
        third.fact_id = "pit-chain-3".to_owned();
        third.revision = 3;
        third.supersedes = Some(second.fact_id.clone());
        third.fact_key = first.fact_key.clone();
        third.observed_at = ts(31);
        third.ingested_at = ts(32);
        let revision_chain = [third, second, first];
        let (selected, _) =
            select_point_in_time(&revision_chain, ts(40)).expect("valid revision chain");
        assert!(
            selected.is_empty(),
            "a revision whose predecessor has no publication time cannot become eligible"
        );
    }

    #[test]
    fn point_in_time_propagates_derived_input_ineligibility() {
        let mut input = reported_fact("pit-derived-input", "100", None);
        input.observed_at = ts(5);
        input.ingested_at = ts(6);
        let derived_a = derived_fact(
            "pit-derived-a",
            "DerivedA",
            vec![input.fact_id.clone()],
            None,
            ts(30),
            ts(31),
        );
        let derived_b = derived_fact(
            "pit-derived-b",
            "DerivedB",
            vec![derived_a.fact_id.clone()],
            None,
            ts(32),
            ts(33),
        );
        let derivation_chain = [input, derived_a, derived_b];
        let (selected, _) =
            select_point_in_time(&derivation_chain, ts(40)).expect("valid derivation chain");
        assert!(
            selected.is_empty(),
            "a derived fact cannot be eligible when its two-level input is ineligible"
        );
    }

    #[test]
    fn historical_cin_collision_is_not_hidden_by_a_later_correction() {
        let first = company_identity();
        let mut second = first.clone();
        second.record_id = "company-record-collision".to_owned();
        second.company_id = CompanyId::new("co-other").expect("company");
        second.identity_key = second.expected_identity_key().expect("identity key");
        second.recorded_at = ts(12);
        let mut correction = second.clone();
        correction.record_id = "company-record-correction".to_owned();
        correction.revision = 2;
        correction.supersedes = Some(second.record_id.clone());
        correction.cin = Some(Cin::new("U72900MH2000PLC999999").expect("cin"));
        correction.recorded_at = ts(30);
        let effective = [first.clone(), second.clone(), correction.clone()];
        validate_company_identity_set(&effective).expect("latest state is unambiguous");
        let valid_at = "2010-01-01".parse().expect("date");
        assert!(
            select_company_identity_as_of(&effective, valid_at, ts(20)).is_err(),
            "a historically ambiguous CIN must fail explicitly even after a later correction"
        );
    }
    #[test]
    fn derived_fact_rejects_a_self_declared_derivation_digest() {
        let input = reported_fact("fact-hash-input", "100", Some(ts(10)));
        let mut output = derived_fact(
            "fact-hash-output",
            "DerivedHash",
            vec![input.fact_id.clone()],
            None,
            ts(20),
            ts(20),
        );
        if let FactOrigin::Derived {
            derivation_sha256, ..
        } = &mut output.origin
        {
            *derivation_sha256 = "d".repeat(64);
        }
        assert!(
            matches!(output.validate(), Err(DomainError::InvalidEvidence(_))),
            "a digest that does not match the derivation commitment is rejected"
        );
    }
}
