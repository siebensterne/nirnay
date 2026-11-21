"""Registered grid-search tests: accounting, registration and falsifiers."""
from __future__ import annotations

from dataclasses import asdict

import pytest

from quantlab.alpha_search import AlphaCosts
from quantlab.experiments import (ExperimentError, ExperimentRegistry, build_search_space,
                                  content_hash, holdout_key)
from quantlab.experiments.parameter_search import generate_configs, grid_search
from quantlab.experiments.reproducibility import assert_report_hash

DATASET_ID = "fixture:grid-search"
DATASET_VERSION = DATASET_ID + ":" + "b" * 16


def _configs(count: int = 4) -> list[dict]:
    return [{"family": "momentum", "lookback": value, "threshold_bps": 0.0}
            for value in range(3, 3 + count)]


def _registered(count: int = 4) -> dict:
    configs = _configs(count)
    validation = {"start_index": 253, "end_index": 336}
    holdout = {"start_index": 337, "end_index": 420}
    costs = asdict(AlphaCosts())
    lineage = {"data_sha256": "a" * 64, "input_sha256": "b" * 64,
               "code_digest": "c" * 64, "config_digest": "d" * 64,
               "source_event_hashes": ["e" * 64], "source_frame_hashes": ["f" * 64],
               "listing_source_hashes": ["1" * 64], "corporate_action_source_hashes": []}
    return {
        "experiment_id": "GRID-0001",
        "hypothesis": "Declared grid screen; not an alpha assertion.",
        "economic_rationale": "Trend persistence is hypothesized but not established.",
        "dataset_ref": "fixture:nse-cash", "dataset_manifest_sha256": "9" * 64,
        "dataset_point_in_time": True, "code_revision": "test-revision", "config": {},
        "status": "registered", "registered_at_utc": "2026-11-12T00:00:00Z",
        "falsification_rule": "Reject unless every declared config has an outcome.",
        "dataset_id": DATASET_ID, "dataset_version": DATASET_VERSION,
        "dataset_lineage": lineage, "universe_id": "fixture-universe",
        "feature_version": "features.v1", "strategy_code_hash": "7" * 64,
        "search_space": build_search_space("momentum", configs,
                                           train_period={"start_index": 1, "end_index": 252}),
        "attempted_configs": [], "validation_period": validation,
        "holdout_period": holdout,
        "holdout_key": holdout_key(DATASET_VERSION, "fixture-universe", "features.v1",
                                   validation, holdout),
        "cost_model": {"assumptions": costs, "cost_model_sha256": content_hash(costs)},
        "execution_assumptions": {"execution_latency_model": "not_modelled",
                                  "queue_model": "not_modelled"},
        "bar_kind": "minute",
        "bar_interval_ns": 60_000_000_000,
        "timestamp_semantics": "bar_close",
        "calendar_id": "fixture:continuous-minute-v1",
        "session_definition": "Fixture continuous minute bars; no exchange session",
        "annualization_basis": "synthetic_declared",
        "periods_per_year": 525_600.0,
        "point_in_time_basis": "chronological_only",
    }


def _spec() -> dict:
    return {"family": ["momentum"], "lookback": [3, 4, 5, 6], "threshold_bps": [0.0]}


def _evaluate(config: dict) -> dict:
    return {"validation_net_mean": float(config["lookback"]) / 100.0, "status": "screened"}


def test_generate_configs_is_deterministic_and_refuses_empty_axes():
    first = generate_configs({"b": [1, 2], "a": ["x", "y"]})
    assert first == [{"a": "x", "b": 1}, {"a": "x", "b": 2},
                     {"a": "y", "b": 1}, {"a": "y", "b": 2}]
    with pytest.raises(ExperimentError, match="non-empty sequence"):
        generate_configs({"a": []})


def test_grid_search_persists_all_attempts_including_losers(tmp_path):
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    registry.register(_registered())
    report, receipt = grid_search(_spec(), _evaluate, registry, "GRID-0001")
    assert receipt.status == "rejected"
    assert report["candidate_family_size"] == 4
    assert_report_hash(report)
    stored = registry.record("GRID-0001")
    attempts = stored.body["attempted_configs"]
    assert len(attempts) == 4
    assert {attempt["outcome"]["validation_net_mean"] for attempt in attempts} == {
        0.03, 0.04, 0.05, 0.06
    }
    assert stored.body["validation"]["claim_boundary"] == (
        "research_screening_only_not_alpha_certification"
    )
    assert registry.verify_integrity()["record_count"] == 2


def test_unregistered_search_refuses_before_evaluation(tmp_path):
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    calls: list[dict] = []
    with pytest.raises(ExperimentError, match="not registered"):
        grid_search(_spec(), calls.append, registry, "GRID-9999")
    assert calls == []


def test_out_of_space_config_is_refused_before_evaluation(tmp_path):
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    registry.register(_registered())
    spec = _spec()
    spec["lookback"] = [3, 4, 5, 999]
    calls: list[dict] = []
    with pytest.raises(ExperimentError, match="outside the registered search space"):
        grid_search(spec, calls.append, registry, "GRID-0001")
    assert calls == []


def test_count_mismatch_is_refused_before_evaluation(tmp_path):
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    registry.register(_registered())
    spec = _spec()
    spec["lookback"] = [3, 4, 5]
    calls: list[dict] = []
    with pytest.raises(ExperimentError, match="expected_trials"):
        grid_search(spec, calls.append, registry, "GRID-0001")
    assert calls == []


def test_nonfinite_evaluator_outcome_is_refused(tmp_path):
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    registry.register(_registered())

    def bad_evaluate(config: dict) -> dict:
        return {"validation_net_mean": float("nan")}

    with pytest.raises(ExperimentError, match="non-canonical outcome"):
        grid_search(_spec(), bad_evaluate, registry, "GRID-0001")
    assert registry.record("GRID-0001").status == "registered"
