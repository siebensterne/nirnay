# Fieldbook architecture and implementation review — 2026-11-22

**Status:** read-only review report. Not an acceptance certificate, not a durability
certification, not an authorisation to collect, and not an API contract.

## 0. Question, scope and method

Question: can `siebensterne/fieldbook` be the authoritative data platform for Nirnay, with
Nirnay as a quantitative consumer, without duplicating domain models or building a second
collector?

Method: a fresh clone of public `main` at commit
`aee91de885ae81c46e43238d7f19c6ee57702ea2` was inspected statically. Five audits were run
against source, not documentation claims: A1 architecture, A2 durability, A3 data semantics,
A4 governance, A5 distribution. Full reports are in `fieldbook-audits/`.

Boundaries observed:

- No Fieldbook file was modified; the separate dirty working checkout at
  `/Users/tourist/Documents/fieldbook` was not touched.
- No collector command was run, no provider or API was contacted, no credential was read.
- Fieldbook CI and retained live captures were **not** independently reproduced: live
  captures are Git-ignored and absent from the source export; stress evidence is synthetic
  loopback; live PostgreSQL checks are opt-in.
- This is a source audit at one commit, not a line-by-line certification of all 433 tracked
  files or of operator-held evidence.

## 1. Ownership map

| Responsibility | Owner | Evidence at `aee91de` |
|---|---|---|
| Source acquisition | Fieldbook | `src/acquisition/**` session workers; live websocket adapters for Kalshi, Polymarket, Hyperliquid, Coinbase, Kraken; OKX/Binance offline discovery only (`src/governance/config.rs:19-32`) |
| Raw archive and lifecycle | Fieldbook | Journal v2 with `receive_seq`/`control_seq`/`observation_seq`, checksums, segments, finalize, replay and verify (`src/integrity/journal.rs:79-82,967-1007,2657-2739`) |
| Normalization | Fieldbook | Live path produces `ExchangeEvent` four variants and two SBE message types (`src/domain/model.rs:845-878`); typed V2 normalization exists only in offline conformance fixtures |
| Point-in-time reference data | Fieldbook (declared) | `ReferenceCatalog`/`ReferenceGraph` exist as Rust types (`src/domain/reference_data.rs:145-359`) but are not wired to live data or to exports |
| Corrections, retention, releases | Fieldbook | Derived exports with content digests and atomic publication (`src/storage/analytics/export.rs`, `scripts/analytics/export_parquet.py:299-370`); no rights proof, correction lineage or release index |
| Research, experiments | Nirnay | No experiment, registry or holdout logic exists in Fieldbook source (A1 grep) |
| Simulation, portfolio, risk | Nirnay | No strategy, portfolio or pre-trade risk logic in Fieldbook source (A1 grep); catalogue is explicitly walled off by `scripts/check_architecture.py:510-533` |
| Execution | Nirnay, separate approval | Fieldbook places no orders and holds no positions; the only `send_orderbook_subscribe` is a market-data subscription |

The split in Nirnay ADR-0001 is therefore supported by the observed code, with one
qualification: Fieldbook's *cross-venue* and *point-in-time* authority is declared, not yet
implemented at this commit. Do not merge the repositories, duplicate the domain model, or
introduce a shared crate before the NDC contract and measured dependency needs justify it.

## 2. Seven review questions — answers

**1. What does Fieldbook already implement?**
A working single-crate collector with seven modules (`src/lib.rs:6-13`), a websocket-free
normalised core, a coordinator, a checksummed raw-frame journal v2, SBE output, replay,
verification, and a derived Parquet/NDJSON export path. Live adapters exist for five venues;
two more are discovery-only. A substantial V2 type library (taxonomy, opaque identity,
PIT reference graph, typed payloads, bitemporal information, exact numeric scales, clock
roles, continuity) exists but is **not** on the live path.

**2. Which system owns each source and each durable record?**
Runtime connections are owned by Fieldbook acquisition session workers. Durable records are
owned by Fieldbook: raw journal (source of truth), SBE capture (single writer), catalog
checkpoint and sidecars. The P5/P6 durable commit store is a separate reference application
and is not wired into the collector. Nirnay owns no source and has no competing archive.

