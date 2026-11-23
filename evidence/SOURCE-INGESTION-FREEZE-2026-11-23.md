# Source-ingestion freeze record — Nirnay — 2026-11-23

**Status:** metadata-only freeze issued under RRP-004. This is a freeze, not a removal. No
provider was contacted, no right was changed, no dataset was accepted, no experiment was
registered, no live execution was enabled, no Git history was rewritten and no production
code was deleted. No commit was created in this worktree.

## 1. Scope and authority

- Repository: `nirnay`; worktree branch `chore/rrp-004`; reviewed base
  `9c29a0e635881bce7e38007a2e709ea5bcd1ae42`.
- Authority: `engineering/retirement/RRP-001.md` §4 priority P0 row 3 and §6 binding rule 4;
  `engineering/retirement/RETIREMENT-ACCEPTANCE.md` Gate RRP-004; the Nirnay retirement
  manifest and dependency crosswalk in the same external directory.
- This record implements the freeze half of RRP-004: ownership metadata, replacement
  conditions, a migration register and a repo gate. It does not authorize a deletion.
- RRP-002 retired the synthetic fixtures under these paths. The decoder, importer, recorder,
  journal and reference source modules were preserved for migration; this record freezes them.
  `evidence/TEST-RETIREMENT-NIRNAY-2026-11-23.md` remains unchanged.
- Machine-readable input: `engineering/source-ingestion-freeze.v1.json`. Gate:
  `scripts/check_source_ingestion_freeze.py`, exposed as `make freeze-check` and included in
  `make check` through `check-static`.

## 2. Frozen paths

The table is an index. Each path then carries its responsibility, Fieldbook target owner,
replacement condition, migration order and no-new-feature rule in full.

| Frozen path | Current responsibility | Fieldbook target owner | Replacement condition |
|---|---|---|---|
| `crates/india-reference` | Exchange reference-master ingestion and local identity allocation | Fieldbook producer owner; NDC contract owner | Accepted NDC India reference projection |
| `crates/market-archive` | Durable raw-frame journal and original-byte ingress | Fieldbook producer owner (raw storage) | Fieldbook raw bytes and NDC archive lineage verified |
| `crates/upstox-feed-proto` | Upstox V3 protobuf schema and decoder source | Fieldbook producer owner (decoding) | Fieldbook owns V3 decode and publishes NDC events |
| `apps/upstox-tools` | Upstox session verification and decode export | Fieldbook producer owner (acquisition) | Fieldbook acquisition and release commands verified |
| `apps/fyers-importer` | Offline FYERS session verification and raw import | Fieldbook producer owner (acquisition/raw storage) | Fieldbook owns FYERS ingestion and NDC publication |
| `apps/fyers-recorder` | FYERS Data Socket callback recorder | Fieldbook producer owner (acquisition) | Fieldbook owns capture and publishes normalized events |
| `research/python/src/quantlab/providers.py` | Provider mapping to canonical events and reconciliation | Fieldbook producer owner (normalization) | Admitted NDC events consumed instead of local mapping |
| `configs/providers/source-products.v1.json` | Tri-state rights registry (all `UNKNOWN`) | Data-rights steward; NDC contract owner | Reviewed rights evidence carried on an NDC release |
| `configs/providers` | Instrument maps, venue maps, watchlists and examples | Fieldbook producer owner; Data-rights steward | NDC supplies identity, universe and watchlist scope |

### 2.1 crates/india-reference

- Current responsibility: versioned exchange reference-master ingestion, local instrument
  identity allocation and effective-dated listing/reference records (CAP-020). Legacy, not
  the canonical source authority.
- Fieldbook target owner: Fieldbook producer owner for point-in-time reference data; NDC
  contract owner for the India reference release.
- Replacement condition: an admitted Fieldbook NDC India reference release with
  release-supplied stable IDs, effective intervals and accepted rights/coverage evidence is
  projected into `india-domain`. This crate is frozen until that NDC reference projection is
  accepted; no removal before acceptance.
- Migration order: Fieldbook adapter -> NDC projection -> verify -> remove old
  importer/archive.
- No-new-feature rule: no new ingestion source, identity allocator, listing record or
  reference authority in this crate.

### 2.2 crates/market-archive

- Current responsibility: durable raw-frame journal, original-byte ingress and shared
  recorder/archive primitives for Upstox and FYERS (CAP-005, CAP-021). A legacy duplicate of
  the Fieldbook raw archive.
- Fieldbook target owner: Fieldbook producer owner for raw storage and immutable archive
  references.
