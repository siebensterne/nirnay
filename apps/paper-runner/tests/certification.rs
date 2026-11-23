#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Retained paper-runner audit checks.
//!
//! D11 retired the end-to-end certification scenarios that constructed
//! invented quote sequences and simulated fills. The two manifest-boundary
//! checks below construct no market observations.

use std::path::PathBuf;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn no_live_or_broker_sdk_dependency() -> TestResult {
    let manifest = include_str!("../Cargo.toml").to_ascii_lowercase();
    for forbidden in ["reqwest", "tokio", "upstox-feed-proto", "fyers", "kite"] {
        assert!(
            !manifest.contains(forbidden),
            "manifest must not depend on {forbidden}"
        );
    }
    Ok(())
}

#[test]
fn public_api_has_no_submission_path() -> TestResult {
    let source =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))?;
    for forbidden in [
        "submit_order",
        "place_order",
        "live_trade",
        "broker_api_key",
    ] {
        assert!(
            !source.contains(forbidden),
            "the harness exposes no {forbidden} path"
        );
    }
    Ok(())
}
