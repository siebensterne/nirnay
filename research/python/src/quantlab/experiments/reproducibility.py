"""Reproducibility helpers for binding declared code, data and reports.

The code-manifest digest is versioned and path-safe: it commits to canonical
repository-relative paths, regular-file type, executable bit, exact byte length
and the SHA-256 of the exact bytes. The same canonical function produces and
verifies the digest. Nothing in this module reads provider data or contacts a
live service.
"""
from __future__ import annotations

import hashlib
import json
import platform
import re
import stat
import subprocess
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Any

from ..data import (
    dataset_identity_from_manifest,
    dataset_lineage_from_manifest,
    dataset_origin_from_manifest,
)
from ..market import canonical_bytes
from .schema import ExperimentError, content_hash

CODE_MANIFEST_SCHEMA_VERSION = 1
_CODE_MANIFEST_DOMAIN = b"NIRNAY-RESEARCH-CODE-MANIFEST-V1\x00"
_MANIFEST_FIELDS = ("path", "file_type", "mode", "size", "sha256")


@dataclass(frozen=True, slots=True)
class _SourceFile:
    """One validated regular file and the exact bytes read from it."""

    path: str
    payload: bytes
    size: int
    mode: str


def _framed(value: str) -> bytes:
    """Return one length-framed UTF-8 field for structured hashing."""
    raw = value.encode("utf-8")
    return len(raw).to_bytes(8, "big") + raw


def _safe_path(item: str | Path, root: Path) -> tuple[str, Path]:
    """Validate a repository-relative POSIX path before any resolution."""
    text = item.as_posix() if isinstance(item, Path) else str(item)
    if (not text or "\\" in text or "\x00" in text or text.startswith("/")
            or re.match(r"^[A-Za-z]:", text) or Path(text).is_absolute()):
        raise ExperimentError(f"declared source path is not repository-relative POSIX: {text!r}")
    parts = PurePosixPath(text).parts
    if ".." in parts:
        raise ExperimentError(f"declared source path contains '..' traversal: {text!r}")
    relative = PurePosixPath(*parts)
    canonical = relative.as_posix()
    if canonical in ("", "."):
        raise ExperimentError("declared source path does not name a file")
    probe = root
    for part in relative.parts:
        probe = probe / part
        if probe.is_symlink():
            raise ExperimentError(f"declared source path has a symlink component: {canonical}")
    target = (root / Path(*relative.parts)).resolve()
    if not target.is_relative_to(root):
        raise ExperimentError(f"declared source path escapes the repository root: {canonical}")
    return canonical, target


def _validated_source(item: str | Path, root: Path) -> _SourceFile:
    """Read one declared source file after rejecting unsafe paths and file types."""
    path, target = _safe_path(item, root)
    try:
        info = target.lstat()
    except OSError as exc:
        raise ExperimentError(f"declared source path is missing: {path}") from exc
    if not stat.S_ISREG(info.st_mode) or target.is_symlink():
        raise ExperimentError(f"declared source path is not a regular file: {path}")
    try:
        payload = target.read_bytes()
    except OSError as exc:
        raise ExperimentError(f"declared source path is unreadable: {path}") from exc
    if len(payload) != info.st_size:
        raise ExperimentError(f"declared source path changed size while reading: {path}")
    executable = stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH
    mode = "executable" if info.st_mode & executable else "regular"
    return _SourceFile(path=path, payload=payload, size=info.st_size, mode=mode)


def _entry(source: _SourceFile) -> dict[str, str | int]:
    """Render a validated source file as a canonical manifest entry."""
    return {"path": source.path, "file_type": "regular", "mode": source.mode,
            "size": source.size, "sha256": hashlib.sha256(source.payload).hexdigest()}


def code_manifest_entries(paths: list[str | Path], *,
                          root: str | Path) -> list[dict[str, str | int]]:
    """Validate declared paths and return entries ordered by UTF-8 path bytes."""
    if not paths:
        raise ExperimentError("code manifest declares no paths")
    base = Path(root).resolve()
    entries = [_entry(_validated_source(item, base)) for item in paths]
    seen: set[str] = set()
    for entry in entries:
        path = str(entry["path"])
        if path in seen:
            raise ExperimentError("declared source paths must be unique")
        seen.add(path)
    return sorted(entries, key=lambda entry: str(entry["path"]).encode("utf-8"))


def code_manifest_digest(entries: list[dict[str, str | int]], *,
                         schema_version: int = CODE_MANIFEST_SCHEMA_VERSION) -> str:
    """Hash canonical entries with domain separation and explicit length framing."""
    if schema_version != CODE_MANIFEST_SCHEMA_VERSION:
        raise ExperimentError(f"unsupported code manifest schema version: {schema_version}")
    digest = hashlib.sha256()
    digest.update(_CODE_MANIFEST_DOMAIN)
    digest.update(_framed("schema_version"))
    digest.update(_framed(str(schema_version)))
    digest.update(_framed("entry_count"))
    digest.update(_framed(str(len(entries))))
    for entry in entries:
        if set(entry) != set(_MANIFEST_FIELDS):
            raise ExperimentError("code manifest entry has an invalid shape")
        for field in _MANIFEST_FIELDS:
            digest.update(_framed(field))
            digest.update(_framed(str(entry[field])))
    return digest.hexdigest()


