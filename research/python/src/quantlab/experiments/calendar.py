# SPDX-License-Identifier: Apache-2.0
"""Checked-in calendar artifacts and expected bar schedules.

Calendar reference data is supplied by the caller as a JSON artifact.
No database, provider service, network
call or clock is used. An artifact declares its own identity, digest inputs,
bar family, session windows, auction treatment, scheduled breaks, early closes
and holidays. ``resolve_calendar`` fails closed on an unknown identifier, a
wrong digest, a conflicting declaration or a dataset-origin mismatch.

``expected_timestamps`` produces the scheduled UTC nanosecond observation grid
for the requested inclusive range. Calendar gaps (weekends, holidays, auctions,
breaks and overnight windows) are absent from that grid rather than filled.
"""
from __future__ import annotations

import json
from collections.abc import Iterator
from datetime import UTC, date, datetime, time, timedelta
from pathlib import Path
from typing import Any
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from .schema import NOT_APPLICABLE_INTERVAL, ExperimentError, content_hash

_FIXTURES = Path(__file__).resolve().parents[3] / "fixtures" / "calendars"
_EPOCH = datetime(1970, 1, 1, tzinfo=UTC)

CALENDAR_ARTIFACTS: dict[str, str] = {
    "synthetic:fixed-1m-v1": "synthetic-fixed-1m-v1.json",
    "synthetic:fixed-5m-v1": "synthetic-fixed-5m-v1.json",
    "synthetic:fixed-15m-v1": "synthetic-fixed-15m-v1.json",
    "synthetic:fixed-1d-v1": "synthetic-fixed-1d-v1.json",
    "synthetic:event-sequence-v1": "synthetic-event-sequence-v1.json",
    "market:fixture-daily-v1": "market-fixture-daily-v1.json",
    "market:fixture-minute-v1": "market-fixture-minute-v1.json",
    "market:fixture-overnight-v1": "market-fixture-overnight-v1.json",
}


def artifact_path(calendar_id: str) -> Path:
    """Return the checked-in artifact path for a registered calendar id."""
    if not isinstance(calendar_id, str) or calendar_id not in CALENDAR_ARTIFACTS:
        raise ExperimentError(f"unknown calendar_id: {calendar_id!r}")
    path = _FIXTURES / CALENDAR_ARTIFACTS[calendar_id]
    if path.is_symlink() or not path.is_file():
        raise ExperimentError(f"calendar artifact is missing or redirected: {calendar_id}")
    return path


