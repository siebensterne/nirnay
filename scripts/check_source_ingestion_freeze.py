#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Fail-closed gate for the RRP-004 Nirnay source-ingestion freeze.

The checked-in manifest lists legacy acquisition, raw-archive, provider-decoder
and provider-config paths that stay in the tree but are frozen. A change under
any frozen path fails this gate unless the same manifest carries an approved
migration record whose evidence file exists in the tree.

Both tracked modifications (staged or unstaged, relative to a base ref) and
untracked files are inspected, so a new file under a frozen directory cannot
enter unnoticed. The gate is binding through ``make freeze-check`` and
``make check``. ``SOURCE_FREEZE_BASE`` selects the comparison ref; the default
is ``main``. Run ``--self-test`` for adversarial unit tests.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import unittest
from collections.abc import Callable
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "engineering" / "source-ingestion-freeze.v1.json"
BASE_ENV = "SOURCE_FREEZE_BASE"
DEFAULT_BASE = "main"
MIGRATION_ORDER = (
    "Fieldbook adapter",
    "NDC projection",
    "verify",
    "remove old importer/archive",
)
REQUIRED_FIELDS = ("record", "freeze_date", "migration_order", "no_new_feature_rule")
RECORD_FIELDS = ("path", "evidence", "approval", "replacement_commit", "date")
COMMIT_RE = re.compile(r"[0-9a-f]{7,40}")
DATE_RE = re.compile(r"\d{4}-\d{2}-\d{2}")


def load_manifest(path: Path) -> dict[str, object]:
    """Load the checked-in freeze manifest as a JSON object."""
    return json.loads(path.read_text(encoding="utf-8"))


def normalize(path: str) -> str:
    """Normalize a repository-relative path for prefix comparison."""
    return path.replace("\\", "/").strip().strip("/")


def matches_prefix(path: str, prefix: str) -> bool:
    """Report whether a path is a frozen prefix or lies beneath it."""
    target = normalize(path)
    head = normalize(prefix)
    return target == head or target.startswith(head + "/")


def frozen_prefixes(manifest: dict[str, object]) -> list[str]:
    """Return the frozen path prefixes declared by the manifest."""
    entries = manifest.get("frozen_paths")
    if not isinstance(entries, list) or not entries:
        raise ValueError("manifest has no frozen_paths entries")
    prefixes: list[str] = []
    for entry in entries:
        if not isinstance(entry, dict) or not isinstance(entry.get("path"), str):
            raise ValueError("each frozen_paths entry needs a string path")
        prefixes.append(normalize(str(entry["path"])))
    return prefixes


def record_errors(record: object) -> list[str]:
    """Return structural errors for one approved migration record."""
    if not isinstance(record, dict):
        return ["approved migration record is not an object"]
    errors: list[str] = []
    for key in RECORD_FIELDS:
        value = record.get(key)
        if not isinstance(value, str) or not value:
            errors.append(f"approved migration record missing `{key}`")
    commit = record.get("replacement_commit")
    if isinstance(commit, str) and commit and COMMIT_RE.fullmatch(commit) is None:
        errors.append("approved migration record has an invalid replacement_commit")
    date = record.get("date")
    if isinstance(date, str) and date and DATE_RE.fullmatch(date) is None:
        errors.append("approved migration record has an invalid date")
    evidence = record.get("evidence")
    if isinstance(evidence, str) and evidence and not evidence.startswith("evidence/"):
        errors.append("approved migration record evidence must live under evidence/")
    return errors


def validate_manifest(manifest: dict[str, object]) -> list[str]:
    """Return structural findings that make the manifest unfit as a gate input."""
    errors: list[str] = []
    for key in REQUIRED_FIELDS:
        if not manifest.get(key):
            errors.append(f"freeze manifest missing required field `{key}`")
    if manifest.get("migration_order") != list(MIGRATION_ORDER):
        errors.append("migration_order does not match the four RRP-004 stages")
    try:
        frozen_prefixes(manifest)
    except ValueError as error:
        errors.append(str(error))
    records = manifest.get("approved_migration_records")
    if not isinstance(records, list):
        errors.append("approved_migration_records must be a list")
    else:
        for index, record in enumerate(records):
            for error in record_errors(record):
                errors.append(f"approved_migration_records[{index}]: {error}")
    return errors


