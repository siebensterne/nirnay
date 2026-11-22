# SPDX-License-Identifier: Apache-2.0
"""Dataset-admission gate tests using abstract scalars and no market data."""
from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from quantlab.cli import main
from quantlab.dataset_admission import (
    CONSUMER_OPERATIONS,
    DatasetAdmissionError,
    admit_published_dataset,
    load_admission,
    validate_dataset_admission,
)
from quantlab.research_jobs import run_job

DIGEST_A = "a" * 64
DIGEST_B = "b" * 64
DIGEST_C = "c" * 64
AS_OF_NS = 1_700_000_000_000_000_000
SCOPE = {
    "provider": "provider-a",
    "product": "product-x",
    "venue": "venue-a",
    "segment": "segment-cash",
}


def _record() -> dict[str, Any]:
    """Build a valid admission record from declared scalar references only."""
    operations = {
        name: {"state": "PERMITTED", "document_reference": "written:rights-record-2026"}
        for name in CONSUMER_OPERATIONS
    }
    return {
        "contract_version": 1,
        "dataset": {
            "dataset_id": "abstract-dataset",
            "dataset_version": "abstract-dataset:v1",
            "dataset_origin": "MARKET",
            "source_scope": dict(SCOPE),
            "universe_id": "abstract-universe",
            "as_of_ns": AS_OF_NS,
            "as_of_basis": "knowledge_time",
            "manifest_sha256": DIGEST_A,
            "data_sha256": DIGEST_B,
        },
        "rights": {
            "record_reference": "written:rights-record-2026",
            "permission_source": "WRITTEN_PERMISSION",
            "review_date": "2026-11-22",
            "valid_until": "2026-12-31",
            "retention_period": "P90D",
            "scope": dict(SCOPE),
            "operations": operations,
        },
        "coverage": {
            "certificate_reference": "coverage:certificate-2026",
            "valid_until": "2026-12-31",
            "verdict": "PASS",
            "completeness": "COMPLETE",
            "scope": dict(SCOPE),
            "universe_id": "abstract-universe",
            "as_of_ns": AS_OF_NS,
        },
        "release": {
            "release_id": "release:abstract-2026",
            "immutable": True,
            "digest_algorithm": "sha256",
            "publication_method": "atomic_directory_rename",
            "publication_identity": "publisher-key-1",
            "archive_reference": "archive:object-1",
            "dataset_id": "abstract-dataset",
            "dataset_version": "abstract-dataset:v1",
            "manifest_sha256": DIGEST_A,
            "data_sha256": DIGEST_B,
        },
        "lineage": {
            "basis": "universe_and_corporate_actions",
            "as_of_basis": "knowledge_time",
            "as_of_ns": AS_OF_NS,
            "universe_id": "abstract-universe",
            "source_scope": dict(SCOPE),
            "universe_version": "universe-2026",
            "universe_source_reference": "universe-source:1",
            "instrument_identity_source_reference": "identity-source:1",
            "corporate_action_source_reference": "action-source:1",
            "universe_source_sha256": DIGEST_C,
            "instrument_identity_source_sha256": DIGEST_C,
            "corporate_action_source_sha256": DIGEST_C,
            "revision": 1,
            "knowledge_time_ns": AS_OF_NS - 1,
        },
    }


def _refused(payload: dict[str, Any], needle: str) -> None:
    """Assert that a record mutation is refused with the named reason."""
    with pytest.raises(DatasetAdmissionError, match=needle):
        validate_dataset_admission(payload)


