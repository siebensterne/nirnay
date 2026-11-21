# SPDX-License-Identifier: Apache-2.0
"""Fail-closed cadence validation for registered experiment bars.

The registered record declares bar kind, fixed sampling interval, timestamp
semantics, calendar and session definition. This module checks those
declarations against the ordered timestamps actually supplied to a search. It
never repairs the series:

- missing bars are not invented from raw timestamp differences;
- exchange holidays, overnight breaks, auction periods and session boundaries
  require explicit calendar and session semantics, which are not inferable from
  raw diffs; a real-market series with such gaps must be normalised under a
  declared calendar before it is registered, not silently interpolated here;
- a synthetic series must declare a synthetic calendar and a fixed interval.

A daily registration is not satisfied by minute-spaced observations, and a
minute registration must match its declared interval exactly.
"""
from __future__ import annotations

from collections.abc import Sequence
from itertools import pairwise
from typing import Any

from .schema import NOT_APPLICABLE_INTERVAL, ExperimentError

_NS_PER_SECOND = 1_000_000_000
_MIN_MINUTE_INTERVAL_NS = 1 * _NS_PER_SECOND
_MAX_MINUTE_INTERVAL_NS = 60 * 60 * _NS_PER_SECOND
# A session-keyed daily bar is at least an exchange trading day apart. This
# rejects a 60-second bar mislabelled as daily even if the declaration agrees.
_MIN_DAILY_INTERVAL_NS = 18 * 60 * 60 * _NS_PER_SECOND
_SYNTHETIC_NAMESPACE = "synthetic:"


def is_synthetic_record(record: dict[str, Any]) -> bool:
    """Report whether the record declares synthetic rather than market data."""
    for field in ("dataset_id", "dataset_ref"):
        value = record.get(field)
        if isinstance(value, str) and "synthetic" in value.lower():
            return True
    # The calendar is not evidence of dataset kind: a market record that
    # declares a synthetic calendar is an inconsistency, not a synthetic set.
    config = record.get("config")
    return isinstance(config, dict) and config.get("synthetic") is True


def _validate_calendar_declaration(record: dict[str, Any], *, synthetic: bool) -> None:
    """Reject missing or mutually inconsistent calendar metadata."""
    calendar_id = record.get("calendar_id")
    session_definition = record.get("session_definition")
    if not isinstance(calendar_id, str) or not calendar_id.strip():
        raise ExperimentError("registered calendar_id is missing")
    if not isinstance(session_definition, str) or not session_definition.strip():
        raise ExperimentError("registered session_definition is missing")
    uses_synthetic_calendar = calendar_id.lower().startswith(_SYNTHETIC_NAMESPACE)
    if synthetic and not uses_synthetic_calendar:
        raise ExperimentError("synthetic dataset requires a synthetic calendar_id")
    if not synthetic and uses_synthetic_calendar:
        raise ExperimentError("market dataset cannot use a synthetic calendar_id")
    if not synthetic and "synthetic" in session_definition.lower():
        raise ExperimentError("market dataset cannot declare a synthetic session_definition")


def _validate_interval_declaration(record: dict[str, Any], *, synthetic: bool) -> int | None:
    """Return the declared fixed interval, or None for non-fixed bar kinds."""
    bar_kind = record.get("bar_kind")
    interval = record.get("bar_interval_ns")
    if synthetic and bar_kind not in {"daily", "minute"}:
        raise ExperimentError(
            "synthetic dataset requires a fixed-interval daily or minute bar kind"
        )
    if interval == NOT_APPLICABLE_INTERVAL:
        if synthetic:
            raise ExperimentError("synthetic dataset requires a fixed bar_interval_ns")
        if bar_kind in {"daily", "minute"}:
            raise ExperimentError(f"{bar_kind} bars require a fixed bar_interval_ns")
        return None
    if type(interval) is not int or interval <= 0:
        raise ExperimentError("bar_interval_ns must be positive or explicitly not_applicable")
    if bar_kind == "minute" and not (
        _MIN_MINUTE_INTERVAL_NS <= interval <= _MAX_MINUTE_INTERVAL_NS
    ):
        raise ExperimentError("minute bar_interval_ns is outside the minute sampling range")
    if bar_kind == "daily" and interval < _MIN_DAILY_INTERVAL_NS:
        raise ExperimentError("daily bar_interval_ns is shorter than an exchange trading day")
    return interval


def validate_bar_cadence(record: dict[str, Any], timestamps_ns: Sequence[int]) -> None:
    """Reject bars whose ordered timestamps disagree with the registered cadence.

    ``timestamps_ns`` must be strictly increasing UTC nanosecond timestamps in
    the exact order supplied to the search. The function is pure and mutates
    neither the record nor the timestamps.
    """
    if isinstance(timestamps_ns, (str, bytes)) or not isinstance(timestamps_ns, Sequence):
        raise ExperimentError("bar timestamps must be an ordered sequence")
    if len(timestamps_ns) < 2:
        raise ExperimentError("at least two ordered timestamps are required for cadence validation")
    for timestamp in timestamps_ns:
        if type(timestamp) is not int or timestamp <= 0:
            raise ExperimentError("bar timestamps must be positive integers")
    for previous, current in pairwise(timestamps_ns):
        if previous >= current:
            raise ExperimentError("bar timestamps must be strictly increasing")

    synthetic = is_synthetic_record(record)
    _validate_calendar_declaration(record, synthetic=synthetic)
    interval = _validate_interval_declaration(record, synthetic=synthetic)
    if interval is None:
        return

    observed = {current - previous for previous, current in pairwise(timestamps_ns)}
    mismatched = sorted(observed - {interval})
    if mismatched:
        raise ExperimentError(
            f"observed bar interval {mismatched[0]} ns disagrees with registered "
            f"bar_interval_ns {interval} ns"
        )
