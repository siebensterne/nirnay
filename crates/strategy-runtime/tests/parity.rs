#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.3 tests: runtime gating and event-surface checks that construct no
//! market observations. D11 retired the synthetic golden artifact and its
//! generator; the artifact contract checks below use in-process contract text
//! only.

use strategy_runtime::{
    ArtifactStatus, //
    ArtifactValidation,
    FrozenArtifact,
    Rational,
    RationalEmaCross,
    SUPPORTED_RULE,
    Strategy,
    StrategyEvent,
};

/// A gated artifact built from pure contract text, with no market data.
fn gated_artifact() -> FrozenArtifact {
    FrozenArtifact {
        candidate_id: "contract-only".to_owned(),
        hypothesis: "schema boundary probe".to_owned(),
        return_source: "none".to_owned(),
        universe: Vec::new(),
        entry_rule: SUPPORTED_RULE.to_owned(),
        exit_rule: "none".to_owned(),
        rebalance_frequency: None,
        experiment_ids: vec!["exp:contract-only".to_owned()],
        validation: ArtifactValidation {
            walk_forward_windows: 1,
            holdout_locked: true,
            multiple_testing_correction: "none".to_owned(),
            deflated_sharpe: "0".to_owned(),
            cost_model: "none".to_owned(),
            probability_of_backtest_overfit: None,
        },
        status: ArtifactStatus::Validated,
        artifact_sha256: String::new(),
    }
}

fn write_json(value: &serde_json::Value, tag: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("nirnay-{tag}-{}.json", std::process::id()));
    std::fs::write(&path, serde_json::to_vec(value).expect("serialize")).expect("write");
    path
}

#[test]
fn candidate_status_is_refused() {
    let mut value = serde_json::to_value(gated_artifact()).expect("serialize artifact");
    value["status"] = serde_json::json!("candidate");
    let path = write_json(&value, "candidate");
    assert!(
        FrozenArtifact::load(&path, SUPPORTED_RULE).is_err(),
        "a research candidate cannot be executed by the runtime"
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn artifact_without_experiment_lineage_is_refused() {
    let mut value = serde_json::to_value(gated_artifact()).expect("serialize artifact");
    value["experiment_ids"] = serde_json::json!([]);
    let path = write_json(&value, "no-lineage");
    assert!(
        FrozenArtifact::load(&path, SUPPORTED_RULE).is_err(),
        "an artifact without experiment lineage is refused"
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn non_monotonic_sequence_is_refused() {
    let artifact = gated_artifact();
    let mut strategy =
        RationalEmaCross::from_artifact(&artifact, 2, 3, 1, 3, 100).expect("strategy");
    let input = StrategyEvent {
        observation_sequence: 5,
        instrument_id: "contract-only".to_owned(),
        midpoint: Rational::integer(1),
        best_bid: 0,
        best_ask: 0,
        position_quantity: 0,
        has_open_order: false,
    };
    strategy.on_event(&input).expect("first");
    assert!(
        strategy.on_event(&input).is_err(),
        "a repeated or regressed observation sequence is refused"
    );
}

#[test]
fn strategy_event_surface_is_narrow() {
    // Structural check: serializing the input exposes exactly the observed
    // fields. No account, cash, order, broker or risk state can be supplied.
    let input = StrategyEvent {
        observation_sequence: 1,
        instrument_id: "contract-only".to_owned(),
        midpoint: Rational::integer(1),
        best_bid: 0,
        best_ask: 0,
        position_quantity: 0,
        has_open_order: false,
    };
    let json = serde_json::to_value(&input).expect("serialize event");
    let object = json.as_object().expect("event is an object");
    let mut names: Vec<&str> = object.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "best_ask",
            "best_bid",
            "has_open_order",
            "instrument_id",
            "midpoint",
            "observation_sequence",
            "position_quantity"
        ],
        "strategy input exposes only the declared event fields"
    );
    for forbidden in ["account", "cash", "broker", "order", "permit", "risk"] {
        assert!(
            !object.contains_key(forbidden),
            "strategy input must not carry {forbidden} state"
        );
    }
}

/// A struct-literal artifact cannot bypass runtime gating through the
/// constructor. This reproduces the reviewer's forged-artifact probe.
#[test]
fn struct_literal_artifact_cannot_bypass_gating() {
    let mut forged = gated_artifact();
    forged.entry_rule = "unsupported_rule".to_owned();
    forged.status = ArtifactStatus::ApprovedForPaper;
    let result = RationalEmaCross::from_artifact(&forged, 2, 3, 1, 3, 100);
    assert!(
        matches!(
            result,
            Err(strategy_runtime::StrategyError::ArtifactRejected(_))
        ),
        "the constructor re-gates the artifact before any strategy exists"
    );
}
