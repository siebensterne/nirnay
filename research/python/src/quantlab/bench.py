"""Opt-in research benchmark harness with measured counters only.

This module times synthetic-dataset build, publication, verification, legacy
backtests and a close-based research screen, plus a concurrent read workload
against one immutable published dataset. Durations, hashes and high-water RSS
come from the executed code path; sizes, seeds and repeat counts are inputs.
Every emitted BENCH-* line is a local observation on synthetic fixtures, not a
throughput or latency certificate and not a pass/fail gate in ``make check``.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import random
import resource
import sys
import tempfile
import time
from concurrent.futures import ProcessPoolExecutor
from datetime import date
from decimal import Decimal
from pathlib import Path
from typing import Any

from quantlab.alpha_search import AlphaCosts, Bar, search_alpha
from quantlab.backtest import BacktestConfig, EventDrivenBacktester, OrderIntent
from quantlab.cli import _load_reference_data
from quantlab.market import canonical_bytes, market_event_from_dict, raw_hash
from quantlab.point_in_time import PITDatasetBuilder, verify_published_dataset, write_dataset
from quantlab.portfolio_backtest import MultiInstrumentPortfolioBacktester, PortfolioBacktestConfig

ROOT = Path(__file__).resolve().parents[4]
FIXTURES = ROOT / "research" / "python" / "fixtures"
BASE_EVENT_NS = 1_735_789_500_000_000_000
CREATED_AT_NS = 1_800_000_000_000_000_000
MIN_BARS = 200


def emit(line: str) -> None:
    """Write one measurement line to stderr without buffering it."""
    print(line, file=sys.stderr, flush=True)


def percentile(values: list[float], fraction: float) -> float:
    """Nearest-rank percentile of an already sorted sequence."""
    if not values:
        return 0.0
    index = round((len(values) - 1) * fraction)
    return values[min(index, len(values) - 1)]


def peak_rss_bytes() -> int:
    """Process high-water resident set size, normalised to bytes."""
    observed = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return observed if sys.platform == "darwin" else observed * 1024


def digest(value: Any) -> str:
    """Stable SHA-256 over the canonical encoding of a value."""
    return hashlib.sha256(canonical_bytes(value)).hexdigest()


def synthetic_events(size: int, seed: int) -> list[Any]:
    """Scale the checked-in synthetic canonical fixture to ``size`` events."""
    template = json.loads(
        (FIXTURES / "canonical-events.example.jsonl").read_text(encoding="utf-8").splitlines()[0]
    )
    events = []
    for index in range(size):
        value = dict(template)
        received = BASE_EVENT_NS + index * 1_000_000_000
        payload = {"seed": seed, "size": size, "index": index}
        value["event_id"] = f"bench-synthetic-{size}-{seed}-{index}"
        value["received_utc_ns"] = received
        value["received_monotonic_ns"] = index
        value["local_sequence"] = index + 1
        value["source_timestamp_ns"] = received
        value["last_trade_time_ns"] = received
        value["raw_sha256"] = raw_hash(payload)
        value["source_frame_sha256"] = raw_hash({"frame": payload})
        events.append(market_event_from_dict(value))
    return events


def synthetic_bars(size: int, seed: int) -> list[Bar]:
    """Generate deterministic positive close bars for the research screen."""
    rng = random.Random(seed)
    price = 100.0
    bars = []
    for index in range(max(size, MIN_BARS)):
        price = max(1.0, price * (1.0 + rng.uniform(-0.002, 0.002)))
        bars.append(Bar(BASE_EVENT_NS + index * 60_000_000_000, price))
    return bars


class _BuyOnce:
    strategy_id = "bench.synthetic.buy-once.v1"

    def __init__(self) -> None:
        self.sent: set[str] = set()

    def on_event(self, event: Any, context: Any) -> OrderIntent | None:
        if event.instrument_id in self.sent or event.event_kind not in {"quote", "depth"}:
            return None
        if event.best_ask is None:
            return None
        self.sent.add(event.instrument_id)
        return OrderIntent(
            client_order_id=f"bench:{event.instrument_id}",
            instrument_id=event.instrument_id,
            side="buy",
            quantity=1,
            limit_price=Decimal("101"),
            time_in_force_ns=5_000_000_000,
        )


def backtest_config() -> BacktestConfig:
    """Deterministic single-provider simulator configuration."""
    return BacktestConfig(
        starting_cash=Decimal("100000"),
        order_latency_ns=1,
        cancel_latency_ns=10,
        fee_bps=Decimal("0"),
        impact_coefficient_bps=Decimal("0"),
        max_participation=Decimal("1"),
        max_book_age_ns=5_000_000_000,
        source_provider="upstox",
    )


def portfolio_config() -> PortfolioBacktestConfig:
    """Deterministic multi-instrument simulator configuration."""
    return PortfolioBacktestConfig(
        starting_cash=Decimal("100000"),
        order_latency_ns=1,
        cancel_latency_ns=10,
        fee_bps=Decimal("0"),
        impact_coefficient_bps=Decimal("0"),
        max_participation=Decimal("1"),
        max_order_notional=Decimal("1000"),
        max_gross_exposure=Decimal("10000"),
        max_book_age_ns=5_000_000_000,
        source_provider="upstox",
    )


def benchmark_core_once(size: int, seed: int, out_dir: str | Path) -> dict[str, float | str]:
    """Run build, write, verify and three research workloads once."""
    events = synthetic_events(size, seed)
    references = _load_reference_data(FIXTURES / "reference-data.example.json")
    started = time.perf_counter()
    rows, manifest = PITDatasetBuilder(references).build(
        events,
        dataset_id="bench-synthetic-nse-session",
        created_at_utc_ns=CREATED_AT_NS,
        universe_id="fixture-universe",
    )
    build_ms = (time.perf_counter() - started) * 1000.0

    workspace = Path(tempfile.mkdtemp(prefix="nirnay-bench-research-", dir=out_dir))
    destination = workspace / "dataset"
    started = time.perf_counter()
    write_dataset(rows, manifest, destination)
    write_ms = (time.perf_counter() - started) * 1000.0

    started = time.perf_counter()
    verified = verify_published_dataset(destination)
    verify_ms = (time.perf_counter() - started) * 1000.0

    started = time.perf_counter()
    result = EventDrivenBacktester(backtest_config()).run(events, _BuyOnce())
    backtest_ms = (time.perf_counter() - started) * 1000.0

    started = time.perf_counter()
    portfolio = MultiInstrumentPortfolioBacktester(portfolio_config()).run(events, _BuyOnce())
    portfolio_ms = (time.perf_counter() - started) * 1000.0

    bars = synthetic_bars(size, seed)
    started = time.perf_counter()
    report = search_alpha(
        bars,
        costs=AlphaCosts(),
        min_train=100,
        min_validation=30,
        min_holdout=30,
        minimum_net_expectancy=0.0,
        periods_per_year=252,
    )
    alpha_ms = (time.perf_counter() - started) * 1000.0

    hashes = {
        "dataset_sha256": manifest.data_sha256,
        "verify_sha256": digest(verified),
        "backtest_sha256": digest(result.summary()),
        "portfolio_sha256": digest(portfolio.summary()),
        "alpha_sha256": digest(report),
    }
    combined = hashlib.sha256()
    for name in (
        "dataset_sha256",
        "verify_sha256",
        "backtest_sha256",
        "portfolio_sha256",
        "alpha_sha256",
    ):
        combined.update(hashes[name].encode("ascii") + b"\n")
    values: dict[str, float | str] = {
        "rows": float(len(rows)),
        "build_ms": build_ms,
        "write_ms": write_ms,
        "verify_ms": verify_ms,
        "backtest_ms": backtest_ms,
        "portfolio_ms": portfolio_ms,
        "alpha_ms": alpha_ms,
        "peak_rss_bytes": float(peak_rss_bytes()),
        "output_sha256": combined.hexdigest(),
        "verify_status": str(verified["status"]),
    }
    values.update(hashes)
    return values


def run_core_suite(size: int, repeats: int, seed: int, out_dir: str | Path) -> list[str]:
    """Run the synthetic research workloads ``repeats`` times and emit lines."""
    runs = [benchmark_core_once(size, seed, out_dir) for _ in range(repeats)]
    outputs = {str(run["output_sha256"]) for run in runs}
    assert len(outputs) == 1, "output hashes must be deterministic across repeats"
    common = (
        f"size={size} rows={int(runs[0]['rows'])} repeats={repeats}"
        f" seed={seed} source=synthetic-local"
    )
    lines = []
    for name, key in (
        ("BUILD", "build_ms"),
        ("WRITE", "write_ms"),
        ("VERIFY", "verify_ms"),
        ("BACKTEST", "backtest_ms"),
        ("PORTFOLIO", "portfolio_ms"),
        ("ALPHA", "alpha_ms"),
    ):
        samples = [float(run[key]) for run in runs]
        ordered = sorted(samples)
        line = (
            f"BENCH-RESEARCH-{name} {common}"
            f" p50_ms={percentile(ordered, 0.50):.3f}"
            f" p99_ms={percentile(ordered, 0.99):.3f}"
            f" p999_ms={percentile(ordered, 0.999):.3f}"
            f" peak_rss_bytes={int(max(float(run['peak_rss_bytes']) for run in runs))}"
        )
        lines.append(line)
    status = runs[0]["verify_status"]
    lines.append(
        f"BENCH-RESEARCH-OUTPUT {common} deterministic=true"
        f" output_sha256={runs[0]['output_sha256']}"
        f" dataset_sha256={runs[0]['dataset_sha256']}"
        f" backtest_sha256={runs[0]['backtest_sha256']}"
        f" portfolio_sha256={runs[0]['portfolio_sha256']}"
        f" alpha_sha256={runs[0]['alpha_sha256']}"
        f" verify_status={status}"
    )
    for line in lines:
        emit(line)
    return lines


def _experiment_worker(job: dict[str, Any]) -> dict[str, Any]:
    """Read-only research worker over one immutable published dataset."""
    started = time.perf_counter()
    root = Path(job["dataset_dir"])
    lines = (root / "events.jsonl").read_text(encoding="utf-8").splitlines()
    rows = [json.loads(line) for line in lines]
    dataset_hash = hashlib.sha256("\n".join(lines).encode("utf-8")).hexdigest()
    report = search_alpha(
        synthetic_bars(job["size"], job["seed"]),
        costs=AlphaCosts(),
        min_train=100,
        min_validation=30,
        min_holdout=30,
        minimum_net_expectancy=0.0,
        periods_per_year=252,
    )
    return {
        "elapsed_ms": (time.perf_counter() - started) * 1000.0,
        "peak_rss_bytes": peak_rss_bytes(),
        "rows": len(rows),
        "dataset_sha256": dataset_hash,
        "report_sha256": digest(report),
    }


def run_experiments_suite(
    size: int, workers: int, repeats: int, seed: int, out_dir: str | Path
) -> list[str]:
    """Run concurrent read-only experiments against one immutable dataset."""
    if workers < 1 or repeats < 2:
        raise ValueError("workers must be positive and repeats must be at least two")
    events = synthetic_events(size, seed)
    references = _load_reference_data(FIXTURES / "reference-data.example.json")
    rows, manifest = PITDatasetBuilder(references).build(
        events,
        dataset_id="bench-experiments",
        created_at_utc_ns=CREATED_AT_NS,
        universe_id="fixture-universe",
    )
    workspace = Path(tempfile.mkdtemp(prefix="nirnay-bench-experiments-", dir=out_dir))
    dataset = workspace / "dataset"
    write_dataset(rows, manifest, dataset)
    before = verify_published_dataset(dataset)["status"]

    latencies: list[float] = []
    max_rss = 0
    combined = hashlib.sha256()
    wall_started = time.perf_counter()
    for _ in range(repeats):
        jobs = [{"dataset_dir": str(dataset), "size": size, "seed": seed}] * workers
        with ProcessPoolExecutor(max_workers=workers) as pool:
            results = list(pool.map(_experiment_worker, jobs))
        latencies.extend(float(result["elapsed_ms"]) for result in results)
        max_rss = max(max_rss, *(int(result["peak_rss_bytes"]) for result in results))
        assert all(result["rows"] == size for result in results), (
            "every worker read the full dataset"
        )
        dataset_hashes = {result["dataset_sha256"] for result in results}
        report_hashes = {result["report_sha256"] for result in results}
        assert len(dataset_hashes) == 1 and len(report_hashes) == 1, (
            "concurrent workers must produce identical hashes"
        )
        combined.update(next(iter(dataset_hashes)).encode("ascii") + b"\n")
        combined.update(next(iter(report_hashes)).encode("ascii") + b"\n")
    wall_seconds = time.perf_counter() - wall_started
    after = verify_published_dataset(dataset)["status"]
    assert after == before, "concurrent reads must not corrupt the published dataset"
    ordered = sorted(latencies)
    line = (
        f"BENCH-EXPERIMENTS workers={workers} repeats={repeats} size={size} seed={seed}"
        f" worker_samples={len(latencies)}"
        f" p50_ms={percentile(ordered, 0.50):.3f}"
        f" p99_ms={percentile(ordered, 0.99):.3f}"
        f" p999_ms={percentile(ordered, 0.999):.3f}"
        f" throughput_per_sec={len(latencies) / wall_seconds:.3f}"
        f" max_worker_rss_bytes={max_rss}"
        f" output_sha256={combined.hexdigest()}"
        f" verify_status={after} source=synthetic-local"
    )
    emit(line)
    return [line]


def benchmark_historical_query_once(size: int, seed: int, out_dir: str | Path
                                    ) -> dict[str, float | str]:
    """Time point-in-time reference lookups over a published synthetic dataset."""
    events = synthetic_events(size, seed)
    references = _load_reference_data(FIXTURES / "reference-data.example.json")
    rows, manifest = PITDatasetBuilder(references).build(
        events,
        dataset_id="bench-historical-query",
        created_at_utc_ns=CREATED_AT_NS,
        universe_id="fixture-universe",
    )
    workspace = Path(tempfile.mkdtemp(prefix="nirnay-bench-query-", dir=out_dir))
    dataset = write_dataset(rows, manifest, workspace / "dataset")
    published = [json.loads(line) for line in
                 (dataset / "events.jsonl").read_text(encoding="utf-8").splitlines()]
    durations: list[float] = []
    resolved: list[str] = []
    for row in published:
        event = row["event"]
        started = time.perf_counter()
        listing = references.resolve(
            event["provider"], event["provider_instrument_id"],
            valid_on=date.fromisoformat(row["session_date_ist"]),
            known_at_ns=event["received_utc_ns"],
        )
        actions = references.known_actions(row["instrument_id"],
                                           known_at_ns=event["received_utc_ns"])
        durations.append((time.perf_counter() - started) * 1_000_000.0)
        resolved.append(f"{listing.listing_id}:{','.join(a.action_id for a in actions)}")
    ordered = sorted(durations)
    return {
        "rows": float(len(published)),
        "queries": float(len(durations) * 2),
        "p50_us": percentile(ordered, 0.50),
        "p99_us": percentile(ordered, 0.99),
        "p999_us": percentile(ordered, 0.999),
        "peak_rss_bytes": float(peak_rss_bytes()),
        "output_sha256": digest(resolved),
    }


def benchmark_end_to_end_once(size: int, seed: int, out_dir: str | Path
                              ) -> dict[str, float | str]:
    """Time synthetic ingest-to-consumer data latency through publication."""
    events = synthetic_events(size, seed)
    references = _load_reference_data(FIXTURES / "reference-data.example.json")
    ingest_ns = time.perf_counter_ns()
    rows, manifest = PITDatasetBuilder(references).build(
        events,
        dataset_id="bench-e2e-latency",
        created_at_utc_ns=CREATED_AT_NS,
        universe_id="fixture-universe",
    )
    workspace = Path(tempfile.mkdtemp(prefix="nirnay-bench-e2e-", dir=out_dir))
    dataset = write_dataset(rows, manifest, workspace / "dataset")
    durations: list[float] = []
    payloads: list[dict[str, Any]] = []
    for line in (dataset / "events.jsonl").read_text(encoding="utf-8").splitlines():
        consumer_ns = time.perf_counter_ns()
        durations.append((consumer_ns - ingest_ns) / 1_000.0)
        payloads.append(json.loads(line))
    ordered = sorted(durations)
    return {
        "rows": float(len(payloads)),
        "samples": float(len(durations)),
        "p50_us": percentile(ordered, 0.50),
        "p99_us": percentile(ordered, 0.99),
        "p999_us": percentile(ordered, 0.999),
        "peak_rss_bytes": float(peak_rss_bytes()),
        "output_sha256": digest(payloads),
    }


def run_latency_suite(size: int, repeats: int, seed: int, out_dir: str | Path) -> list[str]:
    """Run synthetic historical-query and end-to-end latency paths."""
    if repeats < 2:
        raise ValueError("latency suite needs at least two repeats")
    queries = [benchmark_historical_query_once(size, seed, out_dir) for _ in range(repeats)]
    e2e = [benchmark_end_to_end_once(size, seed, out_dir) for _ in range(repeats)]
    for group in (queries, e2e):
        outputs = {str(run["output_sha256"]) for run in group}
        assert len(outputs) == 1, "latency output hashes must be deterministic across repeats"
    common = f"size={size} rows={int(queries[0]['rows'])} repeats={repeats} seed={seed}"
    lines = [(
        f"BENCH-HISTORICAL-QUERY {common} queries={int(queries[0]['queries'])}"
        f" p50_us={percentile(sorted(float(r['p50_us']) for r in queries), 0.50):.3f}"
        f" p99_us={percentile(sorted(float(r['p99_us']) for r in queries), 0.99):.3f}"
        f" p999_us={percentile(sorted(float(r['p999_us']) for r in queries), 0.999):.3f}"
        f" peak_rss_bytes={int(max(float(r['peak_rss_bytes']) for r in queries))}"
        f" output_sha256={queries[0]['output_sha256']} source=synthetic-local"
    )]
    lines.append((
        f"BENCH-E2E-LATENCY {common} samples={int(e2e[0]['samples'])}"
        f" p50_us={percentile(sorted(float(r['p50_us']) for r in e2e), 0.50):.3f}"
        f" p99_us={percentile(sorted(float(r['p99_us']) for r in e2e), 0.99):.3f}"
        f" p999_us={percentile(sorted(float(r['p999_us']) for r in e2e), 0.999):.3f}"
        f" peak_rss_bytes={int(max(float(r['peak_rss_bytes']) for r in e2e))}"
        f" output_sha256={e2e[0]['output_sha256']} source=synthetic-local"
    ))
    for line in lines:
        emit(line)
    return lines


def parse_sizes(raw: str) -> list[int]:
    """Parse a comma-separated list of positive synthetic fixture sizes."""
    sizes = [int(part.strip()) for part in raw.split(",") if part.strip()]
    if not sizes or any(size < MIN_BARS for size in sizes):
        raise ValueError(f"sizes must be comma-separated integers of at least {MIN_BARS}")
    return sizes


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--suite", choices=("core", "experiments", "latency", "all"),
                        default="all")
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--sizes", default="200,800")
    parser.add_argument("--size", type=int, default=800)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--seed", type=int, default=7)
    parser.add_argument("--out", default=None)
    args = parser.parse_args(argv)
    if args.repeats < 2:
        parser.error("repeats must be at least two so a distribution is emitted")
    if args.size < MIN_BARS:
        parser.error(f"size must be at least {MIN_BARS}")
    out_dir = args.out or tempfile.mkdtemp(prefix="nirnay-bench-python-")
    Path(out_dir).mkdir(parents=True, exist_ok=True)
    if args.suite in ("core", "all"):
        for size in parse_sizes(args.sizes):
            run_core_suite(size, args.repeats, args.seed, out_dir)
    if args.suite in ("experiments", "all"):
        run_experiments_suite(args.size, args.workers, args.repeats, args.seed, out_dir)
    if args.suite in ("latency", "all"):
        run_latency_suite(args.size, args.repeats, args.seed, out_dir)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
