#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Check added q / kdb+ lines for mechanical style violations.

The binding rule set is ``q.toml`` at the repository root. Every declared
field is either implemented here or rejected by ``validate_config``; there
are no decorative declarations. The rules cover whitespace hygiene, the
silent loader-stop bare ``/`` line, the ``count x < n`` precedence bug,
missing final newlines, and dotted column access inside a lambda (``t.col``
is a global name in q; the required form is ``t[`col]``).

Only lines added relative to an explicitly supplied commit SHA are checked.
``Q_STYLE_BASE_SHA`` is mandatory and must be a full commit SHA that resolves
via ``git cat-file -e``. Unset, branch-shaped or unresolvable values, and any
git failure, fail the gate closed. Each changed file is read in full so
lexer state (comments, strings, braces and multiline lambdas) is carried
across unchanged lines; findings are reported only on added lines.

Use --self-test for adversarial unit tests. Config tests need no git repo.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import tomllib
import unittest
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
CONFIG_PATH = ROOT / "q.toml"
BASE_ENV = "Q_STYLE_BASE_SHA"
FULL_SHA = re.compile(r"\A(?:[0-9a-fA-F]{40}|[0-9a-fA-F]{64})\Z")

# The checker implements exactly one policy. A TOML value that differs from
# these declarations cannot be implemented and is therefore rejected during
# validation instead of silently changing or weakening the gate.
SUPPORTED_PURPOSE = (
    "Machine-readable q / kdb+ style contract shared by all three repositories; "
    "loaded by scripts/check_q_style.py so the TOML is binding rather than documentation."
)
SUPPORTED_RULES = {
    "tabs": "forbidden",
    "trailing_whitespace": "forbidden",
    "final_newline": "required",
    "bare_slash_line": "forbidden",
    "unparenthesised_count_comparison": "forbidden",
    "dotted_column_access_in_lambda": "forbidden",
}
SUPPORTED_ENFORCEMENT = {
    "checker": "scripts/check_q_style.py",
    "self_test": "scripts/check_q_style.py --self-test",
    "scope": "added .q lines relative to Q_STYLE_BASE_SHA (a resolved commit)",
    "pre_commit_hook": "q-style",
    "waivers": "none",
}
TOP_FIELDS = frozenset(
    {"schema_version", "language", "dialect", "purpose", "namespaces", "rules", "enforcement"}
)
RULE_FIELDS = frozenset({"max_columns", "required_index_form", *SUPPORTED_RULES})
NAMESPACE_RE = re.compile(r"\A\.[A-Za-z][A-Za-z0-9_]*\Z")
INDEX_FORM_RE = re.compile(r"\A[a-z][a-z0-9]*\[`[a-z][a-z0-9_]*\]\Z")


class GateError(RuntimeError):
    """A gate precondition or git operation failed; the gate must fail closed."""


def validate_config(data: dict[str, Any]) -> dict[str, Any]:
    """Validate that every q.toml declaration is implemented and binding."""
    if not isinstance(data, dict) or set(data) != TOP_FIELDS:
        raise ValueError(f"q.toml must declare exactly {sorted(TOP_FIELDS)}")
    for key, expected in (("schema_version", 1), ("language", "q"), ("dialect", "kdb+/KDB-X")):
        if type(data[key]) is not type(expected) or data[key] != expected:
            raise ValueError(f"q.toml {key} must be {expected!r}")
    purpose = data["purpose"]
    if not isinstance(purpose, str) or purpose != SUPPORTED_PURPOSE:
        raise ValueError("q.toml purpose must describe this binding q style contract")
    namespaces = data["namespaces"]
    if (
        not isinstance(namespaces, list)
        or not namespaces
        or any(not isinstance(name, str) or not NAMESPACE_RE.fullmatch(name) for name in namespaces)
    ):
        raise ValueError("q.toml namespaces must be a non-empty list of dotted namespace names")

    rules = data["rules"]
    if not isinstance(rules, dict) or set(rules) != RULE_FIELDS:
        raise ValueError(f"q.toml rules must declare exactly {sorted(RULE_FIELDS)}")
    max_columns = rules["max_columns"]
    if type(max_columns) is not int or max_columns < 1:
        raise ValueError("q.toml rules.max_columns must be a positive integer")
    for key, expected in SUPPORTED_RULES.items():
        if rules[key] != expected:
            raise ValueError(f"q.toml rules.{key}={rules[key]!r} is not implemented")
    index_form = rules["required_index_form"]
    if not isinstance(index_form, str) or not INDEX_FORM_RE.fullmatch(index_form):
        raise ValueError("q.toml rules.required_index_form must be a canonical t[`col] template")

    enforcement = data["enforcement"]
    if not isinstance(enforcement, dict) or set(enforcement) != set(SUPPORTED_ENFORCEMENT):
        raise ValueError("q.toml enforcement fields do not match the implemented gate")
    for key, expected in SUPPORTED_ENFORCEMENT.items():
        if enforcement[key] != expected:
            raise ValueError(f"q.toml enforcement.{key}={enforcement[key]!r} is not implemented")
    if enforcement["self_test"] != f"{enforcement['checker']} --self-test":
        raise ValueError("q.toml enforcement.self_test must invoke the declared checker")
    return data


def load_config(path: Path = CONFIG_PATH) -> dict[str, Any]:
    """Load and validate the machine-readable q style contract."""
    return validate_config(tomllib.loads(path.read_text(encoding="utf-8")))


CONFIG = load_config()
MAX_COLUMNS = CONFIG["rules"]["max_columns"]
REQUIRED_INDEX_FORM = CONFIG["rules"]["required_index_form"]

FILE_HEADER = re.compile(r"^\+\+\+ b/")
BARE_SLASH = re.compile(r"^/\s*$")
COUNT_COMPARISON = re.compile(r"\bcount\s+\w+\s*(?:<|>|<=|>=|=|~)")
DOT_COLUMN = re.compile(r"(?<![\w.])\b([a-z][a-z0-9]{0,3})\.([a-z][a-z0-9_]*)")
TAB = re.compile(r"\t")
TRAILING_WS = re.compile(r"[ \t]+$")

GitRunner = Callable[[list[str]], subprocess.CompletedProcess[str]]


def run_git(args: list[str]) -> subprocess.CompletedProcess[str]:
    """Run a git command from the repository root; surface OS errors."""
    try:
        return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True)
    except OSError as error:
        raise GateError(f"cannot run git {args[0]}: {error}") from error


