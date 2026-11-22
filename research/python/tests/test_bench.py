"""Tests for the opt-in research benchmark harness.

These tests exercise the harness with small synthetic fixtures. They assert
measured-counter presence and hash determinism; no latency or throughput
threshold is a gate.
"""
from __future__ import annotations

import pytest

from quantlab.bench import (
    digest,
    percentile,
    run_core_suite,
    run_experiments_suite,
    run_latency_suite,
    synthetic_events,
)


def _fields(line: str) -> dict[str, str]:
    parts = line.split()
    assert parts and parts[0].startswith("BENCH-"), "line is a benchmark record"
    return {
        key: value
        for key, value in (part.split("=", 1) for part in parts[1:] if "=" in part)
    }


def test_percentile_selects_known_ranks() -> None:
    values = [10.0, 20.0, 30.0, 40.0, 50.0]
    assert percentile(values, 0.0) == 10.0
    assert percentile(values, 0.5) == 30.0
    assert percentile(values, 0.999) == 50.0
    assert percentile([], 0.5) == 0.0


@pytest.mark.market_shaped_fixture
def test_synthetic_fixture_scales_and_is_byte_stable() -> None:
    small = synthetic_events(40, 7)
    large = synthetic_events(200, 7)
    assert len(small) == 40 and len(large) == 200, "fixture size follows the argument"
    assert len({event.event_id for event in large}) == 200, "event ids are unique"
    assert digest([event.as_dict() for event in large]) == digest(
        [event.as_dict() for event in synthetic_events(200, 7)]
    ), "the synthetic fixture is byte-stable for a fixed seed"


@pytest.mark.market_shaped_fixture
def test_core_suite_emits_measured_distributions_and_stable_hashes(tmp_path) -> None:
    lines = run_core_suite(200, 3, 7, tmp_path)
    expected = ("BUILD", "WRITE", "VERIFY", "BACKTEST", "PORTFOLIO", "ALPHA", "OUTPUT")
    assert len(lines) == len(expected), "one aggregate line per measured stage"
    for line, name in zip(lines, expected):
        assert line.startswith(f"BENCH-RESEARCH-{name} "), "stage label is present"
    output = _fields(lines[-1])
    assert len(output["output_sha256"]) == 64, "output hash is a full SHA-256 digest"
    for line in lines[:-1]:
        fields = _fields(line)
        assert int(fields["rows"]) == 200, "row count is measured from the dataset"
        assert int(fields["peak_rss_bytes"]) > 0, "peak RSS is measured"
        assert 0.0 <= float(fields["p50_ms"]) <= float(fields["p999_ms"])
    assert output["deterministic"] == "true", "repeat determinism was asserted"
    assert output["verify_status"] == "verified_internal_integrity"


@pytest.mark.market_shaped_fixture
def test_experiments_suite_reads_one_immutable_dataset_concurrently(tmp_path) -> None:
    first = run_experiments_suite(200, 2, 2, 7, tmp_path)[0]
    again = run_experiments_suite(200, 2, 2, 7, tmp_path)[0]
    fields = _fields(first)
    assert fields["workers"] == "2" and fields["worker_samples"] == "4"
    assert int(fields["max_worker_rss_bytes"]) > 0, "worker RSS is measured"
    assert float(fields["throughput_per_sec"]) > 0.0, "throughput is measured"
    assert fields["verify_status"] == "verified_internal_integrity"
    assert {_fields(again)["output_sha256"]} == {fields["output_sha256"]}, (
        "worker hashes are deterministic across suite runs"
    )


@pytest.mark.market_shaped_fixture
def test_latency_suite_emits_measured_distributions_and_stable_hashes(tmp_path) -> None:
    first = run_latency_suite(200, 2, 7, tmp_path)
    second = run_latency_suite(200, 2, 7, tmp_path)
    assert len(first) == 2
    assert first[0].startswith("BENCH-HISTORICAL-QUERY ") and "queries=" in first[0]
    assert first[1].startswith("BENCH-E2E-LATENCY ") and "samples=" in first[1]
    for line in first:
        fields = _fields(line)
        assert int(fields["size"]) == 200
        assert 0.0 <= float(fields["p50_us"]) <= float(fields["p999_us"])
        assert int(fields["peak_rss_bytes"]) > 0
        assert len(fields["output_sha256"]) == 64
    for before, after in zip(first, second):
        assert _fields(before)["output_sha256"] == _fields(after)["output_sha256"]
