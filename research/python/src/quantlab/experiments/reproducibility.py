"""Reproducibility helpers for binding declared code, data and reports.

Digests here generalize the source-code digest used by the point-in-time
builder over an explicit list of declared paths. Nothing in this module reads
provider data or contacts a live service.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any

from ..data import (dataset_identity_from_manifest, dataset_lineage_from_manifest,
                    dataset_origin_from_manifest)
from ..market import canonical_bytes
from .schema import ExperimentError, content_hash


def environment_digest(paths: list[str | Path], *, root: str | Path | None = None) -> str:
    """Digest the exact bytes of an ordered, deduplicated set of source files."""
    digest = hashlib.sha256()
    base = Path(root).resolve() if root is not None else Path.cwd().resolve()
    resolved: list[Path] = []
    for item in paths:
        candidate = Path(item)
        target = candidate if candidate.is_absolute() else base / candidate
        resolved.append(target.resolve())
    if len(set(resolved)) != len(resolved):
        raise ExperimentError("declared source paths must be unique")
    for path in sorted(resolved, key=str):
        if path.is_symlink() or not path.is_file():
            raise ExperimentError(f"declared source path is not a regular file: {path}")
        digest.update(path.name.encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def strategy_code_digest(paths: list[str | Path], *, root: str | Path | None = None) -> str:
    """Digest declared strategy and feature source files for a record binding."""
    return environment_digest(paths, root=root)


def bind_dataset(manifest: dict[str, Any] | Any, *, artifacts: dict[str, str] | None = None,
                 publication: str = "atomic_directory_rename") -> dict[str, Any]:
    """Return the dataset identity and lineage block for an experiment record."""
    body = manifest.as_dict() if hasattr(manifest, "as_dict") else manifest
    identity = dataset_identity_from_manifest(body)
    payload = {"dataset": body, "artifacts": artifacts or {}, "publication": publication}
    manifest_digest = hashlib.sha256(canonical_bytes(payload)).hexdigest()
    return {**identity, "dataset_origin": dataset_origin_from_manifest(body),
            "dataset_manifest_sha256": manifest_digest,
            "dataset_lineage": dataset_lineage_from_manifest(body)}


def bind_published_dataset(dataset_dir: str | Path) -> dict[str, Any]:
    """Bind a record to the exact published manifest, including artifact hashes."""
    root = Path(dataset_dir)
    manifest_path = root / "manifest.json"
    if manifest_path.is_symlink() or not manifest_path.is_file():
        raise ExperimentError("dataset publication manifest is missing or redirected")
    try:
        payload = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ExperimentError("dataset publication manifest is unreadable") from exc
    if not isinstance(payload, dict) or not isinstance(payload.get("dataset"), dict):
        raise ExperimentError("dataset publication manifest has an invalid shape")
    body = payload["dataset"]
    identity = dataset_identity_from_manifest(body)
    return {**identity, "dataset_origin": dataset_origin_from_manifest(body),
            "dataset_manifest_sha256": content_hash(payload),
            "dataset_lineage": dataset_lineage_from_manifest(body)}


def recompute_report_hash_from_record(report: dict[str, Any]) -> str:
    """Recompute a search report digest after canonical round-tripping."""
    if not isinstance(report, dict):
        raise ExperimentError("search report must be an object")
    body = {key: value for key, value in report.items() if key != "report_sha256"}
    return content_hash(body)


def assert_report_hash(report: dict[str, Any]) -> None:
    """Refuse a search report whose embedded digest does not recompute."""
    expected = report.get("report_sha256") if isinstance(report, dict) else None
    if expected != recompute_report_hash_from_record(report):
        raise ExperimentError("search report digest mismatch")
