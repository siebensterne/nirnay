"""Constrained alpha search with costs, chronologically separated validation.

This module searches a finite, declared family.  It is not an
unbounded genetic-programming engine.  A candidate is never called validated
unless it survives the untouched final holdout under the specified cost model.
"""
from __future__ import annotations

import hashlib
import json
import math
from dataclasses import asdict, dataclass
from statistics import NormalDist
from typing import Iterable, Sequence

from .multiple_testing import holm_adjust
from .validation import cscv_pbo, deflated_sharpe_ratio


@dataclass(frozen=True, slots=True)
class Bar:
    timestamp_ns: int
    close: float

    def __post_init__(self) -> None:
        if type(self.timestamp_ns) is not int or self.timestamp_ns <= 0:
            raise ValueError("bar timestamp_ns must be a positive integer UTC nanosecond timestamp")
        if isinstance(self.close, bool) or not isinstance(self.close, (int, float)):
            raise ValueError("bar close must be a real numeric value")
        if not math.isfinite(float(self.close)) or self.close <= 0:
            raise ValueError("bar close must be positive and finite")


@dataclass(frozen=True, slots=True)
class AlphaConfig:
    family: str
    lookback: int
    threshold_bps: float = 0.0
    fast: int = 3
    slow: int = 10


@dataclass(frozen=True, slots=True)
class AlphaCosts:
    brokerage_bps: float = 2.0
    exchange_and_regulatory_bps: float = 0.5
    taxes_bps: float = 0.5
    spread_bps: float = 2.0
    slippage_bps: float = 1.0
    market_impact_bps: float = 1.0

    def __post_init__(self) -> None:
        for name, value in asdict(self).items():
            if isinstance(value, bool) or not isinstance(value, (int, float)):
                raise ValueError(f"{name} must be a real number of basis points")
            if not math.isfinite(float(value)) or value < 0:
                raise ValueError(f"{name} must be finite and non-negative")

    @property
    def one_way_bps(self) -> float:
        values = asdict(self)
        if any(not math.isfinite(v) or v < 0 for v in values.values()):
            raise ValueError("all cost assumptions must be finite and non-negative")
        return sum(values.values())


@dataclass(frozen=True, slots=True)
class CandidateScore:
    config: AlphaConfig
    train_net_mean: float
    validation_net_mean: float
    validation_sharpe: float
    validation_p_value: float
    validation_holm_p: float
    holdout_net_mean: float | None
    holdout_sharpe: float | None
    holdout_observations: int | None
    holdout_evaluation: str


def _ema(values: Sequence[float], span: int) -> list[float]:
    alpha = 2.0 / (span + 1.0)
    result = [values[0]]
    for value in values[1:]:
        result.append(alpha * value + (1.0 - alpha) * result[-1])
    return result


def _signals(closes: Sequence[float], config: AlphaConfig) -> list[int]:
    n = len(closes)
    out = [0] * n
    if config.family == "momentum":
        for i in range(config.lookback, n):
            change_bps = (closes[i] / closes[i - config.lookback] - 1.0) * 10_000
            out[i] = 1 if change_bps > config.threshold_bps else 0
    elif config.family == "mean_reversion":
        for i in range(config.lookback, n):
            window = closes[i - config.lookback:i]
            mean = sum(window) / len(window)
            variance = sum((x - mean) ** 2 for x in window) / max(1, len(window) - 1)
            sigma = math.sqrt(variance)
            z = (closes[i] - mean) / sigma if sigma > 0 else 0.0
            out[i] = 1 if z < -abs(config.threshold_bps) / 100.0 else 0
    elif config.family == "ema_cross":
        fast, slow = _ema(closes, config.fast), _ema(closes, config.slow)
        out = [int(a > b) for a, b in zip(fast, slow)]
        for i in range(min(config.slow, n)):
            out[i] = 0
    else:
        raise ValueError(f"unknown alpha family: {config.family}")
    return out


