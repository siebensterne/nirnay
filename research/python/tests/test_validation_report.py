"""Independent validation report: tamper, reorder and hash-stability tests."""
from __future__ import annotations

import json
import sqlite3

import pytest

from quantlab.cli import main
from quantlab.experiments import ExperimentError, ExperimentRegistry
from quantlab.reporting.comparison import ComparisonError, freeze_for_comparison
from quantlab.research_jobs import (build_synthetic_dataset, default_job, environment_block,
                                    run_job)
from quantlab.validation_report import build_validation_report, validation_report_bytes


def _frozen_case(tmp_path):
    environment = environment_block(commit="ec0b216f07e7b7e653e8f784b18c6517d6335696")
    _dataset, binding = build_synthetic_dataset(tmp_path / "binding")
    manifest = {"schema_version": 1, "environment": environment,
                "jobs": [default_job(environment, binding)]}
    manifest_path = tmp_path / "jobs.json"
    manifest_path.write_text(json.dumps(manifest, sort_keys=True), encoding="utf-8")
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    run_job(manifest_path, tmp_path / "run", registry.path)
    frozen = freeze_for_comparison(registry.records()[-1].body)
    record_path = tmp_path / "frozen.json"
    record_path.write_text(json.dumps(frozen, sort_keys=True), encoding="utf-8")
    dataset_dir, _again = build_synthetic_dataset(tmp_path / "published")
    return registry, frozen, record_path, dataset_dir


@pytest.mark.market_shaped_fixture
def test_validation_report_is_deterministic(tmp_path) -> None:
    _registry, frozen, record_path, dataset_dir = _frozen_case(tmp_path)
    first = build_validation_report(frozen, dataset_dir)
    from_file = build_validation_report(json.loads(record_path.read_text()), dataset_dir)
    assert validation_report_bytes(first) == validation_report_bytes(from_file)
    assert first["report_sha256"] == from_file["report_sha256"]
    assert len(first["report_sha256"]) == 64
    assert first["claim_scope"] == "internal_integrity_only"
    assert first["dataset_binding"]["status"] == "verified_internal_integrity"
    assert first["integrity"]["chain_verified"] is False


@pytest.mark.market_shaped_fixture
def test_tampered_record_is_refused(tmp_path) -> None:
    _registry, frozen, _record_path, dataset_dir = _frozen_case(tmp_path)
    tampered = dict(frozen)
    tampered["hypothesis"] = "edited after freeze"
    with pytest.raises(ComparisonError, match="tampered"):
        build_validation_report(tampered, dataset_dir)

    removed = dict(frozen)
    removed.pop("record_sha256")
    with pytest.raises(ComparisonError, match="record_sha256"):
        build_validation_report(removed, dataset_dir)


@pytest.mark.market_shaped_fixture
def test_registry_report_resolves_stored_record(tmp_path) -> None:
    registry, frozen, _record_path, dataset_dir = _frozen_case(tmp_path)
    report = build_validation_report(frozen, dataset_dir, registry=registry)
    assert report["integrity"]["chain_verified"] is True
    assert report["integrity"]["record_count"] == 2
    assert report["frozen_record_sha256"] == frozen["record_sha256"]


@pytest.mark.market_shaped_fixture
def test_reordered_registry_chain_is_refused(tmp_path) -> None:
    registry, frozen, _record_path, dataset_dir = _frozen_case(tmp_path)
    connection = sqlite3.connect(registry.path)
    connection.execute("DROP TRIGGER experiment_records_no_update")
    connection.execute("UPDATE experiment_records SET previous_hash=? WHERE sequence=1",
                       ("1" * 64,))
    connection.commit()
    connection.close()
    with pytest.raises(ExperimentError, match="predecessor mismatch"):
        build_validation_report(frozen, dataset_dir, registry=registry)


@pytest.mark.market_shaped_fixture
def test_cli_validation_report_writes_stable_bytes(tmp_path) -> None:
    _registry, _frozen, record_path, dataset_dir = _frozen_case(tmp_path)
    output = tmp_path / "report.json"
    assert main(["validation-report", "--record", str(record_path),
                 "--dataset", str(dataset_dir), "--output", str(output)]) == 0
    first = output.read_bytes()
    second = tmp_path / "report2.json"
    assert main(["validation-report", "--record", str(record_path),
                 "--dataset", str(dataset_dir), "--output", str(second)]) == 0
    assert first == second.read_bytes()
    assert "report_sha256" in json.loads(first)
