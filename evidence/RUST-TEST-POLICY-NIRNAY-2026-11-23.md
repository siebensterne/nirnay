# Rust test policy gate — Nirnay — 2026-11-23

**Status:** implemented. Test-policy and governance record. No dataset, right,
provider operation, experiment, or production source changed. No historical
test source was deleted; withdrawal removes default execution only.

## Rule enforced

Owner directive `evidence/OWNER-DIRECTIVE-2026-11-22.md` (D1-D3) bars synthetic
market data as research evidence and keeps only scalar, abstract-time,
arbitrary-byte, rights, and audit tests active. The Python half of this policy
is enforced by `research/python/tests/conftest.py` and its inventory. This
record covers the Rust workspace.

The mechanism matches Fieldbook W1.1-B. Each affected crate declares a
`[features] market-shaped-tests = []` entry that is absent from `default`.
Prohibited integration targets carry an explicit `[[test]]` declaration with
`required-features = ["market-shaped-tests"]`. Prohibited `#[cfg(test)]` modules
carry `#[cfg(all(test, feature = "market-shaped-tests"))]`. A default gate run
therefore cannot execute withdrawn tests; an explicit `--features
market-shaped-tests` opt-in is required to compile them.

No gate command uses `--all-features`. `scripts/check_repo.py::run("check")`
calls `check_no_all_features_gate()`, which scans `xtask/src/main.rs`, the
`Makefile`, `scripts/check_compile_budget.py`, every workflow under
`.github/workflows/`, and every shell script under `scripts/assurance/`. The
scanner fails closed when any gate command line contains the all-features flag,
when a scanned gate source is missing, or when an arbitrary gate file is added
without being listed by those globs.

## Inventory counts

| Measure | Count |
|---|---:|
| Test functions preserved in source | 711 |
| Policy inventory: permitted before the `temporal_compat` fixture gate | 74 |
| Policy inventory: withdrawn before the `temporal_compat` fixture gate | 637 |
| Test functions executed by the default gate (enforced) | 73 |
| Test functions withdrawn from default execution (enforced) | 638 |
| Doctests executed by the default gate | 1 |
| Manifests declaring `market-shaped-tests` | 25 |
| Manifests with explicit gated `[[test]]` targets | 22 |
| Manifests with `autotests = false` (permitted sibling present) | 2 |
| Prohibited integration target files gated | 42 |
| Prohibited src test modules gated | 23 |
| `#[cfg(test)]` attributes rewritten in src | 28 |
| Fixture test gated inside a permitted integration target | 1 |

The policy inventory equality `74 + 637 = 711` holds, and the enforced
equality `73 + 638 = 711` holds. The two views differ by one test. The brief
records `74 permitted / 637 withdrawn` while also directing that the
fixture-reading test `replay_and_persisted_metadata_decode_identically` in
`crates/india-domain/tests/temporal_compat.rs` be gated. Those two directives
cannot both hold: with the gate in place the default command executes 73 tests
and withdraws 638. The gate is implemented as directed, and both figures are
recorded here so the inventory estimate is not quoted as the executed count.
The 28 rewritten attributes are 22
single-site modules plus all six sites in `crates/market-archive/src/journal.rs`
(lines 28, 45, 56, 83, 135, 354). Changing those six together keeps the private
`FailPoint` enum, the `JournalIo::Failing` variant and arm, the fail-point
constructor, and the test module under one condition, which avoids dead-code
failures under `clippy -D warnings`.

## Gated integration targets (42 files)

| Manifest | Gated test files |
|---|---|
| `apps/fundamental-tools/Cargo.toml` | `tests/contract.rs` |
| `apps/kdb-lab/Cargo.toml` | `tests/q_replay.rs` |
| `apps/paper-runner/Cargo.toml` | `tests/certification.rs`, `tests/recovery_drill.rs` |
| `crates/account-journal/Cargo.toml` | `tests/journal.rs` |
| `crates/authoritative-oms/Cargo.toml` | `tests/account_ledger.rs`, `tests/durability.rs`, `tests/fill_events.rs`, `tests/lifecycle.rs`, `tests/performance.rs`, `tests/permit_binding.rs` |
| `crates/broker-adapter/Cargo.toml` | `tests/contract.rs` |
| `crates/cost-model/Cargo.toml` | `tests/fees.rs` |
| `crates/execution-policy/Cargo.toml` | `tests/execution_policy.rs` |
| `crates/fundamental-domain/Cargo.toml` | `tests/fact_contract.rs` |
| `crates/india-domain/Cargo.toml` | `tests/india_domain.rs` |
| `crates/india-reference/Cargo.toml` | `tests/durability.rs`, `tests/ingestion.rs`, `tests/source_conformance.rs` |
| `crates/market-archive/Cargo.toml` | `tests/ingress.rs`, `tests/performance.rs` |
| `crates/market-book/Cargo.toml` | `tests/mbp.rs`, `tests/reconciliation.rs` |
| `crates/market-event/Cargo.toml` | `tests/fresh_process_replay.rs`, `tests/kernel_contract.rs` |
| `crates/market-kernel/Cargo.toml` | `tests/projection.rs`, `tests/replay.rs` |
| `crates/paper-engine/Cargo.toml` | `tests/capsule.rs`, `tests/engine.rs`, `tests/liquidity.rs`, `tests/paper_run.rs`, `tests/provenance.rs`, `tests/venue_matching.rs` |
| `crates/risk-authority/Cargo.toml` | `tests/authority.rs`, `tests/performance.rs`, `tests/risk_controls.rs` |
| `crates/risk-engine/Cargo.toml` | `tests/risk.rs` |
| `crates/sbe-codec/Cargo.toml` | `tests/differential.rs` |
| `crates/strategy-runtime/Cargo.toml` | `tests/parity.rs` |
| `crates/trading-core/Cargo.toml` | `tests/contracts.rs` |
| `crates/upstox-feed-proto/Cargo.toml` | `tests/decoder.rs`, `tests/mbp.rs` |