- Replacement condition: Fieldbook publishes immutable raw-observation bytes with digests
  and archive references, and an NDC projection carries those references as lineage. Local
  archiving may be retired only after verified consumption.
- Migration order: Fieldbook adapter -> NDC projection -> verify -> remove old
  importer/archive.
- No-new-feature rule: no new recorder, journal writer, archive format or raw-retention
  feature in this crate.

### 2.3 crates/upstox-feed-proto

- Current responsibility: Upstox Market Data Feed V3 protobuf schema and raw-frame
  decoder/source (CAP-012). The decoder source is preserved for migration; its synthetic
  fixture was retired under RRP-002.
- Fieldbook target owner: Fieldbook producer owner for source protocol decoding.
- Replacement condition: Fieldbook owns the V3 decoder and publishes normalized Upstox
  events through NDC; Nirnay consumes admitted events and stops decoding source frames. The
  decoder source stays until that replacement is accepted.
- Migration order: Fieldbook adapter -> NDC projection -> verify -> remove old
  importer/archive.
- No-new-feature rule: no new protocol version, template, decoder path or source-frame
  feature in this crate.

### 2.4 apps/upstox-tools

- Current responsibility: Upstox session verification and decode-export command-line
  tooling (CAP-005, CAP-012). Legacy acquisition and decode tooling.
- Fieldbook target owner: Fieldbook producer owner for acquisition and recorder commands.
- Replacement condition: Fieldbook acquisition and release commands cover session
  verification and decode export, and NDC consumption is verified. These tools are not
  removed before that replacement exists.
- Migration order: Fieldbook adapter -> NDC projection -> verify -> remove old
  importer/archive.
- No-new-feature rule: no new capture, verify-session, decode or export command in this app.

### 2.5 apps/fyers-importer

- Current responsibility: offline FYERS session manifest verification and import into the
  local raw journal (CAP-023). The importer source is preserved for migration; its
  fabricated session fixture was retired under RRP-002.
- Fieldbook target owner: Fieldbook producer owner for source acquisition and raw storage.
- Replacement condition: Fieldbook owns FYERS session ingestion and publishes the raw record
  through NDC; the importer is retired only after accepted replacement. No replacement
  synthetic session may be created.
- Migration order: Fieldbook adapter -> NDC projection -> verify -> remove old
  importer/archive.
- No-new-feature rule: no new session format, import path or journal writer in this app.

### 2.6 apps/fyers-recorder

- Current responsibility: FYERS Data Socket callback recorder and canonical callback
  normalizer (CAP-006). SDK callback JSON is not original wire bytes; no entitlement or
  exchange sequence is verified.
- Fieldbook target owner: Fieldbook producer owner for source acquisition.
- Replacement condition: Fieldbook owns FYERS Data Socket capture and publishes normalized
  callback events through NDC; Nirnay consumes admitted events under a verified rights
  decision. The recorder stays until that replacement is accepted.
- Migration order: Fieldbook adapter -> NDC projection -> verify -> remove old
  importer/archive.
- No-new-feature rule: no new socket, depth tier, symbol cap, callback schema or recorder
  output in this app.

### 2.7 research/python/src/quantlab/providers.py

- Current responsibility: provider-specific Upstox/FYERS mapping to versioned canonical
  events and cross-feed reconciliation (CAP-006, CAP-012, CAP-013). Legacy normalization,
  not a canonical source authority.
- Fieldbook target owner: Fieldbook producer owner for source normalization; Nirnay consumer
  owner consumes admitted NDC events.
- Replacement condition: Fieldbook normalization is published through an admitted NDC
  release with normalizer and event-schema versions; Nirnay consumes those events and keeps
  only diagnostic reconciliation. Provider mapping code is retired after verified
  consumption.
- Migration order: Fieldbook adapter -> NDC projection -> verify -> remove old
  importer/archive.
- No-new-feature rule: no new provider adapter, mapping, mapping heuristic, event type or
  reconciliation output in this module.

### 2.8 configs/providers/source-products.v1.json

- Current responsibility: tri-state source-product rights registry (seven `UNKNOWN`
  operations per product), entitlement references and retention basis (CAP-028). It records
  rights; it does not grant them.
- Fieldbook target owner: Data-rights steward for permission evidence and source
  qualification; NDC contract owner for versioning.
- Replacement condition: rights evidence is reviewed by the data-rights steward and the NDC
  release carries permission grants and document digests. A change of right needs a separate
  versioned owner decision; this file is not edited to imply permission.
- Migration order: Fieldbook adapter -> NDC projection -> verify -> remove old
  importer/archive.
