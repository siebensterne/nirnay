### N9 — Unit tests use the checked-in q harness

`analytics/kdb/mbp-test.q` runs CSV descriptions supplied by the caller. Each
case is `name,expression,expect` with `expect` in {`true`,`false`}. The D11
retirement removed the synthetic `mbp-unit.csv` case set and the `make kdb-q-test`
target; no market-shaped q cases remain in the active tree.

### N10 — Deterministic output is a contract

The emitted `metric,value` document is deterministic for a given input and q
build. Any change to the metric set, order or formatting must update the typed
contract, the JSON schema and the capability records in the same PR. D11 retired
the synthetic golden that pinned those bytes.

## Enforcement

- `scripts/check_q_style.py` checks added `.q` lines in `make check` and CI.
- Schema validation runs for non-market schemas; no market-shaped golden corpus
  remains in the active tree.

`q-style.json` at the repository root is the machine-readable form of this
policy. `scripts/check_q_style.py` loads it at import time, so the declared
column limit and namespace list are binding and no waiver is accepted.

