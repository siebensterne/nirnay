#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Unified whole-tree prose gate, shared byte-for-byte by both workspaces.

This is the single source of truth for banned wording. It unions the established
rules of both workspaces: tautologies, promotional or black-box terms,
first-person pronouns, intent and filler adverbs, vague quantifiers, unsupported
absolutes and informal verbs. Stable IDs make every finding actionable.

Scope: authored prose only. Markdown prose outside fenced and inline code, and
comment lines in source files. Byte-exact vendored references and immutable
evidence are excluded by path and hash-verified by check_style_references.py.

An inline ``prose-audit-ignore: <reason>`` or ``english-audit-ignore: <reason>``
marker suppresses a finding on that line. A marker without a reason is itself a
finding: exemptions must be justified, never silent.

Run from the repository root:
  python3 scripts/check_prose_unified.py            # added prose (binding)
  python3 scripts/check_prose_unified.py --all      # whole-tree sweep
  python3 scripts/check_prose_unified.py --self-test
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASE = (
    os.environ.get("QUALITY_STYLE_BASE")
    or os.environ.get("STYLE_BASE")
    or "main"
)

# (stable_id, compiled pattern, human rationale)
TAUTOLOGY = re.compile(
    r"\b(?:exact same|same exact|end result|final outcome|each and every|"
    r"basic fundamentals|added bonus|free gift|pre[- ]plan(?:ned|ning)?|"
    r"reason why|consensus of opinion|refer back|repeat again|revert back|"
    r"merge together|combine together|currently existing|unexpected surprise|"
    r"true fact|necessary prerequisite|past history|future plans|"
    r"close proximity|mutual cooperation|first introduced|"
    r"it is what it is|needless to say|in order to|as a matter of fact|"
    r"the reason why is because|self[- ]evidently)\b",
    re.IGNORECASE,
)  # prose-audit-ignore: matcher vocabulary required to detect tautologies
BANNED = re.compile(
    r"\b(?:bulletproof|world[- ]class|best[- ]in[- ]class|revolutionary|"
    r"game[- ]changing|cutting[- ]edge|enterprise[- ]grade|nasa[- ]level|"
    r"state[- ]of[- ]the[- ]art|perfect(?:ly)?|forever|unparalleled|"
    r"unmatched\s+(?:performance|quality|results?|speed|reliability|precision)|"
    r"magic|black[ -]box(?:es)?|seamless)\b",
    re.IGNORECASE,
)  # prose-audit-ignore: matcher vocabulary required to detect banned terms
FIRST_PERSON = re.compile(
    r"(?<![A-Za-z0-9_/])(?:I(?![A-Za-z0-9_/])|[Ww]e|[Oo]ur|[Oo]urs|"
    r"[Uu]s|[Mm]y|[Mm]ine|[Mm]e|[Mm]yself|[Oo]urselves)(?:['\u2019](?:m|re|ve|ll|d))?"
    r"(?![A-Za-z0-9_/])"
)  # prose-audit-ignore: matcher vocabulary required to detect first person
INTENT_OR_FILLER = re.compile(
    r"\b(?:intentionally|deliberately|purposely|purposefully|knowingly|"
    r"unintentionally|accidentally|inadvertently|willfully|voluntarily|"
    r"obviously|clearly|simply|just|really|very|extremely|basically|actually|"
    r"undoubtedly|certainly|of course|truly|merely|essentially|trivially)\b",
    re.IGNORECASE,
)  # prose-audit-ignore: matcher vocabulary required to detect intent adverbs
VAGUE = re.compile(
    r"\b(?:some|many|few|several|various|numerous|often|usually|sometimes|"
    r"soon|later|fast|slow|large|small|enough|reasonable|appropriate|"
    r"significant|substantial)\b",
    re.IGNORECASE,
)  # prose-audit-ignore: matcher vocabulary required to detect vague scope
ABSOLUTE = re.compile(
    r"\b(?:always|never|guarantees?|ensures?|exactly once|lossless|"
    r"production[- ]ready|zero downtime|never fails|always available|100%)\b",
    re.IGNORECASE,
)  # prose-audit-ignore: matcher vocabulary for absolute terms
INFORMAL = re.compile(
    r"\b(?:do the needful|make sure|works fine|goes wrong|fix it|handle it|"
    r"stuff|things|nice|good|bad|huge|tiny)\b",
    re.IGNORECASE,
)  # prose-audit-ignore: matcher vocabulary required to detect informal verbs

