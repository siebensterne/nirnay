# SPDX-License-Identifier: Apache-2.0
"""Collection-time enforcement of the synthetic market-data test policy.

Under the owner directive of 2026-11-22 (D1-D3), tests that construct invented
market observations are not active CI evidence. The dated historical inventory
``evidence/MARKET-SHAPED-TEST-INVENTORY-NIRNAY-2026-11-23.json`` records which
legacy tests are withdrawn. Deselection here does not depend on a ``-m``
expression: overriding a marker expression cannot restore a prohibited test.

A module-level AST scan complements the inventory. Any collected test module
that contains a market-domain construct and is absent from the inventory stops
collection with a message that requires an inventory update. Under D11 the
synthetic generators, benchmark suites and CLI route were deleted from the
active tree; this detector is retained so that any reintroduced synthetic
market test is rejected. Source-pattern rejection of prohibited generators,
fake market sessions, mock feeds and synthetic benchmarks is bound separately
through ``scripts/check_synthetic_market_policy.py`` and ``make check-static``.
"""
from __future__ import annotations

import ast
import json
from collections.abc import Iterator
from pathlib import Path
from typing import Any

import pytest

TEST_DIR = Path(__file__).resolve().parent
REPO_ROOT = TEST_DIR.parents[2]
INVENTORY_PATH = REPO_ROOT / "evidence" / "MARKET-SHAPED-TEST-INVENTORY-NIRNAY-2026-11-23.json"

# Market-domain constructors prohibited by D1. Names are matched on the call
# name, including attribute calls such as ``quantlab.market.MarketEvent(...)``.
CONSTRUCTOR_NAMES = frozenset(
    {
        "MarketEvent",
        "BookLevel",
        "BookSnapshot",
        "OrderAdd",
        "OrderCancel",
        "TradePrint",
        "OhlcvBar",
        "Bar",
        "UniverseMembershipRow",
        "SyntheticCorporateAction",
        "SimBookLevel",
    }
)

# Canonical projection quote keys that encode an invented bid/ask observation.
MARKET_KEYS = frozenset({"bidAskQuote", "bidP", "askP", "bidQ", "askQ"})

# Checked-in market-shaped fixture paths named by the withdrawal audit.
FIXTURE_PATH_MARKERS = (
    "alpha-bars.synthetic.csv",
    "alpha-bars-minute.seed-20261121.csv",
    "canonical-events.example.jsonl",
    "reference-data.example.json",
    "fixtures/calendars/",
)

# Names that hold an invented price/bar sequence rather than a scalar.
PRICE_ARRAY_NAMES = frozenset(
    {
        "closes",
        "prices",
        "bars",
        "ohlcv",
        "price_series",
        "down_prices",
        "up_prices",
        "_closes",
        "_prices",
        "_bars",
    }
)


def _walk(node: ast.AST) -> Iterator[ast.AST]:
    """Yield a node and every descendant in source order."""
    yield node
    for child in ast.iter_child_nodes(node):
        yield from _walk(child)


def _call_name(node: ast.Call) -> str | None:
    """Return the bare name of a call target, or ``None`` for complex targets."""
    function = node.func
    if isinstance(function, ast.Name):
        return function.id
    if isinstance(function, ast.Attribute):
        return function.attr
    return None


def scan_module(path: Path) -> list[str]:
    """Return sorted market-domain construct tags found in one Python module."""
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    tags: set[str] = set()
    for node in _walk(tree):
        if isinstance(node, ast.Call):
            name = _call_name(node)
            if name in CONSTRUCTOR_NAMES:
                tags.add(f"constructor:{name}")
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            value = node.value
            if value in MARKET_KEYS:
                tags.add(f"key:{value}")
            for marker in FIXTURE_PATH_MARKERS:
                if marker in value:
                    tags.add(f"fixture_path:{marker}")
        if isinstance(node, (ast.Assign, ast.AnnAssign)):
            targets = [node.target] if isinstance(node, ast.AnnAssign) else node.targets
            value_is_array = isinstance(
                node.value,
                (ast.List, ast.Tuple, ast.ListComp, ast.SetComp, ast.GeneratorExp),
            )
            for target in targets:
                if (
                    isinstance(target, ast.Name)
                    and target.id.lower() in PRICE_ARRAY_NAMES
                    and value_is_array
                ):
                    tags.add(f"price_array:{target.id}")
        if (
            isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
            and node.name.lower() in PRICE_ARRAY_NAMES
            and any(
                isinstance(item, (ast.List, ast.Tuple, ast.ListComp))
                for item in _walk(node)
            )
        ):
            tags.add(f"price_array:{node.name}")
    return sorted(tags)


def load_inventory() -> dict[str, Any]:
    """Load the checked-in inventory of prohibited test modules and tests."""
    return json.loads(INVENTORY_PATH.read_text(encoding="utf-8"))


def _relative_module(path: Path) -> str:
    """Return the repository-relative POSIX path for a test module."""
    return path.relative_to(REPO_ROOT).as_posix()


def _test_name(nodeid: str) -> str:
    """Return the class-qualified test name without parametrization suffixes."""
    parts = nodeid.split("::")[1:]
    if parts:
        parts[-1] = parts[-1].split("[", 1)[0]
    return "::".join(parts)


def _all_test_modules() -> list[Path]:
    """Return every checked-in Python test module outside Git metadata."""
    return sorted(
        path
        for path in REPO_ROOT.rglob("test_*.py")
        if ".git" not in path.parts
        and not {"build", "dist", ".venv", ".pytest_cache", "__pycache__"} & set(path.parts)
    )


def pytest_collection_modifyitems(config: pytest.Config, items: list[pytest.Item]) -> None:
    """Deselect inventoried tests and block uninventoried market-shaped modules."""
    inventory = load_inventory()
    modules: dict[str, Any] = inventory["modules"]

    blockers: list[str] = []
    for path in _all_test_modules():
        module = _relative_module(path)
        constructs = scan_module(path)
        if constructs and module not in modules:
            blockers.append(f"{module}: {', '.join(constructs)}")
    if blockers:
        detail = "\n  ".join(blockers)
        raise pytest.UsageError(
            "market-shaped test policy: collection blocked. Modules with "
            "market-domain constructs are absent from "
            "evidence/MARKET-SHAPED-TEST-INVENTORY-NIRNAY-2026-11-23.json:\n  "
            + detail
            + "\nA historical inventory cannot be amended by an agent edit; "
            "reintroduction requires a versioned owner decision."
        )

    deselected: list[pytest.Item] = []
    for item in items:
        entry = modules.get(_relative_module(item.path))
        if entry is None:
            continue
        targets = set(entry["deselect"])
        name = _test_name(item.nodeid)
        if "*" in targets or name in targets or name.split("::")[-1] in targets:
            deselected.append(item)
    if deselected:
        item_set = set(deselected)
        config.hook.pytest_deselected(items=deselected)
        items[:] = [item for item in items if item not in item_set]
