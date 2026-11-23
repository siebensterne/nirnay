# kdb-lab

Repo-native KDB-X analytics harness. This crate owns the typed contracts
around the checked-in q script `analytics/kdb/mbp_metrics.q`; it does not
reimplement analytics in Rust. KDB-X remains the engine of record.

## Why this exists

Research needs repeatable spread, imbalance, gap and depth measurements from
captured five-level books. An operator-local q script was not versioned and had
no typed contract, so a drifted script or a different engine build could
silently change research numbers. This crate gives the analytics a checked-in
contract, a report schema and a measured reason to exist: one book per source,
never merged. D11 retired the synthetic golden corpus and its replay/verification
commands; only analysis of an explicitly supplied input remains.

## Contract

- Input: header-prefixed CSV with columns
  `seq,recv_ns,bid1_p,bid1_q,ask1_p,ask1_q,spread,imbalance,levels`.
- Output: deterministic `metric,value` lines with the names in
  `src/metrics.rs::METRIC_NAMES`.
- Units: prices and spreads are f64 in the input unit (rupees as exported by
  `upstox-decode-export`). No metric name implies paise. This is a research
  statistics surface, not the authoritative money domain.

## Use

```bash
make kdb-analyze  # analyse KDB_INPUT into KDB_OUT (metrics.csv + report.json)
```

The binary is also callable directly:

```bash
cargo run -p kdb-lab --bin nirnay-kdb -- analyze \
  --input .local/perf/kdb/book.csv --source upstox-live \
  --out .local/perf/kdb-lab-run
```

The q binary is taken from `NIRNAY_Q_BIN` or
`/Users/tourist/.kx/bin/q`. No credential is read, printed or exported by this
crate. Input and output SHA-256 digests in the report make each run reproducible.

## Gates

- `cargo test -p kdb-lab`: CLI parsing tests. The market-shaped golden replay and
  golden-dependent reference/report tests were retired under D11.
- `scripts/validate_contracts.py` validates non-market JSON/TOML schemas.

Live orders, broker writes, licensed session data and cross-source book merging
are outside this crate's scope.