**3. How are raw records committed, indexed, recovered and deleted?**
A journal record is committed only after flush, `sync_all`, footer/checksum, file sync,
`fs::rename` of `.pmj.partial`, and directory sync (`journal.rs:1409-1455`). The manifest is
published separately afterwards — the rename and manifest publication are two durable
operations with no spanning transaction. Recovery is replay plus verification against the
manifest; partial segments and orphan files are detected. Index publication uses atomic
single-file sidecars. Retention, deletion and cross-failure-domain restore are **not**
implemented.

**4. What are the actual consistency and durability guarantees?**
Local, single-host, finalized-run durability with checksum corruption detection and
configurable fsync (`sync_mode=always|interval`). There is no unconditional zero-loss claim:
Fieldbook's own durability contract says so, and the 24-hour zero-loss soak is recorded as
not executed. A2 found correct local mechanisms with five defects to fix and one deferred
item (cross-domain backup/restore). Acknowledgeable `Synced` levels are not the same as a
finalized, manifest-published run.

**5. How are corrections and historical reference data represented?**
In types, not in live data or in releases. Bitemporal `ReferenceCatalog`/`ReferenceGraph`,
`UniversalEnvelope`, revision fields and clock roles exist as Rust code, but usage search
shows them confined to their own tests and offline conformance paths. Export metadata is
explicitly "a capture-scoped metadata snapshot, not a historical reference-data database"
(`docs/analytics-data-contract.md:142-152`). There are no correction or as-of columns in a
release.

**6. Can one immutable dataset release be consumed without copying or mutating raw source?**
Mechanically yes; contractually not yet. `verify_capture` re-hashes inputs and refuses path
escape (`src/storage/analytics.rs:84-135`); export writes only to staging and publishes to
`exports/<export_id>` with one `os.replace` (`export_parquet.py:299-370`); the release is a
derived artifact, so the raw capture is neither mutated nor copied into it. Missing: a
release index, a consumer reference procedure, a rights/retention proof bound to the
release, canonical identity and as-of fields, and a working independent verifier for normal
verified exports (A5 defect D1).

**7. Which NDC requirements already fit, and which need changes?**
See section 4. Receive time and clock domain are present; identity, typed payload,
sequence/gaps, schema versions, raw digests and availability are partial; canonical
instrument identity, rights/retention and corrections/as-of are absent. The export's
`export_id` also omits exporter commit/binary, PyArrow version and config hash despite the
Fieldbook contract document saying otherwise.

## 3. Audit outcomes

| Audit | Decision | Headline |
|---|---|---|
| A1 Architecture | **CORRECT** | Boundaries and dependency direction are sound (acquisition→domain one-way, websocket-free core, transport controls separated per ADR-0002, no research/execution leakage). Fix stale documentation paths, unenforced plane coupling, and the unimplemented ADR-0001 identity migration (DEFER). |
| A2 Durability | **CORRECT** (local) / **DEFER** (cross-domain) | Local finalize/fync/rename chain is real. Fix recovered-open-segment naming, ambiguous segmented acknowledgement, `SyncPolicy::None` acknowledgement, metadata/catalog atomicity and failure propagation. No zero-loss claim. Backup/restore across failure domains is out of scope, not a defect. |
| A3 Data semantics | **CORRECT** / venue onboarding **DEFER** | V2 types are a genuine but disconnected library; live path is aggregated L2 crypto/prediction-market order books only. No NSE, BSE, cash equity, ITCH, MBO, trades, BBO or corporate actions. `DomainPayload` forces missing values to zero; scale/unit are inconsistent; `resolve_venue` ignores observation time. |
| A4 Governance | **CORRECT** — no deferral for live paths | Rights and certification types are fail-closed in isolation but dead code with respect to collection and publication. Default CLI command is live `Collect`; README contains a runnable live recipe; env-based permission gates are self-attested and bypassable by the binary. No venue dimension in the rights query. |
| A5 Distribution | **CORRECT** — do not accept NDC conformance | Non-mutation and atomic publication are sound. Fix verifier D1, `run_id` binding, `export_id` provenance, and the release contract before exposing a release to Nirnay. |

## 4. NDC v1 compatibility (semantic requirements only)

