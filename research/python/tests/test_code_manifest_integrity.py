# SPDX-License-Identifier: Apache-2.0
"""Abstract code-manifest integrity tests over arbitrary text and binary bytes.

Every root is a pytest temporary directory containing plain files. No market
observation, provider contact, dataset, experiment or rights record is used.
"""
from __future__ import annotations

import hashlib
import platform
import subprocess
from pathlib import Path

import pytest

from quantlab.experiments import ExperimentError
from quantlab.experiments.reproducibility import (
    code_manifest_digest,
    code_manifest_entries,
    environment_digest,
    verify_code_manifest,
)
from quantlab.research_jobs import assert_pinned_environment

TEXT_NAME = "notes.txt"
BINARY_NAME = "blob.bin"
CONSTRAINT_NAME = "constraints.txt"
TEXT_BYTES = b"alpha\nbeta\n"
BINARY_BYTES = bytes((0, 1, 2, 254, 255, 10))


def _write(root: Path, relative: str, payload: bytes) -> Path:
    """Write one arbitrary byte payload under a root and return its path."""
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(payload)
    return path


def _fixture(root: Path) -> tuple[list[str], Path]:
    """Create text, binary and constraint files in one root."""
    names = [TEXT_NAME, BINARY_NAME]
    _write(root, TEXT_NAME, TEXT_BYTES)
    _write(root, BINARY_NAME, BINARY_BYTES)
    constraint_text = f"# python {platform.python_version()}\njsonschema==4.26.0\n"
    constraint = _write(root, CONSTRAINT_NAME, constraint_text.encode("utf-8"))
    return names, constraint


def _git(root: Path, *args: str) -> str:
    """Run one Git command in a temporary repository and return its output."""
    result = subprocess.run(["git", "-C", str(root), *args], check=True,
                            capture_output=True, text=True)
    return result.stdout.strip()


def _committed_fixture(root: Path) -> tuple[dict, dict, Path]:
    """Commit an arbitrary-file fixture so exact-commit verification can pass."""
    names, constraint = _fixture(root)
    _git(root, "init", "-q")
    _git(root, "add", ".")
    _git(root, "-c", "user.email=research.invalid", "-c", "user.name=Nirnay",
         "commit", "-q", "-m", "pin-arbitrary-files")
    environment = {
        "schema_version": 1,
        "code_paths": names,
        "code_digest": environment_digest(names, root=root),
        "python_version": platform.python_version(),
        "python_constraint_file": CONSTRAINT_NAME,
        "constraint_sha256": hashlib.sha256(constraint.read_bytes()).hexdigest(),
        "commit": _git(root, "rev-parse", "HEAD"),
    }
    job = {"code_digest": environment["code_digest"], "constraint_file": CONSTRAINT_NAME}
    return environment, job, constraint


def test_same_bytes_same_paths_yield_same_digest(tmp_path: Path) -> None:
    """Identical bytes under identical relative paths digest identically."""
    names, _ = _fixture(tmp_path)
    mirror = tmp_path / "mirror"
    _fixture(mirror)
    assert environment_digest(names, root=tmp_path) == environment_digest(names, root=mirror)


def test_one_byte_change_changes_digest(tmp_path: Path) -> None:
    """A single changed byte in an arbitrary binary file changes the digest."""
    names, _ = _fixture(tmp_path)
    before = environment_digest(names, root=tmp_path)
    changed = BINARY_BYTES[:-1] + bytes((BINARY_BYTES[-1] ^ 1,))
    _write(tmp_path, BINARY_NAME, changed)
    assert environment_digest(names, root=tmp_path) != before


def test_moved_file_changes_digest(tmp_path: Path) -> None:
    """Identical bytes at a different repository-relative path change the digest."""
    names, _ = _fixture(tmp_path)
    before = environment_digest(names, root=tmp_path)
    moved = "nested/" + TEXT_NAME
    _write(tmp_path, moved, TEXT_BYTES)
    (tmp_path / TEXT_NAME).unlink()
    assert environment_digest([moved, BINARY_NAME], root=tmp_path) != before


def test_entry_order_is_canonical(tmp_path: Path) -> None:
    """Declaration order does not affect the canonical path-sorted digest."""
    names, _ = _fixture(tmp_path)
    assert environment_digest(names, root=tmp_path) == environment_digest(
        list(reversed(names)), root=tmp_path
    )


