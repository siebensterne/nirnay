#!/usr/bin/env python3
"""Check Rust style guidance on changed lines.

Binding sources:
  - Linux kernel Rust coding guidelines: https://docs.kernel.org/rust/coding-guidelines.html
  - Google Rust style guide: https://google.github.io/styleguide/rustguide.html

The check is limited to added lines relative to a base ref so
existing code is not churned. Run from the repository root. Set
NIRNAY_STYLE_BASE to compare against a different ref (default: main).

Use ``--self-test`` to run adversarial unit tests of the parser and scanners.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import unittest

ASSERT_MACRO = re.compile(r"\b(?P<name>(?:debug_)?assert(?:_eq|_ne|_matches)?)\s*!\s*\(")
FILE_HEADER = re.compile(r"^\+\+\+ b/")
GLOB_IMPORT = re.compile(r"^use [A-Za-z0-9_:]+::\*;\s*$")
FLAT_MULTI_IMPORT = re.compile(r"^use [A-Za-z0-9_:]+::\{[^}]*,[^}]*\};\s*$")
ALLOWED_GLOBS = {"use super::*;", "use crate::common::*;"}
TWO_VALUE_ASSERTS = {"assert_eq", "assert_ne", "debug_assert_eq", "debug_assert_ne"}


def parse_added_lines(diff_text: str) -> dict[str, list[tuple[int, str]]]:
    """Parse a ``git diff --unified=0`` payload into added lines per file.

    Line numbers come from hunk headers and increment for every added line,
    including across multiple hunks and files. Added content beginning with
    ``++`` is not mistaken for a file header; only ``+++ b/`` is.
    """
    by_file: dict[str, list[tuple[int, str]]] = {}
    current = ""
    number = 0
    for line in diff_text.splitlines():
        if line.startswith("+++ "):
            # A deletion target has no added lines; do not treat its marker as
            # added content left over from the previous file.
            current = line[6:] if FILE_HEADER.match(line) else ""
            number = 0
            continue
        if line.startswith("@@"):
            match = re.search(r"\+(\d+)", line)
            number = int(match.group(1)) if match else 0
            continue
        if line.startswith("+") and number:
            by_file.setdefault(current, []).append((number, line[1:]))
            number += 1
    return by_file


def collect_added_lines(base: str) -> dict[str, list[tuple[int, str]]]:
    result = subprocess.run(
        ["git", "diff", base, "--unified=0", "--", "*.rs"],
        check=True,
        capture_output=True,
        text=True,
    )
    return parse_added_lines(result.stdout)


def _macro_invocation(text: str, start: int) -> str:
    """Return the text of the balanced macro invocation beginning at start."""
    depth = 0
    in_string = False
    escaped = False
    for index in range(start, len(text)):
        char = text[index]
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            continue
        if char == '"':
            in_string = True
        elif char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
            if depth == 0:
                return text[start:index + 1]
    return text[start:]


def _top_level_arguments(invocation: str) -> list[str]:
    """Split a macro invocation into top-level comma-separated arguments."""
    inner = invocation[invocation.find("(") + 1: invocation.rfind(")")]
    args: list[str] = []
    depth = 0
    in_string = False
    escaped = False
    start = 0
    for index, char in enumerate(inner):
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            continue
        if char == '"':
            in_string = True
        elif char in "([{":
            depth += 1
        elif char in ")]}":
            depth -= 1
        elif char == "," and depth == 0:
            args.append(inner[start:index].strip())
            start = index + 1
    tail = inner[start:].strip()
    if tail:
        args.append(tail)
    return [arg for arg in args if arg]


def _is_string_literal(argument: str) -> bool:
    value = argument.strip()
    if value.startswith('"'):
        return value.endswith('"') and len(value) >= 2
    match = re.match(r'^r(#*)"', value)
    if match:
        hashes = match.group(1)
        return value.endswith('"' + hashes)
    return False


def assertion_context_errors(text: str) -> list[tuple[int, str]]:
    """Flag assertions whose final top-level argument is not a context string.

    Arity matters: in ``assert_eq!(actual, "expected")`` the string is the
    expected value, not a context message. Equality assertions therefore need
    at least three arguments; other assertions need at least two.
    """
    errors: list[tuple[int, str]] = []
    for match in ASSERT_MACRO.finditer(text):
        invocation = _macro_invocation(text, match.start())
        args = _top_level_arguments(invocation)
        name = match.group("name")
        minimum = 3 if name in TWO_VALUE_ASSERTS else 2
        if len(args) < minimum or not _is_string_literal(args[-1] if args else ""):
            line = text.count("\n", 0, match.start()) + 1
            errors.append((line, invocation.splitlines()[0][:80]))
    return errors


def check_lines(by_file: dict[str, list[tuple[int, str]]]) -> list[str]:
    errors: list[str] = []
    for path, lines in by_file.items():
        for number, content in lines:
            stripped = content.strip()
            if GLOB_IMPORT.match(stripped) and stripped not in ALLOWED_GLOBS:
                errors.append(f"{path}:{number}: glob import ({stripped!r})")
            if FLAT_MULTI_IMPORT.match(stripped):
                errors.append(
                    f"{path}:{number}: multi-item import must be vertical ({stripped!r})"
                )
            if stripped.startswith("#[allow(") and "//" not in stripped:
                errors.append(f"{path}:{number}: #[allow] without inline justification")
        text = "\n".join(content for _, content in lines)
        for offset, invocation in assertion_context_errors(text):
            number = lines[min(offset - 1, len(lines) - 1)][0] if lines else 0
            errors.append(f"{path}:{number}: assertion without context string: {invocation}")
    return errors


class StyleCheckerTests(unittest.TestCase):
    def test_diff_parser_multiple_hunks_and_files(self) -> None:
        diff = """+++ b/a.rs
