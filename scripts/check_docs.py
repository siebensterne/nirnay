#!/usr/bin/env python3
"""Keep the small canonical documentation set current and its links valid.

Canonical documents change with the code. Dated evidence is immutable once
published and is never edited to look current. This gate enforces that split:

- every canonical and dated document exists;
- stale root-level duplicates of dated evidence are absent;
- the README names both status authorities and carries no local
  current-state block that could drift from them;
- the README document map names every dated evidence file; and
- every local Markdown link in those documents resolves.

Runtime status belongs to the workspace `CURRENT-STATE.json`; repository
capability claims belong to `claims/ledger.json`. A README snapshot cannot
bind a CI run to a tested commit, so reintroducing one is a documentation
error rather than evidence.

It is dependency-free and binding in `make check` and CI.
"""
from __future__ import annotations

import argparse
import re
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

CANONICAL = (
    "README.md",
    "AGENTS.md",
    "CONTRIBUTING.md",
    "SECURITY.md",
    "engineering/verification-policy.md",
)
DATED = (
    "evidence/VERIFICATION-2026-11-17.md",
    "evidence/UPGRADE-VERIFICATION-2026-11-13.md",
    "evidence/BUILD-INVENTORY-2026-11-13.json",
    "BUILD-RESULTS.md",
)
FORBIDDEN_ROOT = (
    "VERIFICATION.md",
    "UPGRADE-VERIFICATION.md",
    "BUILD-INVENTORY.json",
    "NIRNAY-AGENT-OPERATING-BRIEF.md",
)
STATE_MARKER = re.compile(r"<!--\s*CURRENT-STATE")
STATUS_AUTHORITIES = ("CURRENT-STATE.json", "claims/ledger.json")
LINK_RE = re.compile(r"\]\(([^)]+)\)")


def check_status_authority(readme: str) -> list[str]:
    """Reject local status snapshots and require pointers to the authorities."""
    errors: list[str] = []
    if STATE_MARKER.search(readme):
        errors.append(
            "README must not carry a local CURRENT-STATE block; "
            "use workspace CURRENT-STATE.json and claims/ledger.json"
        )
    for name in STATUS_AUTHORITIES:
        if name not in readme:
            errors.append(f"README must point to status authority {name}")
    return errors


def local_links(text: str, source: Path) -> list[Path]:
    """Return existing-or-not local link targets referenced by a document."""
    targets: list[Path] = []
    for raw in LINK_RE.findall(text):
        target = raw.split("#", 1)[0].strip()
        if not target or "://" in target or target.startswith("mailto:"):
            continue
        targets.append((source.parent / target).resolve())
    return targets


def check() -> list[str]:
    """Run every documentation gate and return human-readable errors."""
    errors: list[str] = []
    for name in (*CANONICAL, *DATED):
        path = ROOT / name
        if not path.is_file() or path.stat().st_size == 0:
            errors.append(f"missing or empty canonical document: {name}")
    for name in FORBIDDEN_ROOT:
        if (ROOT / name).exists():
            errors.append(f"stale root-level duplicate must be removed: {name}")

    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    for name in DATED:
        if name not in readme:
            errors.append(f"README document map does not name {name}")
    for name in CANONICAL:
        if Path(name).name not in readme and name not in readme:
            errors.append(f"README does not point to canonical document {name}")

    errors.extend(check_status_authority(readme))

    for name in (*CANONICAL, *DATED):
        path = ROOT / name
        if not path.is_file() or path.suffix != ".md":
            continue
        for target in local_links(path.read_text(encoding="utf-8"), path):
            if not target.exists():
                errors.append(f"{name}: broken local link -> {target.name}")
    return errors


class DocsTest(unittest.TestCase):
    """Adversarial tests for the documentation gate helpers."""

    def test_rejects_local_state_block(self) -> None:
        """A reintroduced snapshot cannot bind a CI run to its tested commit."""
        readme = "\n".join(
            [
                "CURRENT-STATE.json claims/ledger.json",
                "<!--   CURRENT-STATE",
                "code_commit: abc1234",
                "-->",
            ]
        )
        errors = check_status_authority(readme)
        self.assertTrue(any("must not carry" in error for error in errors))

    def test_requires_both_authorities(self) -> None:
        """A README missing either authority is rejected."""
        self.assertTrue(check_status_authority("no pointers here"))
        findings = check_status_authority("no pointers")
        self.assertTrue(any("CURRENT-STATE.json" in e for e in findings))

    def test_clean_status_pointers_pass(self) -> None:
        """Pointer-only status statements are accepted."""
        self.assertEqual(
            check_status_authority("See CURRENT-STATE.json and claims/ledger.json."),
            [],
        )

    def test_skips_external_links(self) -> None:
        """External links are not local paths."""
        source = ROOT / "README.md"
        self.assertEqual(local_links("[x](https://example.invalid/a)", source), [])


def main() -> int:
    """Run tests when asked, otherwise report all documentation errors."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(DocsTest)
        result = unittest.TextTestRunner(verbosity=1).run(suite)
        return 0 if result.wasSuccessful() else 1
    errors = check()
    for error in errors:
        print(error)
    if errors:
        print(f"documentation errors: {len(errors)}", file=sys.stderr)
        return 1
    print(f"docs: canonical={len(CANONICAL)} dated={len(DATED)} clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
