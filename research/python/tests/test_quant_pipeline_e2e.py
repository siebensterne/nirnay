"""End-to-end synthetic acceptance path for research-to-paper workflows.

This test uses checked-in synthetic fixtures. It proves that the
components compose and preserve their declared contracts; it does not certify
provider entitlements, exchange data fidelity, alpha or live execution.
"""
from __future__ import annotations

import csv
import json
from decimal import Decimal
from pathlib import Path

from quantlab.alpha_search import AlphaCosts, Bar, search_alpha
from quantlab.backtest import OrderIntent
from quantlab.cli import _load_reference_data
from quantlab.durable_oms import DurableOMSStore, PaperOutboxDispatcher
from quantlab.market import market_event_from_dict
from quantlab.point_in_time import PITDatasetBuilder, verify_published_dataset, write_dataset
from quantlab.portfolio_backtest import MultiInstrumentPortfolioBacktester, PortfolioBacktestConfig
from quantlab.oms import BrokerOrderIntent

import pytest
pytestmark = pytest.mark.market_shaped_fixture


ROOT = Path(__file__).resolve().parents[3]
FIXTURES = ROOT / "research" / "python" / "fixtures"


class _BuyOnce:
    strategy_id = "e2e.synthetic.buy-once.v1"

    def __init__(self):
        self.sent: set[str] = set()

    def on_event(self, event, context):
        if event.instrument_id in self.sent or event.event_kind not in {"quote", "depth"}:
            return None
        if event.best_ask is None:
            return None
        self.sent.add(event.instrument_id)
        return OrderIntent(
            client_order_id=f"e2e:{event.instrument_id}",
            instrument_id=event.instrument_id,
            side="buy",
            quantity=1,
            limit_price=Decimal("101"),
            time_in_force_ns=5_000_000_000,
        )


class _PaperBroker:
    broker_id = "paper"

    def __init__(self):
        self.submitted = []
        self.cancelled = []

    def submit(self, intent, *, client_order_id):
        self.submitted.append((intent, client_order_id))
        return f"paper-{client_order_id}"

    def cancel(self, broker_order_id, *, client_order_id):
        self.cancelled.append((broker_order_id, client_order_id))


def _canonical_events():
    return [
        market_event_from_dict(json.loads(line))
        for line in (FIXTURES / "canonical-events.example.jsonl").read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]


def test_synthetic_research_to_paper_pipeline_composes_without_claiming_live_readiness(tmp_path):
    # 1. Raw-fixture canonical observations -> bitemporal PIT dataset.
    events = _canonical_events()
    references = _load_reference_data(FIXTURES / "reference-data.example.json")
    rows, manifest = PITDatasetBuilder(references).build(
        events,
        dataset_id="e2e-synthetic-nse-session",
        created_at_utc_ns=1_735_789_503_000_000_000,
        universe_id="fixture-universe",
    )
    assert len(rows) == 3
    assert rows[0].split_price_factor_num == 1
    assert rows[0].split_price_factor_den == 2
    assert rows[0].action_lineage == ("split-fixture-2-for-1",)
    assert manifest.raw_prices_mutated is False
    assert manifest.survivorship_completeness_independently_verified is False

    dataset_dir = write_dataset(rows, manifest, tmp_path / "published-pit")
    verified = verify_published_dataset(dataset_dir)
    assert verified["status"] == "verified_internal_integrity"
    assert verified["row_count"] == 3

    # 2. One-provider event stream -> deterministic event-driven paper model.
    backtest = MultiInstrumentPortfolioBacktester(PortfolioBacktestConfig(
        starting_cash=Decimal("1000"),
        order_latency_ns=1,
        cancel_latency_ns=10,
        fee_bps=Decimal("0"),
        impact_coefficient_bps=Decimal("0"),
        max_participation=Decimal("1"),
        max_order_notional=Decimal("200"),
        max_gross_exposure=Decimal("500"),
        max_book_age_ns=5_000_000_000,
        source_provider="upstox",
    )).run(events, _BuyOnce())
    assert backtest.orders == 1
    assert len(backtest.fills) == 1
    assert backtest.positions == {events[0].instrument_id: 1}
    assert backtest.net_pnl == backtest.gross_pnl - backtest.fees - backtest.impact_cost - backtest.spread_cost
    assert backtest.model.startswith("multi_instrument_l2_queue")
    assert "not exchange FIFO" in backtest.summary()["queue_claim"]

    # 3. Fixed candidate family -> chronological net-of-cost validation report.
    with (FIXTURES / "alpha-bars.synthetic.csv").open(newline="", encoding="utf-8") as stream:
        bars = [Bar(int(row["timestamp_ns"]), float(row["close"])) for row in csv.DictReader(stream)]
    alpha_report = search_alpha(
        bars,
        costs=AlphaCosts(),
        min_train=100,
        min_validation=30,
        min_holdout=30,
        minimum_net_expectancy=0.0,
        periods_per_year=252,
    )
    assert alpha_report["candidate_family_size"] == 25
    assert len(alpha_report["candidate_scores"]) == 25
    assert alpha_report["splits"]["segment_portfolio_policy"].startswith("flat at each segment start")
    assert alpha_report["warning"].startswith("Research screening only")

    # 4. Durable OMS -> transactional outbox -> paper broker only.
    store = DurableOMSStore(tmp_path / "oms" / "oms.sqlite3")
    intent = BrokerOrderIntent(
        client_order_id="e2e-order-001",
        strategy_id="e2e.synthetic.buy-once.v1",
        instrument_id="NSE:SBIN",
        venue="NSE",
        side="buy",
        order_type="limit",
        product="CNC",
        quantity=1,
        limit_price=Decimal("500.00"),
        time_in_force="day",
    )
    accepted = store.accept_intent(intent, broker="paper", now_ns=1_800_000_000_000_000_000)
    assert accepted.status == "PENDING_NEW"
    paper = _PaperBroker()
    working = PaperOutboxDispatcher(store, paper, mode="paper").dispatch_one(
        worker_id="e2e-paper-worker", now_ns=1_800_000_000_000_000_001,
    )
    assert working.status == "WORKING"
    assert len(paper.submitted) == 1
    final = store.apply_broker_event(
        broker="paper",
        client_order_id="e2e-order-001",
        broker_order_id="paper-e2e-order-001",
        event_id="e2e-execution-001",
        status="FILLED",
        sequence=1,
        fill_quantity=1,
        fill_price="500.00",
        execution_id="paper-exec-001",
        now_ns=1_800_000_000_000_000_002,
    )
    assert final.status == "FILLED" and final.filled_quantity == 1
    assert store.verify_event_chain()["status"] == "verified_internal_integrity"

    # Evidence label is part of the contract, not an optional footnote.
    assert manifest.survivorship_completeness_independently_verified is False
    assert "Research screening only" in alpha_report["warning"]
    assert backtest.source_provider == "upstox"