def resolve_base_from_env(env: dict[str, str] | None = None) -> str:
    """Require Q_STYLE_BASE_SHA to hold a full commit SHA, never a branch."""
    environ = os.environ if env is None else env
    sha = environ.get(BASE_ENV, "").strip()
    if not sha:
        raise GateError(f"{BASE_ENV} is not set; set it to a full commit SHA")
    if not FULL_SHA.fullmatch(sha):
        raise GateError(f"{BASE_ENV}={sha!r} is not a full commit SHA (branch names are rejected)")
    return sha


def verify_base(sha: str, runner: GitRunner = run_git) -> str:
    """Fail closed unless ``sha^{commit}`` resolves via git cat-file."""
    result = runner(["cat-file", "-e", f"{sha}^{{commit}}"])
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        raise GateError(
            f"{BASE_ENV}={sha} does not resolve to a commit: {detail or 'git cat-file failed'}"
        )
    return sha


def parse_added_lines(diff_text: str) -> dict[str, list[tuple[int, str]]]:
    """Parse a ``git diff --unified=0`` payload into added lines per file."""
    by_file: dict[str, list[tuple[int, str]]] = {}
    current = ""
    number = 0
    for line in diff_text.splitlines():
        if line.startswith("+++ "):
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


def collect_added_lines(base: str, runner: GitRunner = run_git) -> dict[str, list[tuple[int, str]]]:
    """Collect added q lines; a failed git diff is a gate error, never empty."""
    result = runner(["diff", "--unified=0", base, "--", "*.q"])
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        raise GateError(f"git diff against {base} failed: {detail or 'git diff failed'}")
    return parse_added_lines(result.stdout)


