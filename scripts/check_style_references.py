#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Verify the local copies of the mandated style references.

The repo-level policy names published style guides and keeps local copies under
``engineering/references/``. This gate checks that each third-party document
exists, is non-empty and matches the SHA-256 recorded in the index, and that
the index covers the mandated set exactly. ``index.md`` is project-authored
and cannot contain its own digest, so it is excluded from the hash set.

Published guides are third-party documents; this gate verifies their
retention, not their adoption. Binding rules remain the repository checkers.
"""

from __future__ import annotations

import hashlib
import re
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
INDEX = ROOT / "engineering" / "references" / "index.md"
ROW = re.compile(r"^\| `([^`]+)` \| .* \| `([0-9a-f]{64})` \|$")
MANIFEST = "index.md"

# Mirrors the mandatory list in AGENTS.md plus the discovery index retained in
# the sibling workspace. index.md is excluded because a file cannot hash itself.
MANDATED = frozenset(
    {
        "rustguide.html",
        "rust-coding-guidelines.html",
        "cppguide.html",
        "jsoncguide.html",
        "pyguide.html",
        "shellguide.html",
        "verilogguide.html",
        "docguide-style.html",
        "awesome-style-guide-master.md",
    }
)


def parse_index(text: str) -> dict[str, str]:
    """Return the filename-to-digest mapping from the reference index."""
    rows: dict[str, str] = {}
    for line in text.splitlines():
        match = ROW.match(line)
        if match:
            rows[match.group(1)] = match.group(2)
    return rows


def check(root: Path = ROOT) -> list[str]:
    """Return every reference-integrity error found under ``root``."""
    errors: list[str] = []
    index = root / "engineering" / "references" / MANIFEST
    if not index.is_file():
        return ["engineering/references/index.md is missing"]
    rows = parse_index(index.read_text(encoding="utf-8"))
    rows.pop(MANIFEST, None)
    if set(rows) != MANDATED:
        missing = sorted(MANDATED - set(rows))
        extra = sorted(set(rows) - MANDATED)
        if missing:
            errors.append(f"index is missing mandated references: {missing}")
        if extra:
            errors.append(f"index lists unexpected references: {extra}")
    for name, expected in sorted(rows.items()):
        path = root / "engineering" / "references" / name
        if not path.is_file():
            errors.append(f"reference {name} is missing")
            continue
        data = path.read_bytes()
        if not data:
            errors.append(f"reference {name} is empty")
            continue
        actual = hashlib.sha256(data).hexdigest()
        if actual != expected:
            errors.append(
                f"reference {name} digest mismatch: "
                f"expected {expected}, found {actual}"
            )
    return errors


class ReferenceTests(unittest.TestCase):
    """Adversarial tests for index parsing and digest verification."""

    def test_parse_index(self) -> None:
        """Rows with a three-column layout are parsed."""
        text = "| `a.html` | <https://example.com> | `" + "a" * 64 + "` |\n"
        self.assertEqual(parse_index(text), {"a.html": "a" * 64})

    def test_self_hash_is_ignored(self) -> None:
        """The project-authored index cannot hash itself."""
        text = f"| `{MANIFEST}` | <https://example.com> | `{'b' * 64}` |\n"
        self.assertEqual(parse_index(text), {MANIFEST: "b" * 64})
        self.assertNotIn(MANIFEST, MANDATED)

    def test_missing_reference_is_reported(self) -> None:
        """A mandated document that is absent is a finding."""
        findings = check(ROOT / "does-not-exist")
        self.assertTrue(any("index.md is missing" in e for e in findings))


def main() -> int:
    """Print findings and return a process exit status."""
    if "--self-test" in sys.argv[1:]:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(ReferenceTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1
    findings = check()
    for finding in findings:
        print(f"FAIL: {finding}")
    if findings:
        print(f"style references: {len(findings)} finding(s)")
        return 1
    print(f"PASS: {len(MANDATED)} mandated style references verified")
    return 0


if __name__ == "__main__":
    sys.exit(main())
