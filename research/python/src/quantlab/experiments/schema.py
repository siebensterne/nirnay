"""Closed-set contract for pre-registered research experiment records.

The original eleven-field contract is preserved verbatim so existing callers
remain valid. The registry requires a larger declaration set: immutable dataset
lineage, bar cadence and annualisation declarations, a declared search space,
complete trial accounting, disjoint validation and holdout periods,
cost/execution assumptions and terminal results. All fields are encoded with
strict canonical JSON and validated before any digest is taken.

``dataset_point_in_time`` is a narrow boolean. It never certifies universe or
corporate-action correctness by itself: the separate ``point_in_time_basis``
declaration states what it certifies. ``chronological_only`` certifies ordered
timestamps only; ``universe_and_corporate_actions`` additionally requires dated
universe membership and corporate-action revision lineage. See
``engineering/research/dataset-acceptance-contract.md``.
"""
from __future__ import annotations

import hashlib
import json
import math
from dataclasses import dataclass
from datetime import datetime, timedelta
from pathlib import Path
from typing import Any
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError


class ExperimentError(ValueError):
    """Raised when an experiment is incomplete or internally inconsistent."""


class HoldoutReuseError(ExperimentError):
    """Raised when a selection holdout has already been consumed."""


_BASE_REQUIRED_FIELDS = {
    "experiment_id",
    "hypothesis",
    "economic_rationale",
    "dataset_ref",
    "dataset_manifest_sha256",
    "dataset_point_in_time",
    "code_revision",
    "config",
    "status",
    "registered_at_utc",
    "falsification_rule",
}
_DECLARATION_FIELDS = {
    "dataset_id",
    "dataset_origin",
    "dataset_version",
    "dataset_lineage",
    "universe_id",
    "feature_version",
    "strategy_code_hash",
    "search_space",
    "attempted_configs",
    "validation_period",
    "holdout_period",
    "holdout_key",
    "cost_model",
    "execution_assumptions",
    "bar_kind",
    "bar_interval_ns",
    "bar_timezone",
    "bar_anchor",
    "timestamp_semantics",
    "calendar_id",
    "calendar_source",
    "calendar_timezone",
    "calendar_sha256",
    "session_definition",
    "annualization_basis",
    "periods_per_year",
    "point_in_time_basis",
}
_RESULT_FIELDS = {"results", "diagnostics", "result", "validation"}
_OPTIONAL_FIELDS = {"holdout_use", "replicates", "record_sha256",
                   "universe_version", "feature_digest"}
_ALLOWED_FIELDS = _BASE_REQUIRED_FIELDS | _DECLARATION_FIELDS | _RESULT_FIELDS | _OPTIONAL_FIELDS
_TERMINAL_STATUSES = {"completed", "candidate", "rejected"}
_VALID_STATUSES = {"registered", "running", *_TERMINAL_STATUSES}
_VALID_HOLDOUT_USES = {"selection", "verification_only"}
_LINEAGE_HASH_FIELDS = ("data_sha256", "input_sha256", "code_digest", "config_digest")
_LINEAGE_LIST_FIELDS = (
    "source_event_hashes",
    "source_frame_hashes",
    "listing_source_hashes",
    "corporate_action_source_hashes",
)
_PERIOD_KEYS = {
    frozenset({"start_index", "end_index"}),
    frozenset({"start_ns", "end_ns"}),
}

_BAR_KINDS = frozenset({"daily", "minute", "tick", "event", "other"})
_FIXED_INTERVAL_KINDS = frozenset({"daily", "minute"})
_NON_FIXED_INTERVAL_KINDS = frozenset({"tick", "event"})
_TIMESTAMP_SEMANTICS = frozenset({"bar_open", "bar_close", "observation_time"})
_DATASET_ORIGINS = frozenset({"SYNTHETIC", "MARKET"})
_CALENDAR_SOURCES = frozenset({"checked_in_fixture"})
_POINT_IN_TIME_BASES = frozenset(
    {"none", "chronological_only", "universe_and_corporate_actions"}
)
_EXECUTION_LATENCY_MODELS = frozenset(
    {"not_modelled", "bar_close_assumed", "event_level_modelled"}
)

