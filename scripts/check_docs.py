#!/usr/bin/env python3
"""Keep the small canonical documentation set current and its links valid.

Canonical documents change with the code. Dated evidence is immutable once
published and is never edited to look current. This gate enforces that split:

- every canonical and dated document exists;
- stale root-level duplicates of dated evidence are absent;
- README carries a machine-readable current-state block whose code_commit is
  an ancestor of HEAD and whose verification date and CI reference are present;
- the README document map names every dated evidence file; and
- every local Markdown link in those documents resolves.

It is dependency-free and binding in `make check` and CI.
"""
from __future__ import annotations

import argparse
import datetime as dt
import re
import subprocess
import sys
import unittest
from collections.abc import Callable
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
STATE_RE = re.compile(
    r"<!--\s*CURRENT-STATE\s*\n(?P<body>.*?)\n\s*-->", re.DOTALL
)
LINK_RE = re.compile(r"\]\(([^)]+)\)")


def parse_state(readme: str) -> dict[str, str]:
    """Parse the README current-state block into named fields."""
    match = STATE_RE.search(readme)
    if match is None:
        return {}
    fields: dict[str, str] = {}
    for line in match.group("body").splitlines():
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        fields[key.strip()] = value.strip()
    return fields


# Paths that constitute code or executable contract, as opposed to prose or
# dated evidence. A change to any of these after the declared code_commit
# invalidates the current-state record even though the commit is an ancestor.
CODE_SUFFIXES = (
    ".rs", ".py", ".q", ".sh", ".bash", ".zsh", ".c", ".cc", ".cpp", ".h",
    ".hpp", ".toml", ".json", ".yml", ".yaml", ".proto",
)
CODE_NAMES = frozenset(
    {"Makefile", "Cargo.lock", "Cargo.toml", ".editorconfig", "clippy.toml",
     ".rustfmt.toml", ".ruff.toml", ".gitattributes"}
)
PROSE_DIRS = ("evidence/", "docs/")


def is_code_path(path: str) -> bool:
    """Report whether a changed path is code/contract rather than prose."""
    normalized = path.replace("\\", "/")
    if any(normalized.startswith(prefix) for prefix in PROSE_DIRS):
        return False
    name = normalized.rsplit("/", 1)[-1]
    return name in CODE_NAMES or name.endswith(CODE_SUFFIXES)


def validate_current_state(
    state: dict[str, str],
    ancestor_check: Callable[[str], bool],
    code_changes_check: Callable[[str], list[str]] | None = None,
) -> list[str]:
    """Validate the parsed current-state block against the code-commit contract."""
    errors: list[str] = []
    if not state:
        errors.append("README has no <!-- CURRENT-STATE ... --> block")
        return errors
    code_commit = state.get("code_commit", "")
    if not re.fullmatch(r"[0-9a-f]{7,40}", code_commit):
        errors.append("CURRENT-STATE code_commit is not a hex SHA")
    elif not ancestor_check(code_commit):
        errors.append(
            f"CURRENT-STATE code_commit {code_commit} is not an ancestor of HEAD"
        )
    elif code_changes_check is not None:
        changed = code_changes_check(code_commit)
        if changed:
            errors.append(
                "CURRENT-STATE code_commit is stale: code changed after it in "
                + ", ".join(changed)
            )
    try:
        dt.date.fromisoformat(state.get("verified_on", ""))
    except ValueError:
        errors.append("CURRENT-STATE verified_on is not an ISO date")
    if not state.get("ci_run"):
        errors.append("CURRENT-STATE ci_run is empty (use 'none' with a reason)")
    if not state.get("scope"):
        errors.append("CURRENT-STATE scope is empty")
    return errors


def is_ancestor(commit: str) -> bool:
    """Report whether a commit is in HEAD's history."""
    try:
        result = subprocess.run(
            ["git", "merge-base", "--is-ancestor", commit, "HEAD"],
            cwd=ROOT,
            capture_output=True,
            check=False,
        )
    except OSError:
        return False
    return result.returncode == 0


def code_changes_since(commit: str) -> list[str]:
    """Return code/contract paths changed between a commit and HEAD."""
    try:
        result = subprocess.run(
            ["git", "diff", "--name-only", f"{commit}..HEAD"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError:
        return []
    if result.returncode != 0:
        return []
    return sorted(
        path for path in result.stdout.splitlines() if path and is_code_path(path)
    )


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

    errors.extend(
        validate_current_state(parse_state(readme), is_ancestor, code_changes_since)
    )

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

    def test_parses_current_state(self) -> None:
        """Extract all four fields from a well-formed block."""
        text = "\n".join(
            [
                "<!-- CURRENT-STATE",
                "code_commit: abc1234",
                "verified_on: 2026-11-18",
                "ci_run: none",
                "scope: local",
                "-->",
            ]
        )
        state = parse_state(text)
        self.assertEqual(state["code_commit"], "abc1234")
        self.assertEqual(state["scope"], "local")

    def test_old_commit_field_fails_the_code_commit_contract(self) -> None:
        """A block using the former `commit` name is rejected."""
        old_style = {
            "commit": "abc1234",
            "verified_on": "2026-11-18",
            "ci_run": "none",
            "scope": "local",
        }
        errors = validate_current_state(old_style, lambda _commit: True)
        self.assertTrue(
            any("code_commit is not a hex SHA" in error for error in errors),
            "the old field name fails against the new contract",
        )

    def test_valid_code_commit_block_passes(self) -> None:
        """The new field name passes when its value is an ancestor."""
        current = {
            "code_commit": "abc1234",
            "verified_on": "2026-11-18",
            "ci_run": "none",
            "scope": "local",
        }
        self.assertEqual(validate_current_state(current, lambda _commit: True), [])

    def test_detects_code_changes_after_declared_commit(self) -> None:
        """An older ancestor commit with later code edits must fail."""
        current = {
            "code_commit": "abc1234",
            "verified_on": "2026-11-18",
            "ci_run": "none",
            "scope": "local",
        }
        errors = validate_current_state(
            current,
            lambda _commit: True,
            lambda _commit: ["scripts/check_q_style.py", "analytics/kdb/mbp.q"],
        )
        self.assertTrue(
            any("stale" in error for error in errors),
            "code changes after the declared commit must fail the gate",
        )

    def test_prose_changes_after_commit_are_ignored(self) -> None:
        """Evidence and canonical Markdown edits do not invalidate state."""
        self.assertFalse(is_code_path("evidence/NEW-2026-11-20.md"))
        self.assertFalse(is_code_path("README.md"))
        self.assertFalse(is_code_path("engineering/q-kdb-style.md"))

    def test_code_paths_are_recognised(self) -> None:
        """Source, config and workflow paths count as code."""
        for path in (
            "scripts/check_docs.py",
            "analytics/kdb/mbp.q",
            "Makefile",
            "Cargo.lock",
            ".github/workflows/ci.yml",
            "schemas/analytics/kdb-metrics-report.v1.schema.json",
        ):
            self.assertTrue(is_code_path(path), f"{path} must count as code")

    def test_rejects_missing_block(self) -> None:
        """An absent block produces no fields."""
        self.assertEqual(parse_state("no state here"), {})

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
