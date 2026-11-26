#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Fail-closed verifier for the declared research code digest.

``configs/research/jobs.v1.json`` declares ``environment.code_digest`` over
``environment.code_paths``. This gate binds that declaration to an explicit
source scope: every regular ``.py`` file below
``research/python/src/quantlab/``. The declared path list must equal that
scope exactly, every declared file must be readable and safe, and the digest
must equal the versioned, length-framed hash recomputed from those files with
the same algorithm as ``quantlab.experiments.reproducibility.environment_digest``.

Mismatch, a missing or unbindable file, an unbound path, an omitted bound file
and a malformed digest are all findings. The gate is binding through
``make research-code-digest-check``, ``make check`` and CI. It only proves
source identity: it does not register, schedule or authorize an experiment,
and the manifest's ``jobs`` array stays empty.

Run ``--self-test`` for adversarial unit tests.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import stat
import sys
import tempfile
import unittest
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "configs" / "research" / "jobs.v1.json"
BOUND_SOURCE_PREFIX = "research/python/src/quantlab/"
BOUND_SOURCE_SUFFIX = ".py"
DIGEST_DOMAIN = b"NIRNAY-RESEARCH-CODE-MANIFEST-V1\x00"
DIGEST_SCHEMA_VERSION = 1
DIGEST_FIELDS = ("path", "file_type", "mode", "size", "sha256")
DIGEST_PATTERN = re.compile(r"[0-9a-f]{64}")
EXECUTABLE_BITS = stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH


def load_manifest(root: Path = ROOT) -> dict[str, object]:
    """Load the checked-in research job manifest as a JSON object."""
    payload = json.loads((root / MANIFEST.relative_to(ROOT)).read_text(encoding="utf-8"))
    if not isinstance(payload, dict):
        raise ValueError("research job manifest must be a JSON object")
    return payload


def framed(value: str) -> bytes:
    """Return one length-framed UTF-8 field for structured hashing."""
    raw = value.encode("utf-8")
    return len(raw).to_bytes(8, "big") + raw


def canonical_path(item: object, root: Path) -> str:
    """Return a repository-relative POSIX path after rejecting unsafe forms."""
    if not isinstance(item, str):
        raise ValueError(f"declared source path is not a string: {item!r}")
    if not item or "\\" in item or "\x00" in item or item.startswith("/"):
        raise ValueError(f"declared source path is not repository-relative POSIX: {item!r}")
    if re.match(r"^[A-Za-z]:", item) or Path(item).is_absolute():
        raise ValueError(f"declared source path is not repository-relative POSIX: {item!r}")
    parts = PurePosixPath(item).parts
    if ".." in parts:
        raise ValueError(f"declared source path contains '..' traversal: {item!r}")
    relative = PurePosixPath(*parts)
    canonical = relative.as_posix()
    if canonical in ("", "."):
        raise ValueError("declared source path does not name a file")
    base = root.resolve()
    probe = base
    for part in relative.parts:
        probe = probe / part
        if probe.is_symlink():
            raise ValueError(f"declared source path has a symlink component: {canonical}")
    target = (base / Path(*relative.parts)).resolve()
    if not target.is_relative_to(base):
        raise ValueError(f"declared source path escapes the repository root: {canonical}")
    return canonical


def source_entry(item: object, root: Path) -> dict[str, str | int]:
    """Validate and hash one declared source file into a manifest entry."""
    path = canonical_path(item, root)
    if not (
        path.startswith(BOUND_SOURCE_PREFIX)
        and path.endswith(BOUND_SOURCE_SUFFIX)
        and len(path) > len(BOUND_SOURCE_PREFIX)
    ):
        raise ValueError(f"declared source path is not a bound quantlab source file: {path}")
    target = root / Path(*PurePosixPath(path).parts)
    try:
        info = target.lstat()
    except OSError as error:
        raise ValueError(f"declared source path is missing: {path}") from error
    if not stat.S_ISREG(info.st_mode) or target.is_symlink():
        raise ValueError(f"declared source path is not a regular file: {path}")
    payload = target.read_bytes()
    if len(payload) != info.st_size:
        raise ValueError(f"declared source path changed size while reading: {path}")
    mode = "executable" if info.st_mode & EXECUTABLE_BITS else "regular"
    return {
        "path": path,
        "file_type": "regular",
        "mode": mode,
        "size": info.st_size,
        "sha256": hashlib.sha256(payload).hexdigest(),
    }