# Explicit state for bar kinds without a fixed sampling interval. It is not a
# substitute for a positive integer on fixed-sampling daily/minute bars.
NOT_APPLICABLE_INTERVAL = "not_applicable"

# An explicit declared basis is the only way to choose an arbitrary positive
# annualisation factor; named bases carry a fixed period count and bar family.
SYNTHETIC_ANNUALIZATION_BASIS = "synthetic_declared"
_DAILY_INTERVAL_NS = 86_400_000_000_000
_MINUTE_INTERVAL_NS = 60_000_000_000
# A named basis fixes bar family, bar_interval_ns and periods_per_year together.
# NSE cash has 375 one-minute bars per trading day: 252 * 375 = 94_500.
_NAMED_ANNUALIZATION_BASES: dict[str, tuple[str, int, float]] = {
    "daily_252": ("daily", _DAILY_INTERVAL_NS, 252.0),
    "daily_365": ("daily", _DAILY_INTERVAL_NS, 365.0),
    "minute_nse_cash_94500": ("minute", _MINUTE_INTERVAL_NS, 94_500.0),
    "minute_nse_cash_5m_18900": ("minute", 300_000_000_000, 18_900.0),
    "minute_nse_cash_15m_6300": ("minute", 900_000_000_000, 6_300.0),
    "minute_continuous_525600": ("minute", _MINUTE_INTERVAL_NS, 525_600.0),
}


@dataclass(frozen=True, slots=True)
class HoldoutClaim:
    holdout_key: str
    claim_kind: str
    experiment_id: str
    registered_at_utc: str


@dataclass(frozen=True, slots=True)
class Receipt:
    experiment_id: str
    status: str
    sequence: int
    holdout_key: str
    record_hash: str
    previous_hash: str


def _validate_json_value(value: Any, path: str = "$") -> None:
    """Reject Python values that cannot be represented as strict JSON."""
    if value is None or isinstance(value, (str, bool, int)):
        return
    if isinstance(value, float):
        if not math.isfinite(value):
            raise ExperimentError(f"{path} must not contain NaN or infinity")
        return
    if isinstance(value, list):
        for index, item in enumerate(value):
            _validate_json_value(item, f"{path}[{index}]")
        return
    if isinstance(value, dict):
        for key, item in value.items():
            if not isinstance(key, str):
                raise ExperimentError(f"{path} contains a non-string object key")
            _validate_json_value(item, f"{path}.{key}")
        return
    raise ExperimentError(f"{path} contains a non-JSON value: {type(value).__name__}")


def canonical_json(value: Any) -> bytes:
    """Encode strict JSON with a stable key order and no non-finite numbers."""
    _validate_json_value(value)
    try:
        encoded = json.dumps(
            value,
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=False,
            allow_nan=False,
        )
    except (TypeError, ValueError) as exc:
        raise ExperimentError(f"value cannot be canonically encoded: {exc}") from exc
    return encoded.encode("utf-8")


def content_hash(value: Any) -> str:
    """Return the SHA-256 digest of a canonical JSON representation."""
    return hashlib.sha256(canonical_json(value)).hexdigest()


