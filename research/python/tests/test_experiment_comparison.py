"""Byte-stable experiment comparison and falsifier tests."""
from __future__ import annotations

import json
from pathlib import Path

import pytest

from quantlab.alpha_search import AlphaCosts
from quantlab.cli import main
from quantlab.experiments import (ExperimentRegistry, build_search_space, calendar_binding,
                                  content_hash, holdout_key, make_attempt)
from quantlab.experiments.schema import ExperimentError
from quantlab.reporting import research_report_bytes
from quantlab.reporting.comparison import (ComparisonError, compare_experiments,
                                           freeze_for_comparison)


def _lineage() -> dict:
    return {"data_sha256": "a" * 64, "input_sha256": "b" * 64,
            "code_digest": "c" * 64, "config_digest": "d" * 64,
            "source_event_hashes": ["e" * 64], "source_frame_hashes": [],
            "listing_source_hashes": [], "corporate_action_source_hashes": []}


def _terminal(experiment_id: str, *, start: int = 337, point_in_time: bool = True) -> dict:
    validation = {"start_index": 253, "end_index": 336}
    holdout = {"start_index": start, "end_index": start + 83}
    costs = AlphaCosts()
    cost_dict = {"brokerage_bps": costs.brokerage_bps,
                 "exchange_and_regulatory_bps": costs.exchange_and_regulatory_bps,
                 "taxes_bps": costs.taxes_bps, "spread_bps": costs.spread_bps,
                 "slippage_bps": costs.slippage_bps, "market_impact_bps": costs.market_impact_bps}
    configs = [{"family": "momentum", "lookback": 3, "threshold_bps": 0.0}]
    report = {"status": "rejected_or_unproven", "warning": "Research screening only.",
              "candidate_family_size": 1, "selected": configs[0],
              "untouched_holdout_net_mean": 0.0}
    report["report_sha256"] = content_hash(report)
    return {
        "experiment_id": experiment_id,
        "hypothesis": "Pre-registered research hypothesis, not an alpha claim.",
        "economic_rationale": "Hypothesis under review.",
        "dataset_ref": "fixture:nse-cash:2026-11-11",
        "dataset_manifest_sha256": "9" * 64,
        "dataset_point_in_time": point_in_time,
        "code_revision": "revision-0001",
        "config": {"lookback": 3},
        "status": "rejected",
        "registered_at_utc": "2026-11-12T00:00:00Z",
        "falsification_rule": "Reject unless net holdout expectancy is positive after costs.",
        "dataset_id": "fixture:nse-cash",
        "dataset_origin": "SYNTHETIC",
        "dataset_version": "fixture:nse-cash:" + "a" * 16,
        "dataset_lineage": _lineage(),
        "universe_id": "fixture-universe",
        "feature_version": "features.v1",
        "strategy_code_hash": "7" * 64,
        "search_space": build_search_space("momentum", configs,
                                           train_period={"start_index": 1, "end_index": 252}),
        "attempted_configs": [make_attempt(configs[0], {"validation_net_mean": 0.0})],
        "validation_period": validation,
        "holdout_period": holdout,
        "holdout_key": holdout_key("fixture:nse-cash:" + "a" * 16, "fixture-universe",
                                   "features.v1", validation, holdout),
        "cost_model": {"assumptions": cost_dict,
                       "cost_model_sha256": content_hash(cost_dict)},
        "execution_assumptions": {"execution_latency_model": "not_modelled",
                                  "queue_model": "not_modelled"},
        "bar_kind": "minute",
        "bar_interval_ns": 60_000_000_000,
        "bar_timezone": "UTC",
        "bar_anchor": "bar_close",
        "timestamp_semantics": "bar_close",
        **calendar_binding("synthetic:fixed-1m-v1"),
        "session_definition": "Synthetic continuous one-minute bars; no exchange session",
        "annualization_basis": "synthetic_declared",
        "periods_per_year": 525_600.0,
        "point_in_time_basis": "chronological_only",
        "result": {"status": report["status"], "report_sha256": report["report_sha256"]},
        "validation": {"dataset_point_in_time": point_in_time,
                       "holdout_used_for_selection": False},
        "results": report,
        "diagnostics": {"cscv_pbo": {"uses_final_holdout": False,
                                     "pbo_estimate": 0.5},
                        "deflated_sharpe_validation_only": {"trials": 1}},
    }


