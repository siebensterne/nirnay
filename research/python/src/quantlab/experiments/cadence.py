# SPDX-License-Identifier: Apache-2.0
"""Fail-closed cadence validation for registered experiment bars.

The immutable record declares dataset origin, bar specification, calendar
reference, session semantics and annualisation. This module checks those
declarations against a checked-in calendar artifact and the ordered timestamps
actually supplied to a search. It never repairs the series:

- an absent price bar is not invented from raw timestamp differences;
- weekends, holidays, auctions, scheduled breaks and overnight windows are
  calendar gaps, not fixed elapsed intervals, and are excluded from the
  expected schedule;
- a missing bar inside an active session fails unless an explicit coverage
  status is declared;
- observed timestamps outside the expected schedule fail.

Dataset origin is explicit. Identifier substrings and config booleans are never
used to infer origin, and a config flag cannot override an explicit MARKET
declaration.
"""
from __future__ import annotations

from collections.abc import Mapping, Sequence
from itertools import pairwise
from typing import Any

from .calendar import expected_timestamps, resolve_calendar
from .coverage import validate_status_observations
from .schema import (NOT_APPLICABLE_INTERVAL, ExperimentError, _DATASET_ORIGINS,
                     _TIMESTAMP_SEMANTICS)

_NS_PER_SECOND = 1_000_000_000
_MIN_MINUTE_INTERVAL_NS = 60 * _NS_PER_SECOND
_MAX_MINUTE_INTERVAL_NS = 60 * 60 * _NS_PER_SECOND
_MIN_DAILY_INTERVAL_NS = 18 * 60 * 60 * _NS_PER_SECOND


def dataset_origin(record: dict[str, Any]) -> str:
    """Return the explicit dataset origin; never infer it from config or ids."""
    origin = record.get("dataset_origin")
    if origin not in _DATASET_ORIGINS:
        raise ExperimentError("dataset_origin must be an explicit SYNTHETIC or MARKET value")
    return origin


def is_synthetic_record(record: dict[str, Any]) -> bool:
    """Report the explicit origin without using identifiers or config booleans."""
    return dataset_origin(record) == "SYNTHETIC"


def assert_dataset_origin(record: dict[str, Any], manifest: object) -> None:
    """Require the record origin to match the dataset manifest declaration."""
    origin = dataset_origin(record)
    declared = (manifest.get("dataset_origin") if isinstance(manifest, Mapping)
                else getattr(manifest, "dataset_origin", None))
    if declared not in _DATASET_ORIGINS:
        raise ExperimentError("dataset manifest dataset_origin must be SYNTHETIC or MARKET")
    if declared != origin:
        raise ExperimentError(
            f"record dataset_origin {origin} differs from dataset manifest {declared}"
        )


def _validate_bar_specification(record: dict[str, Any], *, synthetic: bool) -> int | str:
    """Validate bar family, fixed interval, timezone and timestamp anchor."""
    bar_kind = record.get("bar_kind")
    if bar_kind not in {"daily", "minute", "tick", "event", "other"}:
        raise ExperimentError("unknown bar_kind")
    if record.get("bar_timezone") != record.get("calendar_timezone"):
        raise ExperimentError("bar_timezone must match calendar_timezone")
    if record.get("bar_anchor") not in _TIMESTAMP_SEMANTICS:
        raise ExperimentError("unknown bar_anchor")
    if record.get("bar_anchor") != record.get("timestamp_semantics"):
        raise ExperimentError("bar_anchor must match timestamp_semantics")

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
        return NOT_APPLICABLE_INTERVAL
    if type(interval) is not int or interval <= 0:
        raise ExperimentError("bar_interval_ns must be positive or explicitly not_applicable")
    if bar_kind == "minute" and not (
        _MIN_MINUTE_INTERVAL_NS <= interval <= _MAX_MINUTE_INTERVAL_NS
    ):
        raise ExperimentError("minute bar_interval_ns is outside the minute sampling range")
    if bar_kind == "daily" and interval < _MIN_DAILY_INTERVAL_NS:
        raise ExperimentError("daily bar_interval_ns is shorter than an exchange trading day")
    return interval


def validate_bar_cadence(record: dict[str, Any], timestamps_ns: Sequence[int], *,
                         statuses: Mapping[int, str] | Sequence[dict[str, Any]] | None = None
                         ) -> None:
    """Reject bars whose ordered timestamps disagree with the registered schedule.

    ``timestamps_ns`` must be strictly increasing UTC nanosecond timestamps in
    the exact order supplied to a search. ``statuses`` may cover scheduled bars
    that have no price observation. The function is pure and mutates neither the
    record nor the timestamps.
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

    origin = dataset_origin(record)
    session_definition = record.get("session_definition")
    if not isinstance(session_definition, str) or not session_definition.strip():
        raise ExperimentError("registered session_definition is missing")
    interval = _validate_bar_specification(record, synthetic=origin == "SYNTHETIC")
    artifact = resolve_calendar(record)
    if interval == NOT_APPLICABLE_INTERVAL:
        return

    expected = set(expected_timestamps(artifact, timestamps_ns[0], timestamps_ns[-1]))
    observed = set(timestamps_ns)
    unexpected = sorted(observed - expected)
    if unexpected:
        raise ExperimentError(
            f"unexpected bar timestamp {unexpected[0]} ns outside the calendar schedule"
        )
    covered = {} if statuses is None else validate_status_observations(statuses)
    absent = sorted(timestamp for timestamp in expected - observed if timestamp not in covered)
    if absent:
        raise ExperimentError(
            f"missing expected bar at {absent[0]} ns inside an active session"
        )


__all__ = [
    "assert_dataset_origin",
    "dataset_origin",
    "is_synthetic_record",
    "validate_bar_cadence",
]