# The four categories named by the organisation are fail-closed. U005-U007
# are inherited review-level candidates from the sibling workspace's audit:
# valid normative uses ("never route live") are common, so they are reported
# as advisory and never block a gate. U008 (bare ignore marker) is fail-closed.
BINDING_IDS = frozenset({"U001", "U002", "U003", "U004", "U008"})

RULES: tuple[tuple[str, re.Pattern[str], str], ...] = (
    ("U001", TAUTOLOGY, "tautological or redundant construction"),
    ("U002", BANNED, "promotional or black-box term"),
    ("U003", FIRST_PERSON, "first-person pronoun"),
    ("U004", INTENT_OR_FILLER, "intent or filler adverb"),
    ("U005", VAGUE, "vague quantifier or scope"),
    ("U006", ABSOLUTE, "unsupported absolute or guarantee"),
    ("U007", INFORMAL, "informal or ambiguous verb"),
)

IGNORE = re.compile(r"(?:prose|english)-audit-ignore:")
INLINE_CODE = re.compile(r"`[^`]*`")
FENCE = re.compile(r"^\s*(```|~~~)")

EXCLUDED_PREFIXES = (
    "references/",
    "engineering/references/",
    "target/",
    "build/",
    ".local/",
    "captures/",
    "measurements/",
    "fixtures/",
    "evidence/",
    "src/generated/",
    "BUILD-RESULTS.md",
    "LICENSE",
    "NOTICE",
)


def prose_line(path: str, line: str, fenced: bool) -> str:
    """Return authored prose from one line, or an empty string."""
    if fenced:
        return ""
    if path.endswith(".md"):
        if line.lstrip().startswith("    "):
            return ""
        return INLINE_CODE.sub(" ", line)
    stripped = line.strip()
    if path.endswith((".rs", ".cpp", ".cc", ".c", ".h", ".hpp")):
        if stripped.startswith("//"):
            return stripped[2:].strip()
        return ""
    if path.endswith((".py", ".sh", ".bash", ".zsh", ".toml", ".yml", ".yaml")):
        if stripped.startswith("#"):
            return stripped[1:].strip()
        return ""
    return ""


def check_line(path: str, number: int, line: str, fenced: bool) -> list[str]:
    """Return one message per unified rule matched on an authored prose line."""
    if IGNORE.search(line):
        suffix = IGNORE.split(line, maxsplit=1)[1].strip(" :\t")
        if not suffix:
            return [f"{path}:{number}: U008: audit ignore marker has no reason"]
        return []
    text = prose_line(path, line, fenced)
    if not text:
        return []
    errors: list[str] = []
    for rule_id, pattern, rationale in RULES:
        match = pattern.search(text)
        if match is not None:
            errors.append(
                f"{path}:{number}: {rule_id}: {match.group(0)!r} "
                f"({rationale})"
            )
    return errors


def read_fenced(text: str) -> set[int]:
    """Return line numbers inside Markdown fenced code blocks."""
    inside: set[int] = set()
    marker = ""
    for number, line in enumerate(text.splitlines(), 1):
        stripped = line.lstrip()
        if not marker and FENCE.match(stripped):
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


def tracked_files() -> list[str]:
    """Return tracked text paths with authored prose."""
    result = subprocess.run(
        ["git", "ls-files", "-z"], cwd=ROOT, check=True, capture_output=True
    )
    paths = [
        p.decode("utf-8", "surrogateescape")
        for p in result.stdout.split(b"\0")
        if p
    ]
    return [
        p
        for p in paths
        if not p.startswith(EXCLUDED_PREFIXES)
        and Path(p).suffix.lower()
        in {
            ".md",
            ".py",
            ".rs",
            ".sh",
            ".bash",
            ".zsh",
            ".toml",
            ".yml",
            ".yaml",
        }
    ]