def digest_for_entries(entries: list[dict[str, str | int]]) -> str:
    """Hash canonical entries with domain separation and explicit framing."""
    digest = hashlib.sha256()
    digest.update(DIGEST_DOMAIN)
    digest.update(framed("schema_version"))
    digest.update(framed(str(DIGEST_SCHEMA_VERSION)))
    digest.update(framed("entry_count"))
    digest.update(framed(str(len(entries))))
    for entry in entries:
        if set(entry) != set(DIGEST_FIELDS):
            raise ValueError("code manifest entry has an invalid shape")
        for field in DIGEST_FIELDS:
            digest.update(framed(field))
            digest.update(framed(str(entry[field])))
    return digest.hexdigest()


def compute_digest(paths: list[str], *, root: Path) -> str:
    """Recompute the versioned code digest for declared source paths."""
    if not paths:
        raise ValueError("code manifest declares no paths")
    entries = [source_entry(item, root) for item in paths]
    seen: set[str] = set()
    for entry in entries:
        path = str(entry["path"])
        if path in seen:
            raise ValueError(f"declared source paths must be unique: {path}")
        seen.add(path)
    ordered = sorted(entries, key=lambda entry: str(entry["path"]).encode("utf-8"))
    return digest_for_entries(ordered)


def bound_sources(root: Path) -> list[str]:
    """List every regular ``.py`` file under the explicitly bound source prefix."""
    base = root / Path(*PurePosixPath(BOUND_SOURCE_PREFIX).parts)
    if base.is_symlink():
        raise ValueError(f"bound source prefix is a symlink: {BOUND_SOURCE_PREFIX}")
    if not base.is_dir():
        raise ValueError(f"bound source prefix is missing: {BOUND_SOURCE_PREFIX}")
    found: list[str] = []
    for directory, subdirs, filenames in os.walk(base, followlinks=False):
        subdirs[:] = [name for name in subdirs if not (Path(directory) / name).is_symlink()]
        for filename in filenames:
            candidate = Path(directory) / filename
            if not filename.endswith(BOUND_SOURCE_SUFFIX):
                continue
            if candidate.is_symlink() or not candidate.is_file():
                continue
            found.append(candidate.relative_to(root).as_posix())
    return sorted(found)


def check_manifest(payload: object, root: Path) -> list[str]:
    """Return all findings for one manifest payload against ``root``."""
    if not isinstance(payload, dict):
        return ["research job manifest must be a JSON object"]
    environment = payload.get("environment")
    if not isinstance(environment, dict):
        return ["research job manifest environment must be a JSON object"]
    errors: list[str] = []
    path_errors: list[str] = []
    declared = environment.get("code_paths")
    digest = environment.get("code_digest")
    if not isinstance(declared, list) or not declared:
        path_errors.append("environment.code_paths must be a non-empty list")
        declared = []
    if not isinstance(digest, str) or DIGEST_PATTERN.fullmatch(digest) is None:
        errors.append("environment.code_digest is malformed (expected 64 lowercase hex digits)")
        digest = ""
    paths: list[str] = []
    for item in declared:
        try:
            paths.append(canonical_path(item, root))
            source_entry(item, root)
        except (OSError, ValueError) as error:
            path_errors.append(str(error))
    try:
        bound = bound_sources(root)
    except (OSError, ValueError) as error:
        path_errors.append(str(error))
        bound = []
    if len(set(paths)) != len(paths):
        path_errors.append("environment.code_paths contains duplicate paths")
    if paths and bound:
        for path in sorted(set(paths) - set(bound)):
            path_errors.append(f"declared path is not a bound quantlab source file: {path}")
        for path in sorted(set(bound) - set(paths)):
            path_errors.append(f"bound source file is not declared: {path}")
    errors.extend(path_errors)
    if paths and digest and not path_errors:
        try:
            actual = compute_digest(paths, root=root)
        except (OSError, ValueError) as error:
            errors.append(str(error))
        else:
            if actual != digest:
                errors.append(f"code digest mismatch: declared {digest}, recomputed {actual}")
    return sorted(errors)


def check(root: Path = ROOT) -> list[str]:
    """Load the checked-in manifest and return all digest findings."""
    try:
        payload = load_manifest(root)
    except (OSError, json.JSONDecodeError, ValueError) as error:
        return [f"research job manifest could not be read: {error}"]
    return check_manifest(payload, root)