def check_changes(
    changed: list[str],
    prefixes: list[str],
    records: list[object],
    exists: Callable[[str], bool],
) -> list[str]:
    """Return findings for changed frozen paths without an approved record."""
    errors: list[str] = []
    for raw in changed:
        path = normalize(raw)
        if not any(matches_prefix(path, prefix) for prefix in prefixes):
            continue
        authorized = False
        for record in records:
            if not isinstance(record, dict):
                continue
            target = record.get("path")
            if not isinstance(target, str) or not matches_prefix(path, target):
                continue
            if record_errors(record):
                continue
            evidence = str(record.get("evidence", ""))
            if exists(evidence):
                authorized = True
                break
        if not authorized:
            errors.append(
                f"frozen path `{raw}` changed without an approved migration record"
            )
    return sorted(errors)


def git_lines(root: Path, args: list[str]) -> list[str]:
    """Run a read-only git command and return its stdout lines."""
    result = subprocess.run(
        ["git", "-C", str(root), *args],
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        detail = result.stderr.decode("utf-8", errors="replace").strip()
        raise RuntimeError(f"git {' '.join(args)} failed: {detail}")
    return [
        line
        for line in result.stdout.decode("utf-8", errors="replace").splitlines()
        if line
    ]


def resolve_base(root: Path, base: str) -> str:
    """Resolve a local or remote-tracking ref; CI checkouts have no local main."""
    for candidate in (base, f"origin/{base}", f"refs/remotes/origin/{base}"):
        result = subprocess.run(
            ["git", "rev-parse", "--verify", "--quiet", f"{candidate}^{{commit}}"],
            cwd=root, capture_output=True, text=True, check=False,
        )
        if result.returncode == 0:
            return candidate
    raise ValueError(f"freeze comparison ref not found: {base} (tried origin/{base})")


def changed_paths(root: Path, base: str) -> list[str]:
    """Return tracked, staged and untracked paths changed relative to ``base``."""
    resolved = resolve_base(root, base)
    modified = git_lines(root, ["diff", "--name-only", resolved, "--"])
    untracked = git_lines(root, ["ls-files", "--others", "--exclude-standard"])
    return sorted(set(modified) | set(untracked))


def check(root: Path, base: str) -> list[str]:
    """Run the freeze gate and return all findings."""
    if not MANIFEST.is_file():
        return ["freeze manifest is missing"]
    try:
        manifest = load_manifest(MANIFEST)
    except (OSError, json.JSONDecodeError) as error:
        return [f"freeze manifest could not be read: {error}"]
    if not isinstance(manifest, dict):
        return ["freeze manifest must be a JSON object"]
    errors = validate_manifest(manifest)
    try:
        prefixes = frozen_prefixes(manifest)
    except ValueError as error:
        errors.append(str(error))
        prefixes = []
    records = manifest.get("approved_migration_records")
    if not isinstance(records, list):
        records = []
    try:
        paths_changed = changed_paths(root, base)
    except ValueError as error:
        errors.append(str(error))
        return sorted(errors)
    errors.extend(
        check_changes(
            paths_changed,
            prefixes,
            records,
            lambda evidence: (root / evidence).is_file(),
        )
    )
    return sorted(errors)


class FreezeGateTests(unittest.TestCase):
    """Adversarial tests for the freeze gate helpers."""

    def setUp(self) -> None:
        """Build a minimal manifest for each test."""
        self.manifest: dict[str, object] = {
            "record": "evidence/SOURCE-INGESTION-FREEZE-2026-11-23.md",
            "freeze_date": "2026-11-23",
            "migration_order": list(MIGRATION_ORDER),
            "no_new_feature_rule": "no new features",
            "frozen_paths": [
                {"path": "crates/market-archive"},
                {"path": "configs/providers/source-products.v1.json"},
            ],
            "approved_migration_records": [],
        }
        self.prefixes = frozen_prefixes(self.manifest)

    def approval(self) -> dict[str, str]:
        """Return one well-formed approved migration record."""
        return {
            "path": "crates/market-archive",
            "evidence": "evidence/MIGRATION.md",
            "approval": "owner decision",
            "replacement_commit": "0123456789abcdef0123456789abcdef01234567",
            "date": "2027-01-15",
        }

    def test_prefix_uses_path_boundaries(self) -> None:
        """A sibling directory with the same stem is not frozen."""
        self.assertTrue(
            matches_prefix("crates/market-archive/src/ingress.rs", "crates/market-archive")
        )
        self.assertTrue(matches_prefix("crates/market-archive", "crates/market-archive/"))
        self.assertFalse(
            matches_prefix("crates/market-archive-extra/src/lib.rs", "crates/market-archive")
        )

    def test_normalize_accepts_separators_and_slashes(self) -> None:
        """Path normalization removes backslashes and surrounding slashes."""
        self.assertEqual(normalize("/crates\\market-archive/"), "crates/market-archive")

    def test_changed_frozen_path_is_reported(self) -> None:
        """A modified frozen path with no record fails closed."""
        errors = check_changes(
            ["crates/market-archive/src/ingress.rs"],
            self.prefixes,
            [],
            lambda _evidence: True,
        )
        self.assertTrue(any("frozen path" in error for error in errors))

    def test_new_file_under_frozen_directory_is_reported(self) -> None:
        """A new untracked file under a frozen directory fails closed."""
        errors = check_changes(
            ["crates/market-archive/src/new_writer.rs"],
            self.prefixes,
            [],
            lambda _evidence: True,
        )
        self.assertEqual(len(errors), 1)

    def test_non_frozen_change_is_ignored(self) -> None:
        """Ordinary documentation and tooling changes are allowed."""
        self.assertEqual(
            check_changes(
                ["README.md", "scripts/check_source_ingestion_freeze.py"],
                self.prefixes,
                [],
                lambda _evidence: True,
            ),
            [],
        )

    def test_approved_record_authorizes_change(self) -> None:
        """A valid record for the same path with existing evidence permits change."""
        errors = check_changes(
            ["crates/market-archive/src/ingress.rs"],
            self.prefixes,
            [self.approval()],
            lambda _evidence: True,
        )
        self.assertEqual(errors, [])

    def test_record_for_other_path_does_not_authorize(self) -> None:
        """A record for a different frozen path does not permit the change."""
        record = self.approval()
        record["path"] = "configs/providers/source-products.v1.json"
        errors = check_changes(
            ["crates/market-archive/src/ingress.rs"],
            self.prefixes,
            [record],
            lambda _evidence: True,
        )
        self.assertTrue(any("frozen path" in error for error in errors))

    def test_missing_evidence_does_not_authorize(self) -> None:
        """An approval whose evidence file is absent does not permit change."""
        errors = check_changes(
            ["crates/market-archive/src/ingress.rs"],
            self.prefixes,
            [self.approval()],
            lambda _evidence: False,
        )
        self.assertTrue(any("frozen path" in error for error in errors))

    def test_malformed_record_does_not_authorize(self) -> None:
        """A record with a non-hex replacement commit does not permit change."""
        record = self.approval()
        record["replacement_commit"] = "not-a-sha"
        errors = check_changes(
            ["crates/market-archive/src/ingress.rs"],
            self.prefixes,
            [record],
            lambda _evidence: True,
        )
        self.assertTrue(any("frozen path" in error for error in errors))

    def test_empty_frozen_paths_fails_closed(self) -> None:
        """A manifest with no frozen paths is a structural finding."""
        self.manifest["frozen_paths"] = []
        errors = validate_manifest(self.manifest)
        self.assertTrue(any("no frozen_paths" in error for error in errors))

    def test_missing_required_field_is_reported(self) -> None:
        """A manifest without its record pointer is rejected."""
        del self.manifest["record"]
        errors = validate_manifest(self.manifest)
        self.assertTrue(any("missing required field `record`" in error for error in errors))


def main() -> int:
    """Report freeze findings and return a process exit status."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(FreezeGateTests)
        result = unittest.TextTestRunner(verbosity=1).run(suite)
        return 0 if result.wasSuccessful() else 1
    base = os.environ.get(BASE_ENV, DEFAULT_BASE)
    try:
        errors = check(ROOT, base)
    except (OSError, RuntimeError) as error:
        print(f"[FAIL] freeze gate could not run: {error}")
        return 1
    for error in errors:
        print(f"[FAIL] {error}")
    if errors:
        print(f"source-ingestion freeze: {len(errors)} finding(s)", file=sys.stderr)
        return 1
    prefixes = frozen_prefixes(load_manifest(MANIFEST))
    print(f"source-ingestion freeze: {len(prefixes)} frozen path(s), none changed vs {base}")
    for prefix in prefixes:
        print(f"  frozen: {prefix}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
