# A1 — Read-only Architecture Audit

Repo: `siebensterne/fieldbook` fresh clone at `aee91de` (`main`). Scope: docs + `src/lib.rs`, `src/main.rs`, `src/acquisition/**`, `src/domain/**`, `src/distribution/coordinator.rs`, Cargo workspace. **No files in the audited repo were modified; no collection commands, provider/API calls, or credentials were used.** Findings below are from source, not from documentation claims; doc statements are labelled as such.

## 1. Verified implementation facts (code, with citations)

**Workspace / planes.** `Cargo.toml:1-4` declares a workspace whose only member is `xtask`; the product is one crate `fieldbook` (`Cargo.toml:6-12`). The seven "responsibilities" are **modules in one crate**, not separate crates: `src/lib.rs:6-13` declares `acquisition, distribution, domain, governance, integrity, market_state, storage`. Crate-root re-exports (`src/lib.rs:15-59`) preserve legacy paths. So plane boundaries are *conventional*, not compiler-enforced.

**Dependency direction (acquisition → domain is correct).** Adapters import the normalized model: `src/acquisition/exchange/mod.rs:22-25` (`model::MarketInfo`, `products::SourceCapabilities`, `session::SessionFactory`); codecs e.g. `src/acquisition/exchange/{okx,hyperliquid,kraken,binance}/codec.rs:12/10/12/12` import `crate::model`. Domain never imports acquisition: `src/domain/*.rs` import only `util, identity, taxonomy, quality, model, event_model` (`src/domain/mod.rs:4-13`). The core imports `exchange::ExchangeCapabilities` + `model::*` but **no websocket library** (`src/market_state/collector/state.rs:15-35`; grep confirms zero `tungstenite`/`tokio::net` there). WebSockets appear only in `src/acquisition/ws.rs` and three adapter sessions. This matches the charter's "adapters translate into the normalized layer; core has no venue branch."

**Composition/ownership.** `src/distribution/coordinator.rs:1-7,23-41` owns worker supervision, discovery, controls, shutdown; it imports all adapters (`:28-31`) and the collector (`:26`). `src/main.rs:24-222` dispatches commands; live collect calls `coordinator::run` (`:26`). One writer owns SBE output (coordinator doc `:3-4`).

**Transport controls are kept out of market events — verified in code.** `src/market_state/protocol.rs:76-105` defines typed `SessionSignal`/`TransportObservation`; `MarketData` is the only signal allowed to advance activity (`:83-87`, comment `:77-80`). `src/acquisition/session.rs:30-53` keeps `events: Vec<ExchangeEvent>` separate from `signal`, `mechanism`, `observations`, `raw`. Journal v2 is implemented: `JOURNAL_SCHEMA_VERSION = 2`, min readable v1 (`src/integrity/journal.rs:79-82`); three independent sequence domains (`observation_seq`, `receive_seq`, `control_seq`, `:967-969,996-1007`); `read_records()` filters to **inbound data only** (`:2657-2683`), `read_observations()` returns all (`:2710-2739`). Golden fixtures exist for both v1 and v2 (`fixtures/journal-v1-golden/golden/manifest.json`, `fixtures/journal-v2-golden/golden/manifest.json`). ADR-0002 is therefore a genuine implementation, not a plan.

**No Nirnay-like research/execution leakage.** Grep found no `backtest|strategy|alpha|portfolio|risk engine|order entry|signal generation` in `src/`. The only "order" send is a market-data subscription (`src/acquisition/exchange/kalshi/session.rs:316` `send_orderbook_subscribe`). "Simulator"/"simulation" occurrences are durability test infrastructure (`src/storage/application/postgres.rs:1638` `PostgresSimulator`, `src/storage/application.rs:19-20` simulated receipts) — reference-app fault injection, not trading/research simulation. `DomainPayload::{Trade,OrderEvent}` (`src/domain/event_model.rs:166-186`) is a representation of *reported* events for an L3 feed, not execution logic. The reference app is explicitly walled off from market data by `scripts/check_architecture.py:510-533` (forbidden tokens).

