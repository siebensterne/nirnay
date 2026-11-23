#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fundamental_tools::{
    ValidationLimits, //
    validate_jsonl,
}; //
use std::io::Cursor;
use std::process::Command;

#[test]
fn empty_dataset_is_valid_and_has_the_empty_sha256() {
    let report = validate_jsonl(Cursor::new(Vec::<u8>::new()), ValidationLimits::default())
        .expect("empty dataset");
    assert_eq!(report.records, 0, "empty input has no records");
    assert_eq!(
        report.input_sha256, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "empty input must produce the standard SHA-256 digest",
    );
}

#[test]
fn compiled_cli_fails_closed_on_a_torn_jsonl_record() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = directory.path().join("torn.jsonl");
    std::fs::write(&input, b"{}").expect("write torn record");
    let output = Command::new(env!("CARGO_BIN_EXE_nirnay-fundamental"))
        .args(["validate-facts", "--input"])
        .arg(&input)
        .output()
        .expect("run compiled fundamental CLI");
    assert!(!output.status.success(), "a torn record must fail the CLI");
    assert!(
        output.stdout.is_empty(),
        "a failed validation must emit no success report"
    );
}

#[test]
fn shared_contract_vectors_match_the_rust_domain_validator() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (kind, type_name) in [
        ("financial-facts", "financial fact"),
        ("company-identities", "company identity"),
    ] {
        let directory = root
            .join("research/fundamentals/contract-vectors")
            .join(kind);
        let mut checked = 0_usize;
        for entry in std::fs::read_dir(&directory).expect("vector directory") {
            let path = entry.expect("vector entry").path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("vector bytes");
            let accepted = if kind == "financial-facts" {
                serde_json::from_str::<fundamental_domain::FinancialFact>(&text)
                    .ok()
                    .is_some_and(|fact| fact.validate().is_ok())
            } else {
                serde_json::from_str::<fundamental_domain::CompanyIdentityRecord>(&text)
                    .ok()
                    .is_some_and(|record| record.validate().is_ok())
            };
            let expected = path.file_stem().and_then(|value| value.to_str()) == Some("valid");
            assert_eq!(
                accepted, expected,
                "shared contract vectors must agree between schema and domain"
            );
            checked += 1;
        }
        assert!(
            checked > 0,
            "at least one {type_name} vector must be checked"
        );
    }
}