def test_valid_abstract_record_is_admitted() -> None:
    """A complete, internally consistent declaration yields a scalar receipt."""
    receipt = validate_dataset_admission(_record())
    assert receipt.contract_version == 1
    assert receipt.dataset_id == "abstract-dataset"
    assert receipt.dataset_version == "abstract-dataset:v1"
    assert receipt.universe_id == "abstract-universe"
    assert receipt.release_id == "release:abstract-2026"
    assert receipt.manifest_sha256 == DIGEST_A
    assert receipt.data_sha256 == DIGEST_B
    assert receipt.rights_record_reference == "written:rights-record-2026"
    assert receipt.coverage_certificate_reference == "coverage:certificate-2026"
    assert receipt.lineage_basis == "universe_and_corporate_actions"
    assert receipt.as_of_ns == AS_OF_NS
    assert receipt.as_dict()["contract_version"] == 1


@pytest.mark.parametrize("version", [0, 2, "1", None])
def test_contract_version_is_enforced(version: object) -> None:
    """Only the supported contract version is admitted."""
    payload = _record()
    payload["contract_version"] = version
    _refused(payload, "contract_version")


@pytest.mark.parametrize("section", ["dataset", "rights", "coverage", "release", "lineage"])
def test_missing_sections_are_refused(section: str) -> None:
    """A record missing one evidence section is denied."""
    payload = _record()
    del payload[section]
    _refused(payload, "must be an object")


def test_non_object_record_is_refused() -> None:
    """The top-level payload must be a mapping."""
    _refused([], "admission record must be an object")


def test_synthetic_origin_is_refused() -> None:
    """A synthetic dataset cannot be admitted for research."""
    payload = _record()
    payload["dataset"]["dataset_origin"] = "SYNTHETIC"
    _refused(payload, "dataset_origin")


def test_dataset_version_must_extend_dataset_id() -> None:
    """Release identity and dataset identity must agree."""
    payload = _record()
    payload["dataset"]["dataset_version"] = "unrelated:v1"
    _refused(payload, "must extend")


def test_written_rights_source_is_required() -> None:
    """A self-attested or unknown source is not written permission."""
    for source in ("UNKNOWN", "SELF_ATTESTED", None):
        payload = _record()
        payload["rights"]["permission_source"] = source
        _refused(payload, "permission_source")


@pytest.mark.parametrize("state", ["UNKNOWN", "PENDING", "PROHIBITED", None])
def test_denied_rights_state_is_refused(state: object) -> None:
    """Every required operation must carry a PERMITTED state."""
    payload = _record()
    payload["rights"]["operations"]["read_release"]["state"] = state
    _refused(payload, "rights.operations.read_release.state")


def test_rights_must_cite_the_written_record_per_operation() -> None:
    """Each permitted operation must cite the same written rights record."""
    payload = _record()
    payload["rights"]["operations"]["replay"]["document_reference"] = "written:other"
    _refused(payload, "does not cite the written record")


def test_rights_operations_must_cover_the_exact_research_set() -> None:
    """Omitting or adding an operation breaks the exact-scope requirement."""
    payload = _record()
    del payload["rights"]["operations"]["retain_permitted_derived_data"]
    _refused(payload, "exact research set")

    extra = _record()
    extra["rights"]["operations"]["capture"] = {
        "state": "PERMITTED",
        "document_reference": "written:rights-record-2026",
    }
    _refused(extra, "exact research set")


def test_consumer_operations_match_the_documented_set() -> None:
    """The named constant is the five consumer activities, not producer operations."""
    assert CONSUMER_OPERATIONS == (
        "read_release",
        "store_permitted_research_copy",
        "replay",
        "internal_analytics",
        "retain_permitted_derived_data",
    )


@pytest.mark.parametrize("operation", CONSUMER_OPERATIONS)
def test_each_consumer_operation_needs_its_own_permitted_evidence(operation: str) -> None:
    """A denial on one operation cannot be softened by another operation's grant."""
    payload = _record()
    payload["rights"]["operations"][operation]["state"] = "UNKNOWN"
    _refused(payload, f"rights.operations.{operation}.state")


def test_extra_producer_grant_never_authorises_a_consumer_operation() -> None:
    """A permitted producer operation is not an added consumer entitlement."""
    payload = _record()
    payload["rights"]["operations"]["source_contact"] = {
        "state": "PERMITTED",
        "document_reference": "written:rights-record-2026",
    }
    _refused(payload, "exact research set")


