#!/usr/bin/env python3
"""Enforce architecture, dependency inheritance, waiver expiry and claim hygiene."""
from __future__ import annotations
import datetime as dt
import glob
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def _glob(pattern: str, value: str) -> bool:
    return value.startswith(pattern[:-1]) if pattern.endswith("*") else value == pattern


def check_graph(rules: dict, packages: list[dict]) -> list[str]:
    errors: list[str] = []
    package_names = {p["name"] for p in packages}
    placement: dict[str, tuple[str, int | None, bool]] = {}
    for rank, layer in enumerate(rules.get("layer", [])):
        for name in layer.get("crates", []):
            if name in placement:
                errors.append(f"{name}: assigned to more than one layer")
            placement[name] = (layer["name"], rank, bool(layer.get("leaf", False)))
    for name in rules.get("tooling", {}).get("crates", []):
        if name in placement:
            errors.append(f"{name}: assigned to more than one layer")
        placement[name] = ("tooling", None, False)
    for name in placement:
        if name not in package_names:
            errors.append(f"architecture lists `{name}` but no workspace package exists")
    for name in package_names:
        if name not in placement:
            errors.append(f"workspace package `{name}` is not assigned to a layer")

    deny_edges = rules.get("deny_edge", [])
    restricted = set(rules.get("restricted_external", {}).get("crates", []))
    allowed_in = set(rules.get("restricted_external", {}).get("allowed_in", []))
    for package in packages:
        name = package["name"]
        if name not in placement:
            continue
        from_layer, from_rank, _ = placement[name]
        for dep in package.get("dependencies", []):
            dep_name = dep["name"]
            if dep_name in package_names:
                for edge in deny_edges:
                    if _glob(edge["from"], name) and _glob(edge["to"], dep_name):
                        errors.append(f"forbidden edge `{name}` -> `{dep_name}`: {edge['reason']}")
                if from_layer == "tooling":
                    errors.append(f"tooling package `{name}` may not depend on workspace package `{dep_name}`")
                    continue
                if dep_name in placement:
                    to_layer, to_rank, to_leaf = placement[dep_name]
                    if to_leaf:
                        errors.append(f"`{name}` depends on leaf package `{dep_name}`")
                    if from_rank is not None and to_rank is not None and to_rank >= from_rank:
                        errors.append(f"`{name}` ({from_layer}) -> `{dep_name}` ({to_layer}) is not a downward dependency")
            elif dep_name in restricted and from_layer not in allowed_in:
                errors.append(f"`{name}` ({from_layer}) uses restricted external dependency `{dep_name}`")
    return errors


def cargo_metadata() -> list[dict]:
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=ROOT, check=True, text=True, capture_output=True,
    )
    metadata = json.loads(result.stdout)
    members = set(metadata["workspace_members"])
    return [p for p in metadata["packages"] if p["id"] in members]



def static_workspace_packages() -> list[dict]:
    """Manifest-only fallback for environments without the Cargo executable."""
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
    ws_deps = manifest.get("workspace", {}).get("dependencies", {})
    member_patterns = manifest.get("workspace", {}).get("members", [])
    manifests: set[Path] = set()
    for pattern in member_patterns:
        for match in glob.glob(str(ROOT / pattern)):
            candidate = Path(match)
            cargo_toml = candidate / "Cargo.toml" if candidate.is_dir() else candidate
            if cargo_toml.name == "Cargo.toml" and cargo_toml.is_file():
                manifests.add(cargo_toml)
    packages = []
    for cargo_toml in sorted(manifests):
        data = tomllib.loads(cargo_toml.read_text())
        package = data.get("package", {})
        if not package:
            continue
        dependencies = []
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            for dep_name, config in data.get(section, {}).items():
                if isinstance(config, dict) and config.get("workspace") is True:
                    config = ws_deps.get(dep_name, {})
                if isinstance(config, str):
                    config = {"version": config}
                if isinstance(config, dict) and config.get("path"):
                    dep_package = config.get("package", dep_name)
                    dependencies.append({"name": dep_package})
                elif isinstance(config, dict):
                    dependencies.append({"name": config.get("package", dep_name)})
        packages.append({"name": package["name"], "dependencies": dependencies})
    return packages


