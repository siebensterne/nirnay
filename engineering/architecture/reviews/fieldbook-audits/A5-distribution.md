# A5 — Distribution / consumer interface (read-only audit)

Repo: `/private/var/folders/7c/m_f7hs9n445174c0x3gsy5kw0000gn/T/opencode/fieldbook-review` at `aee91de885ae81c46e43238d7f19c6ee57702ea2`. No files modified, no commands that collect, no network/provider/credential access. Semantics only; no transport/storage proposals.

## Owner's question

**Can one immutable dataset release be consumed by Nirnay without copying or mutating the raw source? — Mechanically yes, contractually not yet.**

The raw capture (`captures/<run>/...`, gitignored: `.gitignore:3`) is opened read-only. `verify_capture` recomputes every hash (`src/storage/analytics.rs:53-66,110-135`) and refuses path escape (`:84-100`); it only reads (`evidence.json`, journal manifest/segments, SBE, markets, report — `:111-198`). `export_capture` verifies first, then refuses an existing staging dir and writes **only** to the caller's staging path (`src/storage/analytics/export.rs:1-5,652-656`). `export_parquet.py:7` states "Nothing is written into captures/". The published release is a **derived** artifact (NDJSON + Parquet + manifest) in `exports/<export_id>` (`scripts/analytics/export_parquet.py:299-370`); a consumer reads that directory and never needs the raw capture. So the raw source is neither mutated nor copied into the release.

What is *not* ready: there is no release registry/index, no consumer-facing reference contract, no license/rights proof bound to the release, no corrections/as-of semantics, and the independent verifier rejects normal verified exports (defect D1). A Nirnay consumer today must know the `export_id` directory path out of band; `analytics/q/common.q:2-6` takes the export directory as `.z.x 0` and opens `datasets/<name>.parquet`.

## Verified implementation facts (file:line)

**Raw-source identity (immutable).** `capture_id = SHA256(domain "fieldbook.capture.v1" ‖ length-prefixed journal manifest sha ‖ ordered segment shas ‖ orderbook.sbe ‖ markets.json ‖ report.json sha)` — `src/storage/analytics.rs:28,200-229`; matches `docs/analytics-data-contract.md:9-22`. No paths, random values or wall clock. `EXPORT_SCHEMA_VERSION = 1` (`src/storage/analytics.rs:27`).

**Published artifact / version / digest.** Seven typed NDJSON datasets (`src/storage/analytics/export.rs:734-757`) plus `export-manifest.json` with input digests, logical-content digests, row counts and verification results (`:788-802`). `export_id` = hash of domain `"fieldbook.export.v1"`, `capture_id`, schema version and logical digests (`:32,759-766`). Parquet stage adds `format=fieldbook.analytics.export`, `dataset_schema_version=1`, `pyarrow_version`, `parquet_options`, per-dataset `dataset_schemas`, `logical_content_sha256`, `readback_sha256`, `parquet_sha256`, and a `manifest.sha256` sidecar (`export_parquet.py:23,332-363`). Explicit Arrow schemas and bounded writes with count/schema/readback equality (`:49-197,248-296`).

**Consumer reference.** Release directory `exports/<export_id>`; `export_id` is the release key, files addressed relatively as `datasets/<name>.parquet` and `<name>.ndjson` (`export_parquet.py:344-353`; `analytics/q/common.q:2-6`).

**Atomic publication boundary.** All datasets are written and reconciled inside staging; manifest and digest sidecar are written there; then a single `os.replace(staging, exports/<export_id>)` atomically publishes, and `destination.exists()` refuses overwrite (`export_parquet.py:299-370`). Before that, Rust `write_json_atomic` gives sibling-temp+fsync+rename for the catalog checkpoint (`src/storage/catalog.rs:100-103`), which is a separate live-checkpoint boundary, not the release boundary.

**Non-mutation evidence.** `contained_path` rejects absolute/parent paths (`src/storage/analytics.rs:84-100`); journal bytes are re-verified by the visitor (`src/integrity/journal.rs`, see `docs/requirement-test-matrix.md:74`); `main.rs:176-187` only emits the summary and checks verification results.

**Independent certification.** `verify_export.py:42-127` checks manifest digest, upstream checks, provenance label, row counts, artifact hashes, logical digests, readback equality and Arrow schemas; absent/failed inputs never become PASS (`:49-55,113-119`).

## Missing implementation

