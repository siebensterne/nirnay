#!/usr/bin/env python3
"""Fail-closed gate for forbidden generated paths and Markdown sources.

The repository's root `git-policy.json` declares components and suffixes that
must never be tracked. This gate inspects tracked files and every untracked
file, including ignored ones, so a generated artifact cannot enter a commit
unnoticed. A single root `TODO.md` is the only permitted Markdown file;
generated rustdoc under ignored `target/doc/` is excluded from the Markdown
rule, but tracked files under `target/` are never permitted.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

MARKDOWN_EXTENSIONS = (".md", ".markdown", ".mdown", ".mkd", ".mdx")
ALLOWED_MARKDOWN = "TODO.md"


def load_policy(root: Path) -> dict:
    """Exercise load policy."""
    return json.loads((root / "git-policy.json").read_text(encoding="utf-8"))


def git_paths(root: Path, args: list[str]) -> list[str]:
    """Exercise git paths."""
    result = subprocess.run(
        ["git", "-C", str(root), *args],
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        detail = result.stderr.decode("utf-8", errors="replace").strip()
        raise RuntimeError(f"git {' '.join(args)} failed: {detail}")
    return [part for part in result.stdout.decode("utf-8").split("\0") if part]


def markdown_violation(path: str) -> str | None:
    """Return a reason when a path is a forbidden Markdown source file."""
    normalized = path.replace("\\", "/").strip().strip("/")
    if normalized.startswith("target/doc/"):
        return None
    if normalized == ALLOWED_MARKDOWN:
        return None
    name = normalized.rsplit("/", 1)[-1].lower()
    if any(name.endswith(extension) for extension in MARKDOWN_EXTENSIONS):
        return "forbidden Markdown file"
    if any(name.endswith(extension + ".sha256") for extension in MARKDOWN_EXTENSIONS):
        return "forbidden Markdown sidecar"
    return None


def forbidden(path: str, policy: dict) -> str | None:
    """Exercise forbidden."""
    generated = policy["generated_paths"]
    exceptions = {item["path"].rstrip("/") for item in generated.get("exceptions", [])}
    if any(path == exc or path.startswith(exc + "/") for exc in exceptions):
        return None
    parts = path.split("/")
    for component in generated["forbidden_components"]:
        if component in parts:
            return f"forbidden component `{component}`"
    for prefix in generated["forbidden_component_prefixes"]:
        if any(part.startswith(prefix) for part in parts):
            return f"forbidden component prefix `{prefix}`"
    for suffix in generated["forbidden_suffixes"]:
        if path.endswith(suffix):
            return f"forbidden suffix `{suffix}`"
    return markdown_violation(path)


def validate(root: Path) -> list[str]:
    """Exercise validate."""
    policy = load_policy(root)
    tracked = git_paths(root, ["ls-files", "-z"])
    untracked = git_paths(root, ["ls-files", "--others", "-z"])
    errors: list[str] = []
    for source, paths in (("tracked", tracked), ("untracked", untracked)):
        for path in paths:
            reason = forbidden(path, policy)
            if reason is not None:
                errors.append(f"{source} path `{path}` uses a {reason}")
    for path in tracked:
        if path == "target" or path.startswith("target/"):
            errors.append(f"tracked generated path under target/: `{path}`")
    return sorted(errors)


def main() -> int:
    """Exercise main."""
    root = Path(__file__).resolve().parent.parent
    try:
        errors = validate(root)
    except (OSError, RuntimeError, KeyError, json.JSONDecodeError) as error:
        print(f"[FAIL] generated-path audit could not run: {error}")
        return 1
    if errors:
        print("[FAIL] forbidden generated paths detected:")
        for error in errors:
            print(f"  - {error}")
        return 1
    print("[PASS] no forbidden generated paths in tracked or " "untracked source files")
    return 0


if __name__ == "__main__":
    sys.exit(main())