def check_manifest_hygiene() -> list[str]:
    errors: list[str] = []
    root_manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
    workspace_deps = set(root_manifest.get("workspace", {}).get("dependencies", {}))
    for manifest_path in sorted((ROOT / "crates").glob("*/Cargo.toml")) + sorted((ROOT / "apps").glob("*/Cargo.toml")) + [ROOT / "xtask/Cargo.toml"]:
        data = tomllib.loads(manifest_path.read_text())
        package = data.get("package", {}).get("name", manifest_path.parent.name)
        if data.get("lints", {}).get("workspace") is not True:
            errors.append(f"{package}: missing [lints] workspace = true")
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            for dep_name, config in data.get(section, {}).items():
                if isinstance(config, dict) and config.get("workspace") is True:
                    if dep_name not in workspace_deps:
                        errors.append(f"{package}: `{dep_name}` inherits a dependency not declared in [workspace.dependencies]")
                else:
                    errors.append(f"{package}: `{section}.{dep_name}` must inherit from workspace.dependencies")
    return errors


def check_waivers(today: dt.date | None = None) -> list[str]:
    today = today or dt.date.today()
    config = tomllib.loads((ROOT / "deny.toml").read_text())
    errors: list[str] = []
    for waiver in config.get("advisories", {}).get("ignore", []):
        advisory = waiver.get("id", "<missing-id>")
        reason = waiver.get("reason", "")
        owner = None
        expiry = None
        for token in reason.replace(",", " ").replace("(", " ").replace(")", " ").split():
            if token.startswith("owner="): owner = token.split("=", 1)[1]
            if token.startswith("expires="): expiry = token.split("=", 1)[1].rstrip(".")
        if not owner or owner.lower() in {"unassigned", "todo", "tbd"}:
            errors.append(f"{advisory}: waiver requires an assigned owner")
        if not expiry:
            errors.append(f"{advisory}: waiver requires expires=YYYY-MM-DD")
        else:
            try:
                date = dt.date.fromisoformat(expiry)
                if date < today:
                    errors.append(f"{advisory}: waiver expired {date}; remove or renew it")
            except ValueError:
                errors.append(f"{advisory}: invalid expiry date `{expiry}`")
    return errors


def check_claims() -> list[str]:
    path = ROOT / "claims/ledger.json"
    ledger = json.loads(path.read_text())
    errors: list[str] = []
    if ledger.get("schema_version") != 1:
        errors.append("claims/ledger.json: schema_version must be 1")
    seen: set[str] = set()
    for claim in ledger.get("claims", []):
        cid = claim.get("id", "<missing-id>")
        if cid in seen: errors.append(f"{cid}: duplicate claim id")
        seen.add(cid)
        if not cid.startswith("CLM-"): errors.append(f"{cid}: id must start with CLM-")
        for field in ("statement", "falsifier", "scope"):
            value = claim.get(field)
            if value is None or (isinstance(value, str) and not value.strip()):
                errors.append(f"{cid}: missing non-empty {field}")
        if claim.get("status") not in {"proposed", "implemented", "verified", "falsified"}:
            errors.append(f"{cid}: invalid status `{claim.get('status')}`")
        if claim.get("status") == "verified":
            evidence = claim.get("evidence", {})
            if not evidence.get("command") or not evidence.get("verified_at") or not evidence.get("tests"):
                errors.append(f"{cid}: verified claims require command, verified_at and tests")
            for ref in evidence.get("tests", []):
                if "::" not in ref:
                    errors.append(f"{cid}: test reference `{ref}` must be path::test_name")
                    continue
                file, test_name = ref.split("::", 1)
                test_path = ROOT / file
                if not test_path.is_file() or test_name not in test_path.read_text(errors="ignore"):
                    errors.append(f"{cid}: cited test `{ref}` not found")
    return errors