def test_storing_a_permitted_copy_does_not_imply_derived_retention() -> None:
    """Holding the accepted dataset does not grant retention of derived data."""
    payload = _record()
    payload["rights"]["operations"]["store_permitted_research_copy"]["state"] = "PERMITTED"
    payload["rights"]["operations"]["retain_permitted_derived_data"]["state"] = "DENIED"
    _refused(payload, "rights.operations.retain_permitted_derived_data.state")


def test_rights_record_reference_must_be_a_named_document() -> None:
    """Placeholder references are not written permission records."""
    for reference in ("UNKNOWN", "PENDING", "DENIED", "EXPIRED", "REVOKED",
                      "written:NO-permission", "unspecified:verbally-agreed"):
        payload = _record()
        payload["rights"]["record_reference"] = reference
        _refused(payload, "rights.record_reference")


def test_rights_scope_must_match_dataset_scope() -> None:
    """Rights covering a different venue or product do not cover this dataset."""
    for field in SCOPE:
        payload = _record()
        payload["rights"]["scope"][field] = "other-" + field
        _refused(payload, "rights scope")


def test_rights_review_date_and_retention_are_required() -> None:
    """A dated review, live validity and bounded retention accompany permission."""
    payload = _record()
    payload["rights"]["review_date"] = "22-11-2026"
    _refused(payload, "ISO date")

    stale = _record()
    stale["rights"]["review_date"] = "2019-01-01"
    _refused(stale, "before the dataset as-of date")

    lapsed = _record()
    lapsed["rights"]["valid_until"] = "2020-01-01"
    _refused(lapsed, "rights.valid_until lapsed")

    missing = _record()
    del missing["rights"]["retention_period"]
    _refused(missing, "rights.retention_period")

    for retention in ("forever", "P0D", "P", "PT1H", "0", "-5"):
        unbounded = _record()
        unbounded["rights"]["retention_period"] = retention
        _refused(unbounded, "bounded ISO-8601 days")

    for bounded in ("P90D", "90", "P1Y6M"):
        accepted = _record()
        accepted["rights"]["retention_period"] = bounded
        assert validate_dataset_admission(accepted).rights_record_reference


@pytest.mark.parametrize("verdict", ["UNKNOWN", "PENDING", "FAILED", "FAIL", None])
def test_coverage_verdict_must_pass(verdict: object) -> None:
    """An absent or denied coverage verdict cannot admit a dataset."""
    payload = _record()
    payload["coverage"]["verdict"] = verdict
    _refused(payload, "coverage.verdict")


def test_coverage_completeness_must_pass() -> None:
    """A bounded or unknown completeness claim is not a passing certificate."""
    for completeness in ("PARTIAL", "UNKNOWN", None):
        payload = _record()
        payload["coverage"]["completeness"] = completeness
        _refused(payload, "coverage.completeness")


def test_coverage_certificate_reference_is_required() -> None:
    """A passing verdict needs a named, unexpired certificate reference."""
    for reference in ("", "PENDING", "DENIED", "EXPIRED", "REVOKED", "NO", None):
        payload = _record()
        payload["coverage"]["certificate_reference"] = reference
        _refused(payload, "coverage.certificate_reference")

    missing = _record()
    del missing["coverage"]["valid_until"]
    _refused(missing, "coverage.valid_until is missing")

    lapsed = _record()
    lapsed["coverage"]["valid_until"] = "2019-01-01"
    _refused(lapsed, "coverage.valid_until lapsed")


def test_coverage_must_match_scope_universe_and_as_of() -> None:
    """Certification cannot be borrowed from another scope or as-of instant."""
    payload = _record()
    payload["coverage"]["scope"]["venue"] = "other-venue"
    _refused(payload, "coverage scope")

    universe = _record()
    universe["coverage"]["universe_id"] = "other-universe"
    _refused(universe, "coverage universe_id")

    as_of = _record()
    as_of["coverage"]["as_of_ns"] = AS_OF_NS + 1
    _refused(as_of, "coverage as_of_ns")


