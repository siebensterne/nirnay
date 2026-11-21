#!/usr/bin/env python3
"""Added-lines style gate for every non-Rust language in the repository.

Rust has its own gate (`check_rust_style.py`) plus rustfmt and Clippy. This
script applies the same idea to Python, C/C++, Makefiles, JSON/JSONL and
Markdown: only lines added relative to a base ref are checked, so existing code
is not churned. It has no third-party dependencies and is binding in
`make check`.

Policy sources and the repository's conflict resolutions are recorded in
`engineering/style-policy.md`. The authoritative formatters remain
`clang-format` for C/C++ (pinned in CI) and `ruff` for Python (opt-in until the
existing corpus is reformatted in a dedicated PR).

Run from the repository root. Set `NIRNAY_STYLE_BASE` to compare against a
different ref (default: `main`). Use `--self-test` for adversarial unit tests.
"""

from __future__ import annotations

import io
import json
import os
import re
import subprocess
import sys
import tempfile
import tokenize
import unittest
from pathlib import Path

MAX_PY_COLUMNS = 100
MAX_CPP_COLUMNS = 100
MAX_MD_COLUMNS = 100
MAX_SHELL_COLUMNS = 80

SHELL_EXTS = frozenset({".sh", ".bash", ".zsh"})
CPP_EXTS = frozenset({".c", ".cc", ".cpp", ".cxx", ".h", ".hpp", ".hxx"})
HEADER_EXTS = frozenset({".h", ".hpp", ".hxx"})
JSON_EXTS = frozenset({".json"})
JSONL_EXTS = frozenset({".jsonl"})
MD_EXTS = frozenset({".md"})
# Matcher vocabulary data: long regex constants cannot be wrapped without
# changing the expression. Mirrors fieldbook's documented exemption.
LONG_STRING_FILES = frozenset({"scripts/assurance/english_audit.py"})

FILE_HEADER = re.compile(r"^\+\+\+ b/")
HUNK_HEADER = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@")
HEADING_NEEDS_SPACE = re.compile(r"^\s*#{1,6}[^#\s]")
BARE_EXCEPT = re.compile(r"^\s*except\s*:")
WILDCARD_IMPORT = re.compile(r"^\s*from\s+\S+\s+import\s+\*")
BACKSLASH_CONTINUATION = re.compile(r"\\\s*$")
MUTABLE_DEFAULT = re.compile(r"\bdef\s+\w+\s*\([^)]*=\s*(\[|\{|(?:list|dict|set)\s*\()")
NULL_LITERAL = re.compile(r"\bNULL\b")
USING_NAMESPACE_STD = re.compile(r"^\s*using\s+namespace\s+std\s*;")
TARGET_LINE = re.compile(r"^[^#\t][^:=]*:(?:=|\+|\?|;|\s|$)")
FENCE_MARKER = re.compile(r"^\s*(```+|~~~+)")
SHELL_BACKTICK = re.compile(r"`[^`]+`")
SHELL_LEGACY_ARITH = re.compile(r"(?:^|[\s;(])(?:let|expr)(?:\s|$)|\$\[")
SHELL_BRACKET_TEST = re.compile(
    r"(?:^|[\s;&|()!])\[\s+"
    r"|(?:^|[;&|()!])\s*test\s+"
    r"|\b(?:if|while|until)\s+test\s+"
    r"|/usr/bin/\["
)
SHELL_LOCAL_SUBSHELL = re.compile(r"\b(?:local|declare)\s+\w+=[\"']?\$\(")
SHELL_ALIAS = re.compile(r"(?:^|[\s;(])alias(?:\s|;|\)|$)")
SHELL_EVAL = re.compile(r"(?:^|[\s;(])eval(?:\s|;|\)|$)")
SHELL_HEREDOC = re.compile(r"<<-?\s*['\"]?([A-Za-z0-9_][\w.-]*)['\"]?")


def extension_of(path: str) -> str:
    return Path(path).suffix.lower()


def is_makefile(path: str) -> bool:
    return Path(path).name == "Makefile" or Path(path).name.endswith(".mk")