def scan_all() -> list[str]:
    """Sweep every tracked authored prose line."""
    errors: list[str] = []
    for path in tracked_files():
        text = (ROOT / path).read_text(encoding="utf-8", errors="replace")
        fenced = read_fenced(text) if path.endswith(".md") else set()
        for number, line in enumerate(text.splitlines(), 1):
            errors += check_line(path, number, line, number in fenced)
    return errors


def scan_added(base: str) -> list[str]:
    """Sweep prose lines added relative to ``base``."""
    result = subprocess.run(
        ["git", "diff", "--unified=0", base, "-z"],
        cwd=ROOT,
        check=False,
        capture_output=True,
    )
    if result.returncode != 0:
        detail = result.stderr.decode("utf-8", "replace").strip()
        return [f"cannot diff against {base}: {detail}"]
    errors: list[str] = []
    for raw in result.stdout.split(b"\0"):
        chunk = raw.decode("utf-8", "surrogateescape")
        current = ""
        number = 0
        for line in chunk.splitlines():
            if line.startswith("+++ b/"):
                current = line[6:]
                number = 0
                continue
            if line.startswith("@@"):
                match = re.search(r"\+(\d+)", line)
                number = int(match.group(1)) if match else 0
                continue
            if line.startswith("+") and number and current:
                if not current.startswith(EXCLUDED_PREFIXES):
                    errors += check_line(current, number, line[1:], False)
                number += 1
    return errors


class UnifiedProseTests(unittest.TestCase):
    """Adversarial tests for the unified prose rules."""

    def test_each_category_is_detected(self) -> None:
        """Every unified rule category is detected."""
        samples = [
            ("U001", "This is the exact same result."),
            ("U002", "A state-of-the-art engine."),
            ("U003", "We require evidence."),
            ("U004", "Clearly the gate passes."),
            ("U005", "Several files changed."),
            ("U006", "This always works."),
            ("U007", "Make sure it is correct."),
        ]
        for expected, text in samples:
            findings = check_line("docs/x.md", 1, text, False)
            self.assertTrue(
                any(expected in finding for finding in findings),
                f"{expected} not detected in {text!r}: {findings}",
            )

    def test_code_and_fences_are_excluded(self) -> None:
        """Code spans, fences and identifiers are not prose."""
        self.assertEqual(check_line("docs/x.md", 1, "```", True), [])
        self.assertEqual(
            check_line("docs/x.md", 1, "use `clearly` here", False), []
        )
        self.assertEqual(check_line("src/x.rs", 1, "let We = 1;", False), [])

    def test_first_person_does_not_match_io(self) -> None:
        """The I/O abbreviation is not a pronoun."""
        self.assertEqual(
            check_line("docs/x.md", 1, "I/O is buffered.", False), []
        )

    def test_reasoned_ignore_is_honoured(self) -> None:
        """A justified inline marker suppresses a finding."""
        line = "We require evidence.  # prose-audit-ignore: quoted policy text"
        self.assertEqual(check_line("README.md", 1, line, False), [])

    def test_bare_ignore_is_a_finding(self) -> None:
        """An unexplained marker is itself a finding."""
        findings = check_line(
            "README.md", 1, "text  # prose-audit-ignore:", False
        )
        self.assertTrue(any("U008" in finding for finding in findings))

    def test_rule_ids_are_unique(self) -> None:
        """Stable rule identifiers do not collide."""
        ids = [rule[0] for rule in RULES]
        self.assertEqual(len(ids), len(set(ids)))


def main() -> int:
    """Print findings and return a process exit status."""
    if "--self-test" in sys.argv[1:]:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(
            UnifiedProseTests
        )
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1
    findings = scan_all() if "--all" in sys.argv[1:] else scan_added(BASE)
    binding = [
        f for f in findings if any(f": {rule}:" in f for rule in BINDING_IDS)
    ]
    advisory = [f for f in findings if f not in binding]
    for finding in binding:
        print(f"FAIL: {finding}")
    for finding in advisory:
        print(f"ADVISORY: {finding}")
    if binding:
        print(
            f"unified prose audit: {len(binding)} binding finding(s), "
            f"{len(advisory)} advisory"
        )
        return 1
    print(
        f"PASS: unified prose audit clean "
        f"({len(advisory)} advisory review candidates)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
