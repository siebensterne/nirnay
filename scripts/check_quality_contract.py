#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Verify the shared quality contract against this repository's configs.

The contract is byte-identical in the sibling workspaces. This gate fails
closed when a pinned linter version, rule family, formatter option, shell
check, editor baseline or Rust lint denial drifts from that contract, so the
two repositories cannot silently diverge.

Run from the repository root. Use ``--self-test`` for adversarial unit tests.
"""

from __future__ import annotations

import json
import re
import sys
import tomllib
import unittest
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / "quality-contract.json"


def load_contract(root: Path = ROOT) -> dict[str, Any]:
    """Load and minimally validate the shared contract."""
    data = json.loads((root / CONTRACT.name).read_text(encoding="utf-8"))
    if data.get("schema") != 1:
        raise ValueError("unsupported quality contract schema")
    return data


def check_files(root: Path, contract: dict[str, Any]) -> list[str]:
    """Return missing shared configuration files."""
    return [
        f"missing shared file: {name}"
        for name in contract["shared_files"]
        if not (root / name).is_file()
    ]


def check_ruff_settings(
    contract: dict[str, Any], config: dict[str, Any]
) -> list[str]:
    """Compare Ruff settings against the contract."""
    expected = contract["ruff"]
    errors: list[str] = []
    if config.get("required-version") != expected["required_version"]:
        errors.append("ruff required-version drifted")
    for key in ("force-exclude", "fix", "unsafe-fixes"):
        if config.get(key) is not expected[key.replace("-", "_")]:
            errors.append(f"ruff {key} drifted")
    lint = config.get("lint", {})
    if lint.get("dummy-variable-rgx") != expected["dummy_variable_rgx"]:
        errors.append("ruff dummy-variable-rgx drifted")
    if lint.get("future-annotations") is not expected["future_annotations"]:
        errors.append("ruff future-annotations drifted")
    docstyle = lint.get("pydocstyle", {})
    if docstyle.get("convention") != expected["pydocstyle_convention"]:
        errors.append("ruff pydocstyle convention drifted")
    if lint.get("select") != expected["rule_families"]:
        errors.append("ruff rule families drifted")
    return errors


def parse_rc(text: str) -> dict[str, str]:
    """Parse a ShellCheck-style ``key=value`` configuration file."""
    values: dict[str, str] = {}
    for line in text.splitlines():
        if "=" in line and not line.lstrip().startswith("#"):
            key, value = line.split("=", 1)
            values[key.strip()] = value.strip()
    return values


def check_shellcheck_text(contract: dict[str, Any], text: str) -> list[str]:
    """Compare .shellcheckrc against the contract."""
    expected = contract["shellcheck"]
    values = parse_rc(text)
    errors: list[str] = []
    if values.get("shell") != expected["shell"]:
        errors.append("shell dialect drifted")
    if values.get("external-sources") != "true":
        errors.append("shellcheck external-sources drifted")
    enabled = {
        item.strip()
        for item in values.get("enable", "").split(",")
        if item.strip()
    }
    # .shellcheckrc permits one `enable=` per check; collect all of them.
    enabled |= {
        line.split("=", 1)[1].strip()
        for line in text.splitlines()
        if line.startswith("enable=")
    }
    for name in expected["optional_checks"]:
        if name not in enabled:
            errors.append(f"shellcheck optional check missing: {name}")
    return errors


def parse_ini(text: str) -> dict[str, dict[str, str]]:
    """Parse a simple INI file into section-key-value mappings."""
    sections: dict[str, dict[str, str]] = {}
    current = ""
    for line in text.splitlines():
        if line.startswith("[") and line.endswith("]"):
            current = line[1:-1]
            sections[current] = {}
        elif "=" in line and current:
            key, value = line.split("=", 1)
            sections[current][key.strip()] = value.strip()
    return sections


def check_editorconfig_text(contract: dict[str, Any], text: str) -> list[str]:
    """Compare .editorconfig against the contract."""
    expected = contract["editorconfig"]
    errors: list[str] = []
    if not re.search(r"(?m)^root\s*=\s*true\s*$", text):
        errors.append("editorconfig root flag drifted")
    default = parse_ini(text).get("*", {})
    for key in ("charset", "end_of_line"):
        if default.get(key) != expected[key]:
            errors.append(f"editorconfig {key} drifted")
    for key in ("insert_final_newline", "trim_trailing_whitespace"):
        if default.get(key) != "true":
            errors.append(f"editorconfig {key} drifted")
    return errors


def check_rust_text(contract: dict[str, Any], text: str) -> list[str]:
    """Compare Cargo.toml lint denials against the contract."""
    errors: list[str] = []
    for name in contract["rust"]["denied_rust_lints"]:
        if not re.search(rf"(?m)^\s*{name}\s*=\s*\"deny\"\s*$", text):
            errors.append(f"rust lint not denied: {name}")
    for name in contract["rust"]["denied_clippy_lints"]:
        if not re.search(rf"(?m)^\s*{name}\s*=\s*\"deny\"\s*$", text):
            errors.append(f"clippy lint not denied: {name}")
    return errors


def check_xtask_text(contract: dict[str, Any], files: list[Path]) -> list[str]:
    """Verify the task runner is typed, discoverable and self-tested."""
    expected = contract["xtask"]
    text = "\n".join(
        path.read_text(encoding="utf-8") for path in files if path.is_file()
    )
    errors: list[str] = []
    for name in expected["required_tasks"]:
        if f'"{name}"' not in text:
            errors.append(f"xtask required task missing: {name}")
    if "#[cfg(test)]" not in text:
        errors.append("xtask self-tests missing")
    for shell in ('Command::new("sh")', 'Command::new("bash")'):
        if shell in text:
            errors.append("xtask must not dispatch through a shell")
    return errors


def validate(root: Path = ROOT) -> list[str]:
    """Return every quality-contract violation found under ``root``."""
    contract = load_contract(root)
    errors = check_files(root, contract)
    ruff = root / "ruff.toml"
    if ruff.is_file():
        errors += check_ruff_settings(contract, tomllib.loads(ruff.read_text()))
    shell = root / ".shellcheckrc"
    if shell.is_file():
        errors += check_shellcheck_text(contract, shell.read_text())
    editor = root / ".editorconfig"
    if editor.is_file():
        errors += check_editorconfig_text(contract, editor.read_text())
    cargo = root / "Cargo.toml"
    if cargo.is_file():
        errors += check_rust_text(contract, cargo.read_text())
    xtask = root / "xtask"
    if not (xtask / "Cargo.toml").is_file():
        errors.append("xtask crate is missing")
    else:
        errors += check_xtask_text(
            contract,
            [xtask / "src" / "main.rs", xtask / "src" / "tasks.rs"],
        )
    alias = root / ".cargo" / "config.toml"
    if alias.is_file() and "--locked" not in alias.read_text(encoding="utf-8"):
        errors.append("cargo xtask alias must pin --locked")
    return errors


class ContractTests(unittest.TestCase):
    """Adversarial tests for the quality contract gate."""

    def test_live_contract_is_valid(self) -> None:
        """This repository satisfies its own contract."""
        self.assertEqual(validate(), [])

    def test_ruff_family_drift_is_detected(self) -> None:
        """Deleting a rule family fails the gate."""
        contract = load_contract()
        config = tomllib.loads((ROOT / "ruff.toml").read_text())
        config["lint"]["select"] = config["lint"]["select"][:-1]
        self.assertTrue(check_ruff_settings(contract, config))

    def test_shellcheck_option_drift_is_detected(self) -> None:
        """Deleting an optional check fails the gate."""
        contract = load_contract()
        text = (ROOT / ".shellcheckrc").read_text()
        lines = [
            line
            for line in text.splitlines()
            if "useless-use-of-cat" not in line
        ]
        self.assertTrue(check_shellcheck_text(contract, "\n".join(lines)))

    def test_xtask_task_drift_is_detected(self) -> None:
        """A missing task runner or task name fails the gate."""
        contract = load_contract()
        files = [
            ROOT / "xtask" / "src" / "main.rs",
            ROOT / "xtask" / "src" / "tasks.rs",
        ]
        self.assertEqual(check_xtask_text(contract, files), [])
        findings = check_xtask_text(contract, [])
        for name in contract["xtask"]["required_tasks"]:
            self.assertTrue(any(name in finding for finding in findings))

    def test_rust_denial_drift_is_detected(self) -> None:
        """Removing a Rust lint denial fails the gate."""
        contract = load_contract()
        text = (ROOT / "Cargo.toml").read_text()
        text = text.replace('unsafe_op_in_unsafe_fn = "deny"', "")
        self.assertTrue(check_rust_text(contract, text))

    def test_missing_file_is_detected(self) -> None:
        """A missing shared config file fails the gate."""
        contract = load_contract()
        missing = ROOT / "no-such-directory"
        self.assertTrue(check_files(missing, contract))


def main() -> int:
    """Print findings and return a process exit status."""
    if "--self-test" in sys.argv[1:]:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(ContractTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1
    findings = validate()
    for finding in findings:
        print(f"FAIL: {finding}")
    if findings:
        print(f"quality contract: {len(findings)} finding(s)")
        return 1
    print("PASS: shared quality contract satisfied")
    return 0


if __name__ == "__main__":
    sys.exit(main())