The ten field groups in Nirnay ADR-0001 are approved as **semantic requirements**, not a
frozen schema. No transport or storage technology is chosen by this review.

Structural recommendation adopted: do not place all ten groups at the same level.

- **Release manifest** — dataset identity and source scope; coverage/completeness; rights
  and retention; schema versions; release digest; publication time. This is the immutable
  object an experiment references.
- **Event envelope** — source and instrument identity; event type and typed payload; source
  and receive times; sequence and continuity; raw digest and lineage reference; revision and
  as-of information. Dataset-wide facts are not repeated per event.

| Semantic group | Fieldbook at `aee91de` |
|---|---|
| Source / venue / product identity | Partial — `source`, `shard`, `replica`, `clock_source`, venue label; no provider/feed/product identity or canonical venue id |
| Provider + canonical instrument id | Absent in releases — only `native_symbol`/`serialization_symbol`; canonical id types exist but are not emitted |
| Event type + typed payload | Partial — `record_kind`, `frame_kind`, `control_opcode`, `signal`, `message_kind`; payload bytes stay in the journal; typed taxonomy not exported |
| Source/receive time + clock domain | Present — `received_at_us`, `clock_source`, `observation_seq`; resolution and synchronisation accuracy absent |
| Sequence, continuity, gaps | Partial — three sequences exported; no gap rows or availability field |
| Schema + normalizer versions | Partial — export schema version present; normalizer/version identity incomplete and `export_id` provenance drifts from its documented definition |
| Raw digest + archive reference | Partial — capture and content digests present; no archive URI or release index |
| Rights scope + retention | Absent from releases; fail-closed registry not consulted by export |
| Availability time + record time | Partial — receive time present; availability and record-time roles not mapped in the release |
| Corrections, revision lineage, as-of | Absent |

Before NDC v1 is frozen: define provider event id, local record id and deduplication;
timestamp resolution, clock identity and receive semantics; source/local sequence with
partition scope and continuity; typed product schemas with units, scales and
missing-value semantics (null is not zero); declared coverage, observed counts, gaps,
rejected records and corrections; raw digest and immutable archive reference; operation
scope, document reference and retention period; availability/record/correction-knowledge
times; immutable release identity, manifest and atomic publication boundary; and
compatibility, migration and consumer-rejection rules. These are semantic definitions for
N2, not code to be written now.

## 5. Required changes before integration

1. **Governance must fail closed (A4, blocking).** Wire registry and certification
   into collection, discovery, latency probes and publication; remove the live default
   command and runnable live recipe; add a venue dimension to rights identity.
2. **Fix the distribution verifier and release identity (A5, blocking).** A normal
   `verified` export must be certifiable; bind `run_id` to the journal run; include exporter
   commit/binary, PyArrow version and config hash in `export_id`; publish a release index and
   a consumer reference contract.
3. **Repair durability defects and state the fault model (A2, blocking for reliance).** Do not
   claim cross-domain backup or unconditional zero loss.
4. **Wire identity and typed semantics or keep the claim narrow (A1/A3).** Until ADR-0001's
   identity migration and live typed normalization exist, Fieldbook is a venue-specific
   L2 collector with a strong type library — not an authoritative cross-venue reference
   platform.
5. **Correct stale documentation paths** so evidence can be located by code citation.

## 6. Decision and gates

- The ownership model is **sound**: Fieldbook is the candidate data platform, Nirnay is the
  quantitative platform. Do not merge repositories, duplicate domain models, or add a shared
  crate now.
- Nirnay ADR-0001 remains **Proposed design decision**. It is not Accepted: companion
  architecture review is now complete, but owner ratification of roles has not occurred.
- This review does **not** move any stage past N1. Next gates: N2 NDC semantic review,
  N3 owner ratification, N4 integration design, N5 written rights, N6 dataset acceptance,
  N7 research resumption. N5 precedes any collection; N6 precedes strategy registration.
  No stage authorises live execution.
- Freeze unchanged: INR 0 incremental data budget, 21 rights `UNKNOWN`, dataset acceptance
  `BLOCKED`, M4 holdout untouched, no experiment registered, no provider contact, no new
  collector, database, queue, transport or schema implementation.