**Source / durable-record ownership.** Runtime source connections are owned by acquisition session workers (`src/acquisition/session.rs:1-3,30-53`), keyed at runtime by `native_symbol` (`src/market_state/collector/state.rs:314` `markets: BTreeMap<String, MarketState>`) and worker `SourceId {shard, replica}`. Authoritative durable records owned by Fieldbook: raw-frame journal (source of truth — `src/domain/canonical.rs:18-21`), SBE capture (single writer), catalog checkpoint (`src/storage/catalog.rs`), metadata/report sidecars (`src/storage/persistence.rs`, `report.rs`), and the separate P5/P6 durable commit store (`src/storage/application/durable.rs`, `durable_segmented.rs`) which is **not wired into the collector** (grep: no `use crate::application|analytics` in distribution/market_state/acquisition). Nirnay/consumers would consume these artifacts; nothing in code assigns execution/portfolio ownership to Fieldbook. `nirnay` survives only as a legacy env-var rename note (`docs/programme-status.md:202`).

**Implementation status by component.** Authoritative/live implementations: Kalshi, Polymarket, Hyperliquid H0-H2 (H3 loopback is test-only), dedicated Kalshi lifecycle plane, journal v2, coordinator, writer, catalog. Experiments/additive contracts: `domain/identity|canonical|products|event_model|reference_data`, `acquisition/{arbiter,codec,evidence,packet,stream_slice,supervisor,transport}`, `governance/*`, `integrity/{conformance,quality,coverage_certification}`; P5/P6 reference app and stress labs. Merely planned: Kraken/OKX/Binance/Coinbase H1-H3, general (non-prediction) output contract, ADR-0001 code migration (ADR itself Accepted but code explicitly deferred — `src/domain/identity.rs:1-6`, `docs/adr/0001...:627-641`).

## 2. Seven-responsibility map (actual components)

| Plane | Actual implementations | Status |
|---|---|---|
| acquisition | `acquisition/{session,ws,egress,transport,codec,arbiter,packet,evidence,supervisor,stream_slice,fixture_connectors}.rs`, `exchange/*` | live adapters + V2 additive engine |
| domain | `model, fixed, side, taxonomy, identity, reference_data, event_model, canonical, products, information` | model/fixed authoritative; rest additive contracts |
| market_state | `collector/state.rs`, `orderbook.rs`, `recovery.rs`, `histogram.rs`, `protocol.rs` | authoritative core; `protocol.rs` is transport vocabulary |
| storage | `writer, capacity, catalog, persistence, report, analytics, application/*` | authoritative output/checkpoints; `application/*` = separate reference app |
| integrity | `journal, replay, verify, completeness, coverage, latency, conformance, quality, time` | journal/replay authoritative; rest evidence tooling |
| governance | `config, adapter, modality, rights, coverage_certification` | config authoritative; rest declared policy |
| distribution | `coordinator, cli, metrics, probe, inspect` | authoritative composition/egress diagnostics |

## 3. Missing implementation

- ADR-0001 reference model has **no code**: `src/domain/identity.rs` types are additive (`:1-6`); runtime still keys by `native_symbol` (`state.rs:314`), `MarketInfo` still conflates identity fields (`src/domain/model.rs:585-594`), and `count_selected_markets` still dedupes by parent `exchange_market_id` (`src/storage/capacity.rs:191-202`) — the documented Kalshi undercount persists.
- `NormalizedStore` is test-only in-memory (`src/domain/canonical.rs:23-24`); no durable canonical store. Persistent identity, typed products, canonical events are not migrated into collector ownership (`docs/programme-status.md:435-443`).
- Kraken/OKX/Binance/Coinbase H1-H3; general market output contract; venue enumeration beyond `ExchangeKind`.

## 4. Defects / risks

