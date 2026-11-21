"""Per-experiment independent validation report (internal integrity only).

The CLI reads a frozen experiment record and its published dataset and emits a
deterministic JSON report whose digest recomputes from the report body. It
reuses the registry integrity verifier and the byte-stable comparison summary.
It explicitly does not certify data rights, source completeness, model
correctness, alpha, profitability, entitlement or live readiness.
"""
from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from .experiments.schema import ExperimentError, content_hash
from .experiments.store import ExperimentRegistry
from .experiments.verify import verify_chain, verify_dataset_binding
from .reporting.comparison import (RESEARCH_WARNING, ComparisonError, _assert_frozen,
                                   _summary, freeze_for_comparison)

INTEGRITY_WARNING = (
    "Internal integrity only. This report does not validate data rights, source "
    "completeness, model correctness, alpha, profitability or live readiness."
)


def _chain_head(registry: ExperimentRegistry) -> str:
    """Recompute the registry head from the stored append order."""
    return verify_chain([{
        "sequence": row.sequence,
        "body": row.body,
        "body_sha256": row.body_sha256,
        "previous_hash": row.previous_hash,
        "record_hash": row.record_hash,
    } for row in registry.records()])


def build_validation_report(record: dict[str, Any], dataset_dir: str | Path, *,
                            registry: ExperimentRegistry | None = None,
                            trusted_public_keys: dict[str, str] | None = None
                            ) -> dict[str, Any]:
    """Build a deterministic integrity-only validation report for one record."""
    if not isinstance(record, dict):
        raise ComparisonError("validation report requires a frozen experiment record")
    if registry is not None:
        identifier = record.get("experiment_id")
        if not isinstance(identifier, str):
            identifier = None
        if not identifier:
            raise ComparisonError("registry validation requires an experiment_id")
        stored = registry.record(identifier, terminal_only=True)
        if stored is None:
            raise ComparisonError(f"experiment {identifier} has no terminal record")
        source = stored.body
        integrity: dict[str, Any] = registry.verify_integrity(
            trusted_public_keys=trusted_public_keys, dataset_dir=dataset_dir)
        integrity["chain_head"] = _chain_head(registry)
        integrity["chain_verified"] = True
    else:
        _assert_frozen(record)
        source = record
        integrity = {"status": "frozen_record_digest_verified", "signature_verified": False,
                     "chain_verified": False, "quality_or_entitlement_certified": False}
    frozen = freeze_for_comparison(source)
    binding = verify_dataset_binding(frozen, dataset_dir)
    report: dict[str, Any] = {
        "schema_version": 1,
        "report_type": "independent_validation_report",
        "experiment_id": frozen["experiment_id"],
        "status": frozen["status"],
        "frozen_record_sha256": frozen["record_sha256"],
        "integrity": integrity,
        "dataset_binding": binding,
        "comparison_summary": _summary(frozen),
        "warning": INTEGRITY_WARNING,
        "research_warning": RESEARCH_WARNING,
        "claim_scope": "internal_integrity_only",
    }
    report["report_sha256"] = content_hash(report)
    return report


def validation_report_bytes(report: dict[str, Any]) -> bytes:
    """Encode a validation report with the repository's canonical JSON rules."""
    expected = report.get("report_sha256")
    body = {key: value for key, value in report.items() if key != "report_sha256"}
    if expected != content_hash(body):
        raise ExperimentError("validation report digest mismatch")
    return (json.dumps(report, sort_keys=True, separators=(",", ":"),
                       ensure_ascii=False, allow_nan=False) + "\n").encode("utf-8")


def load_frozen_record(path: str | Path) -> dict[str, Any]:
    """Load a frozen experiment record from disk without validating it."""
    try:
        record = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ExperimentError(f"could not read frozen record: {exc}") from exc
    if not isinstance(record, dict):
        raise ExperimentError("frozen record must be a JSON object")
    return record
