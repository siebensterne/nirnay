#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Fail-closed static gate against reintroduced synthetic market-data sources.

Owner directive D11 removed the active synthetic market generators, fake market
sessions, mock feeds and synthetic market benchmarks. This gate scans source
files for the exact names those implementations used, so a reintroduction is
rejected even if its tests are never run. Dated records under ``evidence/`` are
historical and are not scanned; neither is this gate's own rule table.

The gate is binding through ``make check-static`` and ``make check``. Run
``--self-test`` for adversarial unit tests, including an injected source file.
"""
from __future__ import annotations

import argparse
import re
import sys
import tempfile
import unittest
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SELF_REL = Path(__file__).resolve().relative_to(ROOT).as_posix()

EXCLUDED_DIRS = frozenset(
    {".git", ".venv", "venv", "target", "build", "dist", "__pycache__",
     ".pytest_cache", "extras", "evidence"}
)
SCAN_SUFFIXES = frozenset(
    {".py", ".rs", ".q", ".sh", ".bash", ".zsh", ".c", ".cc", ".cpp", ".h",
     ".hpp", ".yml", ".yaml", ".toml", ".ini", ".json", ".proto"}
)
SCAN_NAMES = frozenset({"Makefile", "Dockerfile"})

# (rule id, human-readable category, source pattern). Patterns name concrete
# generators, session identities, feed names and benchmark entry points rather
# than the word "synthetic", so provenance/rejection fields and dated evidence
# remain expressible without weakening the gate.
RULES: tuple[tuple[str, str, re.Pattern[str]], ...] = (
    (
        "prohibited-generator",
        "synthetic market-data generator",
        re.compile(
            r"\b(?:synthetic_events|synthetic_bars|build_synthetic_dataset|"
            r"generate_synthetic_\w+|make_synthetic_\w+)\s*\(",
            re.IGNORECASE,
        ),
    ),
    (
        "fake-market-session",
        "fake or synthetic market session",
        re.compile(
            r"\b(?:run_demo|quantlab\.demo|synthetic-ema-cross-v1|"
            r"research-job-synthetic-nse-session|bench-synthetic-nse-session|"
            r"synthetic-market-session|fake-market-session)\b",
            re.IGNORECASE,
        ),
    ),
    (
        "mock-feed",
        "mock or fake market-data feed",
        re.compile(
            r"\b(?:mock[_-]?market[_-]?data[_-]?feed|mock[_-]?feed|"
            r"fake[_-]?market[_-]?feed|synthetic[_-]?feed)\b",
            re.IGNORECASE,
        ),
    ),
    (
        "synthetic-benchmark",
        "synthetic market benchmark",
        # Case-sensitive so historical evidence log filenames such as
        # ``bench-research.log`` are not confused with emitted BENCH-* lines.
        re.compile(
            r"\b(?:run_core_suite|run_experiments_suite|run_latency_suite|"
            r"BENCH-RESEARCH|BENCH-EXPERIMENTS|"
            r"BENCH-HISTORICAL-QUERY|BENCH-E2E-LATENCY)\b"
            r"|-m\s+quantlab\.bench\b",
        ),
    ),
)


@dataclass(frozen=True, slots=True)
class Finding:
    """One prohibited source pattern occurrence."""

    path: str
    line: int
    rule: str
    match: str

    def __str__(self) -> str:
        """Render a single actionable finding."""
        return f"{self.path}:{self.line}: {self.rule}: {self.match}"


def is_scan_path(path: Path, root: Path = ROOT) -> bool:
    """Report whether a file is active source scanned by this gate."""
    try:
        relative = path.resolve().relative_to(root.resolve())
    except ValueError:
        return False
    if any(part in EXCLUDED_DIRS for part in relative.parts[:-1]):
        return False
    if relative.as_posix() == SELF_REL:
        return False
    return relative.name in SCAN_NAMES or relative.suffix in SCAN_SUFFIXES


def iter_source_files(root: Path = ROOT) -> list[Path]:
    """Return every active source file under ``root`` in deterministic order."""
    return sorted(path for path in root.rglob("*") if path.is_file() and is_scan_path(path, root))


def scan_text(text: str, *, path: str = "<text>") -> list[Finding]:
    """Return prohibited-pattern findings in one source text."""
    findings: list[Finding] = []
    for line_number, line in enumerate(text.splitlines(), 1):
        for rule_id, _category, pattern in RULES:  # noqa: B007
            for match in pattern.finditer(line):
                findings.append(Finding(path, line_number, rule_id, match.group(0)))
    return findings


def check(root: Path = ROOT) -> list[Finding]:
    """Scan the active source tree and return all findings."""
    findings: list[Finding] = []
    for path in iter_source_files(root):
        text = path.read_text(encoding="utf-8", errors="replace")
        findings.extend(scan_text(text, path=path.relative_to(root).as_posix()))
    return findings


class SyntheticMarketPolicyTests(unittest.TestCase):
    """Adversarial tests for the source-pattern gate."""

    def test_generator_call_is_detected(self) -> None:
        """Verify generator call is detected."""
        text = "def synthetic_events(size, seed):\n    return synthetic_bars(size, seed)\n"
        rules = {finding.rule for finding in scan_text(text)}
        self.assertEqual(rules, {"prohibited-generator"})

    def test_fake_session_identities_are_detected(self) -> None:
        """Verify fake session identities are detected."""
        text = "run_demo()\nPYTHONPATH=research/python/src python3 -m quantlab.cli demo\n"
        findings = scan_text(text)
        self.assertTrue(all(item.rule == "fake-market-session" for item in findings))
        self.assertIn("research-job-synthetic-nse-session",
                      [item.match for item in scan_text(
                          '"dataset_id": "research-job-synthetic-nse-session"')])

    def test_mock_feed_name_is_detected_case_insensitively(self) -> None:
        """Verify mock feed name is detected case insensitively."""
        findings = scan_text("class MockFeed:\n    pass\n")
        self.assertTrue(any(item.rule == "mock-feed" for item in findings))

    def test_synthetic_benchmark_entry_points_are_detected(self) -> None:
        """Verify synthetic benchmark entry points are detected."""
        text = "python3 -m quantlab.bench --suite core\nBENCH-RESEARCH-OUTPUT size=200\n"
        rules = {finding.rule for finding in scan_text(text)}
        self.assertEqual(rules, {"synthetic-benchmark"})

    def test_benign_scalar_source_has_no_findings(self) -> None:
        """Verify benign scalar source has no findings."""
        text = (
            "def percentile(values, fraction):\n"
            "    if not values:\n"
            "        return 0.0\n"
            "    return values[0]\n"
        )
        self.assertEqual(scan_text(text), [])

    def test_finding_carries_line_number(self) -> None:
        """Verify finding carries line number."""
        findings = scan_text("# benign\nfirst = 1\nbuild_synthetic_dataset(out)\n")
        self.assertEqual([item.line for item in findings], [3])

    def test_gate_source_and_evidence_are_not_scanned(self) -> None:
        """Verify gate source and evidence are not scanned."""
        self.assertFalse(is_scan_path(ROOT / SELF_REL))
        self.assertFalse(is_scan_path(ROOT / "evidence" / "OLD-RECORD.md"))
        self.assertTrue(
            is_scan_path(ROOT / "research" / "python" / "src" / "quantlab" / "bench.py"),
            "bench.py is scanned",
        )

    def test_injected_source_file_is_rejected(self) -> None:
        """Verify injected source file is rejected."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            probe = root / "probe.py"
            probe.write_text("def synthetic_bars(size, seed):\n    return []\n", encoding="utf-8")
            findings = check(root)
            self.assertTrue(any(item.rule == "prohibited-generator" for item in findings))
            self.assertTrue(all(item.path == "probe.py" for item in findings))


def main() -> int:
    """Report prohibited source patterns and return a process exit status."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(SyntheticMarketPolicyTests)
        result = unittest.TextTestRunner(verbosity=1).run(suite)
        return 0 if result.wasSuccessful() else 1
    findings = check(ROOT)
    for finding in findings:
        print(f"[FAIL] {finding}")
    if findings:
        print(f"synthetic-market policy: {len(findings)} prohibited pattern(s)", file=sys.stderr)
        return 1
    count = len(iter_source_files(ROOT))
    print(f"synthetic-market policy: {count} source file(s) scanned, 0 prohibited pattern(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
