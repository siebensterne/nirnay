"""Pure manifest tests for the local research job runner.

D11 removed the generated synthetic dataset and the synthetic job entry. These
tests consume only scalar manifest declarations; running a job requires an
already admitted, externally published dataset and is not exercised here.
"""
from __future__ import annotations

import json

import pytest

from quantlab.experiments import ExperimentError
from quantlab.research_jobs import default_job, environment_block, validate_manifest


@pytest.fixture(scope="module")
def environment() -> dict:
    return environment_block(commit="ec0b216f07e7b7e653e8f784b18c6517d6335696")


def _binding() -> dict:
    """Scalar published-release identity; no observation or artifact is built."""
    return {
        "dataset_id": "admitted-release",
        "dataset_version": "admitted-release:0123456789abcdef",
        "dataset_manifest_sha256": "a" * 64,
        "dataset_origin": {},
        "dataset_lineage": {},
    }


def _manifest(environment: dict) -> dict:
    return {"schema_version": 1, "environment": json.loads(json.dumps(environment)),
            "jobs": [default_job(environment, _binding())]}


def test_pure_manifest_accepts_valid_declaration(environment: dict) -> None:
    """A scalar binding and pinned environment validate as a v1 manifest."""
    assert validate_manifest(_manifest(environment))["schema_version"] == 1


def test_pure_manifest_schema_rejects_invalid_cases(environment: dict) -> None:
    """Schema and command allowlist reject malformed scalar declarations."""
    cases = []
    wrong_version = _manifest(environment)
    wrong_version["schema_version"] = 2
    cases.append(wrong_version)

    missing = _manifest(environment)
    del missing["jobs"][0]["dataset_sha256"]
    cases.append(missing)

    bad_hash = _manifest(environment)
    bad_hash["jobs"][0]["dataset_sha256"] = "not-a-digest"
    cases.append(bad_hash)

    not_dry = _manifest(environment)
    not_dry["jobs"][0]["dry_run"] = False
    cases.append(not_dry)

    bad_schedule = _manifest(environment)
    bad_schedule["jobs"][0]["schedule_hint"] = "always-on-daemon"
    cases.append(bad_schedule)

    bad_command = _manifest(environment)
    bad_command["jobs"][0]["command"] = ["curl", "https://example.invalid"]
    cases.append(bad_command)

    extra = _manifest(environment)
    extra["jobs"][0]["shell"] = "echo hi"
    cases.append(extra)

    for case in cases:
        with pytest.raises(ExperimentError):
            validate_manifest(case)


def test_pure_manifest_cross_field_invariants(environment: dict) -> None:
    """Digest drift and duplicate job ids are refused without data access."""
    mismatch = _manifest(environment)
    mismatch["jobs"][0]["code_digest"] = "0" * 64
    with pytest.raises(ExperimentError, match="code_digest"):
        validate_manifest(mismatch)

    duplicate = _manifest(environment)
    duplicate["jobs"].append(dict(duplicate["jobs"][0]))
    with pytest.raises(ExperimentError, match="duplicate"):
        validate_manifest(duplicate)