def load_artifact(calendar_id: str) -> dict[str, Any]:
    """Load one checked-in calendar artifact and verify its self-identity."""
    try:
        artifact = json.loads(artifact_path(calendar_id).read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise ExperimentError(f"calendar artifact is unreadable: {calendar_id}") from exc
    if not isinstance(artifact, dict) or artifact.get("calendar_id") != calendar_id:
        raise ExperimentError(f"calendar artifact identity mismatch: {calendar_id}")
    return artifact


def calendar_digest(calendar_id: str) -> str:
    """Return the canonical SHA-256 digest of a checked-in calendar artifact."""
    return content_hash(load_artifact(calendar_id))


def calendar_binding(calendar_id: str) -> dict[str, str]:
    """Return the immutable calendar-reference fields for an experiment record."""
    artifact = load_artifact(calendar_id)
    return {
        "calendar_id": calendar_id,
        "calendar_source": artifact["calendar_source"],
        "calendar_timezone": artifact["calendar_timezone"],
        "calendar_sha256": calendar_digest(calendar_id),
    }


def _timezone(name: object) -> ZoneInfo:
    if not isinstance(name, str) or not name.strip():
        raise ExperimentError("calendar timezone must be an IANA name or UTC")
    try:
        return ZoneInfo(name)
    except (ValueError, ZoneInfoNotFoundError) as exc:
        raise ExperimentError(f"unknown calendar timezone: {name}") from exc


def resolve_calendar(record: dict[str, Any]) -> dict[str, Any]:
    """Resolve and bind a record to one checked-in calendar artifact."""
    calendar_id = record.get("calendar_id")
    if not isinstance(calendar_id, str) or not calendar_id.strip():
        raise ExperimentError("registered calendar_id is missing")
    artifact = load_artifact(calendar_id)
    if record.get("calendar_sha256") != calendar_digest(calendar_id):
        raise ExperimentError("calendar_sha256 differs from the checked-in calendar digest")
    if record.get("calendar_source") != artifact.get("calendar_source"):
        raise ExperimentError("calendar_source differs from the calendar artifact")
    if record.get("calendar_timezone") != artifact.get("calendar_timezone"):
        raise ExperimentError("calendar_timezone differs from the calendar artifact")
    origin = record.get("dataset_origin")
    if artifact.get("synthetic") and origin != "SYNTHETIC":
        raise ExperimentError("market dataset cannot use a synthetic calendar")
    if not artifact.get("synthetic") and origin != "MARKET":
        raise ExperimentError("synthetic dataset requires a synthetic calendar")
    if record.get("bar_kind") != artifact.get("bar_kind"):
        raise ExperimentError("bar_kind differs from the calendar bar family")
    if record.get("bar_interval_ns") != artifact.get("bar_interval_ns"):
        raise ExperimentError(
            f"bar interval {record.get('bar_interval_ns')!r} disagrees with calendar "
            f"bar_interval_ns {artifact.get('bar_interval_ns')!r}"
        )
    if record.get("bar_anchor") != artifact.get("anchor"):
        raise ExperimentError("bar_anchor differs from the calendar anchor")
    return artifact


def _local_datetime(day: date, clock: str, zone: ZoneInfo) -> datetime:
    hour, minute = (int(part) for part in clock.split(":"))
    return datetime.combine(day, time(hour, minute), tzinfo=zone)


def _to_ns(moment: datetime) -> int:
    utc = moment.astimezone(UTC)
    delta = utc - _EPOCH
    return (delta.days * 86400 + delta.seconds) * 1_000_000_000 + delta.microseconds * 1000


def _to_local_date(timestamp_ns: int, zone: ZoneInfo) -> date:
    utc = datetime.fromtimestamp(timestamp_ns // 1_000_000_000, tz=UTC)
    return utc.astimezone(zone).date()


def _session_windows(
    artifact: dict[str, Any], day: date, zone: ZoneInfo
) -> Iterator[tuple[int, int, str]]:
    """Yield active session windows as (start_ns, end_ns, session) tuples."""
    if day.isoformat() in set(artifact["holidays"]):
        return
    early = {item["date"]: item["end"] for item in artifact["early_closes"]}
    for session in artifact["sessions"]:
        if day.weekday() not in session["weekdays"]:
            continue
        start_clock = session["start"]
        end_clock = early.get(day.isoformat(), session["end"])
        overnight = time.fromisoformat(end_clock) <= time.fromisoformat(start_clock)
        end_day = day + timedelta(days=1) if overnight else day
        start_ns = _to_ns(_local_datetime(day, start_clock, zone))
        end_ns = _to_ns(_local_datetime(end_day, end_clock, zone))
        yield start_ns, end_ns, session


def _exclusions(artifact: dict[str, Any], day: date, zone: ZoneInfo) -> list[tuple[int, int]]:
    windows: list[tuple[int, int]] = []
    auction = artifact.get("auction")
    if isinstance(auction, dict) and auction.get("treatment") == "exclude":
        windows.append((_to_ns(_local_datetime(day, auction["start"], zone)),
                        _to_ns(_local_datetime(day, auction["end"], zone))))
    for item in artifact["scheduled_breaks"]:
        if day.weekday() not in item["weekdays"] or item["treatment"] != "exclude":
            continue
        windows.append((_to_ns(_local_datetime(day, item["start"], zone)),
                        _to_ns(_local_datetime(day, item["end"], zone))))
    return windows


def expected_timestamps(artifact: dict[str, Any], start_ns: int, end_ns: int) -> list[int]:
    """Return scheduled observation timestamps in the inclusive UTC-ns range."""
    if type(start_ns) is not int or type(end_ns) is not int or start_ns > end_ns:
        raise ExperimentError("invalid expected schedule range")
    mode = artifact.get("mode")
    interval = artifact.get("bar_interval_ns")
    if mode == "continuous":
        if interval == NOT_APPLICABLE_INTERVAL:
            return []
        if type(interval) is not int or interval <= 0:
            raise ExperimentError("continuous calendar needs a positive bar_interval_ns")
        return list(range(start_ns, end_ns + 1, interval))
    if mode != "session":
        raise ExperimentError("unknown calendar mode")
    if type(interval) is not int or interval <= 0:
        raise ExperimentError("session calendar needs a positive bar_interval_ns")
    zone = _timezone(artifact["calendar_timezone"])
    first_day = _to_local_date(start_ns, zone) - timedelta(days=1)
    last_day = _to_local_date(end_ns, zone) + timedelta(days=1)
    values: set[int] = set()
    day = first_day
    while day <= last_day:
        for start, end, _ in _session_windows(artifact, day, zone):
            if artifact["bar_kind"] == "daily":
                values.add(start if artifact["anchor"] == "bar_open" else end)
                continue
            excluded = _exclusions(artifact, day, zone)
            cursor = start
            while cursor <= end:
                if all(not (window_start <= cursor < window_end)
                       for window_start, window_end in excluded):
                    values.add(cursor)
                cursor += interval
        day += timedelta(days=1)
    return sorted(value for value in values if start_ns <= value <= end_ns)


__all__ = [
    "CALENDAR_ARTIFACTS",
    "artifact_path",
    "calendar_binding",
    "calendar_digest",
    "expected_timestamps",
    "load_artifact",
    "resolve_calendar",
]