def _registered(experiment_id: str, *, start: int = 337) -> dict:
    record = _terminal(experiment_id, start=start)
    for field in ("result", "validation", "results", "diagnostics"):
        record.pop(field)
    record["status"] = "registered"
    record["attempted_configs"] = []
    return record


def _frozen(experiment_id: str, **kwargs: object) -> dict:
    return freeze_for_comparison(_terminal(experiment_id, **kwargs))


def _refreeze(record: dict) -> dict:
    body = {key: value for key, value in record.items() if key != "record_sha256"}
    return {**body, "record_sha256": content_hash(body)}


@pytest.mark.market_shaped_fixture
def test_byte_stable_comparison_output():
    left = _frozen("EXP-0001")
    right = _frozen("EXP-0002", start=500)
    first = research_report_bytes(compare_experiments(left, right))
    second = research_report_bytes(compare_experiments(left, right))
    assert first == second
    assert json.loads(first)["status"] == "comparable_research_records"
    assert json.loads(first)["integrity"]["signature_verified"] is False


@pytest.mark.market_shaped_fixture
def test_differing_dataset_hash_rejected():
    left = _frozen("EXP-0001")
    right = _frozen("EXP-0002")
    right = _refreeze({**right, "dataset_manifest_sha256": "8" * 64})
    with pytest.raises(ComparisonError, match="dataset manifest"):
        compare_experiments(left, right)


@pytest.mark.market_shaped_fixture
def test_tampered_record_rejected():
    left = _frozen("EXP-0001")
    right = _frozen("EXP-0002")
    right["strategy_code_hash"] = "1" * 64
    with pytest.raises(ComparisonError, match="tampered"):
        compare_experiments(left, right)


@pytest.mark.market_shaped_fixture
def test_non_terminal_record_rejected():
    left = _frozen("EXP-0001")
    right = _frozen("EXP-0002")
    right = _refreeze({**right, "status": "registered"})
    with pytest.raises(ExperimentError, match="terminal"):
        compare_experiments(left, right)


@pytest.mark.market_shaped_fixture
def test_leaky_record_rejected():
    left = _frozen("EXP-0001")
    right = _frozen("EXP-0002", point_in_time=False)
    with pytest.raises(ExperimentError, match="leakage"):
        compare_experiments(left, right)


@pytest.mark.market_shaped_fixture
def test_holdout_reuse_rejected():
    left = _frozen("EXP-0001")
    right = _frozen("EXP-0002")
    with pytest.raises(ComparisonError, match="same holdout"):
        compare_experiments(left, right)


@pytest.mark.market_shaped_fixture
def test_cli_compare_experiments_is_byte_stable_and_refuses_mismatch(tmp_path):
    left = _frozen("EXP-0001")
    right = _frozen("EXP-0002", start=400)
    left_path = tmp_path / "left.json"
    right_path = tmp_path / "right.json"
    left_path.write_text(json.dumps(left, sort_keys=True), encoding="utf-8")
    right_path.write_text(json.dumps(right, sort_keys=True), encoding="utf-8")
    assert main(["compare-experiments", "--left", str(left_path),
                 "--right", str(right_path)]) == 0
    mismatched = tmp_path / "mismatch.json"
    bad = dict(right)
    bad["dataset_manifest_sha256"] = "1" * 64
    bad["record_sha256"] = content_hash({key: value for key, value in bad.items()
                                         if key != "record_sha256"})
    mismatched.write_text(json.dumps(bad, sort_keys=True), encoding="utf-8")
    assert main(["compare-experiments", "--left", str(left_path),
                 "--right", str(mismatched)]) == 2


@pytest.mark.market_shaped_fixture
def test_registry_backed_comparison(tmp_path):
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    for experiment_id, start in (("EXP-0001", 337), ("EXP-0002", 500)):
        registered = _registered(experiment_id, start=start)
        registry.register(registered)
        registry.append_result(_terminal(experiment_id, start=start))
    first = research_report_bytes(compare_experiments("EXP-0001", "EXP-0002",
                                                      registry=registry))
    second = research_report_bytes(compare_experiments("EXP-0001", "EXP-0002",
                                                       registry=registry))
    assert first == second
    payload = json.loads(first)
    assert payload["integrity"]["status"] == "verified_internal_integrity"
    assert payload["left"]["experiment_id"] == "EXP-0001"
