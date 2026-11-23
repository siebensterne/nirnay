# SPDX-License-Identifier: Apache-2.0
"""Versioned dataset-admission gate for research use.

A dataset is admitted only when an admission record satisfies four independent
evidence sections: a written rights record, a passing coverage certificate, an
immutable release identity with matching digests, and point-in-time lineage.
The validator consumes scalar declarations only. It contacts no provider,
ingests no market observation and grants no right. A refusal is a typed error
so every caller can fail closed with a named reason.

This module is not a complete release verifier. It does not verify detached
signatures, recompute canonical manifest bytes, check publisher-key validity
or revocation, or inspect actual data and artifact digests. Full detached
signature verification is a later implementation stage; passing this gate is
not a claim that a release signature or source lineage was verified.
"""
from __future__ import annotations

import datetime as dt
import json
import re
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any

ADMISSION_CONTRACT_VERSION = 1
# The exact consumer-operation set of engineering/ndc-consumer-requirements-v1.md.
# These are Nirnay-side activities, not Fieldbook producer operations. The gate
# requires all five with a PERMITTED state and one written record; an extra
# operation is a mismatch, not an added entitlement.
CONSUMER_OPERATIONS: tuple[str, ...] = (
    "read_release",
    "store_permitted_research_copy",
    "replay",
    "internal_analytics",
    "retain_permitted_derived_data",
)
PERMITTED = "PERMITTED"
WRITTEN_PERMISSION = "WRITTEN_PERMISSION"
COVERAGE_PASS = "PASS"
COVERAGE_COMPLETE = "COMPLETE"
POINT_IN_TIME_BASIS = "universe_and_corporate_actions"
AS_OF_BASIS = "knowledge_time"
DIGEST_ALGORITHM = "sha256"
MARKET_ORIGIN = "MARKET"
PUBLICATION_METHOD = "atomic_directory_rename"
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
PLACEHOLDER_RE = re.compile(
    r"\b(?:UNKNOWN|PENDING|FAILED|FAIL|BLOCKED|PROHIBITED|UNVERIFIED|"
    r"DENIED|EXPIRED|REVOKED|NO|NONE|TBD|PLACEHOLDER|UNSPECIFIED)\b",
    re.IGNORECASE,
)
SCOPE_FIELDS: tuple[str, ...] = ("provider", "product", "venue", "segment")
ISO_DURATION_RE = re.compile(r"P(?:(\d+)Y)?(?:(\d+)M)?(?:(\d+)W)?(?:(\d+)D)?")
INTEGER_DAYS_RE = re.compile(r"[1-9]\d*")
EPOCH = dt.datetime(1970, 1, 1, tzinfo=dt.UTC)


class DatasetAdmissionError(ValueError):
    """The admission record did not satisfy the versioned contract."""


@dataclass(frozen=True, slots=True)
class AdmissionReceipt:
    """Scalar receipt produced only after every admission section passes."""

    contract_version: int
    dataset_id: str
    dataset_version: str
    universe_id: str
    release_id: str
    manifest_sha256: str
    data_sha256: str
    rights_record_reference: str
    coverage_certificate_reference: str
    lineage_basis: str
    as_of_ns: int

    def as_dict(self) -> dict[str, Any]:
        """Return the receipt as a canonical JSON-ready mapping."""
        return {
            "contract_version": self.contract_version,
            "dataset_id": self.dataset_id,
            "dataset_version": self.dataset_version,
            "universe_id": self.universe_id,
            "release_id": self.release_id,
            "manifest_sha256": self.manifest_sha256,
            "data_sha256": self.data_sha256,
            "rights_record_reference": self.rights_record_reference,
            "coverage_certificate_reference": self.coverage_certificate_reference,
            "lineage_basis": self.lineage_basis,
            "as_of_ns": self.as_of_ns,
        }


def _object(value: object, field: str) -> Mapping[str, Any]:
    """Require an object at ``field`` and return it without coercion."""
    if not isinstance(value, Mapping):
        raise DatasetAdmissionError(f"{field} must be an object")
    return value


def _text(container: Mapping[str, Any], key: str, field: str) -> str:
    """Require a non-empty reference and reject denial or placeholder words."""
    value = container.get(key)
    if not isinstance(value, str) or not value.strip():
        raise DatasetAdmissionError(f"{field} is missing or empty")
    text = value.strip()
    if PLACEHOLDER_RE.search(text):
        raise DatasetAdmissionError(f"{field} carries a denial or placeholder value")
    return text


