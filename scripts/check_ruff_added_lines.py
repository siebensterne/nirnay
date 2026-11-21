#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Binding Ruff gate for added Python lines.

The whole-tree Ruff check in ``make python-format-check`` is advisory while the
existing corpus is being migrated. This gate applies the same ``ruff.toml``
policy only to lines added relative to the base ref, so new code cannot add
lint debt. It is fail-closed: a missing Ruff executable is a gate failure.

Run from the repository root. Set ``NIRNAY_STYLE_BASE`` to compare against a
different ref (default: main). Use ``--self-test`` for adversarial unit tests.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONFIG = ROOT / "ruff.toml"
FILE_HEADER = re.compile(r"^\+\+\+ b/")
PY_SOURCE = (".py", ".pyi")


def parse_added_lines(diff_text: str) -> dict[str, set[int]]:
    """Parse a ``git diff --unified=0`` payload into added Python line numbers."""
    by_file: dict[str, set[int]] = {}
    current = ""
    number = 0
    for line in diff_text.splitlines():
        if line.startswith("+++ "):
            current = line[6:] if FILE_HEADER.match(line) else ""
            number = 0
        elif line.startswith("@@"):
            match = re.search(r"\+(\d+)", line)
            number = int(match.group(1)) if match else 0
        elif line.startswith("+") and number:
            if current.endswith(PY_SOURCE):
                by_file.setdefault(current, set()).add(number)
            number += 1
    return by_file


def collect_added_lines(base: str) -> dict[str, set[int]]:
    """Collect added Python lines relative to ``base`` from the worktree."""
    result = subprocess.run(
        ["git", "diff", base, "--unified=0", "--", "*.py", "*.pyi"],
        check=True,
        capture_output=True,
        text=True,
        cwd=ROOT,
    )
    return parse_added_lines(result.stdout)


def run_ruff() -> list[dict[str, object]]:
    """Run the pinned Ruff once over the tree and return JSON findings."""
    import ruff_gate

    binary = ruff_gate.pinned_binary(ruff_gate.pinned_version())
    if binary is None:
        print("FAIL: pinned Ruff not available", file=sys.stderr)
        raise SystemExit(1)
    command = [
        binary,
        "check",
        "--config",
        str(CONFIG),
        "--no-cache",
        "--output-format",
        "json",
        ".",
    ]
    try:
        result = subprocess.run(
            command, check=False, capture_output=True, text=True, cwd=ROOT
        )
    except OSError as error:
        print(f"FAIL: cannot run ruff: {error}", file=sys.stderr)
        raise SystemExit(1) from error
    if result.returncode not in (0, 1):
        tail = result.stderr[-2000:]
        print(f"FAIL: ruff exited {result.returncode}: {tail}", file=sys.stderr)
        raise SystemExit(1)
    findings: list[dict[str, object]] = json.loads(result.stdout or "[]")
    return findings


def relative_path(name: str) -> str:
    """Normalise an absolute Ruff filename to a repository-relative path."""
    try:
        return Path(name).resolve().relative_to(ROOT).as_posix()
    except ValueError:
        return name


def findings_on_added_lines(
    findings: list[dict[str, object]], added: dict[str, set[int]]
) -> list[str]:
    """Return formatted findings whose file and line are newly added."""
    errors: list[str] = []
    for finding in findings:
        path = relative_path(str(finding.get("filename", "")))
        location = finding.get("location")
        row = int(location.get("row", 0)) if isinstance(location, dict) else 0
        if row in added.get(path, set()):
            code = finding.get("code") or "?"
            message = finding.get("message") or ""
            errors.append(f"{path}:{row}: {code} {message}")
    return errors


class RuffAddedLinesTests(unittest.TestCase):
    """Adversarial tests for the added-lines filter and diff parser."""

    def test_parser_records_python_lines(self) -> None:
        """Only added Python lines are recorded."""
        diff = (
            "+++ b/x.py\n@@ -0,0 +1,2 @@\n+a: int = 1\n+b: int = 2\n"
            "+++ b/x.q\n@@ -0,0 +1,1 @@\n+c:1\n"
        )
        self.assertEqual(parse_added_lines(diff), {"x.py": {1, 2}})

    def test_parser_handles_multiple_hunks(self) -> None:
        """Separate hunks in one file accumulate all added lines."""
        diff = (
            "+++ b/a.py\n@@ -1 +1 @@\n+x\n"
            "@@ -10,0 +11 @@\n+y\n"
        )
        self.assertEqual(parse_added_lines(diff), {"a.py": {1, 11}})

    def test_added_finding_is_reported(self) -> None:
        """A finding on an added line fails the gate."""
        findings = [
            {
                "filename": str(ROOT / "x.py"),
                "location": {"row": 3},
                "code": "F401",
                "message": "unused import",
            }
        ]
        errors = findings_on_added_lines(findings, {"x.py": {3}})
        self.assertTrue(any("x.py:3" in e and "F401" in e for e in errors))

    def test_existing_finding_is_ignored(self) -> None:
        """Pre-existing findings do not fail the gate."""
        findings = [
            {
                "filename": str(ROOT / "x.py"),
                "location": {"row": 9},
                "code": "F401",
                "message": "unused import",
            }
        ]
        self.assertEqual(findings_on_added_lines(findings, {"x.py": {3}}), [])

    def test_unknown_file_is_ignored(self) -> None:
        """Findings outside the repository cannot match added lines."""
        findings = [
            {
                "filename": "other/other.py",
                "location": {"row": 3},
                "code": "E501",
                "message": "too long",
            }
        ]
        self.assertEqual(findings_on_added_lines(findings, {"x.py": {3}}), [])


def main() -> int:
    """Print findings and return a process exit status."""
    if "--self-test" in sys.argv[1:]:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(
            RuffAddedLinesTests
        )
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1
    base = os.environ.get("NIRNAY_STYLE_BASE", "main")
    errors = findings_on_added_lines(run_ruff(), collect_added_lines(base))
    if errors:
        for error in errors:
            print(f"FAIL: {error}")
        print(f"{len(errors)} Ruff violation(s) on added Python lines")
        return 1
    print(f"PASS: Ruff clean on added Python lines vs {base}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