def _net_returns(closes: Sequence[float], config: AlphaConfig, costs: AlphaCosts, *,
                 start_return_bar: int = 1, stop_return_bar: int | None = None) -> list[float]:
    """Return independent flat-start/flat-end segment returns.

    Each output item is the close-to-close return ending at bar index ``t``.
    Signals are computed causally using data through bar ``t-1``. Segment
    boundaries reset the simulated position to flat and include entry and
    liquidation turnover costs, while indicator warm-up may use earlier bars.
    ``stop_return_bar`` is exclusive and no greater than len(closes).
    """
    n = len(closes)
    if n < 3:
        return []
    if any(not math.isfinite(float(x)) or x <= 0 for x in closes):
        raise ValueError("closes must be positive finite values")
    stop = n if stop_return_bar is None else stop_return_bar
    if (type(start_return_bar) is not int or type(stop) is not int or
            not (1 <= start_return_bar < stop <= n)):
        raise ValueError("return segment must satisfy 1 <= start < stop <= number of bars")
    signals = _signals(closes, config)
    returns: list[float] = []
    one_way_cost = costs.one_way_bps / 10_000.0
    previous_position = 0
    for t in range(start_return_bar, stop):
        # Signal from the prior close applies to the next close-to-close interval.
        position = signals[t - 1]
        market_return = closes[t] / closes[t - 1] - 1.0
        turnover = abs(position - previous_position)
        returns.append(position * market_return - turnover * one_way_cost)
        previous_position = position
    # Each fold liquidates at its last close; no position or cost is carried
    # across the train/validation/holdout boundary.
    if returns and previous_position:
        returns[-1] -= one_way_cost
    return returns


def _hac_sharpe_pvalue(values: Sequence[float], periods_per_year: float) -> tuple[float, float]:
    n = len(values)
    if n < 5:
        return 0.0, 1.0
    mean = sum(values) / n
    demeaned = [x - mean for x in values]
    gamma0 = sum(x * x for x in demeaned) / n
    if gamma0 <= 0:
        return 0.0, 1.0
    lag = max(1, min(n - 1, int(4 * (n / 100.0) ** (2.0 / 9.0))))
    long_run = gamma0
    for k in range(1, lag + 1):
        gamma = sum(demeaned[t] * demeaned[t - k] for t in range(k, n)) / n
        weight = 1.0 - k / (lag + 1.0)
        long_run += 2.0 * weight * gamma
    se = math.sqrt(max(long_run, 1e-18) / n)
    t_stat = mean / se
    p = 2.0 * (1.0 - NormalDist().cdf(abs(t_stat)))
    sharpe = mean / math.sqrt(gamma0) * math.sqrt(periods_per_year)
    return sharpe, min(1.0, max(0.0, p))


def _score(values: Sequence[float], periods_per_year: float) -> tuple[float, float, float]:
    if not values:
        return 0.0, 0.0, 1.0
    mean = sum(values) / len(values)
    sharpe, p = _hac_sharpe_pvalue(values, periods_per_year)
    return mean, sharpe, p


def default_candidate_family() -> tuple[AlphaConfig, ...]:
    """Finite, predeclared search family; grid size is stored in every report."""
    candidates = [AlphaConfig("momentum", lookback=n, threshold_bps=t)
                  for n in (3, 5, 10, 20) for t in (0.0, 5.0, 10.0)]
    candidates += [AlphaConfig("mean_reversion", lookback=n, threshold_bps=t)
                   for n in (5, 10, 20) for t in (50.0, 100.0, 150.0)]
    candidates += [AlphaConfig("ema_cross", lookback=s, fast=f, slow=s)
                   for f, s in ((3, 8), (5, 13), (8, 21), (10, 30))]
    return tuple(candidates)