- No-new-feature rule: no new right state, entitlement claim, retention term or product
  entry in this registry.

### 2.9 configs/providers

- Current responsibility: provider instrument maps, venue maps, watchlists and entitlement
  or live-account examples. Legacy local inputs to the frozen capture/normalization tooling,
  not canonical identity or universe data.
- Fieldbook target owner: Fieldbook producer owner for instrument identity and watchlist
  scope; Data-rights steward for entitlement evidence.
- Replacement condition: canonical instrument identity, effective-dated universe and
  watchlist scope come from an admitted Fieldbook NDC release; local map and watchlist
  inputs are retired after verified consumption.
- Migration order: Fieldbook adapter -> NDC projection -> verify -> remove old
  importer/archive.
- No-new-feature rule: no new instrument map, venue map, watchlist, entitlement example or
  live-account example under this directory.

## 3. Migration order (binding)

The four stages are ordered. A subsequent stage does not soften an earlier refusal.

| # | Stage | Gate to pass | Prohibited before this stage |
|---|---|---|---|
| 1 | Fieldbook adapter | Producer-owned acquisition, decode, normalization and raw archive exist as reviewed Fieldbook code | Deleting any Nirnay frozen module |
| 2 | NDC projection | An admitted NDC release is projected into Nirnay with exact release identity and digests | Treating a local map or local ID as canonical |
| 3 | Verify | Projection admission, coverage and rights evidence pass, and consumer replay is byte-stable | Claiming acceptance, rights or live readiness from a checksum |
| 4 | Remove old importer/archive | A per-path approved migration record names the replacement commit and is reviewed | Bulk `git rm -r` or history rewriting |

The freeze gate requires an approved migration record in the manifest for every change to a
frozen path. The record names the path, an evidence file under `evidence/`, the owner
approval, the replacement commit and the decision date. No record exists at this base.

## 4. No-new-feature rule (binding)

No new feature, provider, provider map, watchlist, collection path, decoder, archive writer
or reference authority may be added under a frozen path. Fixes that keep existing behaviour
and documentation metadata updates are not new features, but each still needs an approved
migration record to pass the gate. No replacement synthetic market observation may be
created under any of these paths.

## 5. What remains in Nirnay

The freeze removes no responsibility from Nirnay. The following stay as Nirnay-owned
components:

- OMS and execution: `authoritative-oms`, the Python OMS/durable outbox, `execution-policy`
  and broker adapter work, all paper-only with live routing disabled.
- Risk and account: `risk-authority`, `risk-engine`, `account-journal` and the OMS-projected
  account ledger and reconciliation.
- Cost and simulation: `cost-model` and `paper-engine` with their research simulators.
- Trading projection: `india-domain` remains as a trading projection that consumes
  Fieldbook stable IDs, effective intervals and calendar assertions. Its local identity
  allocation and self-contained provenance helper are migration items, not new authority.
- `india-reference` is frozen pending the NDC India reference projection. It is not a second
  raw archive, not an identity authority and not a source to extend.
- Generic acquisition, transport, integrity and governance infrastructure is unchanged;
  RRP-001 §6 rule 5 protects it.

## 6. Rights and acceptance state (unchanged)

| Item | State at this base |
|---|---|
| Source rights | `UNKNOWN` on every product and operation |
| Dataset acceptance | `BLOCKED` while rights evidence is absent |
| Experiment registration | none |
| Live execution | disabled; no order routing authorized |
| History rewrite | none; a per-row removal after replacement uses normal `git rm` |
| Production code deletion | none in this worktree |
| Commits | none created in this worktree |

## 7. Gate and self-test

- `python3 scripts/check_source_ingestion_freeze.py --self-test`: 11 adversarial cases passed
  (prefix boundaries, new untracked files, non-frozen changes, valid and invalid migration
  records, missing evidence and fail-closed manifest checks).
- `make freeze-check`: self-test plus gate passed with no frozen path changed against `main`.
- A probe file added under `crates/market-archive` was detected and the gate returned exit
  status 1; the probe was removed before this record was written.
- The gate inspects `git diff --name-only <base>` and untracked non-ignored files. Set
  `SOURCE_FREEZE_BASE` to a different ref when the review base differs from `main`.

## 8. Residual gates

- Acceptance in `REVIEW-LOG.md` and an exact reviewed commit are still required; no gate in
  this record is marked PASS by wording.
- Every future deletion needs a per-path approved migration record with the replacement
  commit ID, as required by Gate RRP-004.
- RRP-003, RRP-005, RRP-006 and RRP-007 remain open. No India design ratification, dataset
  acceptance or provider operation is implied by this freeze.
