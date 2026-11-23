# D11 residual closure — Nirnay — 2026-11-23

**Status:** executed in worktree `nv-w21d11` on branch `chore/w21-d11`, base and exact
head `371a7add585b4baee4a37205e71022ed9927461c`. This is a new dated record; no commit
was created, no Git history was rewritten and no approved/dated record was edited.

## Authority and boundary

RRP-001 owner directive D11 removes synthetic market-data generators, fake sessions,
mock feeds, market-shaped benchmarks and their commands from the active tree while
preserving W1.1/W1.2 counts. RRP-002 closed 58 manifest paths; this W2.1 step closes the
residual active Python paths that RRP-002 left in place. No provider was contacted, no
right or entitlement was changed, no experiment or profitability claim was run and no
replacement synthetic data was created. Rights remain `UNKNOWN`; live order submission
stays disabled.

## Deleted from the active tree

| Path | Treatment |
|---|---|
| `research/python/src/quantlab/demo.py` | `git rm`. Contained the entire synthetic NSE quote session (prices, `MarketEvent`/`BookLevel` construction, synthetic event IDs and the EMA-cross driver). No replacement was created. |

## Files moved to dated evidence

| From | To |
|---|---|
| `research/python/tests/market_shaped_test_inventory.json` | `evidence/MARKET-SHAPED-TEST-INVENTORY-NIRNAY-2026-11-23.json` |

The `modules` block is byte-preserved: 23 modules, 189 static deselected names and 39
retained pure names. Added fields mark the file as a historical retirement record and
repeat the W1.1/W1.2 figures. `research/python/tests/conftest.py` reads the record at
its new path and continues to deselect legacy market-shaped tests; no prohibited test
was re-enabled. `pytest.ini` no longer applies `-m "not market_shaped_fixture"`; the
marker name stays registered only for detection.

## Modified source and tests

- `research/python/src/quantlab/cli.py`: removed the `demo` import, `_cmd_demo` and the
  `demo` subparser. No CLI route generates a market observation.
- `research/python/src/quantlab/bench.py`: removed `synthetic_events`, `synthetic_bars`,
  `_BuyOnce`, all four benchmark suites, the CLI entry point and their imports. The
  module now exposes only the scalar `percentile` helper.
- `research/python/src/quantlab/research_jobs.py`: removed `build_synthetic_dataset`,
  the `bench.synthetic_events` dependency, the reference-data/PIT imports and the
  generated-dataset constants. The generic runner accepts only an already admitted,
  externally published dataset; an empty job list fails closed with a named error.
- `research/python/tests/test_bench.py`: only the independent scalar percentile test
  remains.
- `research/python/tests/test_research_jobs.py`: the dataset-building fixture and the
  four run-path tests were removed. Three pure manifest/schema tests were retained with
  a scalar release-identity binding and renamed so the historical inventory does not
  deselect them.
- `research/python/tests/test_validation_report.py`: the generated-dataset frozen-case
  fixture and its five integration tests were removed. Four pure digest, encoding,
  refusal and loader tests remain.
- `research/python/tests/test_backtest_and_alpha.py`: removed the `run_demo` import and
  the demo determinism case; the other deselected market-shaped cases are untouched.
- `Makefile`, `.github/workflows/ci.yml`: added the policy checker to `check-static` and
  to the CI source-style job; retired the three synthetic Python benchmark targets.
- `configs/research/jobs.v1.json`: the job bound to `research-job-synthetic-nse-session`
  was retired, leaving `"jobs": []`. The `code_digest` and `code_paths` pin is unchanged
  in this step and is reserved for the next isolated PR.
- `schemas/research/research-job-manifest.v1.schema.json`: an empty job list is valid
  for a retired manifest; the runner still refuses to execute one.

## Static reintroduction gate

`scripts/check_synthetic_market_policy.py` scans active source (Python, Rust, q, shell,
C/C++, workflow, TOML/INI/JSON and `Makefile`) by path and regex. It rejects four
categories: prohibited synthetic generators, fake market sessions/demo drivers, mock
or fake market feeds, and synthetic market benchmarks. Dated `evidence/` records, the
gate's own rule table and historical log filenames are excluded from matching. Its
self-tests cover each category, benign scalar source, line reporting, path exclusions
and an injected source file.

`make check-static` now runs `synthetic-market-policy-check` (self-test plus scan); the
CI source-style job runs the same self-test and scan as a named step. Verbatim results
on this head:

```text
python3 scripts/check_synthetic_market_policy.py --self-test
Ran 8 tests ... OK
python3 scripts/check_synthetic_market_policy.py
synthetic-market policy: 436 source file(s) scanned, 0 prohibited pattern(s)
```

Adversarial injection probe (temporary untracked file, removed after the check): one
file containing `synthetic_bars(`, `run_demo()`, `MockFeed`, `BENCH-RESEARCH` and
`-m quantlab.bench` produced five findings and exit 1; the file was deleted and the
scan returned to zero findings and exit 0. No probe source remains in the tree.

## Historical counts preserved (not current coverage)

| Record | Figure | Status |
|---|---|---|
| W1.1-B Nirnay Python (`ba21945`) | 243 passed / 199 deselected / 2 subtests | historical, unchanged |
| W1.1-D Nirnay Python (`e975be9`) | 251 passed / 199 deselected / 2 subtests | historical, unchanged |
| W1.2-A synthetic policy record | 23 modules / 189 static names / 199 collected deselected / 90 marker-selected | historical, unchanged |
| W1.2-B Nirnay Rust | 711 preserved functions / 73 default / 638 withdrawn / 1 doctest | historical, unchanged |
| RRP-002 (`f6842d2` baseline) | 288 passed / 199 deselected / 2 subtests | historical, unchanged |

## Measured counts on this exact head

| Command | Result |
|---|---|
| `python3 -m pytest -q` | 295 passed, 182 deselected, 0 failed, 2 subtests passed |
| `cargo test --workspace --all-targets --locked` | 129 passed, 0 failed, 0 ignored |
| `cargo test --workspace --doc --locked` | 1 passed, 0 failed |
| `python3 scripts/validate_contracts.py` | 56 active JSON, 43 active TOML, 25 schemas validated |
| `make check-static` | pass (all component gates, including the new policy checker) |
| `make check` | pass (static gates, Rust tests/contracts, schemas, Python tests) |

The 17 removed deselected cases account for the drop from 199 to 182. The 7 added
pure Python tests account for the rise from 288 to 295. Zero tests were skipped for
missing fixtures. Rust figures are unchanged from RRP-002.

## Withdrawn coverage and residual gates

Withdrawn from active execution: the synthetic EMA demo driver, generated NSE-like
quote sessions, synthetic dataset publication used by research jobs and validation
reports, and the Python benchmark suites driven by invented observations. No
production engine, gate, rights path or source-ingestion module was changed.

- No commit was created; acceptance review and an exact reviewed commit remain open.
- The research job manifest code digest and `code_paths` pin are reconciled in a
  separate isolated PR, as directed.
- No historical W1.1/W1.2 count, dated evidence digest or approved contract byte was
  edited. Historical evidence that mentions retired paths was left unchanged.
- Rights remain `UNKNOWN`; no provider contact, collection, experiment, data-rights
  change or live execution occurred.
