"""Local opt-in research job manifest and runner contract tests."""
from __future__ import annotations

import json
from pathlib import Path

import pytest

from quantlab.cli import main
from quantlab.experiments import ExperimentError, ExperimentRegistry
from quantlab.research_jobs import (build_synthetic_dataset, default_job, environment_block,
                                    run_job, validate_manifest)


@pytest.fixture(scope="module")
def binding(tmp_path_factory) -> dict:
    out = tmp_path_factory.mktemp("job-binding")
    _dataset, value = build_synthetic_dataset(out)
    return value


@pytest.fixture(scope="module")
def environment() -> dict:
    return environment_block(commit="ec0b216f07e7b7e653e8f784b18c6517d6335696")


def _manifest(environment: dict, binding: dict) -> dict:
    return {"schema_version": 1, "environment": json.loads(json.dumps(environment)),
            "jobs": [default_job(environment, binding)]}


def _write(manifest: dict, path: Path) -> Path:
    path.write_text(json.dumps(manifest, sort_keys=True), encoding="utf-8")
    return path


def test_manifest_accepts_valid_declaration(environment: dict, binding: dict) -> None:
    assert validate_manifest(_manifest(environment, binding))["schema_version"] == 1


def test_manifest_schema_rejects_invalid_cases(environment: dict, binding: dict) -> None:
    cases = []
    wrong_version = _manifest(environment, binding)
    wrong_version["schema_version"] = 2
    cases.append(wrong_version)

    missing = _manifest(environment, binding)
    del missing["jobs"][0]["dataset_sha256"]
    cases.append(missing)

    bad_hash = _manifest(environment, binding)
    bad_hash["jobs"][0]["dataset_sha256"] = "not-a-digest"
    cases.append(bad_hash)

    not_dry = _manifest(environment, binding)
    not_dry["jobs"][0]["dry_run"] = False
    cases.append(not_dry)

    bad_schedule = _manifest(environment, binding)
    bad_schedule["jobs"][0]["schedule_hint"] = "always-on-daemon"
    cases.append(bad_schedule)

    bad_command = _manifest(environment, binding)
    bad_command["jobs"][0]["command"] = ["curl", "https://example.invalid"]
    cases.append(bad_command)

    extra = _manifest(environment, binding)
    extra["jobs"][0]["shell"] = "echo hi"
    cases.append(extra)

    for case in cases:
        with pytest.raises(ExperimentError):
            validate_manifest(case)


def test_manifest_cross_field_invariants(environment: dict, binding: dict) -> None:
    mismatch = _manifest(environment, binding)
    mismatch["jobs"][0]["code_digest"] = "0" * 64
    with pytest.raises(ExperimentError, match="code_digest"):
        validate_manifest(mismatch)

    duplicate = _manifest(environment, binding)
    duplicate["jobs"].append(dict(duplicate["jobs"][0]))
    with pytest.raises(ExperimentError, match="duplicate"):
        validate_manifest(duplicate)


def test_unpinned_environment_is_refused(environment: dict, binding: dict, tmp_path) -> None:
    manifest = _manifest(environment, binding)
    manifest["environment"]["constraint_sha256"] = "0" * 64
    with pytest.raises(ExperimentError, match="unpinned environment"):
        run_job(_write(manifest, tmp_path / "unpinned.json"), tmp_path / "out",
                tmp_path / "registry.sqlite3")

    absolute = _manifest(environment, binding)
    absolute["environment"]["python_constraint_file"] = "/tmp/constraints.txt"
    absolute["jobs"][0]["constraint_file"] = "/tmp/constraints.txt"
    with pytest.raises(ExperimentError, match="unpinned environment"):
        run_job(_write(absolute, tmp_path / "absolute.json"), tmp_path / "out2",
                tmp_path / "registry2.sqlite3")


def test_stale_dataset_hash_is_refused(environment: dict, binding: dict, tmp_path) -> None:
    manifest = _manifest(environment, binding)
    manifest["jobs"][0]["dataset_sha256"] = "0" * 64
    with pytest.raises(ExperimentError, match="stale dataset hash"):
        run_job(_write(manifest, tmp_path / "stale.json"), tmp_path / "out",
                tmp_path / "registry.sqlite3")

    identity = _manifest(environment, binding)
    suffix = binding["dataset_version"].rsplit(":", 1)[-1]
    identity["jobs"][0]["dataset_id"] = "some-other-dataset"
    identity["jobs"][0]["dataset_version"] = f"some-other-dataset:{suffix}"
    with pytest.raises(ExperimentError, match="stale dataset binding"):
        run_job(_write(identity, tmp_path / "identity.json"), tmp_path / "out2",
                tmp_path / "registry2.sqlite3")


def test_dry_run_is_deterministic_and_appends_a_run_record(
    environment: dict, binding: dict, tmp_path
) -> None:
    path = _write(_manifest(environment, binding), tmp_path / "jobs.json")
    first = run_job(path, tmp_path / "out1", tmp_path / "registry1.sqlite3")
    second = run_job(path, tmp_path / "out2", tmp_path / "registry2.sqlite3")

    assert first.exit_code == 0 and second.exit_code == 0
    assert first.verify_status == "verified"
    assert first.output_sha256 == second.output_sha256
    assert first.dataset_sha256 == second.dataset_sha256 == binding["dataset_manifest_sha256"]
    assert first.command == second.command
    assert first.synthetic is True and first.dry_run is True
    assert first.integrity_status == "verified_internal_integrity"
    assert first.registry_sequence == second.registry_sequence == 2
    for registry_path in (tmp_path / "registry1.sqlite3", tmp_path / "registry2.sqlite3"):
        registry = ExperimentRegistry(registry_path)
        assert [row.status for row in registry.records()] == ["registered", "completed"]
        assert registry.verify_integrity()["record_count"] == 2


def test_cli_run_research_job(environment: dict, binding: dict, tmp_path) -> None:
    path = _write(_manifest(environment, binding), tmp_path / "jobs.json")
    registry = tmp_path / "registry.sqlite3"
    assert main(["run-research-job", "--manifest", str(path), "--out", str(tmp_path / "out"),
                 "--registry", str(registry)]) == 0
    assert ExperimentRegistry(registry).verify_integrity()["record_count"] == 2
