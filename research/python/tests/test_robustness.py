"""Robustness diagnostics reject instability and never consume the holdout."""
from __future__ import annotations

import pytest

from quantlab.experiments import ExperimentError, ExperimentRegistry, make_attempt
from quantlab.experiments.reproducibility import assert_report_hash
from quantlab.validation import (parameter_sensitivity, robustness_digest,
                                 segment_stability)
from test_parameter_search import _configs, _registered


def test_segment_stability_flags_sign_reversal_with_different_magnitudes():
    diagnostic = segment_stability([0.01, 0.02, -0.30, -0.45])
    assert diagnostic["status"] == "unstable"
    assert diagnostic["sign_consistent"] is False
    assert diagnostic["magnitude_ratio"] > 1
    assert diagnostic["uses_final_holdout"] is False


def test_segment_stability_accepts_consistent_signs():
    diagnostic = segment_stability([0.01, 0.02, 0.03, 0.04])
    assert diagnostic["status"] == "stable"
    assert diagnostic["sign_consistent"] is True
    diagnostic = segment_stability([-0.01, -0.02, -0.03])
    assert diagnostic["status"] == "stable"


def test_segment_stability_refuses_nonfinite():
    with pytest.raises(ValueError, match="finite"):
        segment_stability([0.01, float("nan")])
    with pytest.raises(ValueError, match="real numbers"):
        segment_stability([True, False])


def _attempts(scores: list[float]) -> list[dict]:
    attempts = []
    for index, score in enumerate(scores):
        config = {"family": "momentum", "lookback": index + 3, "threshold_bps": 0.0}
        attempt = make_attempt(config, {"validation_net_mean": score})
        attempt["config_sha256"] = f"{index % 4:064d}"
        attempts.append(attempt)
    return attempts


def test_parameter_sensitivity_detects_rank_reversal():
    stable = parameter_sensitivity(_attempts([0.4, 0.3, 0.2, 0.1,
                                              0.4, 0.3, 0.2, 0.1]))
    assert stable["status"] == "stable"
    assert stable["rank_correlation"] == pytest.approx(1.0)
    unstable = parameter_sensitivity(_attempts([0.4, 0.3, 0.2, 0.1,
                                                -0.4, -0.3, -0.2, -0.1]))
    assert unstable["status"] == "unstable"
    assert unstable["rank_correlation"] < 0
    assert unstable["uses_final_holdout"] is False


def test_parameter_sensitivity_refuses_short_or_nonfinite_attempts():
    with pytest.raises(ValueError, match="at least four"):
        parameter_sensitivity(_attempts([0.1, 0.2]))
    attempts = _attempts([0.1, 0.2, 0.3, 0.4])
    attempts[0]["outcome"]["validation_net_mean"] = float("inf")
    with pytest.raises(ValueError, match="finite"):
        parameter_sensitivity(attempts)


def test_robustness_digest_refuses_holdout_flags_and_is_stable():
    diagnostics = {"segment_stability": segment_stability([0.1, 0.2]),
                   "uses_final_holdout": False}
    first = robustness_digest(diagnostics)
    assert first == robustness_digest(diagnostics)
    assert len(first) == 64
    with pytest.raises(ValueError, match="final holdout"):
        robustness_digest({"uses_final_holdout": True, "nested": {"x": 1}})


@pytest.mark.market_shaped_fixture
def test_diagnostic_reading_holdout_fails_terminal_append(tmp_path):
    from quantlab.experiments import content_hash

    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    registered = _registered()
    registry.register(registered)
    attempts = [make_attempt(config, {"validation_net_mean": 0.0})
                for config in _configs()]
    report = {"candidate_family_size": len(attempts), "status": "rejected_or_unproven"}
    report["report_sha256"] = content_hash(report)
    assert_report_hash(report)
    terminal = {
        **registered,
        "status": "completed",
        "attempted_configs": attempts,
        "result": {"status": "rejected_or_unproven", "report_sha256": report["report_sha256"]},
        "validation": {"dataset_point_in_time": True, "holdout_used_for_selection": False},
        "results": report,
        "diagnostics": {"uses_final_holdout": True},
    }
    with pytest.raises(ExperimentError, match="holdout"):
        registry.append_result(terminal)
    assert registry.record("GRID-0001").status == "registered"
