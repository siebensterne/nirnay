#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! Bounded JSONL validation for immutable fundamental datasets.
//!
//! The validator does not fetch, scrape, normalize or publish source documents.
//! It checks company identity and financial fact contracts, revisions, lineage,
//! strict line framing, record limits and the exact input-byte digest. The
//! validator retains typed records in memory for cross-record graph validation;
//! production-scale ingestion requires a durable indexed store and is out of scope.

use std::io::{
    self, //
    BufRead,
    Read,
}; //

use fundamental_domain::{
    CompanyIdentityRecord, //
    FINANCIAL_FACT_SCHEMA_VERSION,
    FactOrigin,
    FinancialFact,
    validate_company_identity_set,
    validate_fact_set,
}; //
use serde::{
    Serialize, //
    de::DeserializeOwned,
}; //
use sha2::{
    Digest, //
    Sha256,
}; //

pub const DEFAULT_MAX_RECORD_BYTES: usize = 1_048_576;
pub const DEFAULT_MAX_RECORDS: u64 = 100_000;
pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 67_108_864;

#[derive(Debug, Clone, Copy)]
pub struct ValidationLimits {
    pub max_record_bytes: usize,
    pub max_records: u64,
    pub max_total_bytes: u64,
}

impl Default for ValidationLimits {
    fn default() -> Self {
        Self {
            max_record_bytes: DEFAULT_MAX_RECORD_BYTES,
            max_records: DEFAULT_MAX_RECORDS,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidationReport {
    pub report_schema_version: u32,
    pub dataset: String,
    pub schema_version: u32,
    pub records: u64,
    pub reported_facts: Option<u64>,
    pub derived_facts: Option<u64>,
    pub input_sha256: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("record {record}: exceeds configured maximum of {limit} bytes")]
    RecordTooLarge { record: u64, limit: usize },
    #[error("record {0}: blank lines are not permitted")]
    BlankLine(u64),
    #[error("record {0}: CRLF is not canonical; JSONL requires LF")]
    CarriageReturn(u64),
    #[error("record {0}: final record is not LF-terminated")]
    TornFinalRecord(u64),
    #[error("record {record}: JSON decode failed: {source}")]
    Json {
        record: u64,
        #[source]
        source: serde_json::Error,
    },
    #[error("record {record}: domain validation failed: {source}")]
    Record {
        record: u64,
        #[source]
        source: fundamental_domain::DomainError,
    },
    #[error("record count exceeds configured maximum of {0}")]
    TooManyRecords(u64),
    #[error("validation limits must be greater than zero")]
    InvalidLimits,
    #[error("input byte count exceeds configured maximum of {0}")]
    InputTooLarge(u64),
    #[error("dataset validation failed: {0}")]
    Dataset(#[from] fundamental_domain::DomainError),
    #[error("JSONL input must not start with a UTF-8 BOM")]
    Bom,
}

/// Validate a complete financial-fact JSONL stream.
pub fn validate_jsonl<R: BufRead>(
    reader: R,
    limits: ValidationLimits,
) -> Result<ValidationReport, ValidationError> {
    let (facts, records, input_sha256) = parse_records::<R, FinancialFact>(reader, limits)?;
    let mut reported_facts = 0_u64;
    let mut derived_facts = 0_u64;
    for (index, fact) in facts.iter().enumerate() {
        fact.validate().map_err(|source| ValidationError::Record {
            record: u64::try_from(index + 1).unwrap_or(u64::MAX),
            source,
        })?;
        match &fact.origin {
            FactOrigin::Reported { .. } => reported_facts = reported_facts.saturating_add(1),
            FactOrigin::Derived { .. } => derived_facts = derived_facts.saturating_add(1),
        }
    }
    validate_fact_set(&facts)?;
    Ok(ValidationReport {
        report_schema_version: 1,
        dataset: "financial_facts".to_owned(),
        schema_version: FINANCIAL_FACT_SCHEMA_VERSION,
        records,
        reported_facts: Some(reported_facts),
        derived_facts: Some(derived_facts),
        input_sha256,
    })
}

/// Validate a complete bitemporal company-identity JSONL stream.
pub fn validate_company_jsonl<R: BufRead>(
    reader: R,
    limits: ValidationLimits,
) -> Result<ValidationReport, ValidationError> {
    let (records_data, records, input_sha256) =
        parse_records::<R, CompanyIdentityRecord>(reader, limits)?;
    for (index, record) in records_data.iter().enumerate() {
        record
            .validate()
            .map_err(|source| ValidationError::Record {
                record: u64::try_from(index + 1).unwrap_or(u64::MAX),
                source,
            })?;
    }
    validate_company_identity_set(&records_data)?;
    Ok(ValidationReport {
        report_schema_version: 1,
        dataset: "company_identities".to_owned(),
        schema_version: 1,
        records,
        reported_facts: None,
        derived_facts: None,
        input_sha256,
    })
}

fn parse_records<R: BufRead, T: DeserializeOwned>(
    reader: R,
    limits: ValidationLimits,
) -> Result<(Vec<T>, u64, String), ValidationError> {
    if limits.max_record_bytes == 0 || limits.max_records == 0 || limits.max_total_bytes == 0 {
        return Err(ValidationError::InvalidLimits);
    }
    // Enforce the aggregate budget before parsing any record so a large input
    // cannot be accepted because an early record is invalid JSON.
    let mut bounded = reader.take(limits.max_total_bytes.saturating_add(1));
    let mut input = Vec::new();
    bounded.read_to_end(&mut input)?;
    if u64::try_from(input.len()).unwrap_or(u64::MAX) > limits.max_total_bytes {
        return Err(ValidationError::InputTooLarge(limits.max_total_bytes));
    }
    let mut reader = std::io::Cursor::new(input);
    let mut records = Vec::new();
    let mut digest = Sha256::new();
    let mut record_count = 0_u64;
    let mut total_bytes = 0_u64;
    loop {
        let next_record = record_count
            .checked_add(1)
            .ok_or(ValidationError::TooManyRecords(limits.max_records))?;
        let Some((line, terminated)) =
            read_bounded_line(&mut reader, limits.max_record_bytes, next_record)?
        else {
            break;
        };
        record_count = next_record;
        let line_bytes = u64::try_from(line.len())
            .map_err(|_| ValidationError::InputTooLarge(limits.max_total_bytes))?;
        total_bytes = total_bytes
            .checked_add(line_bytes)
            .ok_or(ValidationError::InputTooLarge(limits.max_total_bytes))?;
        if total_bytes > limits.max_total_bytes {
            return Err(ValidationError::InputTooLarge(limits.max_total_bytes));
        }
        if record_count > limits.max_records {
            return Err(ValidationError::TooManyRecords(limits.max_records));
        }
        digest.update(&line);
        if !terminated {
            return Err(ValidationError::TornFinalRecord(record_count));
        }
        let payload = &line[..line.len() - 1];
        if record_count == 1 && payload.starts_with(&[0xef, 0xbb, 0xbf]) {
            return Err(ValidationError::Bom);
        }
        if payload.is_empty() {
            return Err(ValidationError::BlankLine(record_count));
        }
        if payload.contains(&b'\r') {
            return Err(ValidationError::CarriageReturn(record_count));
        }
        let decoded = serde_json::from_slice(payload).map_err(|source| ValidationError::Json {
            record: record_count,
            source,
        })?;
        records.push(decoded);
    }
    let input_sha256 = format!("{:x}", digest.finalize());
    Ok((records, record_count, input_sha256))
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    limit: usize,
    record: u64,
) -> Result<Option<(Vec<u8>, bool)>, ValidationError> {
    let mut line = Vec::with_capacity(limit.min(8192));
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Ok(Some((line, false)))
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        if line.len().saturating_add(consumed) > limit {
            return Err(ValidationError::RecordTooLarge { record, limit });
        }
        line.extend_from_slice(&available[..consumed]);
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some((line, true)));
        }
    }
}