def _is_sha256(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 64 and all(
        char in "0123456789abcdef" for char in value
    )


def _require_sha256(value: Any, field: str) -> None:
    if not _is_sha256(value):
        raise ExperimentError(f"{field} must be a lowercase SHA-256 digest")


def _require_nonempty_string(value: Any, field: str) -> None:
    if not isinstance(value, str) or not value.strip():
        raise ExperimentError(f"{field} must be a non-empty string")


def _require_versioned_id(value: object, field: str) -> None:
    """Require a non-empty ``namespace:version`` identifier without whitespace."""
    _require_nonempty_string(value, field)
    parts = value.split(":")
    if len(parts) < 2 or any(not part or any(char.isspace() for char in part) for part in parts):
        raise ExperimentError(f"{field} must be a versioned namespace:id string")


def _require_positive_int(value: object, field: str) -> None:
    if type(value) is not int or value <= 0:
        raise ExperimentError(f"{field} must be a positive integer")


def _require_positive_number(value: object, field: str) -> None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ExperimentError(f"{field} must be a positive finite number")
    if not math.isfinite(float(value)) or value <= 0:
        raise ExperimentError(f"{field} must be a positive finite number")


def _validate_cadence_and_annualization(record: dict[str, Any]) -> None:
    """Bind bar kind, timestamp semantics, calendar and annualisation in the record."""
    bar_kind = record["bar_kind"]
    if bar_kind not in _BAR_KINDS:
        raise ExperimentError("unknown bar_kind")

    interval = record["bar_interval_ns"]
    if bar_kind in _FIXED_INTERVAL_KINDS:
        _require_positive_int(interval, "bar_interval_ns")
    elif bar_kind in _NON_FIXED_INTERVAL_KINDS:
        if interval != NOT_APPLICABLE_INTERVAL:
            raise ExperimentError("tick and event bars require an explicit not_applicable interval")
    elif interval != NOT_APPLICABLE_INTERVAL:
        _require_positive_int(interval, "bar_interval_ns")

    if record["timestamp_semantics"] not in _TIMESTAMP_SEMANTICS:
        raise ExperimentError("unknown timestamp_semantics")
    _require_versioned_id(record["calendar_id"], "calendar_id")
    _require_nonempty_string(record["session_definition"], "session_definition")

    basis = record["annualization_basis"]
    _require_nonempty_string(basis, "annualization_basis")
    periods = record["periods_per_year"]
    _require_positive_number(periods, "periods_per_year")
    if basis == SYNTHETIC_ANNUALIZATION_BASIS:
        return
    declared = _NAMED_ANNUALIZATION_BASES.get(basis)
    if declared is None:
        raise ExperimentError(
            "unknown annualization_basis; use synthetic_declared for explicit periods"
        )
    expected_kind, expected_interval, expected_periods = declared
    if bar_kind != expected_kind:
        raise ExperimentError(f"annualization_basis {basis} requires {expected_kind} bars")
    if interval != expected_interval:
        raise ExperimentError(
            f"bar_interval_ns differs from the declared basis {basis} ({expected_interval} ns)"
        )
    if float(periods) != expected_periods:
        raise ExperimentError(f"periods_per_year differs from the declared basis {basis}")


def _validate_timezone(value: object, field: str) -> None:
    _require_nonempty_string(value, field)
    try:
        ZoneInfo(value)
    except (ValueError, ZoneInfoNotFoundError) as exc:
        raise ExperimentError(f"{field} must be an IANA timezone name or UTC") from exc


def _validate_dataset_and_bar_declaration(record: dict[str, Any]) -> None:
    """Validate explicit origin, bar specification and calendar reference fields."""
    if record["dataset_origin"] not in _DATASET_ORIGINS:
        raise ExperimentError("dataset_origin must be SYNTHETIC or MARKET")
    _validate_timezone(record["bar_timezone"], "bar_timezone")
    anchor = record["bar_anchor"]
    if anchor not in _TIMESTAMP_SEMANTICS:
        raise ExperimentError("unknown bar_anchor")
    if anchor != record["timestamp_semantics"]:
        raise ExperimentError("bar_anchor must match timestamp_semantics")
    if record["calendar_source"] not in _CALENDAR_SOURCES:
        raise ExperimentError("unknown calendar_source")
    _validate_timezone(record["calendar_timezone"], "calendar_timezone")
    _require_sha256(record["calendar_sha256"], "calendar_sha256")


def _validate_point_in_time_declaration(record: dict[str, Any]) -> None:
    """State exactly what the boolean ``dataset_point_in_time`` certifies."""
    basis = record["point_in_time_basis"]
    if basis not in _POINT_IN_TIME_BASES:
        raise ExperimentError("unknown point_in_time_basis")
    declared = record["dataset_point_in_time"]
    if basis == "none" and declared is True:
        raise ExperimentError("point_in_time_basis none cannot certify point-in-time data")
    if basis == "universe_and_corporate_actions":
        if declared is not True:
            raise ExperimentError("full point-in-time basis requires dataset_point_in_time")
        if not record.get("universe_version"):
            raise ExperimentError("full point-in-time basis requires universe_version")
        lineage = record["dataset_lineage"]
        for field in ("listing_source_hashes", "corporate_action_source_hashes"):
            if not lineage[field]:
                raise ExperimentError(f"full point-in-time basis requires {field}")


def _validate_execution_assumptions(value: object) -> None:
    """Require an explicit latency model; refuse bare numeric latency claims."""
    if not isinstance(value, dict) or not value:
        raise ExperimentError("execution_assumptions must be a non-empty object")
    model = value.get("execution_latency_model")
    if model not in _EXECUTION_LATENCY_MODELS:
        raise ExperimentError("execution_latency_model must be an explicit declared state")
    numeric_latencies = [
        key for key, item in value.items()
        if key.endswith("_latency_ns") and isinstance(item, (int, float))
        and not isinstance(item, bool)
    ]
    if numeric_latencies and model != "event_level_modelled":
        raise ExperimentError(
            f"numeric latency {numeric_latencies[0]} requires event_level_modelled"
        )
    if model != "event_level_modelled" and "order_latency_ns" in value:
        raise ExperimentError(
            "order_latency_ns requires execution_latency_model event_level_modelled"
        )


def holdout_key(dataset_version: str, universe_id: str, feature_version: str,
                validation_period: dict[str, Any],
                holdout_period: dict[str, Any],
                universe_version: str | None = None,
                feature_digest: str | None = None) -> str:
    """Derive the identity of a holdout from its immutable declaring inputs."""
    body: dict[str, Any] = {
        "dataset_version": dataset_version,
        "universe_id": universe_id,
        "feature_version": feature_version,
        "validation_period": validation_period,
        "holdout_period": holdout_period,
    }
    if universe_version is not None:
        body["universe_version"] = universe_version
    if feature_digest is not None:
        body["feature_digest"] = feature_digest
    return content_hash(body)


def _validate_period(period: Any, field: str) -> None:
    if not isinstance(period, dict) or frozenset(period) not in _PERIOD_KEYS:
        raise ExperimentError(f"{field} must declare exactly start/end index or ns bounds")
    start_key, end_key = _period_keys(period)
    start, end = period[start_key], period[end_key]
    if type(start) is not int or type(end) is not int or start < 0 or end <= start:
        raise ExperimentError(f"{field} bounds must be non-negative integers with start < end")


def _period_keys(period: dict[str, Any]) -> tuple[str, str]:
    if "start_index" in period:
        return "start_index", "end_index"
    return "start_ns", "end_ns"


def _period_bounds(period: dict[str, Any]) -> tuple[int, int]:
    start_key, end_key = _period_keys(period)
    return period[start_key], period[end_key]


def validate_period_geometry(record: dict[str, Any]) -> None:
    """Require disjoint, chronologically ordered validation and holdout periods."""
    validation = record["validation_period"]
    holdout = record["holdout_period"]
    _validate_period(validation, "validation_period")
    _validate_period(holdout, "holdout_period")
    if frozenset(validation) != frozenset(holdout):
        raise ExperimentError("validation_period and holdout_period must use the same time unit")
    search_space = record["search_space"]
    embargo = search_space.get("embargo_observations", 1)
    if type(embargo) is not int or embargo < 0:
        raise ExperimentError("search_space.embargo_observations must be a non-negative integer")
    validation_start, validation_end = _period_bounds(validation)
    holdout_start, _ = _period_bounds(holdout)
    if holdout_start < validation_end + embargo:
        raise ExperimentError("holdout_period overlaps or is not embargoed from validation_period")
    train_period = search_space.get("train_period")
    if train_period is not None:
        _validate_period(train_period, "search_space.train_period")
        if frozenset(train_period) != frozenset(validation):
            raise ExperimentError("train_period must use the same time unit as validation_period")
        _, train_end = _period_bounds(train_period)
        if validation_start < train_end + embargo:
            raise ExperimentError("validation_period overlaps or is not embargoed from training")


def search_space_digest(search_space: dict[str, Any]) -> str:
    """Digest a search space declaration, excluding its self-referential field."""
    if not isinstance(search_space, dict):
        raise ExperimentError("search_space must be an object")
    body = {key: value for key, value in search_space.items() if key != "search_space_sha256"}
    return content_hash(body)


def build_search_space(family: str, configs: list[dict[str, Any]], *,
                       train_period: dict[str, Any] | None = None,
                       embargo_observations: int = 1,
                       **extra: Any) -> dict[str, Any]:
    """Create a canonical search-space declaration with a recomputable digest."""
    _require_nonempty_string(family, "family")
    if not isinstance(configs, list) or not configs:
        raise ExperimentError("search_space configs must be a non-empty list")
    if type(embargo_observations) is not int or embargo_observations < 0:
        raise ExperimentError("embargo_observations must be a non-negative integer")
    declaration: dict[str, Any] = {
        "family": family,
        "configs": configs,
        "expected_trials": len(configs),
        "embargo_observations": embargo_observations,
        **extra,
    }
    if train_period is not None:
        declaration["train_period"] = train_period
    declaration["search_space_sha256"] = search_space_digest(declaration)
    return declaration


def make_attempt(config: dict[str, Any], outcome: Any) -> dict[str, Any]:
    """Record one attempted configuration, winner or loser, with its digest."""
    if not isinstance(config, dict):
        raise ExperimentError("attempted config must be an object")
    return {"config": config, "config_sha256": content_hash(config), "outcome": outcome}


def _validate_attempted_configs(attempts: Any, field: str = "attempted_configs") -> None:
    if not isinstance(attempts, list):
        raise ExperimentError(f"{field} must be a list")
    for index, attempt in enumerate(attempts):
        if not isinstance(attempt, dict) or set(attempt) != {"config", "config_sha256", "outcome"}:
            raise ExperimentError(f"{field}[{index}] has an invalid shape")
        if not isinstance(attempt["config"], dict):
            raise ExperimentError(f"{field}[{index}].config must be an object")
        if content_hash(attempt["config"]) != attempt["config_sha256"]:
            raise ExperimentError(f"{field}[{index}] config digest mismatch")


def recompute_report_hash(report: dict[str, Any]) -> str:
    """Recompute the search report digest over the report minus the digest field."""
    if not isinstance(report, dict):
        raise ExperimentError("results must be an object")
    body = {key: value for key, value in report.items() if key != "report_sha256"}
    return content_hash(body)


def diagnostic_holdout_flags(diagnostics: Any) -> list[bool]:
    """Collect every explicit claim that a diagnostic consumed the final holdout."""
    flags: list[bool] = []
    if not isinstance(diagnostics, dict):
        return flags
    for key, value in diagnostics.items():
        if key == "uses_final_holdout" and isinstance(value, bool):
            flags.append(value)
        if isinstance(value, dict):
            flags.extend(diagnostic_holdout_flags(value))
    return flags


def validate_experiment(record: dict[str, Any]) -> None:
    """Validate the original eleven-field pre-registration contract."""
    if not isinstance(record, dict):
        raise ExperimentError("experiment record must be an object")

    missing = sorted(_BASE_REQUIRED_FIELDS - record.keys())
    if missing:
        raise ExperimentError(f"missing required fields: {', '.join(missing)}")
    if any(not isinstance(key, str) for key in record):
        raise ExperimentError("experiment record keys must be strings")
    unknown = sorted(record.keys() - _ALLOWED_FIELDS)
    if unknown:
        raise ExperimentError(f"unknown fields: {', '.join(unknown)}")

    for field in (
        "experiment_id",
        "hypothesis",
        "economic_rationale",
        "dataset_ref",
        "code_revision",
        "falsification_rule",
    ):
        _require_nonempty_string(record[field], field)

    if not isinstance(record["dataset_point_in_time"], bool):
        raise ExperimentError("dataset_point_in_time must be a boolean")
    _require_sha256(record["dataset_manifest_sha256"], "dataset_manifest_sha256")

    status = record["status"]
    if not isinstance(status, str) or status not in _VALID_STATUSES:
        raise ExperimentError("unknown experiment status")

    if not isinstance(record["config"], dict):
        raise ExperimentError("config must be an object")
    for optional_object in ("result", "validation"):
        if optional_object in record and not isinstance(record[optional_object], dict):
            raise ExperimentError(f"{optional_object} must be an object")

    timestamp = record["registered_at_utc"]
    if not isinstance(timestamp, str):
        raise ExperimentError("registered_at_utc must be an ISO-8601 UTC string")
    try:
        registered = datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
    except ValueError as exc:
        raise ExperimentError("registered_at_utc must be an ISO-8601 timestamp") from exc
    if registered.tzinfo is None or registered.utcoffset() is None:
        raise ExperimentError("registered_at_utc must include a timezone")
    if registered.utcoffset() != timedelta(0):
        raise ExperimentError("registered_at_utc must use UTC (Z or +00:00)")

    if status in _TERMINAL_STATUSES:
        if not isinstance(record.get("result"), dict):
            raise ExperimentError("terminal experiment status requires result object")
        if not isinstance(record.get("validation"), dict):
            raise ExperimentError("terminal experiment status requires validation object")

    if record["dataset_point_in_time"] is False and status == "candidate":
        raise ExperimentError("a dataset with known point-in-time leakage cannot be a candidate")

    canonical_json(record)


def validate_registered_experiment(record: dict[str, Any]) -> None:
    """Validate the full registry contract, including declaration and results."""
    validate_experiment(record)
    missing = sorted(_DECLARATION_FIELDS - record.keys())
    if missing:
        raise ExperimentError(f"missing registry fields: {', '.join(missing)}")

    _require_nonempty_string(record["dataset_id"], "dataset_id")
    version = record["dataset_version"]
    if not isinstance(version, str) or not version.startswith(record["dataset_id"] + ":"):
        raise ExperimentError("dataset_version must be dataset_id followed by a :hash16 suffix")
    if len(version.split(":")[-1]) != 16 or any(
        char not in "0123456789abcdef" for char in version.split(":")[-1]
    ):
        raise ExperimentError("dataset_version must carry a lowercase 16-hex version suffix")
    _require_nonempty_string(record["universe_id"], "universe_id")
    _require_nonempty_string(record["feature_version"], "feature_version")
    if "universe_version" in record:
        _require_nonempty_string(record["universe_version"], "universe_version")
    if "feature_digest" in record:
        _require_sha256(record["feature_digest"], "feature_digest")
    _require_sha256(record["strategy_code_hash"], "strategy_code_hash")

    lineage = record["dataset_lineage"]
    expected_lineage = set(_LINEAGE_HASH_FIELDS) | set(_LINEAGE_LIST_FIELDS)
    if not isinstance(lineage, dict) or set(lineage) != expected_lineage:
        raise ExperimentError("dataset_lineage must contain exactly the manifest digest fields")
    for field in _LINEAGE_HASH_FIELDS:
        _require_sha256(lineage[field], f"dataset_lineage.{field}")
    for field in _LINEAGE_LIST_FIELDS:
        if not isinstance(lineage[field], list):
            raise ExperimentError(f"dataset_lineage.{field} must be a list")
        for digest in lineage[field]:
            _require_sha256(digest, f"dataset_lineage.{field}")

    search_space = record["search_space"]
    if not isinstance(search_space, dict) or not isinstance(search_space.get("family"), str):
        raise ExperimentError("search_space must declare a family")
    if not search_space["family"].strip():
        raise ExperimentError("search_space family must be non-empty")
    _require_sha256(search_space.get("search_space_sha256"), "search_space_sha256")
    if search_space_digest(search_space) != search_space["search_space_sha256"]:
        raise ExperimentError("search_space digest mismatch")
    expected_trials = search_space.get("expected_trials")
    if type(expected_trials) is not int or expected_trials < 1:
        raise ExperimentError("search_space.expected_trials must be a positive integer")

    _validate_attempted_configs(record["attempted_configs"])
    validate_period_geometry(record)
    _require_sha256(record["holdout_key"], "holdout_key")
    expected_key = holdout_key(record["dataset_version"], record["universe_id"],
                               record["feature_version"], record["validation_period"],
                               record["holdout_period"], record.get("universe_version"),
                               record.get("feature_digest"))
    if record["holdout_key"] != expected_key:
        raise ExperimentError("holdout_key does not match its declaring inputs")

    cost_model = record["cost_model"]
    if not isinstance(cost_model, dict) or set(cost_model) != {"assumptions", "cost_model_sha256"}:
        raise ExperimentError("cost_model must declare assumptions and cost_model_sha256")
    if not isinstance(cost_model["assumptions"], dict):
        raise ExperimentError("cost_model.assumptions must be an object")
    if content_hash(cost_model["assumptions"]) != cost_model["cost_model_sha256"]:
        raise ExperimentError("cost_model digest mismatch")
    _validate_execution_assumptions(record["execution_assumptions"])
    _validate_cadence_and_annualization(record)
    _validate_dataset_and_bar_declaration(record)
    _validate_point_in_time_declaration(record)

    holdout_use = record.get("holdout_use", "selection")
    if holdout_use not in _VALID_HOLDOUT_USES:
        raise ExperimentError("holdout_use must be selection or verification_only")
    replicates = record.get("replicates")
    if holdout_use == "verification_only" and not _require_replicates(replicates):
        raise ExperimentError("verification_only claims must name replicates")
    if holdout_use == "selection" and replicates is not None:
        raise ExperimentError("selection claims must not name replicates")

    record_sha256 = record.get("record_sha256")
    if record_sha256 is not None:
        _require_sha256(record_sha256, "record_sha256")
        body = {key: value for key, value in record.items() if key != "record_sha256"}
        if content_hash(body) != record_sha256:
            raise ExperimentError("record_sha256 digest mismatch")

    if record["status"] in _TERMINAL_STATUSES:
        for field in ("results", "diagnostics"):
            if not isinstance(record.get(field), dict):
                raise ExperimentError(f"terminal experiment status requires {field} object")
        report = record["results"]
        if "report_sha256" in report and recompute_report_hash(report) != report["report_sha256"]:
            raise ExperimentError("results report_sha256 digest mismatch")
        if any(flag is True for flag in diagnostic_holdout_flags(record["diagnostics"])):
            raise ExperimentError("diagnostics may not use the final holdout")


def _require_replicates(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def validate_complete_trial_accounting(record: dict[str, Any]) -> None:
    """Require every attempted configuration to be persisted, including losers."""
    attempts = record["attempted_configs"]
    declared = record["search_space"]["expected_trials"]
    if len(attempts) != declared:
        raise ExperimentError("attempted_configs count differs from the declared search space")
    report_trials = record.get("results", {}).get("candidate_family_size")
    if report_trials is not None and report_trials != len(attempts):
        raise ExperimentError("report trial count exceeds persisted attempts")
    diagnostic = record.get("diagnostics", {}).get("deflated_sharpe_validation_only", {})
    if isinstance(diagnostic, dict) and diagnostic.get("trials") not in (None, len(attempts)):
        raise ExperimentError("DSR trial count differs from persisted attempts")


def freeze_experiment(record: dict[str, Any]) -> tuple[dict[str, Any], str]:
    """Validate and return a detached canonical record plus its content hash."""
    validate_experiment(record)
    frozen = json.loads(canonical_json(record))
    return frozen, content_hash(frozen)


def freeze_registered_experiment(record: dict[str, Any]) -> tuple[dict[str, Any], str]:
    """Validate the full registry contract and return a frozen canonical record."""
    validate_registered_experiment(record)
    frozen = json.loads(canonical_json(record))
    return frozen, content_hash(frozen)


def load_experiment(path: str | Path) -> dict[str, Any]:
    """Load and validate a frozen record from disk without modifying it."""
    try:
        record = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise ExperimentError(f"could not load experiment record: {exc}") from exc
    if not isinstance(record, dict):
        raise ExperimentError("experiment record must be a JSON object")
    validate_experiment(record)
    return record
