"""Canonical simulation namespace re-exports identical objects across submodules."""
from __future__ import annotations

from dataclasses import replace
from decimal import Decimal
from pathlib import Path

from quantlab.sim import (BaselineParams, ConservativeMbpModel, CostModel,
                          EngineConfig, ReplaySeeds, SimulationEngine,
                          compare_records as sim_compare_records,
                          default_cost_model,
                          read_record as sim_read_record)
from quantlab.sim.execution import BaselineParams as ExecBaselineParams
from quantlab.sim.execution import CostModel as ExecCostModel
from quantlab.sim.execution import default_cost_model as exec_cost
from quantlab.sim.replay import (RunRecord, build_record, compare_records,
                                 read_record, record_to_dict, write_record)


def _record() -> RunRecord:
    config = EngineConfig(starting_cash=Decimal("100000"), order_latency_ns=1,
                          cancel_latency_ns=1, max_participation=Decimal("1"))
    params = BaselineParams(order_latency_ns=1, cancel_latency_ns=1,
                            max_participation=Decimal("1"),
                            max_book_age_ns=1_000_000_000,
                            cost_model=default_cost_model())
    seeds = ReplaySeeds(1, 2, 3, 4)
    model = ConservativeMbpModel(params)
    result = SimulationEngine(config).run(tuple(), models=model, seeds=seeds)
    return build_record(result, engine_config=config, model_config=params.to_dict(),
                        seeds=seeds)


def test_namespace_symbols_are_identical_objects():
    assert compare_records is sim_compare_records
    assert read_record is sim_read_record
    assert BaselineParams is ExecBaselineParams
    assert CostModel is ExecCostModel
    assert exec_cost is default_cost_model


def test_canonical_write_and_read_record_round_trip(tmp_path: Path) -> None:
    """Written canonical records round-trip through the replay reader."""
    record = _record()
    path = write_record(record, tmp_path)
    assert read_record(path) == record


def test_canonical_record_round_trip_and_tampered_fill_digest() -> None:
    """A tampered fill digest fails canonical replay comparison."""
    record = _record()
    assert record_to_dict(record)["schema_version"] == 1
    comparison = compare_records(record, record)
    assert comparison.verified is True
    assert comparison.pnl_basis == "recomputed"
    tampered = replace(record, fills_digest="1" * 64)
    failed = compare_records(record, tampered)
    assert failed.verified is False
    assert failed.verified_equal["fills_digest"] is False
