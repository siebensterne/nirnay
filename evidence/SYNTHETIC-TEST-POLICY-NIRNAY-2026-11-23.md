# Synthetic test policy gate — Nirnay — 2026-11-23

**Status:** implemented. Test-policy and governance record. No dataset,
right, provider operation, experiment or production source changed. No
historical test source was deleted; withdrawal removes evidence only.

## Rule enforced

Owner directive `evidence/OWNER-DIRECTIVE-2026-11-22.md` (D1-D3) bars synthetic
market data as research evidence and keeps only scalar or abstract-timestamp
mathematics active. A read-only audit at the base head found market-shaped
tests constructing books, trades, quotes, orders, bars and prices, instrument
universes and corporate-action histories inline, alongside tests already
registered `market_shaped_fixture` in `pytest.ini`.

`-m "not market_shaped_fixture"` in `addopts` is not enforcement: a developer
can override the expression. Two checked-in files now enforce the rule:

| File | Role |
|---|---|
| `research/python/tests/market_shaped_test_inventory.json` | Authority listing prohibited modules, categories and test names. |
| `research/python/tests/conftest.py` | Deselects inventoried tests in `pytest_collection_modifyitems` and fails collection for uninventoried modules containing market-domain constructs. |

The gate scans ASTs for `MarketEvent`, `BookLevel`, `BookSnapshot`, `OrderAdd`,
`OrderCancel`, `TradePrint`, `OhlcvBar`, `Bar`, `UniverseMembershipRow`,
`SyntheticCorporateAction`, `SimBookLevel`, the `bidAskQuote`/`bidP`/`askP`
keys, named price/bar sequences and the five known market fixture paths. A
module with any of these constructs must appear in the inventory. Collection
stops with a clear message when it does not. Deselection does not read `-m`.

## Inventory counts

| Measure | Count |
|---|---:|
| Prohibited modules in inventory | 23 |
| Static test names marked for deselection | 189 |
| Static pure tests retained inside those modules | 39 |
| Collected test items deselected by the gate | 199 |
| Collected items already marked `market_shaped_fixture` | 90 |
| Collected unmarked items newly withdrawn | 109 |
| Active items passing | 251 (243 at W1.1-B commit `ba21945`) |
| Subtests passing | 2 |
| Total collected items at base head | 442 |

The pre-change read-only note estimated "about 140" unmarked constructors and
"89" marked fixture consumers. The checked-in counts above are the enforced
figures. The 90 marked items are the exact set selected by the registered
marker at the base head; the note was low by one. The unmarked total is 109
because 39 scalar, schema, byte and audit tests inside market-shaped modules
are retained under D3. Their names are recorded in each module's `retain`
list in the inventory.

### Modules by category

| Category | Modules | Static deselected tests |
|---|---:|---:|
| `inline_market_constructors` | 6 | 48 |
| `inline_price_sequence` | 1 | 6 |
| `market_shaped_fixture_consumer` | 10 | 59 |
| `mixed` | 6 | 76 |
| **Total** | **23** | **189** |

### Full module inventory

| Module (under `research/python/tests/`) | Category | Deselected | Retained |
|---|---|---:|---:|
| `test_backtest_and_alpha.py` | mixed | 12 | 0 |
| `test_bench.py` | market_shaped_fixture_consumer | 4 | 1 |
| `test_calendar_cadence.py` | market_shaped_fixture_consumer | 19 | 0 |
| `test_canonical_projection.py` | inline_market_constructors | 4 | 0 |
| `test_cli_pipeline.py` | mixed | 3 | 8 |
| `test_corporate_actions.py` | inline_market_constructors | 5 | 0 |
| `test_experiment_comparison.py` | market_shaped_fixture_consumer | 8 | 0 |
| `test_experiment_registry.py` | mixed | 29 | 0 |
| `test_feature_computation.py` | mixed | 5 | 2 |
| `test_feature_lineage.py` | market_shaped_fixture_consumer | 3 | 4 |
| `test_market_pipeline.py` | mixed | 26 | 0 |
| `test_paper_projection_consumption.py` | market_shaped_fixture_consumer | 6 | 0 |
| `test_parameter_search.py` | market_shaped_fixture_consumer | 4 | 2 |
| `test_portfolio_backtest.py` | inline_market_constructors | 4 | 0 |
| `test_quant_pipeline_e2e.py` | mixed | 1 | 0 |
| `test_research_jobs.py` | market_shaped_fixture_consumer | 7 | 0 |
| `test_research_package_shims.py` | market_shaped_fixture_consumer | 2 | 7 |
| `test_robustness.py` | market_shaped_fixture_consumer | 1 | 6 |
| `test_sim_core.py` | inline_market_constructors | 16 | 2 |
| `test_sim_extensions.py` | inline_market_constructors | 15 | 5 |
| `test_strategy_families.py` | inline_price_sequence | 6 | 2 |
| `test_universe.py` | inline_market_constructors | 4 | 0 |
| `test_validation_report.py` | market_shaped_fixture_consumer | 5 | 0 |