1. No published release index/manifest-of-releases, no stable consumer alias, no documented Nirnay reference procedure; `export_id` is only discoverable from the producer's stdout/manifest.
2. No rights/retention/territory/recipient proof bound to a release. `src/governance/rights.rs:1-11,334-448` is a fail-closed registry but is never consulted by `verify_capture`, `export_capture` or `publish`.
3. No canonical NDC envelope in the release. `CanonicalEvent`/`CanonicalKind` (`src/domain/canonical.rs:315-328`), `UniversalEnvelope` with `normalizer_version`, `ObservedTimestamp`, `Provenance` (`src/domain/event_model.rs:71-95,142-151,252-266`) and `InstrumentId`/`VenueId`/`FeedId` (`src/domain/identity.rs:87-106,125-137`) are additive code, not exported columns. Fieldbook ADR-0001 is "Accepted design, not implemented" (`docs/adr/0001-instrument-and-venue-reference-model.md:8-9`; `docs/hyperliquid-integration-plan.md:326`).
4. No corrections / revision lineage / as-of columns; metadata is explicitly "a capture-scoped metadata snapshot, not a historical reference-data database" (`docs/analytics-data-contract.md:142-152`).
5. No gap column in Parquet; no availability/SLO field; no archive URI in the manifest.
6. Postgres interface is a separate generic reference application (`docs/jsonl-postgres-reference.md:3-39,148-169`), not a distribution path for this dataset; still lacks pooling, replication, failover and throughput claims.
7. Release provenance drift: docs say `export_id` includes exporter commit, exporter binary digest, PyArrow version and config hash (`docs/analytics-data-contract.md:24-27`); the implementation includes none of those (`export.rs:759-766`).

## NDC v1 semantic group mapping (semantics only)

| Group | Status | Evidence in the release |
|---|---|---|
| Source/venue/product identity | **Partial** | `source`, `shard`, `replica`, `clock_source`, `venue` (report.exchange), per-instrument `exchange`; no provider/feed/product identity — `SourceId` is a worker identity (`docs/adr/0001…:119-123,432-442`). |
| Provider + canonical instrument id | **Absent** | Only `serialization_symbol`/`native_symbol` with `resolved`/`ambiguous`/`unresolved` (`export.rs:338-358,387-399`); canonical id types exist but are not emitted (`identity.rs:87-106`). |
| Event type + typed payload | **Partial** | `record_kind`, `frame_kind`, `control_opcode`, `signal`, `message_kind` (`export.rs:54-59,283-328,394-409,452-472`); payload bytes stay in journal with `payload_sha256` (`:63,318-321`); typed taxonomy not in release (`canonical.rs:315-328`). |
| Source/receive time + clock domain | **Present** | `received_at_us`, `clock_source`, `date`, `observation_seq` (`export.rs:48-61,304-319`); `exchange_timestamp_us` (`sbe_message`). Resolution/accuracy/sync absent from release (`event_model.rs:71-95`). |
| Sequence/continuity/gaps | **Partial** | `observation_seq`, `receive_seq`, `control_seq` (`:49-51`); completeness/coverage upstream checks (`:776-777`); no exported gap rows; `StreamGap` type unused in release (`canonical.rs:280-305`). |
| Schema + normalizer versions | **Partial** | `journal_schema_version`, SBE `schema_id`/`schema_version`, `export_schema_version`, `dataset_schema_version`; `normalizer_version` not in release (`event_model.rs:262`). |
| Raw digest + archive reference | **Partial** | `input_sha256` covers SBE/markets/report + journal manifest; `payload_sha256` and segment shas feed `capture_id` (`analytics.rs:200-205`); segment shas and journal directory are **not** in the published manifest; no archive reference. |
| Rights + retention | **Absent** | No right fields or authorization proof in manifest; registry unwired (`rights.rs:334-448`). |
| Availability + record time | **Partial** | `received_at_us`/`date` present; `capture_run` carries verification statuses (`export.rs:723-728`) but no availability measure or SLO. |
| Corrections/revision lineage/as-of | **Absent** | Capture-scoped snapshot only (`docs/analytics-data-contract.md:142-152`); no `valid_from/as-of/revision` in datasets; ADR-0001 as-of migration not implemented (`:606-620`). |

## Defects / risks

