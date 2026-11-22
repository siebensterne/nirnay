# ADR-0001 — Data ownership between Nirnay and the future complementary platform

- **Status:** Proposed design decision. No implementation authority, no rights
  grant, no dataset acceptance and no experiment authorization.
- **Date:** 2026-11-22
- **Base:** `53bd0af` on branch `docs/source-qualification` in this
  documentation worktree.
- **Scope:** ownership of source data and its lifecycle; the versioned
  interface between two systems. This ADR does not select transport, runtime
  or storage technology.
- **Governing inputs:**
  `engineering/research/procurement/source-qualification-upstox-nse-2026-11-22.md`;
  `engineering/research/procurement/m43-source-qualification-dossier.md`;
  `evidence/OWNER-DIRECTIVE-2026-11-22.md`;
  `engineering/research/dataset-acceptance-contract.md`;
  `engineering/research/point-in-time-data.md`;
  `engineering/decisions/ADR-0004-live-disabled.md`.

## 1. Context

Phase 1 qualification ends with rights `UNKNOWN`, both candidate datasets
`BLOCKED` and collection `BLOCKED`. No written permission, point-in-time
universe, correction lineage or retention term is held. The owner directive
withholds every provider operation until written permission and owner approval
cover the exact operation, scope, instruments, fields, date range and retention
period. `dataset-acceptance-contract.md` is blocked on the same evidence.

Nirnay is a research and simulation system. A second collector inside Nirnay
would duplicate acquisition, create a competing archive, split provenance and
let two systems disagree on identity, revisions and retention. Ownership must
be settled before integration code is written, and before any collector exists.

This ADR records a future-state decision. It does not assert that the
complementary platform exists, that its repository has been reviewed, or that
any interface field below is implemented. All owner names are role names; the
owner must ratify the natural persons.

## 2. Decision

The **future complementary platform, not Nirnay**, owns source acquisition,
raw storage, normalization, point-in-time reference data and the raw-data
lifecycle. Nirnay owns quantitative research, experiments, simulation,
portfolio, risk and eventual execution under separate authorization. Nirnay
must not run a second collector or a competing archive.

### 2.1 Ownership register

- **Source acquisition owner:** future complementary platform. Nirnay does
  not acquire source data. Scope: provider requests, collection scheduling
  and entitlement checks.
- **Raw storage owner:** future complementary platform. Nirnay holds no
  competing raw archive. Scope: raw bytes, digests and immutable references.
- **Normalization owner:** future complementary platform. Nirnay consumes
  typed events and does not re-normalize raw payloads.
- **Point-in-time reference owner:** future complementary platform. Scope:
  instrument identity, effective-dated universe, corporate-action revisions,
  calendar and session reference data.
- **Raw-data lifecycle owner:** future complementary platform. Scope:
  retention, correction handling, supersession, archive closure and deletion.
- **Research and experiment owner:** Nirnay. Scope: derived research datasets,
  hypotheses, experiment records and results.
- **Simulation, portfolio and risk owner:** Nirnay, subject to ADR-0004.
  Scope: simulation, portfolio accounting, risk evaluation and eventual
  execution under a separate authorization.
- **Versioned interface owner:** **Nirnay Data Contract (NDC) v1** is the
  proposed interface name, published and versioned by the complementary
  platform and consumed by Nirnay. Nirnay does not author upstream data or a
  competing interface.

NDC v1 is a proposed name, not an existing artifact. Its schema is not written
by this ADR. A schema change requires a new contract version, a migration
record and owner approval. The complementary platform publishes an immutable
dataset release; Nirnay declares the consumed version on each experiment
record and does not mutate upstream bytes.

### 2.2 Contract fields required on every release

The fields below are required in NDC v1. They describe data handed over, not a
claim about entitlement or truth.

| # and field | Required meaning |
|---|---|
| 1 Source, venue, segment, product | Named identity per payload; no venue inferred from a ticker |
| 2 Provider and canonical ID | Provider key plus canonical ID, valid and known intervals |
| 3 Event type and typed payload | Typed product payload per event; no generic instrument or event |
| 4 Source, receive, clock | Source field and domain, receive time, clock labels |
| 5 Sequence, continuity, gaps | Provider sequence where supplied, local sequence, gap status |
| 6 Schema and normalizer versions | Contract schema version and normalizer version per record |
| 7 Raw digest and archive reference | Digest of raw bytes and immutable archive object reference |
| 8 Rights scope and retention | Document reference, permitted operations, scope and retention |
| 9 Availability and record time | Release availability and reference knowledge time |
| 10 Corrections and as-of lineage | Supersession chain, revision knowledge time and as-of basis |