def has_shell_shebang(path: str) -> bool:
    """Catch extensionless shell scripts, which the style guide permits."""
    try:
        with open(path, "rb") as handle:
            first = handle.readline(128)
    except OSError:
        return False
    if not first.startswith(b"#!"):
        return False
    tokens = re.findall(rb"[\w./-]+", first)
    shell_tokens = {b"sh", b"bash", b"zsh", b"/bin/sh", b"/bin/bash", b"/bin/zsh"}
    return any(
        token in shell_tokens
        or token.endswith(b"/sh")
        or token.endswith(b"/bash")
        or token.endswith(b"/zsh")
        for token in tokens
    )


def is_supported(path: str) -> bool:
    ext = extension_of(path)
    return (
        ext == ".py"
        or ext in CPP_EXTS
        or ext in SHELL_EXTS
        or ext in JSON_EXTS
        or ext in JSONL_EXTS
        or ext in MD_EXTS
        or is_makefile(path)
        or has_shell_shebang(path)
    )


def parse_header_path(line: str) -> str | None:
    """Return the path from a `+++ ` header, including git-quoted paths."""
    if line.startswith('+++ "') and line.endswith('"'):
        try:
            decoded = json.loads(line[4:])
        except json.JSONDecodeError:
            return None
        return decoded[2:] if decoded.startswith("b/") else decoded
    if FILE_HEADER.match(line):
        return line[6:]
    return None


def parse_added_lines(diff_text: str) -> dict[str, list[tuple[int, str]]]:
    """Parse `git diff --unified=0` into added lines keyed by path.

    Split only on `\\n` so a CRLF survives in the captured content; using
    `splitlines()` would make the CRLF rule impossible to enforce.
    """
    by_file: dict[str, list[tuple[int, str]]] = {}
    current = ""
    number = 0
    for line in diff_text.split("\n"):
        # Only a real `+++ b/...` header resets the path. An added source line
        # whose content begins with `++ ` also renders as `+++ ...` and must be
        # treated as content, not as a header.
        path = parse_header_path(line.rstrip("\r")) if line.startswith("+++ ") else None
        if path is not None:
            current = path
            number = 0
            continue
        if line.rstrip("\r") == "+++ /dev/null":
            current = ""
            number = 0
            continue
        if line.startswith("@@"):
            match = HUNK_HEADER.match(line)
            number = int(match.group(1)) if match else 0
            continue
        if line.startswith("+") and number and current:
            by_file.setdefault(current, []).append((number, line[1:]))
            number += 1
    return by_file


def collect_added_lines(base: str) -> dict[str, list[tuple[int, str]]]:
    result = subprocess.run(
        ["git", "diff", base, "--unified=0"],
        check=True,
        capture_output=True,
    )
    # Decode raw bytes: text mode would translate CRLF to LF and hide defects.
    added = parse_added_lines(result.stdout.decode("utf-8", errors="replace"))
    # Vendored references are byte-exact third-party documents verified by
    # check_style_references.py; they are not authored source and are excluded
    # from formatting and prose checks (same rule as fieldbook).
    excluded = ("engineering/references/", "references/")
    return {
        path: lines
        for path, lines in added.items()
        if is_supported(path) and not path.startswith(excluded)
    }


def trailing_whitespace(line: str) -> bool:
    return line.rstrip() != line


def strip_literals(line: str, comment_marker: str, *, blank_strings: bool = True) -> str:
    """Truncate comments and optionally blank quoted strings."""
    output: list[str] = []
    quote = ""
    escaped = False
    index = 0
    while index < len(line):
        char = line[index]
        if quote:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == quote:
                quote = ""
            output.append(" " if blank_strings else char)
            index += 1
            continue
        if char in "\"'":
            quote = char
            output.append(" " if blank_strings else char)
            index += 1
            continue
        if line.startswith(comment_marker, index):
            break
        output.append(char)
        index += 1
    return "".join(output)