def test_duplicate_normalized_path_rejected(tmp_path: Path) -> None:
    """A path repeated in a different but equivalent spelling is refused."""
    names, _ = _fixture(tmp_path)
    with pytest.raises(ExperimentError, match="unique"):
        environment_digest([names[0], "./" + names[0]], root=tmp_path)


def test_missing_file_rejected(tmp_path: Path) -> None:
    """A declared path with no file on disk is refused."""
    names, _ = _fixture(tmp_path)
    with pytest.raises(ExperimentError, match="missing"):
        environment_digest([*names, "absent.txt"], root=tmp_path)


def test_symlink_file_and_symlink_component_rejected(tmp_path: Path) -> None:
    """A symlink file and a symlinked directory component are both refused."""
    _fixture(tmp_path)
    (tmp_path / "alias.txt").symlink_to(tmp_path / TEXT_NAME)
    (tmp_path / "alias-dir").symlink_to(tmp_path, target_is_directory=True)
    for candidate in ("alias.txt", "alias-dir/" + TEXT_NAME):
        with pytest.raises(ExperimentError, match="symlink"):
            environment_digest([candidate], root=tmp_path)


def test_escape_root_and_traversal_rejected(tmp_path: Path) -> None:
    """A relative path that resolves outside the root is refused."""
    _fixture(tmp_path)
    with pytest.raises(ExperimentError, match=r"traversal|escapes"):
        environment_digest(["nested/../../outside.txt"], root=tmp_path)


def test_absolute_path_rejected(tmp_path: Path) -> None:
    """An absolute declaration is refused even when it points inside the root."""
    _fixture(tmp_path)
    with pytest.raises(ExperimentError, match="repository-relative"):
        environment_digest([str(tmp_path / TEXT_NAME)], root=tmp_path)


def test_directory_as_file_rejected(tmp_path: Path) -> None:
    """A directory declared as a file is refused."""
    _fixture(tmp_path)
    (tmp_path / "folder").mkdir()
    with pytest.raises(ExperimentError, match="regular file"):
        environment_digest(["folder"], root=tmp_path)


def test_unsupported_schema_version_rejected(tmp_path: Path) -> None:
    """Only the code-defined manifest schema version is accepted."""
    names, _ = _fixture(tmp_path)
    entries = code_manifest_entries(names, root=tmp_path)
    assert code_manifest_digest(entries) == environment_digest(names, root=tmp_path)
    with pytest.raises(ExperimentError, match="unsupported"):
        environment_digest(names, root=tmp_path, schema_version=2)
    with pytest.raises(ExperimentError, match="unsupported"):
        code_manifest_digest(entries, schema_version=2)


def test_verification_recomputes_claims_from_disk(tmp_path: Path) -> None:
    """The gate recomputes the digest instead of trusting the declared claim."""
    environment, job, _ = _committed_fixture(tmp_path)
    verified = verify_code_manifest(environment, job, root=tmp_path)
    assert verified["code_digest"] == environment["code_digest"]
    assert assert_pinned_environment(environment, job, root=tmp_path)["code_digest"] == (
        environment["code_digest"]
    )


def test_verification_rejects_forged_claims(tmp_path: Path) -> None:
    """Forged digests, pins, runtime, commit and version all fail closed."""
    environment, job, _ = _committed_fixture(tmp_path)
    cases = [
        (dict(environment, code_digest="0" * 64), job, "digest mismatch"),
        (environment, dict(job, code_digest="0" * 64), "unverified"),
        (dict(environment, constraint_sha256="0" * 64), job, "constraint file digest"),
        (dict(environment, python_version="9.9.9"), job,
         r"not pinned|runtime Python version"),
        (dict(environment, commit="0" * 40), job, "exact HEAD"),
        (dict(environment, schema_version=2), job, "unsupported"),
    ]
    for candidate, candidate_job, message in cases:
        with pytest.raises(ExperimentError, match=message):
            verify_code_manifest(candidate, candidate_job, root=tmp_path)


def test_verification_rejects_dirty_source_tree(tmp_path: Path) -> None:
    """An exact commit pin is refused while the working tree has changes."""
    environment, job, _ = _committed_fixture(tmp_path)
    _write(tmp_path, "untracked-notes.md", b"untracked\n")
    with pytest.raises(ExperimentError, match="dirty"):
        verify_code_manifest(environment, job, root=tmp_path)