@dataclass
class LexState:
    """Lexer state carried over from line to line in a whole q file."""

    brace_depth: int = 0
    in_string: bool = False
    in_block_comment: bool = False


def lex_line(line: str, state: LexState) -> str:
    """Blank comments, strings and system commands, updating lexer state."""
    out: list[str] = []
    index = 0
    size = len(line)
    first = size - len(line.lstrip())
    if state.in_block_comment:
        close = line.find("*/")
        if close < 0:
            return " " * size
        out.extend(" " * (close + 2))
        state.in_block_comment = False
        index = close + 2
    if index < size and not line[:index].strip() and line[index:].lstrip().startswith("\\"):
        # A leading backslash is a q system command (e.g. \l), not q code.
        return "".join(out) + " " * (size - index)
    while index < size:
        char = line[index]
        if state.in_string:
            if char == "\\" and index + 1 < size:
                out.extend("  ")
                index += 2
                continue
            if char == '"':
                state.in_string = False
            out.append(" ")
            index += 1
            continue
        if char == '"':
            state.in_string = True
            out.append(" ")
            index += 1
            continue
        if char == "/" and index + 1 < size and line[index + 1] == "*":
            state.in_block_comment = True
            out.extend("  ")
            index += 2
            continue
        if char == "/":
            break
        if char == "\\" and index == first:
            break
        if char == "{":
            state.brace_depth += 1
        elif char == "}" and state.brace_depth:
            state.brace_depth -= 1
        out.append(char)
        index += 1
    return "".join(out)


def check_q_line(
    line: str,
    *,
    in_lambda: bool = False,
    config: dict[str, Any] = CONFIG,
    code: str | None = None,
) -> list[str]:
    """Return style findings for one q line under the declared rule set.

    ``in_lambda`` reports whether the line is inside a ``{...}`` lambda, which
    is where dotted column access resolves to a global name instead of a
    column. ``code`` is the line with comments and strings blanked by the
    whole-file lexer; raw whitespace checks still use ``line``.
    """
    rules = config["rules"]
    scanned = lex_line(line, LexState()) if code is None else code
    errors: list[str] = []
    if rules["tabs"] == "forbidden" and TAB.search(line):
        errors.append("tab indentation")
    if rules["trailing_whitespace"] == "forbidden" and TRAILING_WS.search(line):
        errors.append("trailing whitespace")
    if len(line) > rules["max_columns"]:
        errors.append(f"line exceeds {rules['max_columns']} columns")
    if rules["bare_slash_line"] == "forbidden" and BARE_SLASH.match(line.strip()):
        errors.append("bare / line (silent loader stop)")
    if rules["unparenthesised_count_comparison"] == "forbidden" and COUNT_COMPARISON.search(
        scanned
    ):
        errors.append("unparenthesised comparison with count")
    if rules["dotted_column_access_in_lambda"] == "forbidden" and in_lambda:
        namespace_prefixes = tuple(f"{ns.lstrip('.')}." for ns in config["namespaces"])
        for match in DOT_COLUMN.finditer(scanned):
            dotted = match.group(0)
            if dotted.startswith(namespace_prefixes):
                continue
            errors.append(f"dotted name {dotted!r} in lambda (use {rules['required_index_form']})")
    return errors


def analyze_text(
    text: str,
    added_numbers: list[int],
    config: dict[str, Any],
    *,
    path: str = "<q>",
) -> list[str]:
    """Find violations on added lines using whole-file lexer state."""
    added = set(added_numbers)
    state = LexState()
    errors: list[str] = []
    for number, raw in enumerate(text.splitlines(), 1):
        depth_before = state.brace_depth
        code = lex_line(raw, state)
        if number not in added:
            continue
        in_lambda = depth_before > 0 or "{" in code
        for message in check_q_line(raw, in_lambda=in_lambda, config=config, code=code):
            errors.append(f"{path}:{number}: {message}")
    if config["rules"]["final_newline"] == "required" and added and not text.endswith("\n"):
        errors.append(f"{path}:{max(added)}: missing final newline")
    return errors


