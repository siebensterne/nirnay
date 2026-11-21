#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Verify Apache-2.0 license, NOTICE and SPDX headers on added source files.

The two sibling workspaces share this gate verbatim. It fails closed when the
license text is altered, the NOTICE attribution is missing, or a newly added
authored source file lacks an SPDX identifier. Vendored references and build
outputs are not authored source and are excluded.

Run from the repository root. Set QUALITY_STYLE_BASE (or the repository's
STYLE_BASE variable) to compare against a different ref. Use --self-test.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPDX = "SPDX-License-Identifier: Apache-2.0"
AUTHORED = (
    ".rs",
    ".py",
    ".sh",
    ".q",
    ".c",
    ".cc",
    ".cpp",
    ".cxx",
    ".h",
    ".hpp",
    ".hxx",
)
EXCLUDED = (
    "references/",
    "engineering/references/",
    "target/",
    "build/",
    ".local/",
)


def base_ref() -> str:
    """Return the configured comparison ref."""
    return os.environ.get("QUALITY_STYLE_BASE") or os.environ.get("STYLE_BASE") or "main"


def added_paths(root: Path = ROOT, base: str | None = None) -> list[str]:
    """Return added and untracked authored paths relative to ``root``."""
    ref = base or base_ref()
    tracked = subprocess.run(
        ["git", "diff", "--name-only", "--diff-filter=A", ref],
        check=False,
        capture_output=True,
        text=True,
        cwd=root,
    )
    if tracked.returncode != 0:
        # Hosted PR checkouts may not contain a local `main` ref. Fall back to
        # the remote-tracking base, then to the full tracked set rather than
        # skipping the license gate entirely.
        fallback = "origin/main" if ref == "main" else "main"
        alternate = subprocess.run(
            ["git", "diff", "--name-only", "--diff-filter=A", fallback],
            check=False,
            capture_output=True,
            text=True,
            cwd=root,
        )
        if alternate.returncode == 0:
            tracked = alternate
        else:
            everything = subprocess.run(
                ["git", "ls-files"],
                check=True,
                capture_output=True,
                text=True,
                cwd=root,
            )
            tracked = everything
    untracked = subprocess.run(
        ["git", "ls-files", "--others", "--exclude-standard"],
        check=True,
        capture_output=True,
        text=True,
        cwd=root,
    )
    paths = set(tracked.stdout.splitlines()) | set(
        untracked.stdout.splitlines()
    )
    return sorted(
        path
        for path in paths
        if path.endswith(AUTHORED)
        and not path.startswith(EXCLUDED)
        and (root / path).is_file()
    )


def check_spdx(path: str, root: Path = ROOT) -> list[str]:
    """Return a finding when an added file lacks the SPDX identifier."""
    lines = (root / path).read_text(encoding="utf-8").splitlines()[:10]
    if SPDX in "\n".join(lines):
        return []
    return [f"{path}: missing {SPDX} in first ten lines"]


def check_notice(root: Path = ROOT) -> list[str]:
    """Verify the license text and NOTICE attribution."""
    errors: list[str] = []
    license_text = (root / "LICENSE").read_text(encoding="utf-8")
    if "Apache License" not in license_text[:200]:
        errors.append("LICENSE is not Apache-2.0")
    if "Version 2.0" not in license_text[:200]:
        errors.append("LICENSE version is not 2.0")
    notice = (root / "NOTICE").read_text(encoding="utf-8")
    if "Copyright 2026 Kartik Jangid" not in notice:
        errors.append("NOTICE copyright is missing")
    if "22kartikjangid@gmail.com" not in notice:
        errors.append("NOTICE author email is missing")
    return errors


def validate(root: Path = ROOT, base: str | None = None) -> list[str]:
    """Return every license and provenance finding."""
    errors = check_notice(root)
    for path in added_paths(root, base):
        errors += check_spdx(path, root)
    return errors


class ProvenanceTests(unittest.TestCase):
    """Adversarial tests for license and SPDX detection."""

    def test_spdx_present(self) -> None:
        """A header in the first lines is accepted."""
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "example.py"
            path.write_text(
                f"#!/usr/bin/env python3\n# {SPDX}\n", encoding="utf-8"
            )
            self.assertEqual(check_spdx("example.py", Path(directory)), [])

    def test_spdx_missing(self) -> None:
        """A file without the identifier fails."""
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "example.py"
            path.write_text("print('x')\n", encoding="utf-8")
            errors = check_spdx("example.py", Path(directory))
            self.assertTrue(any("missing" in error for error in errors))

    def test_vendored_paths_excluded(self) -> None:
        """Reference documents are not treated as authored source."""
        self.assertIn("references/", EXCLUDED)
        self.assertIn("engineering/references/", EXCLUDED)

    def test_notice_requires_author(self) -> None:
        """Notice content is mandatory."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "LICENSE").write_text(
                "Apache License\nVersion 2.0\n", encoding="utf-8"
            )
            (root / "NOTICE").write_text(
                "Copyright 2026 Someone\n", encoding="utf-8"
            )
            errors = check_notice(root)
            self.assertTrue(any("author email" in error for error in errors))


def main() -> int:
    """Print findings and return a process exit status."""
    if "--self-test" in sys.argv[1:]:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(
            ProvenanceTests
        )
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1
    findings = validate()
    for finding in findings:
        print(f"FAIL: {finding}")
    if findings:
        print(f"license provenance: {len(findings)} finding(s)")
        return 1
    print("PASS: Apache-2.0 license, NOTICE and SPDX headers verified")
    return 0


if __name__ == "__main__":
    sys.exit(main())
