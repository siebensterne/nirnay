"""Scalar benchmark utilities.

Owner directive D11 retired the synthetic market-data generators, the fake
market-session fixtures and every benchmark suite driven by invented market
observations. Only the scalar nearest-rank percentile helper remains; this
module emits no BENCH-* lines, exposes no benchmark CLI and reads no fixture.
"""
from __future__ import annotations


def percentile(values: list[float], fraction: float) -> float:
    """Nearest-rank percentile of an already sorted sequence."""
    if not values:
        return 0.0
    index = round((len(values) - 1) * fraction)
    return values[min(index, len(values) - 1)]
