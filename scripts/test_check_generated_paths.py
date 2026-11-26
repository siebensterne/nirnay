# SPDX-License-Identifier: Apache-2.0
"""Tests for the generated-path and one-Markdown gate."""

from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

from check_generated_paths import forbidden, markdown_violation, validate


class GeneratedPathTest(unittest.TestCase):
    """Generated output and Markdown are rejected from the source surface."""

    def test_markdown_examples(self) -> None:
        """Only the root TODO.md is permitted as Markdown."""
        self.assertIsNone(markdown_violation("TODO.md"))
        for path in ("README.md", "src/example.md", "subdir/TODO.md", "EXAMPLE.MD"):
            with self.subTest(path=path):
                self.assertIsNotNone(markdown_violation(path))
        self.assertIsNone(markdown_violation("target/doc/static.files/LICENSE.md"))

    def test_tracked_target_file_is_rejected(self) -> None:
        """A tracked rustdoc file is generated output and must fail the gate."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "git-policy.json").write_text(
                json.dumps(
                    {
                        "generated_paths": {
                            "forbidden_components": ["target"],
                            "forbidden_component_prefixes": [],
                            "forbidden_suffixes": [],
                        }
                    }
                ),
                encoding="utf-8",
            )
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            (root / "target" / "doc").mkdir(parents=True)
            (root / "target" / "doc" / "index.html").write_text("html", encoding="utf-8")
            subprocess.run(["git", "-C", str(root), "add", "target/doc/index.html"], check=True)
            findings = validate(root)
            self.assertTrue(any("under target/" in item for item in findings), findings)
            self.assertIsNotNone(
                forbidden(
                    "target/doc/index.html", json.loads((root / "git-policy.json").read_text())
                )
            )


if __name__ == "__main__":
    unittest.main()
