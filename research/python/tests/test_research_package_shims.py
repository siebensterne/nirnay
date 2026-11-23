"""Tests for research package helpers and the canonical simulation digest."""
from __future__ import annotations

from dataclasses import asdict
from pathlib import Path

import json
from decimal import Decimal

import pytest


from quantlab.alpha_search import AlphaCosts
from quantlab.cli import _load_reference_data
from quantlab.data import dataset_identity_from_manifest, dataset_lineage_from_manifest
from quantlab.experiments.reproducibility import (assert_report_hash, bind_dataset,
                                                  environment_digest,
                                                  recompute_report_hash_from_record,
                                                  strategy_code_digest)
from quantlab.experiments.verify import signed_head_message
from quantlab.market import market_event_from_dict
from quantlab.point_in_time import PITDatasetBuilder
from quantlab.reporting import research_report_bytes
from quantlab.sim import cost_assumptions_digest
from quantlab.strategies import DeterministicEmaCross
from quantlab.validation import (assert_no_holdout_leakage, cscv_pbo, deflated_sharpe_ratio,
                                 detect_holdout_leakage)

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = ROOT / "research" / "python" / "fixtures"


def _manifest():
    events = [market_event_from_dict(json.loads(line)) for line in
              (FIXTURES / "canonical-events.example.jsonl").read_text().splitlines() if line]
    references = _load_reference_data(FIXTURES / "reference-data.example.json")
    _, manifest = PITDatasetBuilder(references).build(
        events, dataset_id="shim-fixture", created_at_utc_ns=1_735_789_503_000_000_000,
        universe_id="fixture-universe",
    )
    return manifest


@pytest.mark.market_shaped_fixture
def test_dataset_lineage_and_identity_helpers():
    manifest = _manifest()
    lineage = dataset_lineage_from_manifest(manifest)
    assert set(lineage) == {"data_sha256", "input_sha256", "code_digest", "config_digest",
                            "source_event_hashes", "source_frame_hashes",
                            "listing_source_hashes", "corporate_action_source_hashes"}
    identity = dataset_identity_from_manifest(manifest)
    assert identity["dataset_id"] == "shim-fixture"
    assert identity["dataset_version"] == manifest.dataset_id


@pytest.mark.market_shaped_fixture
def test_bind_dataset_returns_lineage_and_digest():
    binding = bind_dataset(_manifest())
    assert len(binding["dataset_manifest_sha256"]) == 64
    assert binding["dataset_lineage"]["data_sha256"] == "a" * 64 or len(
        binding["dataset_lineage"]["data_sha256"]
    ) == 64


def test_environment_digest_is_path_order_stable_and_refuses_duplicates(tmp_path):
    first = tmp_path / "a.py"
    second = tmp_path / "b.py"
    first.write_text("x = 1\n", encoding="utf-8")
    second.write_text("y = 2\n", encoding="utf-8")
    assert environment_digest([second, first]) == environment_digest([first, second])
    assert strategy_code_digest([first, second]) == environment_digest([first, second])
    with pytest.raises(ValueError, match="unique"):
        environment_digest([first, first])


def test_cost_assumptions_digest_matches_canonical_values():
    costs = AlphaCosts()
    digest = cost_assumptions_digest(costs)
    assert digest == cost_assumptions_digest(asdict(costs))
    assert digest != cost_assumptions_digest(AlphaCosts(brokerage_bps=99.0))
    with pytest.raises(ValueError):
        cost_assumptions_digest("not-a-cost-model")


def test_report_digest_helpers_reject_tampering():
    report = {"status": "rejected_or_unproven", "candidate_family_size": 1}
    report["report_sha256"] = recompute_report_hash_from_record(report)
    assert_report_hash(report)
    report["candidate_family_size"] = 2
    with pytest.raises(ValueError, match="mismatch"):
        assert_report_hash(report)


def test_report_bytes_are_byte_stable():
    payload = {"b": 2, "a": 1}
    assert research_report_bytes(payload) == b'{"a":1,"b":2}\n'


def test_signed_head_message_is_domain_separated():
    message = signed_head_message("a" * 64)
    assert message.startswith(b"NIRNAY-EXPERIMENT-REGISTRY-HEAD-V1\x00")
    assert len(message) == 32 + len(b"NIRNAY-EXPERIMENT-REGISTRY-HEAD-V1\x00")
    with pytest.raises(ValueError):
        signed_head_message("not-hex")


def test_validation_shim_keeps_diagnostics_and_adds_leakage_detector():
    matrix = [[float(index) for index in range(20)], [float(index) for index in range(20)]]
    assert cscv_pbo(matrix, blocks=6)["uses_final_holdout"] is False
    assert deflated_sharpe_ratio([0.01] * 10, trials=2)["trials"] == 2
    result = detect_holdout_leakage({"dataset_point_in_time": True})
    assert result["status"] == "holdout_leakage_detected"
    with pytest.raises(ValueError):
        assert_no_holdout_leakage({"dataset_point_in_time": True})


def test_strategy_shim_keeps_deterministic_fixture():
    from quantlab.backtest import StrategyContext

    strategy = DeterministicEmaCross(instrument_id="in.x")
    context = StrategyContext(0, False, Decimal("0"))
    assert strategy.strategy_id == "fixture.ema-cross.v2"
    assert context.position_quantity == 0
    with pytest.raises(ValueError):
        DeterministicEmaCross(instrument_id="in.x", fast=7, slow=3)