def _enum(container: Mapping[str, Any], key: str, field: str, expected: str) -> str:
    """Require an exact declared verdict and name the observed value on refusal."""
    value = container.get(key)
    if value == expected and isinstance(value, str):
        return value
    raise DatasetAdmissionError(f"{field} is {value!r}, expected {expected!r}")


def _flag(container: Mapping[str, Any], key: str, field: str, expected: bool) -> bool:
    """Require a boolean flag, refusing the truthy integers accepted by ``==``."""
    value = container.get(key)
    if type(value) is bool and value is expected:
        return value
    raise DatasetAdmissionError(f"{field} is {value!r}, expected {expected!r}")


def _positive_int(container: Mapping[str, Any], key: str, field: str) -> int:
    """Require a positive integer timestamp, count or revision."""
    value = container.get(key)
    if type(value) is not int or value <= 0:
        raise DatasetAdmissionError(f"{field} must be a positive integer")
    return value


def _digest(container: Mapping[str, Any], key: str, field: str) -> str:
    """Require a lowercase hex SHA-256 digest."""
    value = container.get(key)
    if not isinstance(value, str) or SHA256_RE.fullmatch(value) is None:
        raise DatasetAdmissionError(f"{field} must be a lowercase sha256 digest")
    return value


def _utc_date_from_ns(ns: int) -> dt.date:
    """Convert a nanosecond epoch instant to its UTC calendar date."""
    try:
        moment = EPOCH + dt.timedelta(microseconds=ns // 1_000)
    except (OverflowError, ValueError) as exc:
        raise DatasetAdmissionError("as-of instant is outside the UTC date range") from exc
    return moment.date()


def _bounded_retention(container: Mapping[str, Any], key: str, field: str) -> str:
    """Require bounded positive days or a bounded ISO-8601 date duration."""
    text = _text(container, key, field)
    if INTEGER_DAYS_RE.fullmatch(text) is not None:
        return text
    match = ISO_DURATION_RE.fullmatch(text)
    if match is not None and any(item is not None and int(item) > 0
                                 for item in match.groups()):
        return text
    raise DatasetAdmissionError(
        f"{field} must be bounded ISO-8601 days or positive integer days"
    )


def _validity_date(container: Mapping[str, Any], key: str, field: str,
                   as_of_date: dt.date, *, required: bool = False) -> dt.date | None:
    """Require a validity bound that has not lapsed at the as-of date."""
    value = container.get(key)
    if value is None:
        if required:
            raise DatasetAdmissionError(f"{field} is missing")
        return None
    if not isinstance(value, str):
        raise DatasetAdmissionError(f"{field} must be an ISO date")
    try:
        bound = dt.date.fromisoformat(value)
    except ValueError as exc:
        raise DatasetAdmissionError(f"{field} is not an ISO date") from exc
    if bound < as_of_date:
        raise DatasetAdmissionError(f"{field} lapsed before the dataset as-of date")
    return bound


def _scope(value: object, field: str) -> dict[str, str]:
    """Require the four exact source-scope dimensions with named values."""
    scope = _object(value, field)
    if set(scope) != set(SCOPE_FIELDS):
        raise DatasetAdmissionError(
            f"{field} must name exactly provider, product, venue and segment"
        )
    return {name: _text(scope, name, f"{field}.{name}") for name in SCOPE_FIELDS}


def _match(left: object, right: object, label: str) -> None:
    """Require two internally related declarations to be equal."""
    if left != right:
        raise DatasetAdmissionError(f"{label} differs between admission sections")


def _validate_rights(record: Mapping[str, Any], scope: dict[str, str],
                     as_of_date: dt.date) -> str:
    """Require a written rights record covering the exact research operations."""
    rights = _object(record.get("rights"), "rights")
    reference = _text(rights, "record_reference", "rights.record_reference")
    _enum(rights, "permission_source", "rights.permission_source", WRITTEN_PERMISSION)
    review_date = _text(rights, "review_date", "rights.review_date")
    try:
        reviewed = dt.date.fromisoformat(review_date)
    except ValueError as exc:
        raise DatasetAdmissionError("rights.review_date is not an ISO date") from exc
    if reviewed < as_of_date:
        raise DatasetAdmissionError("rights.review_date is before the dataset as-of date")
    _validity_date(rights, "valid_until", "rights.valid_until", as_of_date)
    _bounded_retention(rights, "retention_period", "rights.retention_period")
    operations = _object(rights.get("operations"), "rights.operations")
    expected = set(CONSUMER_OPERATIONS)
    if set(operations) != expected:
        missing = sorted(expected - set(operations))
        extra = sorted(set(operations) - expected)
        raise DatasetAdmissionError(
            f"rights.operations must cover the exact research set; missing={missing} extra={extra}"
        )
    for operation in CONSUMER_OPERATIONS:
        evidence = _object(operations[operation], f"rights.operations.{operation}")
        _enum(evidence, "state", f"rights.operations.{operation}.state", PERMITTED)
        cited = _text(evidence, "document_reference",
                      f"rights.operations.{operation}.document_reference")
        if cited != reference:
            raise DatasetAdmissionError(
                f"rights.operations.{operation} does not cite the written record"
            )
    _match(_scope(rights.get("scope"), "rights.scope"), scope, "rights scope")
    return reference


def _validate_coverage(record: Mapping[str, Any], scope: dict[str, str],
                       universe_id: str, as_of_ns: int, as_of_date: dt.date) -> str:
    """Require a passing coverage certificate bound to the same declared scope."""
    coverage = _object(record.get("coverage"), "coverage")
    _enum(coverage, "verdict", "coverage.verdict", COVERAGE_PASS)
    _enum(coverage, "completeness", "coverage.completeness", COVERAGE_COMPLETE)
    certificate = _text(coverage, "certificate_reference", "coverage.certificate_reference")
    _validity_date(coverage, "valid_until", "coverage.valid_until", as_of_date,
                   required=True)
    _match(_scope(coverage.get("scope"), "coverage.scope"), scope, "coverage scope")
    _match(coverage.get("universe_id"), universe_id, "coverage universe_id")
    _match(coverage.get("as_of_ns"), as_of_ns, "coverage as_of_ns")
    return certificate


def _validate_release(record: Mapping[str, Any], dataset_id: str, dataset_version: str,
                      manifest_sha256: str, data_sha256: str) -> str:
    """Require an immutable release identity whose digests match the dataset."""
    release = _object(record.get("release"), "release")
    release_id = _text(release, "release_id", "release.release_id")
    _flag(release, "immutable", "release.immutable", True)
    _enum(release, "digest_algorithm", "release.digest_algorithm", DIGEST_ALGORITHM)
    _enum(release, "publication_method", "release.publication_method", PUBLICATION_METHOD)
    _text(release, "publication_identity", "release.publication_identity")
    _text(release, "archive_reference", "release.archive_reference")
    _match(release.get("dataset_id"), dataset_id, "release dataset_id")
    _match(release.get("dataset_version"), dataset_version, "release dataset_version")
    _match(release.get("manifest_sha256"), manifest_sha256, "release manifest_sha256")
    _match(release.get("data_sha256"), data_sha256, "release data_sha256")
    return release_id


def _validate_lineage(record: Mapping[str, Any], scope: dict[str, str],
                      universe_id: str, as_of_ns: int) -> str:
    """Require point-in-time provenance with non-leaking as-of semantics."""
    lineage = _object(record.get("lineage"), "lineage")
    basis = _enum(lineage, "basis", "lineage.basis", POINT_IN_TIME_BASIS)
    _enum(lineage, "as_of_basis", "lineage.as_of_basis", AS_OF_BASIS)
    _match(lineage.get("as_of_ns"), as_of_ns, "lineage as_of_ns")
    _match(lineage.get("universe_id"), universe_id, "lineage universe_id")
    _match(_scope(lineage.get("source_scope"), "lineage.source_scope"), scope,
           "lineage scope")
    _text(lineage, "universe_version", "lineage.universe_version")
    _text(lineage, "universe_source_reference", "lineage.universe_source_reference")
    _text(lineage, "instrument_identity_source_reference",
          "lineage.instrument_identity_source_reference")
    _text(lineage, "corporate_action_source_reference",
          "lineage.corporate_action_source_reference")
    _digest(lineage, "universe_source_sha256", "lineage.universe_source_sha256")
    _digest(lineage, "instrument_identity_source_sha256",
            "lineage.instrument_identity_source_sha256")
    _digest(lineage, "corporate_action_source_sha256",
            "lineage.corporate_action_source_sha256")
    _positive_int(lineage, "revision", "lineage.revision")
    knowledge_ns = _positive_int(lineage, "knowledge_time_ns", "lineage.knowledge_time_ns")
    if knowledge_ns > as_of_ns:
        raise DatasetAdmissionError("lineage knowledge_time_ns is later than the as-of instant")
    return basis


def validate_dataset_admission(payload: object) -> AdmissionReceipt:
    """Validate a scalar admission record; refuse every missing or denied field."""
    record = _object(payload, "admission record")
    version = record.get("contract_version")
    if type(version) is not int or version != ADMISSION_CONTRACT_VERSION:
        raise DatasetAdmissionError(
            f"admission contract_version {version!r} is not {ADMISSION_CONTRACT_VERSION}"
        )
    dataset = _object(record.get("dataset"), "dataset")
    _enum(dataset, "dataset_origin", "dataset.dataset_origin", MARKET_ORIGIN)
    dataset_id = _text(dataset, "dataset_id", "dataset.dataset_id")
    dataset_version = _text(dataset, "dataset_version", "dataset.dataset_version")
    if not dataset_version.startswith(dataset_id + ":"):
        raise DatasetAdmissionError("dataset.dataset_version must extend dataset.dataset_id")
    scope = _scope(dataset.get("source_scope"), "dataset.source_scope")
    universe_id = _text(dataset, "universe_id", "dataset.universe_id")
    as_of_ns = _positive_int(dataset, "as_of_ns", "dataset.as_of_ns")
    _enum(dataset, "as_of_basis", "dataset.as_of_basis", AS_OF_BASIS)
    manifest_sha256 = _digest(dataset, "manifest_sha256", "dataset.manifest_sha256")
    data_sha256 = _digest(dataset, "data_sha256", "dataset.data_sha256")
    as_of_date = _utc_date_from_ns(as_of_ns)
    rights_reference = _validate_rights(record, scope, as_of_date)
    certificate = _validate_coverage(record, scope, universe_id, as_of_ns, as_of_date)
    release_id = _validate_release(record, dataset_id, dataset_version,
                                   manifest_sha256, data_sha256)
    basis = _validate_lineage(record, scope, universe_id, as_of_ns)
    return AdmissionReceipt(
        contract_version=version,
        dataset_id=dataset_id,
        dataset_version=dataset_version,
        universe_id=universe_id,
        release_id=release_id,
        manifest_sha256=manifest_sha256,
        data_sha256=data_sha256,
        rights_record_reference=rights_reference,
        coverage_certificate_reference=certificate,
        lineage_basis=basis,
        as_of_ns=as_of_ns,
    )


def load_admission(path: str | Path) -> object:
    """Read one local admission record without following a redirected path."""
    target = Path(path)
    if target.is_symlink() or not target.is_file():
        raise DatasetAdmissionError("admission record is missing or redirected")
    try:
        return json.loads(target.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise DatasetAdmissionError("admission record is unreadable or not valid JSON") from exc


def admit_published_dataset(dataset_dir: str | Path, admission_path: str | Path
                            ) -> dict[str, Any]:
    """Bind a validated admission record to the exact published release."""
    from .experiments.reproducibility import bind_published_dataset
    from .point_in_time import verify_published_dataset

    receipt = validate_dataset_admission(load_admission(admission_path))
    root = Path(dataset_dir)
    if root.is_symlink() or not root.is_dir():
        raise DatasetAdmissionError("published dataset directory is missing or redirected")
    verification = verify_published_dataset(root)
    if verification.get("status") != "verified_internal_integrity":
        raise DatasetAdmissionError("published release failed internal integrity verification")
    binding = bind_published_dataset(root)
    if binding.get("dataset_origin") != MARKET_ORIGIN:
        raise DatasetAdmissionError("a synthetic dataset cannot be admitted for research")
    expected = {
        "dataset_id": receipt.dataset_id,
        "dataset_version": receipt.dataset_version,
        "dataset_manifest_sha256": receipt.manifest_sha256,
    }
    for name, value in expected.items():
        if binding.get(name) != value:
            raise DatasetAdmissionError(f"published release {name} differs from the record")
    lineage = binding.get("dataset_lineage")
    if not isinstance(lineage, Mapping) or lineage.get("data_sha256") != receipt.data_sha256:
        raise DatasetAdmissionError("published release data_sha256 differs from the record")
    return {**receipt.as_dict(), "verification_status": verification["status"]}