def python_docstring_lines(text: str) -> set[int]:
    """Line numbers inside triple-quoted strings, where prose may use punctuation.

    Uses the standard tokenizer so triple quotes inside comments or ordinary
    strings cannot toggle the state for the rest of the file.
    """
    result: set[int] = set()
    try:
        tokens = tokenize.generate_tokens(io.StringIO(text).readline)
        for token in tokens:
            if token.type == tokenize.STRING and ('"""' in token.string or "'''" in token.string):
                for number in range(token.start[0], token.end[0] + 1):
                    result.add(number)
    except (tokenize.TokenError, IndentationError):
        pass
    return result


def check_python_line(
    line: str, *, in_docstring: bool = False, allow_long_lines: bool = False
) -> list[str]:
    """Return style findings for one added Python line."""
    errors: list[str] = []
    code = strip_literals(line, "#")
    if trailing_whitespace(line):
        errors.append("trailing whitespace")
    if re.match(r"^\s*\t", line):
        errors.append("tab indentation; use four spaces")
    if (
        not allow_long_lines
        and len(line) > MAX_PY_COLUMNS
        and "http://" not in line
        and "https://" not in line
    ):
        errors.append(f"line exceeds {MAX_PY_COLUMNS} columns")
    if not in_docstring:
        if ";" in code:
            errors.append("do not put a statement semicolon in Python source")
        if BARE_EXCEPT.match(line):
            errors.append("bare `except:` is forbidden; catch a specific exception")
        if WILDCARD_IMPORT.match(line):
            errors.append("wildcard import is forbidden")
        if BACKSLASH_CONTINUATION.search(line):
            errors.append("explicit backslash continuation; use implicit joining")
        if MUTABLE_DEFAULT.search(code):
            errors.append("mutable default argument; use None")
    return errors


def check_cpp_line(line: str, *, header: bool, cpp: bool) -> list[str]:
    errors: list[str] = []
    code = re.sub(r"/\*.*?\*/", "", strip_literals(line, "//"))
    if trailing_whitespace(line):
        errors.append("trailing whitespace")
    if "\t" in line:
        errors.append("tab character; .clang-format uses spaces")
    if len(line) > MAX_CPP_COLUMNS:
        errors.append(f"line exceeds {MAX_CPP_COLUMNS} columns")
    if cpp and NULL_LITERAL.search(code):
        errors.append("use nullptr instead of NULL")
    if header and USING_NAMESPACE_STD.match(line):
        errors.append("using-directive is forbidden in a header")
    return errors


def check_json_line(line: str) -> list[str]:
    errors: list[str] = []
    if trailing_whitespace(line):
        errors.append("trailing whitespace")
    leading = re.match(r"^[\t ]*", line)
    indent = leading.group(0) if leading else ""
    if "\t" in indent:
        errors.append("tab indentation; JSON uses two spaces")
    elif len(indent) % 2:
        errors.append("indentation must be a multiple of two spaces")
    return errors


def shell_heredoc_lines(text: str) -> set[int]:
    """Lines inside terminated here-documents, where content is literal data.

    An unterminated here-document is never exempted: exempting the remainder of
    the file would let a single malformed delimiter hide every shell rule.
    """
    lines = text.splitlines()
    result: set[int] = set()
    index = 0
    while index < len(lines):
        match = SHELL_HEREDOC.search(lines[index])
        if match:
            delimiter = match.group(1)
            closing = next(
                (
                    offset
                    for offset in range(index + 1, len(lines))
                    if lines[offset].strip() == delimiter
                ),
                None,
            )
            if closing is None:
                index += 1
                continue
            for offset in range(index, closing + 1):
                result.add(offset + 1)
            index = closing + 1
            continue
        index += 1
    return result


def shell_code_view(line: str) -> str:
    """Drop comments and single-quoted literals, keep double-quoted content.

    In bash, backticks and `$(...)` inside double quotes are real command
    substitutions, so they must remain visible to the rules. Single-quoted
    content is literal and comments are not code.
    """
    output: list[str] = []
    single = False
    double = False
    escaped = False
    for char in line:
        if escaped:
            output.append(char if double else " ")
            escaped = False
        elif char == "\\" and (single or double):
            escaped = True
            output.append(char if double else " ")
        elif char == "'" and not double:
            single = not single
            output.append(" ")
        elif char == '"' and not single:
            double = not double
            output.append('"')
        elif char == "#" and not single and not double:
            break
        else:
            output.append(" " if single else char)
    return "".join(output)


