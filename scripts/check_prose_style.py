#!/usr/bin/env python3
"""Binding prose rules for Markdown source.

Rule set (stable IDs; changes require style-policy review):

- PROSE-001: no first-person pronouns. They assert an actor instead of an
  invariant, owner or dated record. ``I/O`` is not a pronoun.
- PROSE-002: no intent adverbs. Words such as ``clearly`` report the author's
  posture; the text must state the evidence instead.
- PROSE-003: no promotional or black-box terms. A component that cannot be
  explained is not described as one.
- PROSE-004: no tautological filler. A sentence must add a checkable claim.

The checker scans Markdown only. Inline code spans and fenced/indented code are
excluded because identifiers and quoted source are not prose. By default it
checks lines added relative to the base ref; ``--all`` sweeps every tracked
Markdown file for remediation work.

Run from the repository root. ``--self-test`` runs adversarial parser tests.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASE_REF = os.environ.get("NIRNAY_STYLE_BASE", "main")
GENERATED_MARKDOWN_PREFIX = "target/doc/"

RULES: tuple[tuple[str, re.Pattern[str]], ...] = (
    (
        "PROSE-001",
        re.compile(
            r"(?<![A-Za-z0-9_])(?:I(?!/)|[Ww]e|[Oo]ur|us)(?![A-Za-z0-9_])",
        ),
    ),
    (
        "PROSE-002",
        re.compile(
            r"\b(?:clearly|obviously|simply|just|basically|actually|"
            r"essentially|merely|trivially)\b",
            re.IGNORECASE,
        ),
    ),
    (
        "PROSE-003",
        re.compile(
            r"\b(?:magic|black[ -]box(?:es)?|state[ -]of[ -]the[ -]art|"
            r"world[ -]class|best[ -]in[ -]class|revolutionary|seamless)\b",
            re.IGNORECASE,
        ),
    ),
    (
        "PROSE-004",
        re.compile(
            r"\b(?:it is what it is|needless to say|in order to|"
            r"as a matter of fact|the reason why is because|self[ -]evidently)\b",
            re.IGNORECASE,
        ),
    ),
)

INLINE_CODE = re.compile(r"`[^`]*`")


def prose_text(line: str) -> str:
    """Remove inline code and code-like indentation from a Markdown line."""
    if line.startswith("    ") or line.lstrip().startswith("```"):
        return ""
    return INLINE_CODE.sub(" ", line)


def fenced_lines(text: str) -> set[int]:
    """Return line numbers inside fenced code blocks."""
    inside: set[int] = set()
    marker = ""
    for number, line in enumerate(text.splitlines(), 1):
        stripped = line.lstrip()
        if not marker and (stripped.startswith("```") or stripped.startswith("~~~")):
            marker = stripped[:3]
            inside.add(number)
            continue
        if marker and stripped.startswith(marker):
            marker = ""
            inside.add(number)
            continue
        if marker:
            inside.add(number)
    return inside


def check_prose_line(line: str) -> list[str]:
    """Return one message per rule matched in a Markdown prose line."""
    text = prose_text(line)
    errors: list[str] = []
    for rule_id, pattern in RULES:
        match = pattern.search(text)
        if match is not None:
            errors.append(
                f"{rule_id}: {match.group(0)!r} in prose: {line.strip()[:100]!r}"
            )
    return errors


def tracked_markdown() -> list[str]:
    result = subprocess.run(
        ["git", "ls-files", "*.md"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return [
        path
        for path in result.stdout.splitlines()
        if path and not path.startswith(GENERATED_MARKDOWN_PREFIX)
    ]


def added_markdown(base: str) -> dict[str, list[tuple[int, str]]]:
    diff = subprocess.run(
        ["git", "diff", "--unified=0", base, "--", "*.md"],
        cwd=ROOT,
        check=True,
        capture_output=True,
    )
    by_file: dict[str, list[tuple[int, str]]] = {}
    current = ""
    number = 0
    for raw in diff.stdout.decode("utf-8", errors="replace").splitlines():
        if raw.startswith("+++ b/"):
            current = raw[6:]
            number = 0
            continue
        if raw.startswith("@@"):
            found = re.search(r"\+(\d+)", raw)
            number = int(found.group(1)) if found else 0
            continue
        if raw.startswith("+") and number and current:
            # Vendored references are third-party documents, not project
            # prose; they are hash-pinned by check_style_references.py.
            if current.startswith(
                ("engineering/references/", "references/", GENERATED_MARKDOWN_PREFIX)
            ):
                number += 1
                continue
            by_file.setdefault(current, []).append((number, raw[1:]))
            number += 1
    return by_file


def scan(scan_all: bool, base: str) -> list[str]:
    errors: list[str] = []
    if scan_all:
        for path in tracked_markdown():
            text = (ROOT / path).read_text(encoding="utf-8")
            fenced = fenced_lines(text)
            for number, line in enumerate(text.splitlines(), 1):
                if number in fenced:
                    continue
                for message in check_prose_line(line):
                    errors.append(f"{path}:{number}: {message}")
        return errors
    for path, lines in added_markdown(base).items():
        text = (ROOT / path).read_text(encoding="utf-8")
        fenced = fenced_lines(text)
        for number, line in lines:
            if number in fenced:
                continue
            for message in check_prose_line(line):
                errors.append(f"{path}:{number}: {message}")
    return errors


class ProseCheckerTests(unittest.TestCase):
    def test_first_person_is_rejected(self) -> None:
        self.assertTrue(any("PROSE-001" in e for e in check_prose_line("we require evidence")))

    def test_io_is_not_first_person(self) -> None:
        self.assertEqual(check_prose_line("The I/O path is bounded."), [])

    def test_intent_adverb_is_rejected(self) -> None:
        self.assertTrue(any("PROSE-002" in e for e in check_prose_line("This is clearly safe.")))

    def test_banned_term_is_rejected(self) -> None:
        self.assertTrue(any("PROSE-003" in e for e in check_prose_line("a black box component")))

    def test_tautology_is_rejected(self) -> None:
        self.assertTrue(any("PROSE-004" in e for e in check_prose_line("It is what it is.")))

    def test_inline_code_is_exempt(self) -> None:
        self.assertEqual(check_prose_line("Use `we` as an identifier."), [])

    def test_fence_and_indented_code_are_exempt(self) -> None:
        self.assertEqual(check_prose_line("```"), [])
        self.assertEqual(check_prose_line("    we write code"), [])

    def test_fenced_content_is_excluded(self) -> None:
        text = "# Title\n\n```text\nwe clearly use magic\n```\n\nwe remain guilty\n"
        fenced = fenced_lines(text)
        self.assertTrue({3, 4, 5}.issubset(fenced), "fence body and markers are excluded")
        self.assertNotIn(7, fenced, "prose after the fence is not excluded")

    def test_io_and_roman_enumeration_are_not_first_person(self) -> None:
        self.assertEqual(check_prose_line("The I/O path is bounded."), [])
        self.assertEqual(check_prose_line("Proof provided (i) and (ii) holds."), [])
        self.assertEqual(check_prose_line("The US venue is distinct."), [])

    def test_safe_prose_passes(self) -> None:
        self.assertEqual(
            check_prose_line("The store commits the revision and head atomically."), []
        )

    def test_multiple_rules_report_separately(self) -> None:
        messages = check_prose_line("We clearly use magic.")
        self.assertEqual(len(messages), 3, "each distinct rule is reported")

    def test_lowercase_pronoun_is_rejected(self) -> None:
        self.assertTrue(any("PROSE-001" in e for e in check_prose_line("our result stands")))


def main() -> int:
    if "--self-test" in sys.argv:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(ProseCheckerTests)
        result = unittest.TextTestRunner(verbosity=1).run(suite)
        return 0 if result.wasSuccessful() else 1
    scan_all = "--all" in sys.argv
    errors = scan(scan_all, BASE_REF)
    for error in errors:
        print(f"FAIL: {error}")
    if errors:
        print(f"prose style: {len(errors)} finding(s)", file=sys.stderr)
        return 1
    print("prose style: no findings")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