@@ -1,2 +1,3 @@
 context
+first
+second
@@ -10,1 +11,2 @@
 context
+third
+++ b/b.rs
@@ -0,0 +1,1 @@
+fourth
"""
        self.assertEqual(
            parse_added_lines(diff),
            {
                "a.rs": [(1, "first"), (2, "second"), (11, "third")],
                "b.rs": [(1, "fourth")],
            },
        )

    def test_dev_null_marker_is_not_an_added_line(self) -> None:
        diff = "+++ b/a.rs\n@@ -1,1 +1,1 @@\n+real\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-removed\n"
        self.assertEqual(parse_added_lines(diff), {"a.rs": [(1, "real")]})

    def test_added_line_beginning_with_double_plus_is_kept(self) -> None:
        diff = "+++ b/a.rs\n@@ -1,1 +1,1 @@\n+++content\n"
        self.assertEqual(parse_added_lines(diff), {"a.rs": [(1, "++content")]})

    def test_assertion_last_argument_is_context(self) -> None:
        good = 'assert_eq!(left, right, "context");\nassert!(matches!(x, Err(E)), "bad state");'
        self.assertEqual(assertion_context_errors(good), [])

    def test_expected_value_string_is_not_context(self) -> None:
        bad = 'assert_eq!(actual, "expected");'
        self.assertEqual(len(assertion_context_errors(bad)), 1)

    def test_trailing_comma_is_not_an_empty_context(self) -> None:
        good = 'assert_eq!(actual, expected, "context",);'
        self.assertEqual(assertion_context_errors(good), [])

    def test_raw_string_context_is_accepted(self) -> None:
        good = 'assert_eq!(actual, expected, r#"raw context"#);'
        self.assertEqual(assertion_context_errors(good), [])

    def test_debug_assertions_are_checked(self) -> None:
        bad = 'debug_assert_eq!(left, right);\ndebug_assert!(condition);'
        self.assertEqual(len(assertion_context_errors(bad)), 2)

    def test_string_literal_inside_expression_is_not_context(self) -> None:
        bad = 'assert_eq!(parse("x"), Ok("x"));'
        self.assertEqual(len(assertion_context_errors(bad)), 1)

    def test_nested_macro_without_context_is_flagged(self) -> None:
        bad = 'assert!(matches!(value, Some("nested")));'
        self.assertEqual(len(assertion_context_errors(bad)), 1)

    def test_multiline_assertion_with_context_passes(self) -> None:
        good = '''assert!(
    matches!(value, Some("nested")),
    "value must be present"
);'''
        self.assertEqual(assertion_context_errors(good), [])

    def test_assertion_error_carries_line_number(self) -> None:
        errors = assertion_context_errors('let x = 1;\nassert!(x > 0);')
        self.assertEqual(errors[0][0], 2)

    def test_glob_and_flat_imports_are_flagged(self) -> None:
        lines = {"x.rs": [(1, "use std::io::*;"), (2, "use std::io::{Read, Write};"), (3, "use super::*;")]}
        errors = check_lines(lines)
        self.assertTrue(any("glob import" in error for error in errors))
        self.assertTrue(any("must be vertical" in error for error in errors))
        self.assertFalse(any(":3:" in error for error in errors))


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(StyleCheckerTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1

    base = os.environ.get("NIRNAY_STYLE_BASE", "main")
    errors = check_lines(collect_added_lines(base))
    if errors:
        for error in errors:
            print(f"FAIL: {error}")
        print(f"{len(errors)} Rust style violation(s) on changed lines")
        return 1
    print(f"PASS: Rust style guidance satisfied on changed lines vs {base}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