def test_release_identity_must_be_immutable_and_named() -> None:
    """A mutable or anonymous release is not an immutable publication."""
    payload = _record()
    payload["release"]["immutable"] = False
    _refused(payload, "release.immutable")

    truthy = _record()
    truthy["release"]["immutable"] = 1
    _refused(truthy, "release.immutable")

    anonymous = _record()
    anonymous["release"]["publication_identity"] = "UNKNOWN"
    _refused(anonymous, "release.publication_identity")

    unarchived = _record()
    del unarchived["release"]["archive_reference"]
    _refused(unarchived, "release.archive_reference")


def test_release_method_and_digest_algorithm_are_declared() -> None:
    """The release must name its publication method and digest algorithm."""
    payload = _record()
    payload["release"]["publication_method"] = "mutable-overwrite"
    _refused(payload, "release.publication_method")

    algorithm = _record()
    algorithm["release"]["digest_algorithm"] = "md5"
    _refused(algorithm, "release.digest_algorithm")


def test_release_digests_must_match_the_dataset() -> None:
    """Release digests are checked against the dataset declaration."""
    for field in ("manifest_sha256", "data_sha256"):
        payload = _record()
        payload["release"][field] = DIGEST_C
        _refused(payload, f"release {field}")


def test_release_identity_fields_must_match_the_dataset() -> None:
    """A release for another dataset identity is refused."""
    payload = _record()
    payload["release"]["dataset_id"] = "other-dataset"
    _refused(payload, "release dataset_id")

    version = _record()
    version["release"]["dataset_version"] = "abstract-dataset:v2"
    _refused(version, "release dataset_version")


@pytest.mark.parametrize("digest", [DIGEST_A.upper(), "abc", "z" * 64, ""])
def test_digest_fields_must_be_lowercase_sha256(digest: object) -> None:
    """Malformed digests are refused before any consistency check."""
    payload = _record()
    payload["dataset"]["manifest_sha256"] = digest
    _refused(payload, "lowercase sha256")


def test_lineage_basis_must_be_point_in_time() -> None:
    """Ordered observations alone do not certify universe and action lineage."""
    for basis in ("chronological_only", "none", "UNKNOWN", None):
        payload = _record()
        payload["lineage"]["basis"] = basis
        _refused(payload, "lineage.basis")


def test_lineage_as_of_basis_must_match_the_dataset() -> None:
    """The as-of basis is one declaration across the record."""
    payload = _record()
    payload["lineage"]["as_of_basis"] = "event_time"
    _refused(payload, "lineage.as_of_basis")


def test_lineage_must_match_dataset_scope_universe_and_as_of() -> None:
    """Lineage cannot describe a different scope, universe or as-of instant."""
    for field, value in (("source_scope", None), ("universe_id", "other-universe"),
                         ("as_of_ns", AS_OF_NS + 1)):
        payload = _record()
        if field == "source_scope":
            payload["lineage"][field]["venue"] = "other-venue"
            needle = "lineage scope"
        else:
            payload["lineage"][field] = value
            needle = f"lineage {field}"
        _refused(payload, needle)


def test_lineage_knowledge_time_must_not_follow_the_as_of_instant() -> None:
    """Knowledge acquired after the as-of instant would leak future state."""
    payload = _record()
    payload["lineage"]["knowledge_time_ns"] = AS_OF_NS + 1
    _refused(payload, "knowledge_time_ns is later")


def test_lineage_source_references_and_revision_are_required() -> None:
    """Universe and corporate-action provenance must be named and revisioned."""
    for field in ("universe_version", "universe_source_reference",
                  "instrument_identity_source_reference",
                  "corporate_action_source_reference"):
        payload = _record()
        del payload["lineage"][field]
        _refused(payload, f"lineage.{field}")

    revision = _record()
    revision["lineage"]["revision"] = 0
    _refused(revision, "lineage.revision")


