# Migration map archive — Nirnay — 2026-11-23

**Status:** dated archival record issued under RRP-005. This record changes no
data right, provider operation, dataset acceptance, experiment state or Git
history. No commit was created in this worktree.

**Authority:** `engineering/retirement/RRP-001.md` §4 priority P1 row 5
(redundant specifications and documentation, CONSOLIDATE) and §6 binding rule 7;
`engineering/retirement/RETIREMENT-ACCEPTANCE.md` Gate RRP-005.

## 1. Archived artifact

| Field | Value |
|---|---|
| Original path | `engineering/migration-map.csv` |
| Disposition | ARCHIVE, then remove from the active tree |
| Bytes at removal | 4206 |
| SHA-256 of original bytes | `6192b639fbbcb79635b066459cb0d3e8a389da4454890c2975d2a44b51d50f85` |
| Removal worktree | `chore/rrp-005` (no commit created) |
| Replacement | this record; topic authority map in `evidence/RRP-005-CONSOLIDATION-2026-11-23.md` |

The CSV was a legacy promotion and removal ledger for the predecessor monolith
and the `.local/extras/` reference workspace. Its rows are historical. Paths
under `.local/extras/` were owner-local references held outside the active tree;
this working tree has no `.local/` directory. This record preserves the rows
verbatim and does not reclassify them as current capability or compliance
evidence.

## 2. Verbatim archived bytes

```csv
source,destination,state,notes
crates/types,crates/market-types,migrated,package name now matches directory
crates/book,crates/market-book,migrated,internal dependency path updated
crates/domain,crates/india-domain,migrated,India-specific domain retained explicitly
crates/trading-core,crates/trading-core,migrated,currently depends on India domain; not claimed universal
crates/cost-model,crates/cost-model,migrated,effective-dated exact arithmetic retained
crates/account-journal,crates/account-journal,migrated,append-only journal retained
crates/risk-engine,crates/risk-engine,migrated,fail-closed risk library retained
crates/execution-policy,crates/execution-policy,migrated,signed execution policy retained; no RSA dependency in the active workspace
crates/paper-engine,crates/paper-engine,migrated,paper engine retained; simulator still needs work
crates/upstox-feed-proto,crates/upstox-feed-proto,migrated,provider-specific V3 decoder/schema active; verified raw-frame decode-export added; preserve provider protocol semantics
legacy,extras,promoted-reference,useful tested subset promoted from legacy/ to .local/extras/ as the separate nirnay-extras workspace; auxiliary/parallel/duplicate material moved to the external snapshot
src,not-promoted,archived,predecessor monolith removed to external backup; unique India modules retained under .local/extras/src/venue/india
cpp,verification/cpp-oracle,promoted-verification-only,C++20 fixed-point/order-book oracle isolated from production; CMake/CTest passed 1/1; duplicate predecessor numeric sources removed
rust,.local/extras/rust,retained-reference,Rust conformance/SBE drivers retained as reference; port to active types before use
q,not-promoted,archived,no approved kdb+ entitlement/licence/operations case in active scope
prediction-market fixtures,not-promoted,archived,redistribution rights/provenance not established; removed to external backup
measurements,not-promoted,archived,predecessor performance/smoke records are not transferable evidence; removed to external backup
legacy/crates,not-promoted,removed,parallel copies of active crates; active crates canonical
legacy/experimental,not-promoted,removed,byte-identical copies of cpp/q/upstox-feed-proto
legacy/archive,not-promoted,removed,byte-identical snapshots of engineering/claims/company
legacy/schemas/execution+india+research+upstox,not-promoted,removed,byte-identical to active schemas
legacy/cpp/src,verification/cpp-oracle,removed,duplicates of the active oracle; SBE/conformance probes retained under .local/extras/cpp
legacy/tests,not-promoted,archived,prediction-market/session/replay tests depend on removed monolith; India tests retained under .local/extras/tests
legacy/engineering,not-promoted,archived,historical registries not current capability or compliance evidence; reference-store contract retained under .local/extras/engineering
legacy/claims,not-promoted,archived,unverified historical claims; conformance report recorded gate_passed false
legacy/company,not-promoted,archived,historical business thesis
legacy/deploy,not-promoted,archived,old runtime deployment
.local/extras/Cargo.toml,.local/extras/Cargo.toml,tested-reference,separate nirnay-extras workspace; cargo test passes 96 tests and C++ probes pass 1/1; excluded from root workspace
.local/extras/src/venue/india,.local/extras/src/venue/india,retained-reference,unique India ingestion/durable store; not active; 96 tests pass
.local/extras/tests/fixtures,.local/extras/tests/fixtures,retained-reference,differential conformance corpus
.local/extras/cpp,.local/extras/cpp,retained-reference,SBE/conformance probes; C++ test passes 1/1
.local/extras/schemas/challenge,.local/extras/schemas/challenge,retained-reference,synthetic SBE schema
.local/extras/cpp/src/sbe.cpp,crates/sbe-codec/src/decode.rs,promoted,self-contained challenge SBE decoder with typed fail-closed errors
.local/extras/tests/golden/sample_replay.sbe,crates/sbe-codec/tests/golden/sample_replay.sbe,promoted-then-retired,verbatim golden with provenance digest af291b4050e14; D11 removed the active golden from the tree
.local/extras/LICENSE,crates/sbe-codec/LICENSE,promoted,MIT licence retained with the promoted decoder
```

## 3. Reference resolution

Mutable documents that named the original path (`README.md` and
`engineering/extras-audit.md`) now point at this record. Dated evidence files
that name the original path are immutable historical statements; they resolve
to this record through the path map in the RRP-005 consolidation record.
