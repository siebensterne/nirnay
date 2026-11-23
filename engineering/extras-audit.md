# Extras promotion and retirement audit

**Scope:** `.local/extras/**`, the promoted reference workspace retained from the
predecessor Nirnay Quant ZIP. The predecessor material was formerly stored under
`legacy/`; the useful, tested subset was promoted to `.local/extras/` on 2026-11-13 and
the auxiliary, parallel and duplicate material was removed to an external
snapshot.  
**Policy:** preserve provenance; promote by contract and tests; keep the active
production graph free of predecessor dependencies.

## Status

| Item | State |
|---|---|
| Directory | `.local/extras/` |
| Cargo package | `nirnay-extras` (library crate `nirnay_extras`) |
| Workspace | Separate `[workspace]`; **not** a member of the root workspace |
| Tests | `cargo test --manifest-path .local/extras/Cargo.toml` — **96 passed** (5 unit, 42 ingestion, 49 durable-store) |
| C++ probes | `ctest --test-dir <build>` — **1/1 passed** |
| Runtime / broker path | Not imported, not linked, not required |
| Licence | MIT, Copyright (c) 2026 Kartik Jangid (`.local/extras/LICENSE`) |

`.local/extras/` is named extras rather than legacy because it contains tested,
licensed reference material that may be promoted later. It is not dead code and
it is not a second production tree.

## Promoted into the active workspace

These predecessor assets are already active and must not be re-imported from
`.local/extras/`:

| Predecessor material | Active destination | Evidence / caveat |
|---|---|---|
| Upstox V3 Protobuf schema/decoder and provider protocol work | `schemas/upstox/v3/`, `crates/upstox-feed-proto/`, `apps/upstox-tools/` | Active decoder plus `upstox-decode-export`; capture sequence remains local, not exchange sequence. |
| India instrument listing and corporate-action concepts | `research/python/src/quantlab/point_in_time.py`, `schemas/research/` | Bitemporal listing and universe-membership records, immutable action revisions, exact split/bonus ratios and source-hash lineage. Not a complete official historical-security master. |
| C++ fixed-point arithmetic | `verification/cpp-oracle/` | Independent verification-only C++20 target; D11 retired the market-book/oracle sources; not wired into production Rust or a broker path. |
| Architecture, verification and provenance conventions | `engineering/` docs, `scripts/check_repo.py`, active contracts | Selective concepts rewritten against active module boundaries; predecessor JSON registries were not copied wholesale. |

## Retained under `.local/extras/`

- `src/venue/india/**`: unique NSE/BSE master ingestion, ISIN-first
  reconciliation, bitemporal snapshots, durable SQLite store, backup/restore and
  source archive.
- `tests/india_reference_ingestion.rs` (42 tests) and `tests/reference_store.rs`
  (49 tests): parser/reconciliation and crash/restart/backup acceptance suites.
- `engineering/reference-store-contract.json`: frozen verification groups
  referenced by the durable-store suite.
- `cpp/`: SBE decoder and Rust/C++ conformance probes. Shared pure numeric
  sources come from `verification/cpp-oracle/`; the market-book sources were
  retired under D11.
- `tests/fixtures/`, `tests/golden/`, `schemas/challenge/`, `schemas/internal/`:
  historical synthetic differential/SBE fixtures and contracts; active root
  `fixtures/india/` was retired under D11.
- `scripts/conformance.py`, `rust/`: reference harnesses. The Rust drivers still
  target the archived monolith types and need porting before use.
- `README.md`, `README.upstream.archive`, `UPSTREAM-ARCHIVE.txt`, `LICENSE`,
  `evidence/conformance_report.json`.

The historical conformance report records `gate_passed: false` with empty
conformance and fixture lists. Do not cite predecessor SBE/BOOK/REPLAY claims as
verified evidence.

The SBE decoder and golden corpus are inventoried in
`evidence/SBE-GAP-2-INVENTORY-2026-11-19.md`. Each obligation is mapped to a
proposed active Rust home and an independent differential test. That inventory
is a scope record, not a promotion: no file has been copied and `.local/extras/`
retirement remains blocked until every obligation has an active home.

## Removed to the external snapshot

The following were removed from the active tree because they were auxiliary,
parallel or duplicate. They are preserved in the external snapshot and must not
be re-added without an ADR, provenance/licence check, domain owner and executable
tests:

- parallel copies of active crates (`legacy/crates/**`);
- the predecessor monolith runtime/collector/session/paper code and
  archived prediction-market adapters;
- `experimental/**`, `archive/**`, `claims/**`, `company/**`, historical
  engineering registries, `measurements/**`, `q/**`, `benches/**`, `deploy/**`,
  `.github/**`, `xtask/**`;
- duplicate schemas, duplicate C++ numeric sources, non-India tests and
  prediction-market fixtures;
- old manifests, lockfiles, CI/deploy files and generated caches.

```text
External snapshot: ../legacy-snapshot-2026-11-13/legacy-full-backup-2026-11-13.tar.gz
SHA-256:            a49e864308a7e58d667a86049dd80e8ff2c631cd762e50a74aff60007464f9b1
Removed manifest:   ../legacy-snapshot-2026-11-13/legacy-removed-manifest-2026-11-13.txt
```

The cleanup and promotion are recorded in `engineering/migration-map.csv`; the
detailed evidence is in `engineering/legacy-audit-2026-11-13.md`.

## Rules for `.local/extras/`

1. `.local/extras/**` stays excluded from the root Cargo workspace, default Python test
   discovery and production runtime paths.
2. No active code may import, copy or depend on `.local/extras/**` without an ADR,
   source provenance, licence check, domain owner and executable tests.
3. Promote individual modules, never the predecessor monolith. A promotion that
   adds SQLite or filesystem I/O to a pure domain crate needs a separate crate
   and an architecture dependency edge.
4. Add every promotion or removal to `engineering/migration-map.csv`.
5. Keep `.local/extras/LICENSE` and `.local/extras/UPSTREAM-ARCHIVE.txt` with any copy or
   promotion.
6. `.local/extras/` is not scheduled for deletion. If it is ever retired, retain the
   external snapshot and the licence first.

## Re-audit 2026-11-19

The SBE decoder, golden corpus and licence now have an active home in
`crates/sbe-codec` with independent differential evidence
(`evidence/SBE-GAP-2-PROMOTION-2026-11-19.md`), closing GAP-2. The full
classification is in `evidence/EXTRAS-REAUDIT-2026-11-19.md`.

Retirement remains blocked: RSV-08 is an unratified owner decision, the
numeric/book oracle has no byte-for-byte Rust differential, and the book
conformance driver, `scripts/conformance.py` and four remaining India helper
modules have no demonstrated active home or abandonment decision. The shared
temporal goldens and India fixtures do have active homes; see the re-audit
record for the corrected classification.

## Prose sweep disposition 2026-11-19

The full-tree prose sweep recorded one finding in `.local/extras/README.md:91`
(PROSE-002). It is left unchanged to preserve predecessor provenance; active
repositories and canonical documents carry no rewritten predecessor text.
The finding remains visible in `evidence/PROSE-SWEEP-2026-11-19.md` and is not
counted as resolved.
