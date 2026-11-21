#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Run the pinned Ruff gate, resolving the version-locked binary explicitly.

`python3 -m ruff` can resolve a different, older Ruff from a user's PATH even
when `requirements-dev.txt` and `ruff.toml` pin 0.17.5. This wrapper checks
candidate interpreters in priority order and selects the one whose Ruff matches
the pinned version, so the result does not depend on ambient PATH.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def pinned_version() -> str:
    """Read the pinned Ruff version from requirements-dev.txt."""
    source = ROOT / "ruff-requirements.txt"
    if not source.is_file():
        source = ROOT / "requirements-dev.txt"
    for line in source.read_text().splitlines():
        if line.strip().startswith("ruff=="):
            return line.strip().split("==", 1)[1]
    raise SystemExit("FAIL: ruff is not pinned in requirements-dev.txt")


def binary_version(binary: str) -> str | None:
    """Return the version reported by a Ruff executable, if any."""
    try:
        result = subprocess.run(
            [binary, "--version"],
            check=False,
            capture_output=True,
            text=True,
            cwd=ROOT,
        )
    except OSError:
        return None
    if result.returncode == 0:
        return result.stdout.strip().split()[-1]
    return None


def pinned_binary(version: str) -> str | None:
    """Find a Ruff executable with the pinned version."""
    path = ROOT / ".local" / "ruff"
    if path.is_file() and binary_version(str(path)) == version:
        return str(path)
    found = shutil.which("ruff")
    if found and binary_version(found) == version:
        return found
    return None


def main() -> int:
    """Run Ruff with the pinned config using the matching interpreter."""
    version = pinned_version()
    binary = pinned_binary(version)
    if binary is None:
        print(
            f"FAIL: Ruff {version} not found; install requirements-dev.txt",
            file=sys.stderr,
        )
        return 1
    command = [binary, "check", "--config", "ruff.toml", "--no-cache"]
    command.extend(sys.argv[1:] or ["."])
    print(f"ruff {version} via {binary}")
    return subprocess.run(command, cwd=ROOT, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