class CodeDigestGateTests(unittest.TestCase):
    """Adversarial tests for the declared code digest gate."""

    def setUp(self) -> None:
        """Build a small bound source tree and a matching manifest payload."""
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        (self.root / BOUND_SOURCE_PREFIX).mkdir(parents=True)
        self.first = self.write_source("alpha.py", b"x = 1\n")
        self.second = self.write_source("beta.py", b"y = 2\n")
        self.paths = [self.first, self.second]
        self.digest = compute_digest(self.paths, root=self.root)
        self.payload: dict[str, object] = {
            "schema_version": 1,
            "environment": {"code_paths": list(self.paths), "code_digest": self.digest},
            "jobs": [],
        }

    def write_source(self, name: str, payload: bytes) -> str:
        """Write one bound source file and return its repository-relative path."""
        relative = BOUND_SOURCE_PREFIX + name
        (self.root / relative).write_bytes(payload)
        return relative

    def findings(self) -> list[str]:
        """Run the manifest check against the temporary tree."""
        return check_manifest(self.payload, self.root)

    def test_matching_digest_passes(self) -> None:
        """A digest over exactly the bound files passes."""
        self.assertEqual(self.findings(), [])

    def test_one_byte_source_change_fails(self) -> None:
        """Changing one byte of a bound source is a digest mismatch."""
        (self.root / self.first).write_bytes(b"x = 9\n")
        self.assertTrue(any("code digest mismatch" in item for item in self.findings()))

    def test_missing_bound_file_fails(self) -> None:
        """A declared source that is gone fails closed."""
        (self.root / self.second).unlink()
        self.assertTrue(any("missing" in item for item in self.findings()))

    def test_malformed_digest_fails(self) -> None:
        """Non-hex, short, uppercase and non-string digests are rejected."""
        for candidate in ("0" * 63, "A" * 64, "g" * 64, 12, ""):
            with self.subTest(candidate=candidate):
                environment = self.payload["environment"]
                assert isinstance(environment, dict)
                environment["code_digest"] = candidate
                self.assertTrue(any("malformed" in item for item in self.findings()))

    def test_unbound_path_fails(self) -> None:
        """A digest that claims a path outside the bound prefix fails."""
        outside = "crates/example.rs"
        (self.root / "crates").mkdir(exist_ok=True)
        (self.root / outside).write_text("fn main() {}", encoding="utf-8")
        environment = self.payload["environment"]
        assert isinstance(environment, dict)
        environment["code_paths"] = [self.first, outside]
        findings = self.findings()
        self.assertTrue(any("not a bound quantlab source file" in item for item in findings))

    def test_undeclared_bound_file_fails(self) -> None:
        """A new bound source omitted from the declaration fails closed."""
        self.write_source("gamma.py", b"z = 3\n")
        self.assertTrue(any("not declared" in item for item in self.findings()))

    def test_traversal_path_fails(self) -> None:
        """A path escaping the repository root is unbindable."""
        environment = self.payload["environment"]
        assert isinstance(environment, dict)
        environment["code_paths"] = [self.first, "../outside.py"]
        self.assertTrue(any("traversal" in item for item in self.findings()))

    def test_checked_in_manifest_matches_bound_source(self) -> None:
        """The checked-in digest must match the checked-in bound source tree."""
        self.assertEqual(check(ROOT), [])


def main() -> int:
    """Report digest findings and return a process exit status."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(CodeDigestGateTests)
        result = unittest.TextTestRunner(verbosity=1).run(suite)
        return 0 if result.wasSuccessful() else 1
    errors = check()
    for error in errors:
        print(f"[FAIL] {error}")
    if errors:
        print(f"research code digest: {len(errors)} finding(s)", file=sys.stderr)
        return 1
    manifest = load_manifest()
    environment = manifest["environment"]
    assert isinstance(environment, dict)
    print(f"research code digest: {len(bound_sources(ROOT))} bound source file(s) verified")
    print(f"  digest: {environment['code_digest']}")
    print("  scope: all *.py files under " + BOUND_SOURCE_PREFIX)
    print("  source identity only; no experiment is registered or authorized")
    return 0


if __name__ == "__main__":
    sys.exit(main())
