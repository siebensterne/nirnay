#!/usr/bin/env python3
"""Binding integrity check for the style gate itself.

A formatter gate is only useful if nobody can silently remove or weaken it.
This checker asserts that the Makefile, CI workflow and both style checkers
still contain the required wiring, language coverage and test inventory. It is
itself invoked from `make check`, from the CI `source-style` job and from the
independent `python-research` CI job, so disabling one path is not enough.

Adding a new language or rule requires updating the expectations here in the
same PR; removing a rule or a test below the declared minimum fails the gate.
"""

from __future__ import annotations

import re
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

REQUIRED_TEXT: dict[str, tuple[str, ...]] = {
    "Makefile": (
        "style: rust-style source-style python-ruff-style q-style prose-style",
        "$(MAKE) style",
        "python3 scripts/ruff_gate.py",
        "python3 scripts/check_ruff_added_lines.py",
        "python3 scripts/check_style_references.py",
        "python3 scripts/check_quality_contract.py",
        "python3 scripts/check_license_provenance.py",
        "prose-style:",
        "python3 scripts/check_prose_style.py --self-test",
        "python3 scripts/check_prose_style.py",
        "python3 scripts/check_q_style.py --self-test",
        "python3 scripts/check_q_style.py",
        "python3 scripts/check_rust_style.py --self-test",
        "python3 scripts/check_source_style.py --self-test",
        "python3 scripts/check_style_gate.py",
    ),
    ".github/workflows/ci.yml": (
        "source-style:",
        "fetch-depth: 0",
        "NIRNAY_STYLE_BASE: origin/main",
        "python3 scripts/check_rust_style.py",
        "python3 scripts/check_source_style.py",
        "python3 scripts/check_prose_style.py",
        "python3 scripts/check_q_style.py",
        "python3 scripts/ruff_gate.py",
        "python3 scripts/check_ruff_added_lines.py",
        "python3 scripts/check_style_references.py",
        "python3 scripts/check_quality_contract.py",
        "python3 scripts/check_license_provenance.py",
        "python3 scripts/check_style_gate.py",
    ),
    "scripts/check_q_style.py": (
        "def check_q_line(",
        "--self-test",
        "def main()",
    ),
    "scripts/check_license_provenance.py": (
        "def check_notice(",
        "def check_spdx(",
        "--self-test",
        "def main()",
    ),
    "scripts/ruff_gate.py": (
        "def pinned_binary(",
        "def pinned_version(",
        "def main()",
    ),
    "scripts/check_quality_contract.py": (
        "def validate(",
        "def check_ruff_settings(",
        "--self-test",
        "def main()",
    ),
    "scripts/check_ruff_added_lines.py": (
        "def parse_added_lines(",
        "def findings_on_added_lines(",
        "--self-test",
        "def main()",
    ),
    "scripts/check_style_references.py": (
        "def parse_index(",
        "def check(",
        "--self-test",
        "def main()",
    ),
    "scripts/check_rust_style.py": (
        "ASSERT_MACRO",
        "--self-test",
        "def check_lines(",
        "def main()",
    ),
    "scripts/check_source_style.py": (
        "def check_python_line(",
        "def check_cpp_line(",
        "def check_shell_line(",
        "def check_makefile_line(",
        "def check_json_line(",
        "def check_markdown_line(",
        "SHELL_EXTS",
        "--self-test",
        "def main()",
    ),
}

EXPECTED_LANGUAGE_FUNCTIONS: tuple[str, ...] = (
    "def check_python_line(",
    "def check_cpp_line(",
    "def check_shell_line(",
    "def check_makefile_line(",
    "def check_json_line(",
    "def check_markdown_line(",
)

MIN_TESTS: dict[str, int] = {
    "scripts/check_license_provenance.py": 4,
    "scripts/check_quality_contract.py": 5,
    "scripts/check_ruff_added_lines.py": 5,
    "scripts/check_style_references.py": 3,
    "scripts/check_rust_style.py": 13,
    "scripts/check_source_style.py": 26,
    "scripts/check_style_gate.py": 7,
    "scripts/check_prose_style.py": 12,
    "scripts/check_q_style.py": 16,
}


def behavioural_checks(root: Path = ROOT) -> list[str]:
    """Run canonical bad lines so a gutted rule fails every wiring path."""
    if root != ROOT:
        # Synthetic trees in self-tests contain placeholder text, not modules.
        return []
    scripts = str(root / "scripts")
    if scripts not in sys.path:
        sys.path.insert(0, scripts)
    try:
        import check_rust_style as rust
        import check_source_style as source
        import check_prose_style as prose
        import check_q_style as qstyle
    except ImportError as error:
        return [f"cannot import style checker: {error}"]

    failures: list[str] = []

    def expect(function: object, text: str, needle: str, **kwargs: object) -> None:
        messages = function(text, **kwargs)  # type: ignore[operator]
        if not any(needle in message for message in messages):  # type: ignore[union-attr]
            failures.append(f"rule behaviour lost: {needle!r} for {text!r}")

    expect(prose.check_prose_line, "we require evidence", "PROSE-001")
    expect(qstyle.check_q_line, "/", "bare /")
    expect(qstyle.check_q_line, "if[count args < 2;x]", "count")
    expect(source.check_python_line, "x = 1;", "semicolon")
    expect(source.check_cpp_line, "int *p = NULL;", "nullptr", header=False, cpp=True)
    expect(source.check_shell_line, "echo `date`", "backticks")
    expect(source.check_shell_line, "eval", "eval")
    expect(source.check_makefile_line, "all: ", "trailing whitespace")
    expect(source.check_json_line, '\t"a": 1', "tab")
    expect(
        source.check_markdown_line,
        "##Bad",
        "space",
        in_fence=False,
        bare_fence_opening=False,
        is_fence_marker=False,
    )
    if not rust.assertion_context_errors("assert!(x);"):
        failures.append("rule behaviour lost: Rust assertion context")
    return failures