def check_shell_line(line: str, *, in_heredoc: bool = False) -> list[str]:
    errors: list[str] = []
    if trailing_whitespace(line):
        errors.append("trailing whitespace")
    if in_heredoc:
        return errors
    code = shell_code_view(line)
    if "\t" in line:
        errors.append("tab indentation; use two spaces")
    words = code.split()
    if len(line) > MAX_SHELL_COLUMNS and not (
        "http://" in line
        or "https://" in line
        or any(len(word) > MAX_SHELL_COLUMNS for word in words)
    ):
        errors.append(f"line exceeds {MAX_SHELL_COLUMNS} columns")
    if SHELL_BACKTICK.search(code):
        errors.append("use $(command) instead of backticks")
    if SHELL_EVAL.search(code):
        errors.append("eval is forbidden")
    if SHELL_LEGACY_ARITH.search(code):
        errors.append("use ((...)) or $((...)) instead of let/expr/$[...]")
    if SHELL_BRACKET_TEST.search(code):
        errors.append("use [[ ... ]] instead of [ ... ], test or /usr/bin/[")
    if SHELL_LOCAL_SUBSHELL.search(code):
        errors.append("declare local variables separately from command substitution")
    if SHELL_ALIAS.search(code):
        errors.append("aliases are forbidden in scripts; use a function")
    if "$*" in code:
        errors.append('use "$@" instead of $*')
    return errors


def check_makefile_line(line: str) -> list[str]:
    errors: list[str] = []
    if trailing_whitespace(line):
        errors.append("trailing whitespace")
    if "\r" in line:
        errors.append("CRLF line ending; use LF")
    return errors


def makefile_recipe_violations(text: str) -> set[int]:
    """Line numbers where a recipe command is indented with spaces."""
    violations: set[int] = set()
    lines = text.splitlines()
    previous_was_target = False
    for number, line in enumerate(lines, start=1):
        if not line.strip() or line.lstrip().startswith("#"):
            previous_was_target = False
            continue
        if line.startswith("\t"):
            previous_was_target = False
            continue
        if previous_was_target and line.startswith(" "):
            violations.add(number)
        previous_was_target = bool(TARGET_LINE.match(line))
    return violations


def markdown_fence_lines(text: str) -> tuple[set[int], set[int]]:
    """Return (fence markers opening without a language, fenced-content lines)."""
    bare_openings: set[int] = set()
    content: set[int] = set()
    inside = False
    for number, line in enumerate(text.splitlines(), start=1):
        match = FENCE_MARKER.match(line)
        if match:
            if not inside and line[match.end() :].strip() == "":
                bare_openings.add(number)
            inside = not inside
            continue
        if inside:
            content.add(number)
    return bare_openings, content


def check_markdown_line(
    line: str,
    *,
    in_fence: bool,
    bare_fence_opening: bool,
    is_fence_marker: bool,
) -> list[str]:
    errors: list[str] = []
    if trailing_whitespace(line):
        errors.append("trailing whitespace")
    if bare_fence_opening:
        errors.append("declare a language on fenced code blocks")
    if in_fence or is_fence_marker:
        return errors
    if HEADING_NEEDS_SPACE.match(line):
        errors.append("ATX heading needs a space after `#`")
    is_heading = bool(re.match(r"^#{1,6}\s", line))
    is_table = line.count("|") >= 2
    is_indented_code = line.startswith("    ")
    has_link = "](" in line or "http://" in line or "https://" in line
    if len(line) > MAX_MD_COLUMNS and not (is_heading or is_table or is_indented_code or has_link):
        errors.append(f"prose line exceeds {MAX_MD_COLUMNS} columns")
    return errors


