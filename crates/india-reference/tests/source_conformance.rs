#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Real exchange source conformance gate.
//!
//! This test is ignored in the default suite by design. It certifies parser
//! compatibility only when an operator supplies dated source files with
//! provenance in `NIRNAY_SOURCE_FIXTURES_DIR`. Synthetic parser tests prove CSV
//! grammar and rule behavior; they are not evidence of exchange compatibility.

use std::collections::BTreeSet;
use std::fmt::Debug;
use std::path::{
    Path, //
    PathBuf,
};

use india_reference::{
    Limits, //
    SnapshotScope,
    SourceBatch,
    source,
};
use serde::Deserialize;
use sha2::Digest;
use trading_core::Venue;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    file: String,
    source_url: String,
    retrieved_at_utc: String,
    sha256: String,
    size_bytes: u64,
    format_version: String,
    declared_coverage: String,
    expected_accepted: usize,
    expected_excluded: usize,
    expected_rejected: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureSet {
    nse: Fixture,
    bse: Fixture,
}

fn verify_fixture(root: &Path, fixture: &Fixture) -> Vec<u8> {
    let path = root.join(&fixture.file);
    assert!(path.is_file(), "fixture file must exist");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        bytes.len() as u64,
        fixture.size_bytes,
        "fixture size must match provenance"
    );
    let digest = format!("{:x}", sha2::Sha256::digest(&bytes));
    assert_eq!(
        digest, fixture.sha256,
        "fixture bytes must match provenance hash"
    );
    assert!(
        !fixture.source_url.is_empty() && !fixture.retrieved_at_utc.is_empty(),
        "fixture must record source URL and retrieval time"
    );
    bytes
}

/// Parse structured coverage from the provenance declaration.
///
/// Parsed forms: `full:NSE`, `full:BSE`, `full:NSE+BSE` and `partial`.
/// The dated gate then requires exactly the source's own venue, so combined
/// and partial declarations are rejected for a per-source fixture.
fn declared_scope(text: &str, expected: Venue) -> SnapshotScope {
    let scope = match text {
        "partial" => SnapshotScope::Partial,
        "full:NSE" => SnapshotScope::Full {
            covered_venues: BTreeSet::from([Venue::Nse]),
        },
        "full:BSE" => SnapshotScope::Full {
            covered_venues: BTreeSet::from([Venue::Bse]),
        },
        "full:NSE+BSE" => SnapshotScope::Full {
            covered_venues: BTreeSet::from([Venue::Nse, Venue::Bse]),
        },
        other => panic!("unsupported declared coverage: {other}"),
    };
    assert_scope_covers(&scope, expected);
    scope
}

fn assert_scope_covers(scope: &SnapshotScope, expected: Venue) {
    let exact = SnapshotScope::Full {
        covered_venues: BTreeSet::from([expected]),
    };
    assert_eq!(
        *scope, exact,
        "dated source must declare exactly its own venue coverage"
    );
}

#[test]
#[ignore = "requires operator-supplied dated NSE and BSE source files with provenance"]
fn dated_exchange_sources_conform() {
    let root = PathBuf::from(
        std::env::var_os("NIRNAY_SOURCE_FIXTURES_DIR")
            .expect("real source conformance requires NIRNAY_SOURCE_FIXTURES_DIR"),
    );
    let manifest: FixtureSet = serde_json::from_slice(
        &std::fs::read(root.join("provenance.json")).expect("provenance.json is required"),
    )
    .expect("fixture provenance must match the active schema");

    assert!(
        manifest.nse.format_version == "nse-cash-equity-v1",
        "unsupported NSE format version must fail closed"
    );
    assert!(
        matches!(
            manifest.bse.format_version.as_str(),
            "bse-security-master-v1" | "bse-security-master-v2"
        ),
        "unsupported BSE format version must fail closed"
    );
    let nse_scope = declared_scope(&manifest.nse.declared_coverage, Venue::Nse);
    let bse_scope = declared_scope(&manifest.bse.declared_coverage, Venue::Bse);
    assert_scope_covers(&nse_scope, Venue::Nse);
    assert_scope_covers(&bse_scope, Venue::Bse);

    let nse_bytes = verify_fixture(&root, &manifest.nse);
    let bse_bytes = verify_fixture(&root, &manifest.bse);

    let nse = source::nse::parse_nse_cash_master(
        &nse_bytes,
        &manifest.nse.format_version,
        nse_scope.clone(),
        Limits::default(),
    )
    .expect("NSE source must parse under its declared version");
    let bse = source::bse::parse_bse_master(
        &bse_bytes,
        &manifest.bse.format_version,
        bse_scope.clone(),
        Limits::default(),
    )
    .expect("BSE source must parse under its declared version");

    assert!(
        nse.is_complete(),
        "NSE: every physical row has exactly one outcome"
    );
    assert!(
        bse.is_complete(),
        "BSE: every physical row has exactly one outcome"
    );
    assert!(
        nse.physical_rows > 0 && bse.physical_rows > 0,
        "dated source files must contain data rows"
    );
    assert!(
        !nse.accepted.is_empty() && !bse.accepted.is_empty(),
        "dated sources must yield accepted rows; otherwise the format is unsupported"
    );
    check_counts(&manifest.nse, &nse);
    check_counts(&manifest.bse, &bse);

    let nse_repeat = source::nse::parse_nse_cash_master(
        &nse_bytes,
        &manifest.nse.format_version,
        nse_scope,
        Limits::default(),
    )
    .expect("repeat NSE parse");
    let bse_repeat = source::bse::parse_bse_master(
        &bse_bytes,
        &manifest.bse.format_version,
        bse_scope,
        Limits::default(),
    )
    .expect("repeat BSE parse");
    assert_outcomes_equal(&nse, &nse_repeat);
    assert_outcomes_equal(&bse, &bse_repeat);
}

fn check_counts<T>(fixture: &Fixture, batch: &SourceBatch<T>) {
    assert_eq!(
        batch.accepted.len(),
        fixture.expected_accepted,
        "accepted count must match provenance"
    );
    assert_eq!(
        batch.excluded.len(),
        fixture.expected_excluded,
        "excluded count must match provenance"
    );
    assert_eq!(
        batch.rejected.len(),
        fixture.expected_rejected,
        "rejected count must match provenance"
    );
    assert_eq!(
        batch.physical_rows,
        fixture.expected_accepted + fixture.expected_excluded + fixture.expected_rejected,
        "provenance outcomes must cover every physical row"
    );
}

fn assert_outcomes_equal<T: PartialEq + Debug>(first: &SourceBatch<T>, second: &SourceBatch<T>) {
    assert_eq!(
        first.accepted, second.accepted,
        "accepted rows are reproducible"
    );
    assert_eq!(
        first.excluded, second.excluded,
        "exclusions are reproducible"
    );
    assert_eq!(
        first.rejected, second.rejected,
        "rejections are reproducible"
    );
}

#[test]
fn unknown_coverage_declaration_fails_closed() {
    let result = std::panic::catch_unwind(|| declared_scope("full:XYZ", Venue::Nse));
    assert!(result.is_err(), "an unknown coverage declaration must fail");
}