## 3. Binding constraints and invariants

### 3.1 Precision, loss and scale

- Source timestamp resolution is preserved. Storing a millisecond source
  timestamp as an integer nanosecond value is a unit change, not extra
  precision; the release states the original field, unit and domain.
- "Zero-loss" is stated only against a named fault model: process crash,
  restart, truncated frame, provider reconnect and upstream gap each need a
  stated expectation. Upstream discontinuities are reconciled explicitly; a
  local gap record does not assert that the provider sent every event.
- Partitioning and scale changes come from measured workload: observed
  volume, burst rate, replay duration and recovery objective. No partition
  scheme or scaling mechanism is chosen from intuition or a provider ceiling.
- Daily OHLCV, five-level displayed quotes and order-by-order tape are
  separate capability tiers. A tier carries only the event types its source
  documents. The tape tier is not assumed available from the documented
  Upstox public feed; a five-level quote snapshot is not that tier.

### 3.2 Build gates and forbidden substitutions

- **No technology choice here.** Kafka, gRPC, NATS, shared Rust crates and a
  database are not chosen by this ADR. Transport, RPC, shared code and storage
  selection require a measured requirement and a separate decision.
- **Review before integration.** The companion repository's actual
  architecture is read and recorded before integration code is written.
  Documentation, an owner's verbal summary or an assumed topology is not a
  substitute for that review.
- **No collector before rights.** No collector, recorder, scraper or archive
  is built until written rights and owner approval exist for the exact
  operation, scope, instruments, fields, date range and retention period.
- **No forbidden acquisition behaviour.** No HTML scraping, no ban evasion,
  no rate-limit circumvention and no undocumented endpoints. Public terms and
  documented limits bind any future acquisition work.
- **No second data platform inside Nirnay.** No competing archive, duplicate
  collector or parallel normalization stack may be created in this repository.
  Research datasets are derived outputs with declared lineage.

## 4. Consequences

Positive consequences: one owner per raw-data fact; one archive; typed
payloads; explicit rights and retention metadata; reproducibility through
versioned contract references; a clear stop condition when rights are missing.

Negative consequences: Nirnay depends on an external platform that is not yet
ratified; a contract version change costs a migration; research cannot start
from source data until the platform, rights and interface all exist. The
dependency is accepted in preference to an unauthorized duplicate collector.

## 5. Failure cases

1. Nirnay adds a recorder because a date range looks convenient. Result: two
   archives with different digests and no single acquisition owner. Response:
   §2.1 non-owner roles and §3.2 build gate.
2. A release stores millisecond timestamps in nanosecond integers and claims
   higher precision. Response: §3.1 resolution rule and contract field 4.
3. A feed reports `UNKNOWN` retention and a researcher treats that as
   permission. Response: contract field 8 and the acceptance contract.
4. Depth snapshots are used as an order-by-order tape. Response: §3.1 tier
   separation; no aggressor or queue facts from displayed levels.
5. A transport is chosen from the rate-limit page. Response: §3.2 technology
   gate; limits constrain workload, not architecture choice.
6. Integration code is written against an assumed companion architecture.
   Response: §3.2 review gate; work stops until the actual repository is read.
7. A survivorship comparison uses present-day keys over long history.
   Response: contract fields 2, 9 and 10; point-in-time reference data stays
   with the platform, and the acceptance contract decides admissibility.

## 6. Rejection conditions

This decision is revised if the owner assigns source acquisition, raw storage,
normalization, point-in-time reference data or the raw-data lifecycle to
Nirnay; if no complementary platform is ratified; if a measured requirement
shows that one of the named owners cannot hold its state; or if written rights
for the intended operations are refused. A technology choice or a schema
proposal does not revise this ownership decision.

## 7. Owner dependencies

1. Ratify the named owner roles in §2.1 and the natural persons behind them.
2. Confirm that the future complementary platform is the intended acquisition
   and raw-data owner before any integration design starts.
3. Supply permission to review the companion repository's actual architecture.
4. Supply written rights and approval before any collector is built.
5. Approve NDC v1 field definitions and any subsequent version before schema
   work.
6. Supply measured workload evidence before any partitioning or scale
   decision.

Until item 4 is satisfied, no provider operation, collector, ingest or
experiment is authorized. This ADR changes no right, no capability record and
no claim record. No commit, push or merge is made for this decision.