The three permitted targets remain un-gated:
`crates/india-domain/tests/temporal_compat.rs`,
`crates/market-archive/tests/capability.rs`, and
`crates/ops-runtime/tests/ops.rs`. The first two are listed explicitly because
their packages set `autotests = false`; `ops-runtime` is untouched. In
`temporal_compat.rs` the fixture-reading test
`replay_and_persisted_metadata_decode_identically` and its `InstrumentWire`
import are gated behind the opt-in feature, so the default run keeps the
abstract-time wire goldens only.

`apps/cost-cli`, `apps/fyers-importer`, and `apps/upstox-tools` declare the
feature for their gated src modules but contain no Rust integration target,
so no `[[test]]` entries were added.

## Gated src test modules (23 modules, 28 attributes)

| Crate | Gated src files |
|---|---|
| `cost-cli` | `src/lib.rs` |
| `fundamental-tools` | `src/lib.rs` |
| `fyers-importer` | `src/main.rs`, `src/mbp.rs` |
| `kdb-lab` | `src/metrics.rs`, `src/observation.rs`, `src/reference.rs`, `src/report.rs` |
| `paper-runner` | `src/lib.rs` |
| `upstox-tools` | `src/bin/upstox_decode_export.rs`, `src/bin/upstox_kernel_session.rs`, `src/bin/upstox_recorder.rs`, `src/bin/upstox_smoke.rs` |
| `fundamental-domain` | `src/lib.rs` |
| `india-domain` | `src/corporate_actions.rs` |
| `market-archive` | `src/capture.rs`, `src/frame.rs`, `src/journal.rs` (six sites), `src/manifest.rs` |
| `paper-engine` | `src/market.rs`, `src/state.rs` |
| `trading-core` | `src/model.rs`, `src/risk.rs` |

Permitted src modules left unchanged: `apps/kdb-lab/src/main.rs`,
`crates/market-archive/src/continuity.rs`, `crates/market-book/src/orderbook.rs`,
`crates/market-types/src/fixed.rs`, `crates/market-types/src/model.rs`,
`crates/trading-core/src/time.rs`, and `xtask/src/main.rs`.

## Coverage withdrawn from the default gate

The following assertions no longer run under a default gate command. No
capability claim rests on them.

- book, quote, order-add, order-cancel and trade-print construction and replay
  determinism, including queue behavior and fill costs;
- invented price and bar sequences, OHLCV feature cases, strategy parity and
  paper-engine capsule, liquidity and venue-matching scenarios;
- raw-frame ingress, journal fail-point, capture, manifest and performance
  harnesses that fabricate observations;
- event identity, kernel projection/replay, SBE differential and feed decoder
  cases driven by invented market payloads;
- corporate-action and India-domain market-shaped cases, cost-model schedules,
  execution-policy and risk scenarios built from fabricated books;
- OMS account ledger, durability, fill-event, lifecycle and permit-binding
  scenarios, broker-contract cases and certification/recovery drills;
- benchmark harnesses that generate market observations: `bench-ingress` and
  `bench-risk-oms` in the `Makefile`.

Retained active categories: pure scalar and abstract-time mathematics,
arbitrary-byte and JSONL reader tests, rights and authorization tests, audit
and hash-chain integrity tests, temporal wire-compatibility goldens, orderbook
and market-type scalar tests, ops-runtime health and backup tests, and xtask
policy self-tests.

## Commands and boundaries

Default gates keep plain commands: xtask `test` and `lint`,
`.github/workflows/ci.yml`, `.github/workflows/nightly.yml`,
`scripts/assurance/polish.sh`, and `scripts/check_compile_budget.py`. The
fundamental-tools invented-fixture validation step was removed from `ci.yml`.
The benchmark targets append `--features market-shaped-tests` explicitly and
stay outside `check` and CI pass/fail gates.

```bash
cargo test --workspace --all-targets --locked
cargo test --workspace --doc --locked
cargo xtask lint
python3 scripts/check_repo.py check
cargo test -p market-archive --test performance --features market-shaped-tests -- --ignored
```

The last line is an operator opt-in, not a gate. It compiles withdrawn source
to measure counters; emitted numbers are not throughput or latency
certificates. Reintroducing a withdrawn test into a default gate needs an owner
decision and a versioned amendment, not an agent edit.

## New count and historical figures

The enforced figures at this commit are 711 preserved test functions, 73
executed by the default command, and 638 withdrawn, plus one doctest. These
were re-derived from the gated source and from the exact output of
`cargo test --workspace --all-targets --locked` and
`cargo test --workspace --doc --locked`. The policy inventory figures
`74 permitted / 637 withdrawn` are retained in the table above as the
pre-gate view that treats the `temporal_compat` fixture test as permitted.

The earlier review shorthand `711 tests / 558-equivalent Rust` was a
pre-change historical estimate for the base head. It is historical, is not an
enforced count, and must not be quoted as current coverage. Withdrawal removes
default execution, not source. Dated records remain immutable; this file is a
new dated record and the 2026-11-22 and 2026-11-23 policy records were not
edited.