Category and reason text, detected constructs and the exact `deselect` and
`retain` test-name lists live in the inventory file.

## Exact inspection commands

Run from the repository root. These commands inspect state; none enables a
withdrawn test.

```bash
python3 -m pytest -q
python3 -m pytest -q -m market_shaped_fixture
python3 -m pytest --collect-only -q -m ""
python3 -c 'import json; d=json.load(open("research/python/tests/market_shaped_test_inventory.json")); print(len(d["modules"]), sum(len(v["deselect"]) for v in d["modules"].values()))'
python3 -c 'import json; d=json.load(open("research/python/tests/market_shaped_test_inventory.json")); print(*d["modules"], sep="\n")'
```

Observed outputs at this head:

```text
python3 -m pytest -q
  251 passed, 199 deselected, 2 subtests passed

python3 -m pytest -q -m market_shaped_fixture
  450 deselected

python3 -m pytest --collect-only -q -m ""
  251/450 tests collected (199 deselected)
```

The second command confirms that overriding the marker cannot restore a
prohibited test. The inventory count command prints `23 189`.

### Gate failure probe

Adding a test module that constructs a `BookLevel` without editing the
inventory stops collection:

```text
ERROR: market-shaped test policy: collection blocked. Modules with
market-domain constructs are absent from
research/python/tests/market_shaped_test_inventory.json:
  research/python/tests/test_zzz_gate_probe.py: constructor:BookLevel
Update the inventory with the module and its prohibited tests before adding
or restoring these tests.
```

The probe file was removed after the check. The probe is a gate exercise, not
a source change.

## New test count and historical figures

The active suite at the current head `e975be9` reports `251 passed, 199
deselected, 2 subtests passed`. This is the current figure, re-derived from
exact command output under W1.2-A.

The figure `243 passed, 199 deselected, 2 subtests passed` was correct at the
W1.1-B code commit `ba21945`; W1.1-D added eight admission tests, giving 251.
The earlier figures `352 passed, 90 deselected, 2 subtests passed` at base
`72bac0c`, and the `300 passed, 90 deselected` figure in the 2026-11-22
withdrawal record, are historical counts of those heads. They are no longer
current coverage and must not be quoted as such.

## Coverage withdrawn from active CI

The following assertions no longer run. No capability claim rests on them.

- canonical event identity, raw-frame hash binding and provider payload shape;
- provider quote, depth and last-trade observation normalization;
- book, trade and order replay determinism, queue behavior and fill costs;
- OHLCV feature computation and feature lineage tamper detection;
- corporate-action and universe-membership point-in-time behavior;
- strategy signals and alpha screens driven by invented price sequences;
- experiment registry, cadence, manifest, report and research-job guards that
  read market-shaped fixtures;
- demo and benchmark harnesses that fabricate market observations.

Retained active categories: pure mathematics, scalar and abstract-timestamp
tests, rights and authorization tests, audit and integrity tests, schema
tests, arbitrary-byte and JSONL reader tests, and risk, OMS, ledger and KMS
infrastructure tests. The 39 retained tests inside mixed modules are named in
the inventory.

## Boundaries and failure cases

- The underlying code is unchanged. Withdrawal removes evidence, not behavior.
- No right changes from `UNKNOWN`; no dataset was accepted or admitted; no
  provider was contacted; no experiment or profitability claim was run.
- A retained test reading an archived fixture without an inventory update is a
  failure case; the module AST scan closes it when the construct is one of the
  scanned forms.
- Reintroducing a withdrawn test needs an owner decision and a versioned
  amendment, not an agent edit.
- Dated records remain immutable. This file is a new dated record; the
  2026-11-22 withdrawal record was not edited.
