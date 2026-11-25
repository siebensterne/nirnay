# Nirnay Quant — research-to-paper trading workspace

Nirnay is the independent consumer for NDC releases: research, simulation,
portfolio construction, risk and OMS. It is not a broker-connected or
exchange-approved algo product, and live order routing is hard-disabled.

## Status authority

Cross-repository status, freezes and authorizations live in the workspace
`CURRENT-STATE.json`. Capability claims and falsifiers live in
[claims/ledger.json](claims/ledger.json); `engineering/capabilities.json` is
superseded. This README carries no status snapshot: a CI identifier alone does
not bind a run to the commit it tested. `scripts/check_docs.py` enforces this.

## Ownership and boundaries

- Python (`research/python/src/quantlab`) owns research and statistical validation.
- Rust crates own event identity, books, portfolio state, risk, OMS and journal.
- `analytics/kdb/` q scripts own bounded metric computation; `q-style.json` is the style contract.
- Fieldbook owns acquisition, raw storage, normalization, rights and NDC production.
- No shared crate, broker, database, provider contact or live transport.

## Build and verification

- Python 3.12+; install the research package from `research/python[dev,research,security]`.
- `make check` — full local gate; `make check-static` — static gates only.
- `make verify-local` — local verification suite; `make assure` — polish and re-check.
- `cargo xtask --help` lists binding gates (`check`, `arch`, `waivers`, `claims`).
- `cmake --workflow --preset strict-gcc` or `strict-clang` builds the strict C++20 oracle.
- Hosted CI runs on PRs. Record run ID, exact head SHA, date and scope as
  evidence; never infer status from a green run.

## Architecture entry points

- `Cargo.toml` and crate manifests define the Rust dependency graph.
- `engineering/architecture/decisions/ADR-INDEX.md` owns ADR identities.
- `engineering/verification-policy.md` defines claim states.
- `engineering/data-governance.md` owns data, rights, acceptance and retention.
- Workspace `NDC-REGISTRY.md` owns the NDC contract version.

## Data, research and execution limits

Provider rights remain `UNKNOWN` (21 products); dataset acceptance is blocked
and the data budget is INR 0. A URL, token or checksum grants no right. D11
retired synthetic market-shaped generators, fixtures and demos; the M4 holdout
is untouched. Experiments require preregistration; strategies emit intents
only, and risk and OMS remain independent authorities. Pure mathematical and
abstract protocol tests are not market evidence.

## Documents

- Canonical: `README.md`, `AGENTS.md`, `CONTRIBUTING.md`, `SECURITY.md`,
  `engineering/verification-policy.md`.
- Dated evidence: `BUILD-RESULTS.md`, `evidence/VERIFICATION-2026-11-17.md`,
  `evidence/UPGRADE-VERIFICATION-2026-11-13.md`,
  `evidence/BUILD-INVENTORY-2026-11-13.json`.
- Other files under `evidence/` are immutable dated records, not active
  navigation. `ADR-INDEX.md` indexes accepted decisions.

A capability is admitted by contract, executable tests and scoped evidence,
not by a demo, a backtest or a green gate from another commit.