def validate(root: Path = ROOT) -> list[str]:
    """Return one message per missing or weakened gate requirement."""
    errors: list[str] = []
    for relative, needles in REQUIRED_TEXT.items():
        path = root / relative
        if not path.is_file():
            errors.append(f"{relative}: required gate file is missing")
            continue
        text = path.read_text()
        for needle in needles:
            if needle not in text:
                errors.append(f"{relative}: missing required gate text {needle!r}")
    for relative, minimum in MIN_TESTS.items():
        path = root / relative
        if not path.is_file():
            continue
        tests = len(re.findall(r"^[ \t]+def test_", path.read_text(), re.M))
        if tests < minimum:
            errors.append(f"{relative}: {tests} self-tests, expected at least {minimum}")
    ci = root / ".github/workflows/ci.yml"
    if ci.is_file():
        # Count dedicated `run:` steps, not the --self-test line inside a
        # multi-line block, so deleting either integrity step is detected.
        count = sum(
            1
            for line in ci.read_text().splitlines()
            if line.strip() == "run: python3 scripts/check_style_gate.py"
        )
        if count < 2:
            errors.append(f"ci.yml: {count} dedicated integrity steps, expected at least 2")
    makefile = root / "Makefile"
    if makefile.is_file():
        count = sum(
            1
            for line in makefile.read_text().splitlines()
            if line.strip() == "python3 scripts/check_style_gate.py"
        )
        if count < 1:
            errors.append("Makefile: dedicated style gate invocation missing")
    errors.extend(behavioural_checks(root))
    return errors


class GateIntegrityTests(unittest.TestCase):
    def fake_root(self, directory: str) -> Path:
        root = Path(directory)
        for relative in REQUIRED_TEXT:
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(
                "\n".join(REQUIRED_TEXT[relative])
                + "\n"
                + "    def test_example() -> None:\n        pass\n" * MIN_TESTS.get(relative, 1)
            )
            if relative == ".github/workflows/ci.yml":
                path.write_text(
                    path.read_text()
                    + "      - name: Integrity one\n"
                    + "        run: python3 scripts/check_style_gate.py\n"
                    + "      - name: Integrity two\n"
                    + "        run: python3 scripts/check_style_gate.py\n"
                )
        return root

    def test_behavioural_checks_pass(self) -> None:
        self.assertEqual(behavioural_checks(), [])

    def test_language_coverage_cannot_be_removed(self) -> None:
        required = REQUIRED_TEXT["scripts/check_source_style.py"]
        for needle in EXPECTED_LANGUAGE_FUNCTIONS:
            self.assertIn(needle, required)
        self.assertEqual(len(EXPECTED_LANGUAGE_FUNCTIONS), 6)
        self.assertIn("scripts/check_q_style.py", REQUIRED_TEXT)
        self.assertIn("def check_q_line(", REQUIRED_TEXT["scripts/check_q_style.py"])

    def test_complete_gate_passes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(validate(self.fake_root(directory)), [])

    def test_removed_makefile_wiring_is_detected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.fake_root(directory)
            path = root / "Makefile"
            path.write_text(path.read_text().replace("$(MAKE) style", ""))
            self.assertTrue(any("Makefile" in e for e in validate(root)))

    def test_removed_ci_integrity_step_is_detected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.fake_root(directory)
            path = root / ".github/workflows/ci.yml"
            path.write_text(
                path.read_text().replace(
                    "run: python3 scripts/check_style_gate.py", "run: echo skip", 1
                )
            )
            self.assertTrue(any("dedicated integrity steps" in e for e in validate(root)))

    def test_removed_language_rule_is_detected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.fake_root(directory)
            path = root / "scripts/check_source_style.py"
            path.write_text(path.read_text().replace("def check_shell_line(", ""))
            self.assertTrue(any("check_shell_line" in e for e in validate(root)))

    def test_removed_self_test_is_detected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.fake_root(directory)
            path = root / "scripts/check_source_style.py"
            lines = path.read_text().splitlines()
            lines = [line for line in lines if "def test_" not in line]
            path.write_text("\n".join(lines))
            self.assertTrue(any("self-tests" in e for e in validate(root)))


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(GateIntegrityTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1
    errors = validate()
    if errors:
        for error in errors:
            print(f"FAIL: {error}")
        print(f"{len(errors)} style-gate integrity violation(s)")
        return 1
    print("PASS: style gate wiring and rule inventory are intact")
    return 0


if __name__ == "__main__":
    sys.exit(main())