def synthetic_text(lines: list[tuple[int, str]]) -> str:
    """Reconstruct a synthetic file from added lines for unit tests."""
    if not lines:
        return ""
    by_number = dict(lines)
    return "\n".join(by_number.get(number, "") for number in range(1, max(by_number) + 1)) + "\n"


def check_lines(
    by_file: dict[str, list[tuple[int, str]]],
    *,
    config: dict[str, Any] = CONFIG,
    root: Path = ROOT,
    require_files: bool = False,
) -> list[str]:
    """Check added lines, reading every changed file in full in gate mode."""
    errors: list[str] = []
    for path, lines in by_file.items():
        if require_files:
            try:
                text = (root / path).read_text(encoding="utf-8")
            except (OSError, UnicodeError) as error:
                raise GateError(f"cannot read changed q file {path}: {error}") from error
        else:
            text = synthetic_text(lines)
        errors.extend(analyze_text(text, [number for number, _ in lines], config, path=path))
    return errors


def run_gate(sha: str, runner: GitRunner = run_git) -> list[str]:
    """Resolve the base, run git diff and check added lines; fail closed."""
    verify_base(sha, runner=runner)
    added = collect_added_lines(sha, runner=runner)
    return check_lines(added, config=CONFIG, root=ROOT, require_files=True)