def check_determinism() -> list[str]:
    errors: list[str] = []
    money_paths = [ROOT / "crates/trading-core/src/money.rs"]
    for path in money_paths:
        if path.exists():
            text = path.read_text()
            if re.search(r"\bf(?:32|64)\b", text):
                errors.append(f"{path.relative_to(ROOT)}: floating point is forbidden in money primitives")
    replay_roots = [ROOT / "crates/account-journal/src", ROOT / "crates/paper-engine/src", ROOT / "crates/market-archive/src"]
    for folder in replay_roots:
        for path in folder.rglob("*.rs") if folder.exists() else []:
            text = path.read_text()
            for forbidden in ("SystemTime::now", "Instant::now", "std::collections::HashMap"):
                if forbidden in text:
                    errors.append(f"{path.relative_to(ROOT)}: forbidden nondeterministic API `{forbidden}`")
    return errors


GATE_COMMAND_SOURCES = (
    "xtask/src/main.rs",
    "Makefile",
    "scripts/check_compile_budget.py",
)

# Built at runtime so the detector never matches its own source text.
FORBIDDEN_GATE_FLAG = "--all-" + "features"


def check_no_all_features_gate() -> list[str]:
    """Fail closed if any checked-in gate command enables every Cargo feature."""
    errors: list[str] = []
    sources = list(GATE_COMMAND_SOURCES)
    workflows = sorted((ROOT / ".github" / "workflows").glob("*.yml"))
    assurance = sorted((ROOT / "scripts" / "assurance").glob("*.sh"))
    sources.extend(str(path.relative_to(ROOT)) for path in workflows)
    sources.extend(str(path.relative_to(ROOT)) for path in assurance)
    for relative in sorted(sources):
        path = ROOT / relative
        if not path.is_file():
            errors.append(f"{relative}: gate command source is missing")
            continue
        for number, line in enumerate(path.read_text(errors="ignore").splitlines(), start=1):
            stripped = line.strip()
            if stripped.startswith(("#", "//")):
                continue
            if FORBIDDEN_GATE_FLAG in line:
                errors.append(
                    f"{relative}:{number}: gate command must not use {FORBIDDEN_GATE_FLAG}; "
                    "request the opt-in feature explicitly"
                )
    return errors


def run(command: str) -> int:
    errors: list[str] = []
    if command == "check":
        errors.extend(check_no_all_features_gate())
    if command in {"check", "arch"}:
        try:
            rules = tomllib.loads((ROOT / "architecture/layers.toml").read_text())
            try:
                packages = cargo_metadata()
            except FileNotFoundError:
                print("WARN: cargo not installed; checking dependency edges from Cargo manifests", file=sys.stderr)
                packages = static_workspace_packages()
            errors.extend(check_graph(rules, packages))
            errors.extend(check_manifest_hygiene())
            errors.extend(check_determinism())
        except (OSError, subprocess.CalledProcessError, json.JSONDecodeError, KeyError, tomllib.TOMLDecodeError) as exc:
            errors.append(f"architecture check could not complete: {exc}")
    if command in {"check", "waivers"}:
        errors.extend(check_waivers())
    if command in {"check", "claims"}:
        try: errors.extend(check_claims())
        except (OSError, ValueError, json.JSONDecodeError) as exc: errors.append(f"claims check could not complete: {exc}")
    if command not in {"check", "arch", "waivers", "claims"}:
        print("usage: check_repo.py [check|arch|waivers|claims]", file=sys.stderr)
        return 2
    if errors:
        for error in errors: print(f"FAIL: {error}", file=sys.stderr)
        print(f"{len(errors)} repository gate(s) failed", file=sys.stderr)
        return 1
    print(f"PASS: {command} repository gates")
    return 0

if __name__ == "__main__":
    raise SystemExit(run(sys.argv[1] if len(sys.argv) > 1 else "check"))