1. **Stale documentation paths (high-confidence).** `docs/design-decisions.md:32-59,112,172,202,234,316-323,376-383,429-436` and ADR-0002 `:217-218` cite `src/exchange/mod.rs`, `src/collector/state.rs`, `src/model.rs`, `src/fixed.rs`, `src/journal.rs`, `src/protocol.rs`, `src/coordinator.rs` etc. — **all 14 verified missing** after the domain reorganisation. Verification must be done against code, not these paths.
2. **Cross-plane coupling is convention, not enforced.** Only `storage/application*` has an automated import guard (`check_architecture.py:510-533`). `src/acquisition/exchange/mod.rs:23` imports `capacity::count_selected_markets` from the **storage** plane; `src/acquisition/session.rs:15-23` imports `journal` (integrity) and `protocol`/`time` (market_state/integrity); `src/integrity/*` imports `governance` and `acquisition`. Direction is defensible but not structurally guaranteed.
3. **Transport vocabulary placement.** `src/market_state/protocol.rs:1` houses session/transport facts consumed by acquisition and integrity. Functionally fine, semantically odd for "market_state"; a strict reading of the charter's boundary 3/4 would place it in a transport or domain module.
4. **Authority wording risk.** "Authoritative data platform" currently rests on raw journal + SBE + catalog, not on the accepted universal identity/event model (`programme-status.md:429-443`). Cross-venue/dark-pool scenarios from ADR-0001 are not representable at runtime.
5. **Evidence not independently verifiable in this clone.** Live captures are Git-ignored (`programme-status.md:456-459`); stress evidence is synthetic loopback (`:466-473`); live PostgreSQL tests are opt-in (`:424-425`).

## 5. Existing tests and their limits

- Core/deterministic: `tests/replay_fixture.rs` (same core under fixture timestamps), `src/market_state/collector/state/tests.rs`, `src/integrity/journal/tests.rs` (incl. `:418` v2 interleaving, `:529` epoch resets).
- Boundaries: `tests/e2e_mock_exchange.rs:1821` (exact payload accounting) and `:2099-2168` (controls do not advance `receive_seq` / force reconnect) — directly protects the ADR-0002 invariant. Also `tests/kalshi_lifecycle_plane.rs`, `tests/egress_lab.rs`, `tests/session_recovery.rs`, `tests/hyperliquid_{books,telemetry}.rs`, `tests/source_conformance.rs`, `tests/coverage_certificate.rs`.
- P5/P6 reference app: `tests/durable_pipeline.rs`, `network_source.rs`, `bounded_batch.rs`, `segmented_*.rs`, `jsonl_postgres_reference.rs`; live DB tests ignored/operator-supplied (`FIELDBOOK_TEST_PG_URL`), loopback TCP only.
- Limits: no live venue certification, no production PostgreSQL in CI, no replication/performance certificate; capacity generators and live tiers are `#[ignore]`; no test exercises the *absence* of Nirnay logic (it is absent by code inspection).

## 6. Required changes

1. Update doc paths to current module locations (or publish a legacy→current path map) so evidence can be located; do not treat doc-only statements as verification.
2. Extend `check_architecture.py` with plane-direction checks; relocate `count_selected_markets` out of `storage` (into domain or adapter-local) so acquisition does not depend on storage.
3. Before claiming authoritative cross-venue reference data, implement ADR-0001 migration: wire identity types into collector, fix the `exchange_market_id` counting key, rename runtime `SourceId` to worker identity.
4. Decide and document the home of `protocol.rs`; keep it out of `market_state` if strict layering is required.
5. Keep P5/P6 reference app and stress labs explicitly non-authoritative and unwired.

## 7. Decision

**CORRECT.** Ownership boundaries and dependency directions are fundamentally right for a data platform with a downstream consumer: acquisition→domain is one-way, the core is websocket-free, transport controls are provably separated from market events (ADR-0002 implemented and test-protected), and no research/execution/portfolio/risk logic leaks in. Corrections are required before the system can be called an authoritative *cross-venue* platform: stale documentation paths, unenforced plane coupling, and the unimplemented ADR-0001 identity model (DEFER to a migration PR). The raw journal/SBE/catalog are correctly Fieldbook-owned durable records; the durability "simulators" are reference-app test infrastructure, not owned experiments.