def environment_digest(paths: list[str | Path], *, root: str | Path | None = None,
                       schema_version: int = CODE_MANIFEST_SCHEMA_VERSION) -> str:
    """Generate the versioned code-manifest digest from declared source paths."""
    base = root if root is not None else Path.cwd()
    entries = code_manifest_entries(paths, root=base)
    return code_manifest_digest(entries, schema_version=schema_version)


def _git_output(root: Path, *args: str) -> str:
    """Run a read-only Git query and return its stripped standard output."""
    try:
        result = subprocess.run(["git", "-C", str(root), *args], capture_output=True,
                                text=True, check=False)
    except OSError as exc:
        raise ExperimentError("repository commit verification is unavailable") from exc
    if result.returncode != 0:
        raise ExperimentError(f"repository commit verification failed: {' '.join(args)}")
    return result.stdout.strip()


def _is_ancestor(root: Path, commit: str) -> bool:
    """Report whether a declared commit is in the repository's HEAD history."""
    try:
        result = subprocess.run(["git", "-C", str(root), "merge-base", "--is-ancestor",
                                 commit, "HEAD"], capture_output=True, check=False)
    except OSError:
        return False
    return result.returncode == 0


def verify_code_manifest(environment: dict[str, Any], job: dict[str, Any] | None = None, *,
                         root: str | Path, require_exact_commit: bool = True,
                         runtime_version: str | None = None) -> dict[str, str]:
    """Recompute the code-manifest pin from disk and refuse unverified claims."""
    base = Path(root).resolve()
    version = environment.get("schema_version", CODE_MANIFEST_SCHEMA_VERSION)
    if version != CODE_MANIFEST_SCHEMA_VERSION:
        raise ExperimentError("unsupported code manifest schema version")
    paths = environment.get("code_paths")
    if not isinstance(paths, list) or not paths:
        raise ExperimentError("code manifest declares no paths")
    declared_digest = environment.get("code_digest")
    if not isinstance(declared_digest, str) or not re.fullmatch(r"[0-9a-f]{64}", declared_digest):
        raise ExperimentError("declared code digest is malformed")
    digest = environment_digest(paths, root=base)
    if digest != declared_digest:
        raise ExperimentError("code manifest digest mismatch")

    constraint = environment.get("python_constraint_file")
    if not isinstance(constraint, str) or not constraint:
        raise ExperimentError("declared constraint file is malformed")
    constraint_file = _validated_source(constraint, base)
    constraint_sha256 = hashlib.sha256(constraint_file.payload).hexdigest()
    if constraint_sha256 != environment.get("constraint_sha256"):
        raise ExperimentError("constraint file digest mismatch")
    try:
        text = constraint_file.payload.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise ExperimentError("constraint file is not UTF-8") from exc
    declared_version = environment.get("python_version")
    if not isinstance(declared_version, str) or not re.fullmatch(r"\d+\.\d+(?:\.\d+)?",
                                                                declared_version):
        raise ExperimentError("declared Python version is malformed")
    if declared_version not in text:
        raise ExperimentError("Python version is not pinned in the constraint file")
    for number, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if "==" not in stripped:
            raise ExperimentError(
                f"unpinned environment: dependency at line {number} is not pinned"
            )
    runtime = runtime_version if runtime_version is not None else platform.python_version()
    if runtime != declared_version and not runtime.startswith(declared_version + "."):
        raise ExperimentError("runtime Python version does not match the declared pin")

    commit = environment.get("commit")
    if not isinstance(commit, str) or not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ExperimentError("declared repository commit is missing or malformed")
    head = _git_output(base, "rev-parse", "HEAD")
    if require_exact_commit:
        if head != commit:
            raise ExperimentError("declared repository commit is not the exact HEAD")
        if _git_output(base, "status", "--porcelain", "--untracked-files=normal"):
            raise ExperimentError("source tree is dirty; exact commit verification refused")
    elif not _is_ancestor(base, commit):
        raise ExperimentError("declared repository commit is not in HEAD history")

    if job is not None:
        if job.get("code_digest") != digest:
            raise ExperimentError("job code manifest digest is unverified")
        if job.get("constraint_file") != constraint:
            raise ExperimentError("job does not use the declared environment pin")
    return {"constraint_sha256": constraint_sha256, "code_digest": digest,
            "commit": commit, "runtime_version": runtime}


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