def test_lineage_source_digests_are_required() -> None:
    """Source references carry immutable digests."""
    for field in ("universe_source_sha256", "instrument_identity_source_sha256",
                  "corporate_action_source_sha256"):
        payload = _record()
        payload["lineage"][field] = "UNKNOWN"
        _refused(payload, f"lineage.{field}")


def test_positive_integer_fields_are_typed() -> None:
    """Booleans and zero cannot stand in for timestamps or revisions."""
    payload = _record()
    payload["dataset"]["as_of_ns"] = True
    _refused(payload, "dataset.as_of_ns must be a positive integer")

    negative = _record()
    negative["lineage"]["knowledge_time_ns"] = 0
    _refused(negative, "lineage.knowledge_time_ns must be a positive integer")


def test_load_admission_refuses_missing_or_malformed_records(tmp_path: Path) -> None:
    """Local record loading fails closed on absent or malformed files."""
    with pytest.raises(DatasetAdmissionError, match="missing or redirected"):
        load_admission(tmp_path / "absent.json")
    path = tmp_path / "bad.json"
    path.write_text("not-json", encoding="utf-8")
    with pytest.raises(DatasetAdmissionError, match="not valid JSON"):
        load_admission(path)


def test_admit_published_dataset_requires_a_published_release(tmp_path: Path) -> None:
    """A valid record cannot admit a dataset that does not exist."""
    path = tmp_path / "admission.json"
    path.write_text(json.dumps(_record(), sort_keys=True), encoding="utf-8")
    with pytest.raises(DatasetAdmissionError, match="published dataset directory is missing"):
        admit_published_dataset(tmp_path / "absent", path)


def test_cli_admit_dataset_refuses_denied_record_without_touching_data(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    """The CLI names the denied field before reading any dataset."""
    payload = _record()
    payload["rights"]["operations"]["store_permitted_research_copy"]["state"] = "UNKNOWN"
    path = tmp_path / "admission.json"
    path.write_text(json.dumps(payload, sort_keys=True), encoding="utf-8")
    code = main(["admit-dataset", "--dataset", str(tmp_path / "absent"),
                 "--admission", str(path)])
    assert code == 2
    assert "rights.operations.store_permitted_research_copy.state" in capsys.readouterr().err


def test_cli_verify_dataset_for_research_requires_admission(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    """Research verification refuses a dataset with no admission record."""
    code = main(["verify-dataset", str(tmp_path / "absent"), "--for-research"])
    assert code == 2
    assert "requires --admission" in capsys.readouterr().err


def test_cli_run_research_job_requires_admission_and_dataset(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    """A research job refuses to start without both inputs and writes no artifact."""
    registry = tmp_path / "registry.sqlite3"
    base = ["run-research-job", "--manifest", str(tmp_path / "jobs.json"),
            "--registry", str(registry)]
    code = main([*base, "--dataset", str(tmp_path / "release")])
    assert code == 2
    assert "require --dataset and --admission" in capsys.readouterr().err
    assert not registry.exists()

    code = main([*base, "--admission", str(tmp_path / "admission.json")])
    assert code == 2
    assert "require --dataset and --admission" in capsys.readouterr().err
    assert not registry.exists()

    with pytest.raises(DatasetAdmissionError, match="both an admitted dataset directory"):
        run_job(tmp_path / "jobs.json", registry)
    assert not registry.exists()


def test_cli_admit_dataset_refuses_unreadable_record(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    """A missing admission file is a clear CLI failure."""
    code = main(["admit-dataset", "--dataset", str(tmp_path / "absent"),
                 "--admission", str(tmp_path / "absent.json")])
    assert code == 2
    assert "admission record is missing" in capsys.readouterr().err