#[cfg(all(test, feature = "market-shaped-tests"))]
mod tests {
    use std::io::Cursor;

    use super::*;
    use fundamental_domain::{
        AccountingBasis, //
        Cin,
        CompanyId,
        CompanyIdentityRecord,
        CurrencyCode,
        EvidenceLocator,
        ExactDecimal,
        ExpandedName,
        FactOrigin,
        FinancialConcept,
        FinancialFact,
        FinancialUnit,
        Magnitude,
        ReportingPeriod,
        StatementScope,
    }; //
    use jiff::Timestamp;
    use jiff::civil::Date;

    fn timestamp(second: i64) -> Timestamp {
        Timestamp::new(second, 0).expect("timestamp")
    }

    fn fact(id: &str, value: &str) -> FinancialFact {
        let mut fact = FinancialFact {
            schema_version: 1,
            fact_id: id.to_owned(),
            fact_key: String::new(),
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
                start: Date::new(2025, 4, 1).expect("start"),
                end: Date::new(2026, 3, 31).expect("end"),
            },
            statement_scope: StatementScope::Consolidated,
            accounting_basis: AccountingBasis::IndAs,
            dimensions: Default::default(),
            unit: FinancialUnit::Currency {
                currency: CurrencyCode::new("INR").expect("currency"),
                magnitude: Magnitude::Crore,
            },
            value: ExactDecimal::new(value, 2).expect("decimal"),
            published_at: Some(timestamp(10)),
            observed_at: timestamp(11),
            ingested_at: timestamp(12),
            origin: FactOrigin::Reported {
                source_id: "nse.corporate_filings".to_owned(),
                filing_id: "filing-annual-2026".to_owned(),
                document_sha256: "a".repeat(64),
                locator: EvidenceLocator::PdfTable {
                    page: 100,
                    table: Some(2),
                    row_label: "Revenue from operations".to_owned(),
                    column_label: "FY 2025-26".to_owned(),
                },
            },
        };
        fact.refresh_fact_key().expect("key");
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
            valid_from: Date::new(2000, 1, 1).expect("date"),
            valid_to: None,
            recorded_at: timestamp(10),
            source_id: "mca21.company_master".to_owned(),
            source_document_sha256: "c".repeat(64),
            locator: EvidenceLocator::JsonPointer {
                pointer: "/company/name".to_owned(),
            },
        };
        record.refresh_identity_key().expect("identity key");
        record
    }

    fn jsonl(facts: &[FinancialFact]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for fact in facts {
            bytes.extend_from_slice(&serde_json::to_vec(fact).expect("encode"));
            bytes.push(b'\n');
        }
        bytes
    }

    #[test]
    fn accepts_one_valid_record_and_hashes_exact_input() {
        let bytes = jsonl(&[fact("fact-revenue-v1", "1234500")]);
        let report =
            validate_jsonl(Cursor::new(&bytes), ValidationLimits::default()).expect("valid");
        assert_eq!(report.records, 1, "one fact record should be counted");
        assert_eq!(
            report.reported_facts,
            Some(1),
            "reported facts should be counted"
        );
        assert_eq!(
            report.derived_facts,
            Some(0),
            "derived facts should be counted"
        );
        assert_eq!(
            report.input_sha256,
            format!("{:x}", Sha256::digest(&bytes)),
            "the digest must cover the exact input bytes",
        );
    }

    #[test]
    fn rejects_a_torn_final_record() {
        let mut bytes = serde_json::to_vec(&fact("fact-revenue-v1", "100")).expect("encode");
        assert!(
            matches!(
                validate_jsonl(Cursor::new(&bytes), ValidationLimits::default()),
                Err(ValidationError::TornFinalRecord(1))
            ),
            "a final record without LF must be rejected",
        );
        bytes.push(b'\n');
        assert!(
            validate_jsonl(Cursor::new(bytes), ValidationLimits::default()).is_ok(),
            "an LF-terminated record should be accepted",
        );
    }

    #[test]
    fn rejects_blank_lines_crlf_and_bom() {
        assert!(
            matches!(
                validate_jsonl(Cursor::new(b"\n"), ValidationLimits::default()),
                Err(ValidationError::BlankLine(1))
            ),
            "blank JSONL records must be rejected",
        );
        assert!(
            matches!(
                validate_jsonl(Cursor::new(b"{}\r\n"), ValidationLimits::default()),
                Err(ValidationError::CarriageReturn(1))
            ),
            "CRLF is not the canonical record delimiter",
        );
        assert!(
            matches!(
                validate_jsonl(
                    Cursor::new(b"\xef\xbb\xbf{}\n"),
                    ValidationLimits::default()
                ),
                Err(ValidationError::Bom)
            ),
            "a UTF-8 BOM must be rejected",
        );
    }

    #[test]
    fn rejects_records_over_the_byte_limit() {
        let result = validate_jsonl(
            Cursor::new(vec![b'a'; 8]),
            ValidationLimits {
                max_record_bytes: 7,
                max_records: 10,
                max_total_bytes: 1024,
            },
        );
        assert!(
            matches!(
                result,
                Err(ValidationError::RecordTooLarge {
                    record: 1,
                    limit: 7
                })
            ),
            "a record over the configured byte limit must be rejected",
        );
    }

    #[test]
    fn rejects_input_over_total_byte_limit() {
        let bytes = b"{}\n{}\n";
        let result = validate_jsonl(
            Cursor::new(bytes),
            ValidationLimits {
                max_record_bytes: 16,
                max_records: 10,
                max_total_bytes: 3,
            },
        );
        assert!(
            matches!(result, Err(ValidationError::InputTooLarge(3))),
            "the aggregate input byte limit must be enforced",
        );
    }

    #[test]
    fn validates_company_identity_records() {
        let encoded = serde_json::to_vec(&company_identity()).expect("encode");
        let mut bytes = encoded;
        bytes.push(b'\n');
        let report = validate_company_jsonl(Cursor::new(bytes), ValidationLimits::default())
            .expect("valid company identity");
        assert_eq!(
            report.dataset, "company_identities",
            "company dataset must be labelled"
        );
        assert_eq!(
            report.records, 1,
            "one company identity record should be counted"
        );
    }

    #[test]
    fn rejects_duplicate_ids_and_broken_revision_chains() {
        let first = fact("fact-revenue-v1", "100");
        assert!(
            matches!(
                validate_jsonl(
                    Cursor::new(jsonl(&[first.clone(), first.clone()])),
                    ValidationLimits::default()
                ),
                Err(ValidationError::Dataset(
                    fundamental_domain::DomainError::DuplicateRevisionId(_)
                ))
            ),
            "duplicate fact revision IDs must be rejected",
        );
        let mut third = first.clone();
        third.fact_id = "fact-revenue-v3".to_owned();
        third.revision = 3;
        third.supersedes = Some("fact-revenue-v2".to_owned());
        third.value = ExactDecimal::new("90", 2).expect("decimal");
        assert!(
            matches!(
                validate_jsonl(
                    Cursor::new(jsonl(&[first, third])),
                    ValidationLimits::default()
                ),
                Err(ValidationError::Dataset(
                    fundamental_domain::DomainError::InvalidRevision { .. }
                ))
            ),
            "revision gaps and missing predecessors must be rejected",
        );
    }
}
