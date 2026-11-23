"""Pure scalar test for the remaining benchmark utility.

Owner directive D11 retired the synthetic market-data generators and every
benchmark suite driven by invented market observations. Only the scalar
percentile helper and this independent test remain.
"""
from __future__ import annotations

from quantlab.bench import percentile


def test_percentile_selects_known_ranks() -> None:
    values = [10.0, 20.0, 30.0, 40.0, 50.0]
    assert percentile(values, 0.0) == 10.0
    assert percentile(values, 0.5) == 30.0
    assert percentile(values, 0.999) == 50.0
    assert percentile([], 0.5) == 0.0