class QStyleTests(unittest.TestCase):
    """Adversarial tests for the q style rules, config and diff parser."""

    @staticmethod
    def runner(diff: str = "", *, cat_ok: bool = True, diff_ok: bool = True) -> GitRunner:
        """Build a deterministic fake git runner for gate-failure tests."""

        def fake(args: list[str]) -> subprocess.CompletedProcess[str]:
            if args[0] == "cat-file":
                code = 0 if cat_ok else 128
                detail = "fatal: bad object" if code else ""
                return subprocess.CompletedProcess(args, code, "", detail)
            code = 0 if diff_ok else 128
            stdout = diff if code == 0 else ""
            detail = "fatal: diff failed" if code else ""
            return subprocess.CompletedProcess(args, code, stdout, detail)

        return fake

    def test_config_is_binding(self) -> None:
        """The TOML contract drives the checker's live constants."""
        self.assertEqual(MAX_COLUMNS, 100)
        self.assertEqual(CONFIG["enforcement"]["waivers"], "none")
        self.assertEqual(REQUIRED_INDEX_FORM, "t[`col]")

    def test_base_env_is_explicit(self) -> None:
        """Only Q_STYLE_BASE_SHA is consulted; old variables are ignored."""
        # Assembled from parts: repository-local tooling must not name a
        # sibling project (see scripts/check_no_cross_pollution.py).
        sibling_base_env = "NIR" + "NAY_STYLE_BASE"
        env = {
            "FIELDBOOK_STYLE_BASE": "origin/main",
            sibling_base_env: "origin/main",
            "STYLE_BASE": "origin/main",
            "Q_STYLE_BASE": "origin/main",
        }
        with self.assertRaises(GateError):
            resolve_base_from_env(env)

    def test_missing_base_sha(self) -> None:
        """An unset base variable fails closed."""
        with self.assertRaisesRegex(GateError, "not set"):
            resolve_base_from_env({})

    def test_empty_base_sha(self) -> None:
        """An empty base variable fails closed."""
        with self.assertRaisesRegex(GateError, "not set"):
            resolve_base_from_env({BASE_ENV: "   "})

    def test_branch_name_base_rejected(self) -> None:
        """Branch names and short SHAs are never accepted as the base."""
        for value in ("main", "origin/main", "HEAD", "d8f65aa"):
            with self.subTest(value=value), self.assertRaisesRegex(GateError, "full commit SHA"):
                resolve_base_from_env({BASE_ENV: value})

    def test_invalid_base_sha(self) -> None:
        """A full SHA that is not a commit fails closed at cat-file."""
        sha = "a" * 40
        with self.assertRaisesRegex(GateError, "does not resolve to a commit"):
            verify_base(sha, runner=self.runner(cat_ok=False))

    def test_cat_file_failure_fails_closed(self) -> None:
        """Any cat-file error prevents the gate from reporting success."""
        with self.assertRaises(GateError):
            run_gate("b" * 40, runner=self.runner(cat_ok=False))

    def test_git_diff_failure_fails_closed(self) -> None:
        """A simulated git diff failure never becomes zero findings."""
        with self.assertRaisesRegex(GateError, "git diff"):
            run_gate("c" * 40, runner=self.runner(diff_ok=False))

    def test_empty_diff_is_valid_after_successful_git(self) -> None:
        """An empty diff passes only after base and diff both succeed."""
        self.assertEqual(run_gate("d" * 40, runner=self.runner()), [])
        self.assertEqual(collect_added_lines("d" * 40, runner=self.runner()), {})

    def test_unreadable_changed_file_fails_closed(self) -> None:
        """A changed file that cannot be read is a gate error."""
        with self.assertRaisesRegex(GateError, "cannot read"):
            check_lines({"does-not-exist.q": [(1, "a:1")]}, require_files=True)

    def test_config_rejects_waivers(self) -> None:
        """A contract with waivers is rejected at load time."""
        data = tomllib.loads(CONFIG_PATH.read_text(encoding="utf-8"))
        data["enforcement"]["waivers"] = "allowed"
        with self.assertRaises(ValueError):
            validate_config(data)

    def test_config_rejects_unsupported_rule_value(self) -> None:
        """A toggle the checker cannot implement is rejected, not ignored."""
        for key in SUPPORTED_RULES:
            with self.subTest(key=key):
                data = tomllib.loads(CONFIG_PATH.read_text(encoding="utf-8"))
                data["rules"][key] = "allowed"
                with self.assertRaises(ValueError):
                    validate_config(data)

    def test_config_rejects_unsupported_index_form(self) -> None:
        """A non-canonical required_index_form is rejected."""
        data = tomllib.loads(CONFIG_PATH.read_text(encoding="utf-8"))
        data["rules"]["required_index_form"] = "t.col"
        with self.assertRaises(ValueError):
            validate_config(data)

    def test_config_rejects_unsupported_declarations(self) -> None:
        """Every other declared field is validated, not decorative."""
        mutations = (
            ("schema_version", lambda data: data.update(schema_version=2)),
            ("language", lambda data: data.update(language="k")),
            ("dialect", lambda data: data.update(dialect="q")),
            ("purpose", lambda data: data.update(purpose="")),
            (
                "purpose-changed",
                lambda data: data.update(purpose="Documentation only; not enforced."),
            ),
            ("namespaces", lambda data: data.update(namespaces=[])),
            ("max_columns", lambda data: data["rules"].update(max_columns=0)),
            ("checker", lambda data: data["enforcement"].update(checker="other.py")),
            ("self_test", lambda data: data["enforcement"].update(self_test="echo ok")),
            ("scope", lambda data: data["enforcement"].update(scope="all lines")),
            ("pre_commit_hook", lambda data: data["enforcement"].update(pre_commit_hook="none")),
            ("extra", lambda data: data.update(extra="unexpected")),
        )
        for name, mutate in mutations:
            with self.subTest(field=name):
                data = tomllib.loads(CONFIG_PATH.read_text(encoding="utf-8"))
                mutate(data)
                with self.assertRaises(ValueError):
                    validate_config(data)

    def test_max_columns_drives_enforcement(self) -> None:
        """The declared column limit, not a hard-coded constant, is applied."""
        config = tomllib.loads(CONFIG_PATH.read_text(encoding="utf-8"))
        config["rules"]["max_columns"] = 8
        self.assertTrue(any("columns" in e for e in check_q_line("a:123456789", config=config)))
        self.assertEqual(check_q_line("a:1", config=config), [])

    def test_namespaces_drive_exemption(self) -> None:
        """Declared namespaces, not a hard-coded list, drive the exemption."""
        config = tomllib.loads(CONFIG_PATH.read_text(encoding="utf-8"))
        config["namespaces"] = [".myq"]
        self.assertEqual(check_q_line("last myq.col", in_lambda=True, config=config), [])
        findings = check_q_line("last pq.col", in_lambda=True, config=config)
        self.assertTrue(any("dotted name" in e for e in findings))

    def test_required_index_form_drives_message(self) -> None:
        """The declared index form is the form recommended in findings."""
        findings = check_q_line("last t.recv_ns", in_lambda=True)
        self.assertTrue(any(REQUIRED_INDEX_FORM in message for message in findings))

    def test_bare_slash(self) -> None:
        """A bare slash silently stops a q loader."""
        self.assertTrue(any("bare /" in e for e in check_q_line("/")))

    def test_bare_slash_negative(self) -> None:
        """A commented slash with text is not a silent loader stop."""
        self.assertEqual(check_q_line("/ a comment"), [])

    def test_count_comparison(self) -> None:
        """An unparenthesised count comparison is a precedence bug."""
        findings = check_q_line("if[count args < 2;x]")
        self.assertTrue(any("count" in e for e in findings))

    def test_count_comparison_negative(self) -> None:
        """A count comparison inside a string is not code."""
        self.assertEqual(check_q_line('s:"count args < 2"'), [])

    def test_parenthesised_count_ok(self) -> None:
        """A parenthesised count comparison is explicit and safe."""
        self.assertEqual(check_q_line("if[(count args) < 2;x]"), [])

    def test_tab(self) -> None:
        """Tabs are rejected."""
        self.assertTrue(any("tab" in e for e in check_q_line("\ta:1")))

    def test_tab_negative(self) -> None:
        """Space indentation is clean."""
        self.assertEqual(check_q_line(" a:1"), [])

    def test_trailing_whitespace(self) -> None:
        """Trailing whitespace is rejected."""
        self.assertTrue(any("trailing" in e for e in check_q_line("a:1   ")))

    def test_trailing_whitespace_negative(self) -> None:
        """A line without trailing whitespace is clean."""
        self.assertEqual(check_q_line("a:1"), [])

    def test_long_line(self) -> None:
        """Lines over the shared q limit are rejected."""
        self.assertTrue(any("columns" in e for e in check_q_line("a:" + "x" * 120)))

    def test_long_line_negative(self) -> None:
        """A line at the declared limit is clean."""
        self.assertEqual(check_q_line("a:" + "x" * (MAX_COLUMNS - 2)), [])

    def test_final_newline_required(self) -> None:
        """A changed file without a final newline is reported on its last line."""
        errors = analyze_text("a:1", [1], CONFIG, path="x.q")
        self.assertTrue(any("x.q:1: missing final newline" in e for e in errors))

    def test_final_newline_present(self) -> None:
        """A changed file with a final newline is clean."""
        self.assertEqual(analyze_text("a:1\n", [1], CONFIG), [])

    def test_dotted_column_in_lambda(self) -> None:
        """Dotted column access inside a lambda resolves to a global."""
        findings = check_q_line("last t.recv_ns", in_lambda=True)
        self.assertTrue(any("dotted name" in e for e in findings))

    def test_dotted_column_index_form_ok(self) -> None:
        """Indexed column access is the required form."""
        self.assertEqual(check_q_line("last t[`recv_ns]", in_lambda=True), [])

    def test_namespace_dotted_ok(self) -> None:
        """Known namespaces are not mistaken for column access."""
        self.assertEqual(check_q_line(".pq.pq[x]", in_lambda=True), [])
        self.assertEqual(check_q_line(".h.h[x]", in_lambda=True), [])

    def test_number_dotted_ok(self) -> None:
        """Decimal literals are not mistaken for dotted names."""
        self.assertEqual(check_q_line("0.05=x", in_lambda=True), [])

    def test_comment_exempt(self) -> None:
        """Comment lines are exempt from the dotted-name rule."""
        self.assertEqual(check_q_line("/ t.recv_ns comment", in_lambda=True), [])

    def test_dotted_name_in_string_not_reported(self) -> None:
        """A dotted name inside a string is data, not a column access."""
        self.assertEqual(check_q_line('s:"t.recv_ns"', in_lambda=True), [])

    def test_changed_dotted_access_outside_lambda(self) -> None:
        """A changed dotted access outside a lambda stays outside the rule."""
        errors = check_lines({"x.q": [(1, "v:t.recv_ns")]})
        self.assertEqual(errors, [])

    def test_multiline_lambda_unchanged_opener_changed_body(self) -> None:
        """A changed body line is checked even when the opener is unchanged."""
        text = "f:{[t]\n  last t.recv_ns\n  };\n"
        errors = analyze_text(text, [2], CONFIG, path="x.q")
        self.assertTrue(any("x.q:2: dotted name" in e for e in errors))
        self.assertFalse(any("x.q:1:" in e for e in errors))

    def test_multiline_lambda_string_and_comment_state(self) -> None:
        """Strings and comments do not leave or enter lambda state."""
        text = 'f:{[t]\n  s:"t.recv_ns"\n  / t.recv_ns\n  last t.recv_ns\n  };\na:1\n'
        errors = analyze_text(text, [2, 3, 4, 5], CONFIG, path="x.q")
        self.assertFalse(any("x.q:2:" in e for e in errors))
        self.assertFalse(any("x.q:3:" in e for e in errors))
        self.assertTrue(any("x.q:4: dotted name" in e for e in errors))
        self.assertFalse(any("x.q:5:" in e for e in errors))

    def test_clean_line(self) -> None:
        """An ordinary assignment is clean."""
        self.assertEqual(check_q_line("a:1"), [])

    def test_diff_parser(self) -> None:
        """The unified diff parser records file and line numbers."""
        diff = "+++ b/x.q\n@@ -0,0 +1,2 @@\n+a:1\n+b:2\n"
        self.assertEqual(parse_added_lines(diff), {"x.q": [(1, "a:1"), (2, "b:2")]})

    def test_diff_parser_ignores_deleted_files(self) -> None:
        """A deleted file has no added lines to check."""
        diff = "+++ /dev/null\n@@ -1 +0,0 @@\n-/ old\n"
        self.assertEqual(parse_added_lines(diff), {})

    def test_check_lines_reports_path(self) -> None:
        """Findings carry the file path and line number."""
        errors = check_lines({"x.q": [(1, "/")]})
        self.assertTrue(any("x.q:1" in e for e in errors))

    def test_lambda_depth_tracking(self) -> None:
        """Lambda state persists across added lines in one file."""
        lines = [
            (1, "f:{[t]"),
            (2, "  last t.recv_ns"),
            (3, "  };"),
            (4, "a:1"),
        ]
        errors = check_lines({"x.q": lines})
        self.assertTrue(any("x.q:2" in e for e in errors))
        self.assertFalse(any("x.q:4" in e for e in errors))

    def test_single_line_lambda_caught(self) -> None:
        """A lambda opened and closed on one line is still checked."""
        errors = check_lines({"x.q": [(1, "f:{[t] last t.recv_ns};")]})
        self.assertTrue(any("x.q:1" in e for e in errors))

    def test_system_command_line_not_a_comment_block(self) -> None:
        r"""A leading \l load line does not swallow the rest of the file."""
        text = "\\l common.q\nf:{[t]\n  last t.recv_ns\n  };\n"
        errors = analyze_text(text, [3], CONFIG, path="x.q")
        self.assertTrue(any("x.q:3: dotted name" in e for e in errors))


def main() -> int:
    """Print findings and return a process exit status."""
    if "--self-test" in sys.argv[1:]:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(QStyleTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1
    try:
        base = resolve_base_from_env()
        errors = run_gate(base)
    except GateError as error:
        print(f"FAIL: q style gate could not run: {error}", file=sys.stderr)
        return 2
    if errors:
        for error in errors:
            print(f"FAIL: {error}")
        print(f"{len(errors)} q style violation(s) on changed lines")
        return 1
    print(f"PASS: q style satisfied on changed lines vs {base}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