def check_changed_file(path: str, added: list[tuple[int, str]]) -> dict[int, list[str]]:
    """Return violations by line number for one changed file."""
    errors: dict[int, list[str]] = {}
    add = lambda number, message: errors.setdefault(number, []).append(message)  # noqa: E731
    ext = extension_of(path)
    added_numbers = {number for number, _ in added}

    if ext == ".py":
        try:
            docstring_lines = python_docstring_lines(Path(path).read_text())
        except OSError:
            docstring_lines = set()
        for number, line in added:
            for message in check_python_line(
                line,
                in_docstring=number in docstring_lines,
                allow_long_lines=path in LONG_STRING_FILES,
            ):
                add(number, message)

    elif ext in CPP_EXTS:
        for number, line in added:
            for message in check_cpp_line(
                line, header=ext in HEADER_EXTS, cpp=ext in {".cc", ".cpp", ".cxx"}
            ):
                add(number, message)

    elif is_makefile(path):
        for number, line in added:
            for message in check_makefile_line(line):
                add(number, message)
        try:
            text = Path(path).read_text()
        except OSError:
            return errors
        for number in makefile_recipe_violations(text) & added_numbers:
            add(number, "recipe command must start with a tab")

    elif ext in SHELL_EXTS or has_shell_shebang(path):
        try:
            text = Path(path).read_text()
        except OSError:
            text = ""
        heredocs = shell_heredoc_lines(text)
        lines = text.splitlines()
        fully_added = bool(lines) and len(added_numbers) >= len(lines)
        if fully_added and lines and lines[0].strip() != "#!/bin/bash":
            add(1, "executable shell files must start with #!/bin/bash")
        for number, line in added:
            for message in check_shell_line(line, in_heredoc=number in heredocs):
                add(number, message)

    elif ext in JSON_EXTS:
        try:
            text = Path(path).read_text()
        except OSError:
            return errors
        try:
            json.loads(text)
        except json.JSONDecodeError as error:
            add(error.lineno, f"invalid JSON: {error.msg}")
        lines = text.splitlines()
        # Only flag a missing final newline when the final line was itself
        # added; legacy files that predate the rule are not churned.
        if text and not text.endswith("\n") and len(lines) in added_numbers:
            add(len(lines), "missing final newline")
        for number, line in added:
            if number > len(lines):
                continue
            for message in check_json_line(lines[number - 1]):
                add(number, message)

    elif ext in JSONL_EXTS:
        try:
            text = Path(path).read_text()
        except OSError:
            return errors
        for number, line in added:
            if not line.strip():
                continue
            if trailing_whitespace(line):
                add(number, "trailing whitespace")
            try:
                json.loads(line)
            except json.JSONDecodeError as error:
                add(number, f"invalid JSON record: {error.msg}")
        last = len(text.splitlines())
        if text and not text.endswith("\n") and last in added_numbers:
            add(last, "missing final newline")

    elif ext in MD_EXTS:
        try:
            text = Path(path).read_text()
        except OSError:
            return errors
        bare_openings, content = markdown_fence_lines(text)
        lines = text.splitlines()
        fully_added = len(added_numbers) >= len(lines)
        for number, line in added:
            if number > len(lines):
                continue
            for message in check_markdown_line(
                line,
                in_fence=number in content,
                bare_fence_opening=number in bare_openings,
                is_fence_marker=bool(FENCE_MARKER.match(line)),
            ):
                add(number, message)
        if fully_added:
            headings = [
                number for number, line in enumerate(lines, start=1) if line.startswith("# ")
            ]
            if len(headings) > 1:
                for number in headings[1:]:
                    add(number, "document has more than one H1 heading")

    return errors


