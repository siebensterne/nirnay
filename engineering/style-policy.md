# Source style policy

This repository applies one binding rule to every language: **formatters and
linters run on all files; style guidance is enforced on lines added relative to
`main`**. That keeps existing code from being churned while making every new
line comply. The rule is implemented by `scripts/check_rust_style.py` for Rust
and `scripts/check_source_style.py` for every other language, and is wired into
`make check` and CI.

## Sources and conflict resolution

- **Rust**: Linux kernel Rust coding guidelines and Google Rust style guide,
  with the repository's resolution (one `use` per module, kernel vertical
  import layout with a trailing empty comment). No exceptions.
- **Python**: Google Python Style Guide. This policy keeps its import, exception,
  semicolon, backslash-continuation and mutable-default rules. The repository
  uses a 100-column limit (`.editorconfig`, `ruff.toml`) rather than Google's
  80; the 100-column convention is already established here.
- **C/C++**: Google C++ Style Guide, expressed by the pinned `.clang-format`
  (LLVM base, two-space indent, 100 columns, `nullptr`, regioned includes).
  The Linux kernel C style (eight-space tabs) is **not** used for
  application C++; it is a kernel constraint, not a repository convention.
- **Shell**: Google Shell Style Guide. Executable scripts start with
  `#!/bin/bash`, use two-space indentation, stay within 80 columns, prefer
  `[[ ... ]]`, `$(...)`, `((...))` and `"$@"`, and avoid `eval`, backticks,
  `let`, `expr`, `test`, `/usr/bin/[`, aliases and unquoted `$*`. Shell keeps
  Google's 80-column limit while Python, C/C++ and Markdown retain the
  repository's 100-column convention. Extensionless scripts with a shell
  shebang are detected; a file with no shebang and no recognised extension is
  not classified as shell.
- **Makefile**: GNU Make semantics plus the repository's tab-indented recipe
  rule in `.editorconfig`.
- **JSON/JSONL**: Google JSON Style Guide constraints that are mechanical:
  valid JSON only (no comments), double-quoted keys and strings, two-space
  indentation, a final newline. Semantic naming rules are not enforced because
  existing schema property names are part of published contracts.
- **Markdown**: Google Markdown style guide, published as the Google developer
  documentation style guide — ATX headings with a space after `#`, a single H1,
  fenced blocks with a declared language, no trailing whitespace. Google's
  published prose limit is 80 columns; this repository uses 100 for prose with
  exceptions for headings, tables, links and code blocks. That deviation is
  deliberate and predates the binding checker; changing it would reflow the
  whole documentation corpus and is out of scope for a narrow change. The
  checker enforces the repository limit, not the published one.

A web search for published style guides from leading HFT/quantitative trading
firms (Jane Street, Citadel, Two Sigma, HRT, Optiver) found no authoritative
public coding-style documents; results were interview-preparation material.
No rule is attributed to those firms. The low-latency
discipline in the repository's ADRs (allocation control, no filesystem durability on a
hot path, exact financial types) remains the binding domain constraint and is
enforced by code and tests, not by a formatter.

## Binding tools

| Language | Formatter/linter | Added-lines gate | Where it runs |
|---|---|---|---|
| Rust | `cargo fmt`, Clippy `-D warnings` | `check_rust_style.py` | `make check`, CI |
| Python | `ruff` (whole-tree advisory, see below) | `check_source_style.py`, `check_ruff_added_lines.py` | `make check`, CI |
| C/C++ | pinned `clang-format==21.1.8` | `check_source_style.py` | `make check`, CI |
| Shell | none available | `check_source_style.py` | `make check`, CI |
| Makefile | none available | `check_source_style.py` | `make check`, CI |
| JSON/JSONL | none needed | `check_source_style.py` | `make check`, CI |
| Markdown | none available | `check_source_style.py` | `make check`, CI |

Run the gates directly:

```sh
make style
python3 scripts/check_rust_style.py --self-test
python3 scripts/check_source_style.py --self-test
python3 scripts/check_style_gate.py --self-test
```

`ruff` is configured in `ruff.toml` and available via `make python-format-check`,
but whole-tree checks are **not binding** yet: roughly 59 existing files predate
it and would be reformatted (measured with ruff 0.16.5; the exact count is
version-dependent). A dedicated PR must reformat the corpus before whole-tree
ruff checks can join `make check`. New Python lines are already binding: the
same policy is enforced by `scripts/check_ruff_added_lines.py` in `make style`
and CI, so new code cannot add lint debt. The checkers themselves are
ruff-formatted.

## Hardening the gate itself

`scripts/check_style_gate.py` is a binding integrity check that verifies:

- `make check` still invokes `style`, and `style` still invokes both style
  checkers and the integrity checker;
- the CI `source-style` job still runs both checkers with full history;
- all six language check functions still exist in `check_source_style.py`
  (Rust is checked by `check_rust_style.py`);
- the adversarial self-test inventory has not shrunk below its declared
  minimum (13 Rust, 26 source-style, 7 integrity tests);
- the integrity checker is invoked twice in CI and once in the Makefile;
- canonical bad lines still produce violations, so a rule whose body is
  replaced with `return []` fails the gate even if its name and tests remain.

The behavioural self-tests also run explicitly in the CI `source-style` job.
The integrity checker runs from `make style`, from that CI job and
independently from the CI `python-research` job. Removing or weakening a rule,
a checker, a self-test or the wiring therefore fails the build. Adding a
language or rule requires updating its expectations in the same PR.

## Markdown prose rules

`scripts/check_prose_style.py` enforces four rules on added Markdown lines and
provides `--all` for remediation sweeps. Rule IDs are stable; changing a pattern
requires style-policy review.

| Rule | Rejects | Rationale |
|---|---|---|
| PROSE-001 | first-person pronouns | invariants, owners and dated records replace actors |
| PROSE-002 | intent adverbs such as `clearly` and `simply` | evidence replaces posture |
| PROSE-003 | promotional terms such as `black box` and `state of the art` | every layer stays explainable |
| PROSE-004 | tautological filler such as `needless to say` | each sentence adds a checkable claim |

Inline code spans, fenced blocks and indented code are excluded because
identifiers and quoted source are not prose. The checker is wired into `make
style`, the CI `source-style` job and the gate-integrity checker.

## Adding a rule

Add the rule and an adversarial self-test to the relevant checker, document the
policy source here, raise the minimum in `check_style_gate.py` when applicable,
then run `make check`. A rule that cannot be expressed on added lines without
churning existing code does not belong in the binding gate.
