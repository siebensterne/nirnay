import datetime as dt
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

import check_generated_paths
from check_repo import check_graph, check_waivers


class ArchitectureTests(unittest.TestCase):
    def setUp(self):
        self.rules = {
            "layer": [
                {"name": "low", "crates": ["domain"]},
                {"name": "high", "crates": ["service"]},
                {"name": "apps", "leaf": True, "crates": ["cli"]},
            ],
            "tooling": {"crates": ["xtask"]},
            "restricted_external": {"crates": ["tokio"], "allowed_in": ["apps"]},
            "deny_edge": [{"from": "risk-*", "to": "strategy-*", "reason": "separation"}],
        }

    def pkg(self, name, deps):
        return {"name": name, "dependencies": [{"name": d} for d in deps]}

    def test_downward_edges_pass(self):
        packages = [self.pkg("domain", []), self.pkg("service", ["domain"]), self.pkg("cli", ["service", "tokio"]), self.pkg("xtask", [])]
        self.assertEqual(check_graph(self.rules, packages), [])

    def test_upward_leaf_and_unknown_crates_fail(self):
        packages = [self.pkg("domain", ["service"]), self.pkg("service", ["cli"]), self.pkg("cli", []), self.pkg("xtask", []), self.pkg("unassigned", [])]
        errors = check_graph(self.rules, packages)
        self.assertTrue(any("not a downward" in e for e in errors))
        self.assertTrue(any("leaf package" in e for e in errors))
        self.assertTrue(any("not assigned" in e for e in errors))

    def test_forbidden_edges_and_restricted_external_fail(self):
        rules = dict(self.rules)
        rules["layer"] = [
            {"name": "low", "crates": ["strategy-alpha"]},
            {"name": "high", "crates": ["risk-engine"]},
            {"name": "apps", "leaf": True, "crates": ["cli"]},
        ]
        pkgs = [self.pkg("strategy-alpha", []), self.pkg("risk-engine", ["strategy-alpha", "tokio"]), self.pkg("cli", []), self.pkg("xtask", [])]
        errors = check_graph(rules, pkgs)
        self.assertTrue(any("forbidden edge" in e for e in errors))
        self.assertTrue(any("restricted external" in e for e in errors))


class WaiverTests(unittest.TestCase):
    def test_expired_waiver_fails(self):
        from unittest.mock import patch
        import check_repo
        fake = """[advisories]\nignore = [{ id='RUSTSEC-X', reason='owner=team expires=2026-01-01' }]"""
        with patch.object(check_repo, 'ROOT') as root:
            # The production function reads deny.toml; test the token logic by writing a temp file.
            import tempfile
            from pathlib import Path
            with tempfile.TemporaryDirectory() as d:
                (Path(d) / 'deny.toml').write_text(fake)
                root.__truediv__.return_value = Path(d) / 'deny.toml'
                errs = check_waivers(dt.date(2026, 11, 12))
        self.assertTrue(any('expired' in e for e in errs))


class GeneratedPathTests(unittest.TestCase):
    """Adversarial tests for the generated-path and Markdown gate."""

    def policy(self) -> dict:
        """Return a minimal generated-path policy for the tests."""
        return {
            "generated_paths": {
                "forbidden_components": [".DS_Store", "__pycache__"],
                "forbidden_component_prefixes": ["build-"],
                "forbidden_suffixes": [".pyc"],
                "exceptions": [{"path": "target/doc"}],
            }
        }

    def test_root_todo_is_allowed(self) -> None:
        """The one allowed root Markdown file passes."""
        self.assertIsNone(
            check_generated_paths.forbidden("TODO.md", self.policy())
        )

    def test_markdown_paths_are_rejected(self) -> None:
        """Every other Markdown name, path and case is refused."""
        cases = (
            "README.md",
            "src/example.md",
            "docs/example.markdown",
            "subdir/TODO.md",
            "NOTES.MD",
            "notes.MDX",
            "notes.mdown",
            "notes.mkd",
            "notes.md.sha256",
        )
        for path in cases:
            with self.subTest(path=path):
                self.assertIsNotNone(
                    check_generated_paths.forbidden(path, self.policy())
                )

    def test_generated_exception_excludes_markdown_rule(self) -> None:
        """Declared generated output stays outside the Markdown rule."""
        for path in ("target/doc/index.html", "target/doc/LICENSE.md"):
            with self.subTest(path=path):
                self.assertIsNone(
                    check_generated_paths.forbidden(path, self.policy())
                )

    def test_tracked_and_ignored_untracked_markdown_is_detected(self) -> None:
        """Tracked, untracked and ignored Markdown are all reported."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            (root / "git-policy.json").write_text(json.dumps(self.policy()))
            (root / ".gitignore").write_text("ignored/\n")
            for relative in ("tracked.md", "untracked.md", "ignored/notes.md"):
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("notes\n")
            subprocess.run(
                ["git", "-C", str(root), "add", "tracked.md"], check=True
            )
            findings = check_generated_paths.validate(root)
        self.assertTrue(any("tracked path `tracked.md`" in f for f in findings))
        self.assertTrue(any("untracked path `untracked.md`" in f for f in findings))
        self.assertTrue(any("ignored/notes.md" in f for f in findings))


if __name__ == '__main__':
    unittest.main()