- **D1 (high): verifier rejects verified captures.** `verify_export.py:71-73` sets `journal_provenance_label = PASS` only when the manifest status equals `"legacy_incomplete"`. A normal capture has `"verified"` (`src/storage/analytics.rs:159-163`) and therefore scores FAIL, so `overall` can never be PASS (`:113-119`). The unit test hardcodes `legacy_incomplete` (`test_verify_export.py:72,94`), masking this.
- **D2 (medium): mislabeled identity.** `journal_observation.run_id` is filled with `journal_code_commit` (`export.rs:676,285`), not the journal run id; the correct value is available and used in `capture_run.journal_run_id` (`:710`).
- **D3 (medium): export_id identity divergence** from the documented derivation (docs `:24-27` vs `export.rs:759-766`). Re-exports with a different PyArrow/compression/toolchain reuse the same `export_id`, collide on `destination.exists()` (`export_parquet.py:367-368`), and are not distinguishable by release key.
- **D4 (medium): unverified claims in `capture_run`.** `journal_schema_version: 2`, `sbe_schema_id: 1`, `sbe_schema_version: 2` are hardcoded (`export.rs:711,715-716`); `anchored_symbols` is set to the metadata instrument count (`:719`) rather than a computed anchor; `export_verification_status` stays `"pending"` even when `unknown = 0` (`:723-728`).
- **D5 (low): publish TOCTOU.** `destination.exists()` then `os.replace` (`export_parquet.py:367-369`) could overwrite a concurrently created release directory; no release lock/cas.
- **D6 (medium): no consumer-side trust chain.** No release index, no signed/immutable alias, no rights proof; a consumer cannot prove it consumed the latest intended release or that redistribution is licensed.
- **D7 (context): readiness gates open.** 24h soak, security and deployment reviews and "CI on exact commit" are unexecuted/pending (`docs/operating-envelope.md:132-155`); `global-data-architecture.md:3-6` calls V2 a design target.

## Existing tests and their limits

- `src/storage/analytics/tests.rs:13-41,43-51,53-93,95-103`: capture verification, deterministic `capture_id`, tamper rejection, traversal rejection — **all four retained-capture tests are `#[ignore]`** and require operator-local captures; only the traversal test runs by default. No test asserts the published-manifest content for a real verified capture.
- `src/storage/analytics/export/tests.rs:10-30,32-44,46-51` run offline; the end-to-end `all_captures_export_to_staging` is `#[ignore]` (`:59-61`) and its fixture list is hardcoded. No test checks `run_id` (D2) or derived schema fields (D4).
- `scripts/analytics/test_export_parquet.py:28-74`: schemas, binary digests, unknown-field rejection, single-row round trip. No test of atomic publication, collision, toolchain variance or multi-dataset release.
- `scripts/analytics/test_verify_export.py:54-95,97-106`: models a **legacy** export only; no `verified` status case, so D1 is untested.
- `scripts/analytics/test_verify_hdb.py`, `test_verify_q_queries.py` (matrix `docs/requirement-test-matrix.md:79-80`): offline KDB-X reconciliation; they verify query results, not distribution semantics, rights or as-of.

## Required changes

1. Fix D1: accept all declared provenance labels (or invert the check to flag legacy explicitly) and add a `verified`-status certificate test.
2. Fix D2/D4: use `stream.run_id` for `journal_observation.run_id`; derive schema versions from artifacts/status; emit an accurate `anchored_symbols` and a truthful export certification status.
3. Reconcile `export_id` with the documented derivation or correct the doc; include toolchain/version inputs so a release key identifies the actual published artifact; make publication compare-and-set.
4. Add a release-level contract: versioned manifest-of-releases with `capture_id`, `export_id`, dataset digests, rights/retention proof and a consumer reference procedure; include journal segment digests and an archive reference in the manifest.
5. Only after Fieldbook ADR-0001 implementation: expose canonical instrument/provider/venue ids, event envelope fields, gaps, rights, corrections and as-of in exported datasets (currently **DEFER** — design not implemented).
6. Add non-ignored, self-contained fixture tests for export → publish → independent verify, including collision and toolchain-change cases.

## Decision

**CORRECT.** The immutability/non-mutation core and atomic single-directory publication are sound and fit for a derived, read-only release; the owner's "no raw-source copy/mutation" requirement is already satisfied. However D1 prevents verified releases from certifying, and the consumer contract, provenance identity and NDC semantics are incomplete. Accept the non-mutation and atomic-publish mechanisms; require 1–4 and 6 before exposing a release to Nirnay, and formally **DEFER** item 5 until the accepted ADR-0001 migration exists. Do not claim ACCEPT of NDC v1 conformance at this commit.
