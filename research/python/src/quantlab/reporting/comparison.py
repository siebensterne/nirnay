"""Byte-stable comparison of two frozen, terminal research experiments."""
from __future__ import annotations

from typing import Any

from ..experiments.schema import (ExperimentError, content_hash,
                                   validate_complete_trial_accounting,
                                   validate_registered_experiment)
from ..experiments.store import ExperimentRegistry
from ..validation.leakage_detection import assert_no_holdout_leakage


class ComparisonError(ExperimentError):
    """Raised when two records cannot be compared under the research contract."""


RESEARCH_WARNING = (
    "Research screening only. Comparison does not certify alpha, profitability, "
    "data rights, entitlement or live readiness."
)


def freeze_for_comparison(record: dict[str, Any]) -> dict[str, Any]:
    """Attach a self-digest to a frozen record for file-based comparison."""
    validate_registered_experiment(record)
    frozen = dict(record)
    frozen.pop("record_sha256", None)
    frozen["record_sha256"] = content_hash(frozen)
    return frozen


def _assert_frozen(record: dict[str, Any]) -> None:
    expected = record.get("record_sha256")
    if not isinstance(expected, str):
        raise ComparisonError("comparison requires a record_sha256 digest")
    body = {key: value for key, value in record.items() if key != "record_sha256"}
    if content_hash(body) != expected:
        raise ComparisonError("comparison record is tampered: record_sha256 mismatch")


def _summary(record: dict[str, Any]) -> dict[str, Any]:
    results = record["results"]
    diagnostics = record["diagnostics"]
    return {
        "experiment_id": record["experiment_id"],
        "status": record["status"],
        "dataset_manifest_sha256": record["dataset_manifest_sha256"],
        "dataset_version": record["dataset_version"],
        "feature_version": record["feature_version"],
        "strategy_code_hash": record["strategy_code_hash"],
        "holdout_key": record["holdout_key"],
        "holdout_use": record.get("holdout_use", "selection"),
        "attempted_configs": len(record["attempted_configs"]),
        "selected_config": results.get("selected"),
        "holdout_net_mean": results.get("untouched_holdout_net_mean"),
        "report_sha256": results.get("report_sha256"),
        "cscv_pbo": diagnostics.get("cscv_pbo", {}).get("pbo_estimate"),
    }


def compare_experiments(left: dict[str, Any], right: dict[str, Any], *,
                        registry: ExperimentRegistry | None = None,
                        trusted_public_keys: dict[str, str] | None = None,
                        dataset_dir: str | None = None) -> dict[str, Any]:
    """Compare two frozen terminal records, rejecting unsafe comparisons."""
    integrity: dict[str, Any] = {"status": "record_digests_verified", "signature_verified": False}
    if registry is not None:
        verified = registry.verify_integrity(trusted_public_keys=trusted_public_keys,
                                             dataset_dir=dataset_dir)
        integrity = {"status": verified["status"],
                     "signature_verified": verified["signature_verified"]}
        resolved = []
        for record in (left, right):
            identifier = record if isinstance(record, str) else record["experiment_id"]
            stored = registry.record(identifier, terminal_only=True)
            if stored is None:
                raise ComparisonError(f"experiment {identifier} has no terminal record")
            resolved.append(stored.body)
        left, right = resolved
    else:
        _assert_frozen(left)
        _assert_frozen(right)
    for record in (left, right):
        if not isinstance(record, dict):
            raise ComparisonError("comparison inputs must be objects")
        validate_registered_experiment(record)
        if record["status"] not in {"completed", "candidate", "rejected"}:
            raise ComparisonError("only terminal experiments can be compared")
        validate_complete_trial_accounting(record)
        assert_no_holdout_leakage(record)
    if left["dataset_manifest_sha256"] != right["dataset_manifest_sha256"]:
        raise ComparisonError("experiments bind different dataset manifest digests")
    if left["dataset_lineage"] != right["dataset_lineage"]:
        raise ComparisonError("experiments bind different dataset lineage digests")
    if left["dataset_version"] != right["dataset_version"]:
        raise ComparisonError("experiments bind different dataset versions")
    if left["holdout_key"] == right["holdout_key"] and all(
        record.get("holdout_use", "selection") == "selection" for record in (left, right)
    ):
        raise ComparisonError("the same holdout was used for selection in both experiments")

    summary_left, summary_right = _summary(left), _summary(right)
    comparable_fields = (
        "feature_version", "strategy_code_hash", "holdout_key", "attempted_configs",
        "selected_config", "holdout_net_mean", "report_sha256", "cscv_pbo",
    )
    differences = sorted(
        f"{field}: {summary_left[field]!r} != {summary_right[field]!r}"
        for field in comparable_fields
        if summary_left[field] != summary_right[field]
    )
    return {"status": "comparable_research_records", "warning": RESEARCH_WARNING,
            "left": summary_left, "right": summary_right,
            "differences": differences, "integrity": integrity}
