# SPDX-License-Identifier: Apache-2.0
"""Explicit observation-coverage statuses for scheduled bars.

A scheduled bar can be absent because an instrument did not trade, was not
listed, was suspended or halted, or because the market was closed. Those cases
are declared as statuses; a missing price bar is never turned into a zero price
or a forward-filled observation. Status records carry ``price=None`` and an
explicit ``fabricated=False`` marker so downstream statistics cannot mistake
them for measurements.
"""
from __future__ import annotations

from collections.abc import Mapping, Sequence
from typing import Any

from .schema import ExperimentError

OBSERVATION_STATUSES: tuple[str, ...] = (
    "no_trade",
    "not_listed",
    "suspended",
    "halted",
    "market_closed",
    "missing",
)


def explicit_status_observation(timestamp_ns: int, status: str, *,
                                instrument_id: str = "") -> dict[str, Any]:
    """Create a status-only observation at a scheduled timestamp."""
    if type(timestamp_ns) is not int or timestamp_ns <= 0:
        raise ExperimentError("status observation needs a positive integer timestamp_ns")
    if status not in OBSERVATION_STATUSES:
        raise ExperimentError(f"unknown observation status: {status!r}")
    if not isinstance(instrument_id, str):
        raise ExperimentError("status observation instrument_id must be a string")
    return {
        "timestamp_ns": timestamp_ns,
        "status": status,
        "instrument_id": instrument_id,
        "price": None,
        "bar": None,
        "fabricated": False,
    }


def validate_status_observations(observations: Mapping[int, str] | Sequence[dict[str, Any]]
                                 ) -> dict[int, str]:
    """Validate explicit statuses and map them by scheduled timestamp.

    Both a ``{timestamp_ns: status}`` mapping and a sequence of status records
    are accepted. Any record carrying a numeric price or a truthy
    ``fabricated`` flag is rejected: absence must remain absence.
    """
    result: dict[int, str] = {}
    if isinstance(observations, Mapping):
        items: Sequence[tuple[Any, Any]] = list(observations.items())
    elif isinstance(observations, Sequence) and not isinstance(observations, (str, bytes)):
        for observation in observations:
            if not isinstance(observation, dict):
                raise ExperimentError("status observation must be an object")
            if "price" in observation and observation["price"] is not None:
                raise ExperimentError("a status observation must not carry a price")
            if observation.get("fabricated") is True:
                raise ExperimentError("a status observation must not be marked fabricated")
        items = [(observation.get("timestamp_ns"), observation.get("status"))
                 for observation in observations]
    else:
        raise ExperimentError("status observations must be a mapping or a sequence")
    for timestamp, status in items:
        if type(timestamp) is not int or timestamp <= 0:
            raise ExperimentError("status timestamps must be positive integers")
        if status not in OBSERVATION_STATUSES:
            raise ExperimentError(f"unknown observation status: {status!r}")
        if timestamp in result and result[timestamp] != status:
            raise ExperimentError("conflicting statuses for one scheduled timestamp")
        result[timestamp] = status
    return result


__all__ = [
    "OBSERVATION_STATUSES",
    "explicit_status_observation",
    "validate_status_observations",
]
