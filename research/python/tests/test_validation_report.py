"""Pure scalar tests for validation-report digest and encoding helpers.

D11 retired the generated synthetic published dataset and the market-shaped
frozen records used by the former integration tests. No dataset or market
observation is built here; only digest and byte-encoding helpers are exercised.
"""
from __future__ import annotations

import json
from pathlib import Path

import pytest

from quantlab.experiments.schema import ExperimentError, content_hash
from quantlab.reporting.comparison import RESEARCH_WARNING, ComparisonError
from quantlab.validation_report import (INTEGRITY_WARNING, build_validation_report,
                                         load_frozen_record, validation_report_bytes)


def _report() -> dict:
    body = {
        "schema_version": 1,
        "report_type": "independent_validation_report",
        "experiment_id": "PURE-DIGEST-0000000000000000",
        "status": "completed",
        "frozen_record_sha256": "a" * 64,
        "integrity": {"status": "frozen_record_digest_verified", "chain_verified": False},
        "dataset_binding": {
            "status": "verified_internal_integrity",
            "dataset_id": "admitted-release",
            "dataset_version": "admitted-release:0123456789abcdef",
            "manifest_sha256": "b" * 64,
        },
        "comparison_summary": {},
        "warning": INTEGRITY_WARNING,
        "research_warning": RESEARCH_WARNING,
        "claim_scope": "internal_integrity_only",
    }
    body["report_sha256"] = content_hash(body)
    return body


def test_validation_report_bytes_are_byte_stable() -> None:
    """A digest-bound report encodes to identical bytes on repeated calls."""
    report = _report()
    first = validation_report_bytes(report)
    assert first == validation_report_bytes(report)
    assert first.endswith(b"\n")
    assert json.loads(first) == report


def test_validation_report_bytes_refuse_tampered_digest() -> None:
    """A changed report digest fails closed before encoding."""
    report = _report()
    report["report_sha256"] = "0" * 64
    with pytest.raises(ExperimentError, match="digest mismatch"):
        validation_report_bytes(report)


def test_build_validation_report_refuses_non_mapping_record(tmp_path: Path) -> None:
    """A non-mapping record is refused before any dataset path is read."""
    with pytest.raises(ComparisonError, match="frozen experiment record"):
        build_validation_report([], tmp_path)


def test_load_frozen_record_rejects_non_object(tmp_path: Path) -> None:
    """A JSON list is not a frozen record object."""
    path = tmp_path / "record.json"
    path.write_text(json.dumps([1, 2, 3]), encoding="utf-8")
    with pytest.raises(ExperimentError, match="must be a JSON object"):
        load_frozen_record(path)