def search_alpha(bars: Sequence[Bar], *, costs: AlphaCosts | None = None,
                 candidates: Sequence[AlphaConfig] | None = None,
                 train_fraction: float = 0.60, validation_fraction: float = 0.20,
                 min_train: int = 100, min_validation: int = 30,
                 min_holdout: int = 30, minimum_net_expectancy: float = 0.0,
                 require_statistical_significance: bool = False,
                 periods_per_year: float = 252.0, max_candidates: int = 250) -> dict[str, object]:
    if not (0.4 <= train_fraction <= 0.8 and 0.1 <= validation_fraction <= 0.3
            and train_fraction + validation_fraction < 0.9):
        raise ValueError("invalid chronological train/validation split")
    if any(type(x) is not int or x < 5 for x in (min_train, min_validation, min_holdout)):
        raise ValueError("minimum split sizes must be integers >= 5")
    if isinstance(periods_per_year, bool) or not isinstance(periods_per_year, (int, float)) or not math.isfinite(periods_per_year) or periods_per_year <= 0:
        raise ValueError("periods_per_year must be a positive finite number")
    if type(max_candidates) is not int or max_candidates < 1:
        raise ValueError("max_candidates must be a positive integer")
    if not math.isfinite(minimum_net_expectancy):
        raise ValueError("minimum_net_expectancy must be finite")
    if len(bars) < min_train + min_validation + min_holdout:
        raise ValueError("insufficient observations for train, validation and untouched holdout")
    if any(bars[i].timestamp_ns >= bars[i + 1].timestamp_ns for i in range(len(bars) - 1)):
        raise ValueError("bars must be strictly increasing by timestamp")
    if any(not math.isfinite(b.close) or b.close <= 0 for b in bars):
        raise ValueError("bar closes must be positive and finite")
    cost_model = costs or AlphaCosts()
    family = tuple(default_candidate_family() if candidates is None else candidates)
    if not family:
        raise ValueError("candidate family must not be empty")
    if len(family) > max_candidates:
        raise ValueError(f"candidate family exceeds configured max_candidates={max_candidates}")
    if len(set(family)) != len(family):
        raise ValueError("candidate family contains duplicate configurations")
    for config in family:
        if config.family not in {"momentum", "mean_reversion", "ema_cross"}:
            raise ValueError(f"unsupported candidate family: {config.family}")
        if type(config.lookback) is not int or config.lookback < 2:
            raise ValueError("candidate lookback must be an integer >= 2")
        if config.family == "ema_cross" and (type(config.fast) is not int or type(config.slow) is not int or config.fast < 2 or config.slow <= config.fast):
            raise ValueError("EMA candidate requires 2 <= fast < slow")
        if isinstance(config.threshold_bps, bool) or not isinstance(config.threshold_bps, (int, float)) or not math.isfinite(config.threshold_bps) or config.threshold_bps < 0:
            raise ValueError("candidate threshold_bps must be a finite non-negative number")
    closes = [b.close for b in bars]
    n = len(bars)
    train_end = int(n * train_fraction)
    validation_end = train_end + int(n * validation_fraction)
    # Split on return end-bar indices, with one embargoed return at each
    # boundary. Every fold starts and ends flat; indicator warm-up may use
    # historical closes that were observable before that fold begins.
    train_start, train_stop = 1, train_end
    validation_start, validation_stop = train_end + 1, validation_end
    holdout_start, holdout_stop = validation_end + 1, n
    train_count = train_stop - train_start
    validation_count = validation_stop - validation_start
    holdout_count = holdout_stop - holdout_start
    if train_count < min_train or validation_count < min_validation or holdout_count < min_holdout:
        raise ValueError("split sizes fall below minimum observations after one-return embargo")

    prelim: list[tuple[AlphaConfig, float, float, float, list[float]]] = []
    for config in family:
        train = _net_returns(closes, config, cost_model,
                             start_return_bar=train_start, stop_return_bar=train_stop)
        validation = _net_returns(closes, config, cost_model,
                                  start_return_bar=validation_start, stop_return_bar=validation_stop)
        if len(train) < min_train or len(validation) < min_validation:
            raise ValueError("candidate history is too short for requested split sizes")
        train_mean = sum(train) / len(train)
        val_mean, val_sharpe, p = _score(validation, periods_per_year)
        prelim.append((config, train_mean, val_mean, p, validation))
    adjusted = holm_adjust([x[3] for x in prelim])
    # Validation chooses one frozen model. The holdout is read only after this
    # selection and cannot be used to choose another candidate.
    selected_index = max(range(len(prelim)), key=lambda i: (prelim[i][2], -adjusted[i],
                                                             prelim[i][0].family,
                                                             prelim[i][0].lookback,
                                                             prelim[i][0].fast,
                                                             prelim[i][0].slow))
    selected = prelim[selected_index]
    holdout_returns = _net_returns(closes, selected[0], cost_model,
                                    start_return_bar=holdout_start, stop_return_bar=holdout_stop)
    hold_mean, hold_sharpe, hold_p = _score(holdout_returns, periods_per_year)
    passes = hold_mean > minimum_net_expectancy
    if require_statistical_significance:
        passes = passes and adjusted[selected_index] < 0.05 and hold_p < 0.05
    scores = []
    for index, item in enumerate(prelim):
        chosen = index == selected_index
        scores.append(CandidateScore(
            config=item[0],
            train_net_mean=item[1],
            validation_net_mean=item[2],
            validation_sharpe=_score(item[4], periods_per_year)[1],
            validation_p_value=item[3],
            validation_holm_p=adjusted[index],
            holdout_net_mean=hold_mean if chosen else None,
            holdout_sharpe=hold_sharpe if chosen else None,
            holdout_observations=len(holdout_returns) if chosen else None,
            holdout_evaluation="evaluated" if chosen else "not_evaluated",
        ))
    dataset_bytes = "\n".join(f"{b.timestamp_ns},{float(b.close):.17g}" for b in bars).encode()
    # Overfitting diagnostics use only the development partition (train + validation).
    # The final holdout is never included in candidate-family/CSCV diagnostics.
    development_matrix = [
        _net_returns(closes, config, cost_model, start_return_bar=1, stop_return_bar=validation_end)
        for config in family
    ]
    try:
        pbo_diagnostic: dict[str, object] = cscv_pbo(development_matrix, blocks=6)
    except ValueError as exc:
        pbo_diagnostic = {"status": "not_computable", "reason": str(exc),
                          "uses_final_holdout": False}
    dsr_diagnostic = deflated_sharpe_ratio(selected[4], trials=len(family))
    report: dict[str, object] = {
        "schema_version": 1,
        "status": "positive_holdout_screen_passed_not_alpha_certified" if passes else "rejected_or_unproven",
        "warning": "Research screening only. HAC p-values are approximate; this OHLC close-based model omits event-level fills, queue uncertainty, borrow, financing, taxes not explicitly supplied, lot/tick, auctions and market capacity. A positive holdout is not alpha certification or permission to trade.",
        "dataset_sha256": hashlib.sha256(dataset_bytes).hexdigest(),
        "observations": n,
        "data_time_range_utc_ns": [bars[0].timestamp_ns, bars[-1].timestamp_ns],
        "periods_per_year_for_sharpe_annualisation": periods_per_year,
        "return_unit": "decimal return per bar, after configured one-way turnover costs",
        "splits": {
            "train_return_end_bar_indices_inclusive": [train_start, train_stop - 1],
            "validation_return_end_bar_indices_inclusive": [validation_start, validation_stop - 1],
            "untouched_holdout_return_end_bar_indices_inclusive": [holdout_start, holdout_stop - 1],
            "untouched_holdout": [holdout_start, holdout_stop - 1],
            "embargoed_return_end_bar_indices": [train_end, validation_end],
            "segment_portfolio_policy": "flat at each segment start and liquidated with exit turnover cost at segment end",
        },
        "candidate_family_size": len(family),
        "costs": {**asdict(cost_model), "one_way_total_bps": cost_model.one_way_bps},
        "selection_metric": "validation net mean return; Holm-adjusted p-value tie-break",
        "selected": asdict(selected[0]),
        "selected_train_net_mean": selected[1],
        "selected_validation_net_mean": selected[2],
        "selected_validation_holm_p": adjusted[selected_index],
        "untouched_holdout_net_mean": hold_mean,
        "untouched_holdout_sharpe": hold_sharpe,
        "untouched_holdout_p_value_approx": hold_p,
        "passes_minimum_net_expectancy": passes,
        "multiple_testing_diagnostics": {
            "cscv_pbo": pbo_diagnostic,
            "deflated_sharpe_validation_only": dsr_diagnostic,
            "diagnostic_policy": "research-only; not a standalone deployment or capital-allocation gate",
        },
        "candidate_scores": [asdict(x) for x in scores],
    }
    report["report_sha256"] = hashlib.sha256(json.dumps(report, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    return report