class StyleCheckerTests(unittest.TestCase):
    def test_parser_handles_multiple_files_and_hunks(self) -> None:
        diff = """+++ b/a.py
@@ -1,1 +1,2 @@
 context
+good
@@ -10,1 +11,1 @@
-old
+new
+++ /dev/null
@@ -1,1 +0,0 @@
-removed
+++ b/src/b.cpp
@@ -0,0 +1,1 @@
+int x = 1
"""
        self.assertEqual(
            parse_added_lines(diff),
            {"a.py": [(1, "good"), (11, "new")], "src/b.cpp": [(1, "int x = 1")]},
        )

    def test_python_rules(self) -> None:
        self.assertEqual(check_python_line("x = 1"), [])
        self.assertIn("trailing whitespace", check_python_line("x = 1 "))
        self.assertIn("tab indentation; use four spaces", check_python_line("\tx = 1"))
        self.assertTrue(any("semicolon" in m for m in check_python_line("x = 1;")))
        self.assertTrue(any("bare" in m for m in check_python_line("except:")))
        self.assertTrue(any("wildcard" in m for m in check_python_line("from x import *")))
        self.assertTrue(any("backslash" in m for m in check_python_line("x = 1 + \\")))
        self.assertTrue(any("mutable" in m for m in check_python_line("def f(x=" + "[]" + "):")))

    def test_python_allows_urls_and_string_semicolons(self) -> None:
        long_url = "x = 'https://example.com/" + "a" * 120 + "'"
        self.assertEqual(check_python_line(long_url), [])
        self.assertEqual(check_python_line("text = 'a;b'"), [])

    def test_cpp_rules(self) -> None:
        self.assertEqual(check_cpp_line("int x;", header=False, cpp=True), [])
        self.assertTrue(
            any("nullptr" in m for m in check_cpp_line("x = NULL;", header=False, cpp=True))
        )
        self.assertTrue(
            any(
                "using-directive" in m
                for m in check_cpp_line("using namespace std;", header=True, cpp=True)
            )
        )
        self.assertEqual(check_cpp_line("using namespace std;", header=False, cpp=True), [])
        self.assertTrue(any("tab" in m for m in check_cpp_line("\tint x;", header=False, cpp=True)))

    def test_makefile_recipe_detection(self) -> None:
        text = "all: build\n\tmake build\nbuild:\n  echo wrong\n"
        self.assertEqual(makefile_recipe_violations(text), {4})

    def test_markdown_fences_and_headings(self) -> None:
        text = "# Title\n\n```\ncode\n```\n\n##Bad\n"
        bare_openings, content = markdown_fence_lines(text)
        self.assertEqual(bare_openings, {3})
        self.assertEqual(content, {4})
        self.assertTrue(
            any(
                "language" in m
                for m in check_markdown_line(
                    "```", in_fence=False, bare_fence_opening=True, is_fence_marker=True
                )
            )
        )
        self.assertTrue(
            any(
                "space" in m
                for m in check_markdown_line(
                    "##Bad", in_fence=False, bare_fence_opening=False, is_fence_marker=False
                )
            )
        )

    def test_json_and_jsonl(self) -> None:
        self.assertEqual(json.loads('{"a": 1}'), {"a": 1})
        with self.assertRaises(json.JSONDecodeError):
            json.loads("{ // comment\n}")

    def test_crlf_survives_diff_parsing(self) -> None:
        diff = "+++ b/Makefile\r\n@@ -0,0 +1,1 @@\r\n+\techo hi\r\n"
        parsed = parse_added_lines(diff)
        self.assertEqual(parsed, {"Makefile": [(1, "\techo hi\r")]})
        self.assertTrue(any("CRLF" in m for m in check_makefile_line("\techo hi\r")))

    def test_added_line_beginning_with_double_plus_is_content(self) -> None:
        diff = "+++ b/a.cpp\n@@ -0,0 +1,2 @@\n+++ i;\n+NULL;\n"
        self.assertEqual(
            parse_added_lines(diff),
            {"a.cpp": [(1, "++ i;"), (2, "NULL;")]},
        )

    def test_hunk_header_with_function_context(self) -> None:
        diff = "+++ b/a.cpp\n@@ -1,0 +2 @@ int foo() { return x+1; }\n+int y;\n"
        self.assertEqual(parse_added_lines(diff), {"a.cpp": [(2, "int y;")]})

    def test_json_indent_rules(self) -> None:
        self.assertEqual(check_json_line('  "a": 1'), [])
        self.assertTrue(any("tab" in m for m in check_json_line('\t"a": 1')))
        self.assertTrue(any("multiple" in m for m in check_json_line('   "a": 1')))
        self.assertEqual(check_json_line('    "a": 1'), [])

    def test_final_newline_is_only_checked_on_added_last_line(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "legacy.json"
            path.write_text('{\n  "a": 1\n}')
            mid_edit = check_changed_file(str(path), [(2, '  "a": 2')])
            self.assertNotIn(3, mid_edit)
            last_edit = check_changed_file(str(path), [(3, "}")])
            self.assertTrue(any("final newline" in m for m in last_edit.get(3, [])))

    def test_null_rule_ignores_comments_strings_and_headers(self) -> None:
        self.assertEqual(check_cpp_line("// NULL is legacy", header=False, cpp=True), [])
        self.assertEqual(check_cpp_line('auto s = "NULL";', header=False, cpp=True), [])
        self.assertEqual(check_cpp_line("#define X NULL", header=True, cpp=False), [])
        self.assertTrue(
            any("nullptr" in m for m in check_cpp_line("int *p = NULL;", header=False, cpp=True))
        )

    def test_python_rules_resist_evasion(self) -> None:
        self.assertTrue(any("semicolon" in m for m in check_python_line("x = 1; y = 2")))
        for text in ("def f(a=[1]):", "def f(a=set()):", "def f(a=dict()):"):
            self.assertTrue(any("mutable" in m for m in check_python_line(text)), text)

    def test_markdown_tilde_fences_and_table_shape(self) -> None:
        text = "# T\n\n~~~\ncode\n~~~\n"
        bare, content = markdown_fence_lines(text)
        self.assertEqual(bare, {3})
        self.assertEqual(content, {4})
        long_table = "| " + "x" * 120 + " |\n"
        self.assertTrue(
            all(
                "exceeds" not in m
                for m in check_markdown_line(
                    long_table.rstrip("\n"),
                    in_fence=False,
                    bare_fence_opening=False,
                    is_fence_marker=False,
                )
            )
        )

    def test_collect_added_lines_preserves_crlf_on_real_git_diff(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            subprocess.run(["git", "config", "user.email", "t@local"], cwd=root, check=True)
            subprocess.run(["git", "config", "user.name", "Test"], cwd=root, check=True)
            makefile = root / "Makefile"
            makefile.write_bytes(b"all:\n\techo hi\n")
            subprocess.run(["git", "add", "Makefile"], cwd=root, check=True)
            subprocess.run(["git", "commit", "-q", "-m", "init"], cwd=root, check=True)
            makefile.write_bytes(b"all:\n\techo hi\r\n")
            original = Path.cwd()
            try:
                os.chdir(root)
                added = collect_added_lines("HEAD")
                violations = check_changed_file("Makefile", added["Makefile"])
            finally:
                os.chdir(original)
            self.assertEqual(added, {"Makefile": [(2, "\techo hi\r")]})
            self.assertTrue(any("CRLF" in m for m in violations.get(2, [])))

    def test_quoted_path_header(self) -> None:
        diff = '+++ "b/with space.py"\n@@ -0,0 +1,1 @@\n+x = 1\n'
        self.assertEqual(parse_added_lines(diff), {"with space.py": [(1, "x = 1")]})

    def test_docstring_state_uses_tokenizer(self) -> None:
        self.assertEqual(python_docstring_lines('# note """\nx = 1\n'), set())
        self.assertEqual(python_docstring_lines('"""doc\nmore"""\nx = 1\n'), {1, 2})
        self.assertEqual(check_python_line("except:", in_docstring=True), [])

    def test_null_rule_ignores_block_comments(self) -> None:
        self.assertEqual(check_cpp_line("/* NULL */", header=False, cpp=True), [])

    def test_shell_rules(self) -> None:
        self.assertEqual(check_shell_line('echo "${value}"'), [])
        for snippet, expected in [
            ("value=`date`", "backticks"),
            ('echo "`date`"', "backticks"),
            ('var="`command arg`"', "backticks"),
            ('x="$( expr 4 + 4 )"', "((...))"),
            ('echo "$( [ -f file ] )"', "[[ ... ]]"),
            ('eval "${cmd}"', "eval"),
            ("let x=1", "((...))"),
            ("i=$[2 * 3]", "((...))"),
            ("if [ -f file ]; then", "[[ ... ]]"),
            ("[ -f file ] && echo yes", "[[ ... ]]"),
            ("test -f file", "[[ ... ]]"),
            ("! [ -f file ]", "[[ ... ]]"),
            ("x=$([ -f file ] && echo y)", "[[ ... ]]"),
            ('local x="$(cmd)"', "separately"),
            ('declare x="$(cmd)"', "separately"),
            ("eval", "eval"),
            ("eval;", "eval"),
            ("alias", "aliases"),
            ("alias ll='ls -l'", "aliases"),
            ('echo "$*"', "$@"),
            ("\techo hi", "two spaces"),
            ("echo hi ", "trailing whitespace"),
        ]:
            self.assertTrue(any(expected in m for m in check_shell_line(snippet)), snippet)

    def test_shell_line_length_and_heredoc(self) -> None:
        long_line = "echo " + "word " * 20
        self.assertTrue(any("exceeds" in m for m in check_shell_line(long_line)))
        self.assertEqual(check_shell_line("weird `content` here", in_heredoc=True), [])
        text = "#!/bin/bash\ncat <<EOF\n`literal`\nEOF\n"
        self.assertTrue(2 in shell_heredoc_lines(text))
        self.assertTrue(3 in shell_heredoc_lines(text))

    def test_hyphenated_heredoc_and_unterminated_fail_closed(self) -> None:
        terminated = "#!/bin/bash\ncat <<'SQL-DUMP'\n`literal`\nSQL-DUMP\necho `date`\n"
        exempt = shell_heredoc_lines(terminated)
        self.assertEqual(exempt, {2, 3, 4})
        self.assertNotIn(5, exempt)
        self.assertTrue(any("backticks" in m for m in check_shell_line("echo `date`")))
        unterminated = "#!/bin/bash\ncat <<'EOF-X'\necho `date`\n"
        self.assertNotIn(3, shell_heredoc_lines(unterminated))
        self.assertTrue(any("backticks" in m for m in check_shell_line("echo `date`")))

    def test_shell_false_positive_guards(self) -> None:
        self.assertEqual(check_shell_line("# use `cmd` for this"), [])
        self.assertEqual(check_shell_line("echo 'literal `tick` here'"), [])
        self.assertEqual(check_shell_line("a[0]=1"), [])
        self.assertEqual(check_shell_line("ls [abc]"), [])
        self.assertEqual(check_shell_line("grep test file"), [])
        self.assertEqual(check_shell_line("echo " + "a" * 90), [])

    def test_env_sh_shebang_is_detected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runner"
            path.write_text("#!/usr/bin/env sh\necho hi\n")
            self.assertTrue(has_shell_shebang(str(path)))
            self.assertTrue(is_supported(str(path)))
            python = Path(directory) / "script"
            python.write_text("#!/usr/bin/env python3\nprint('hi')\n")
            self.assertFalse(has_shell_shebang(str(python)))

    def test_extensionless_shell_script_is_covered(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runner"
            path.write_text("#!/bin/bash\necho hi\n")
            self.assertTrue(has_shell_shebang(str(path)))
            self.assertTrue(is_supported(str(path)))
            violations = check_changed_file(str(path), [(1, "#!/bin/bash"), (2, "echo `date`")])
            self.assertTrue(any("backticks" in m for m in violations.get(2, [])))

    def test_supported_extensions(self) -> None:
        self.assertTrue(is_supported("a.py"))
        self.assertTrue(is_supported("Makefile"))
        self.assertTrue(is_supported("x.hxx"))
        self.assertTrue(is_supported("a.jsonl"))
        self.assertFalse(is_supported("image.png"))


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(StyleCheckerTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1

    base = os.environ.get("NIRNAY_STYLE_BASE", "main")
    by_file = collect_added_lines(base)
    failures: list[str] = []
    for path, added in sorted(by_file.items()):
        for number, messages in sorted(check_changed_file(path, added).items()):
            for message in messages:
                failures.append(f"{path}:{number}: {message}")
    if failures:
        for failure in failures:
            print(f"FAIL: {failure}")
        print(f"{len(failures)} source style violation(s) on added lines")
        return 1
    print(f"PASS: source style guidance satisfied on added lines vs {base}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
