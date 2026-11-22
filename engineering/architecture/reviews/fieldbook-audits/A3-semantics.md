# A3 — Data Semantics Audit (READ-ONLY)

Repo: `fieldbook-review` @ `aee91de885ae81c46e43238d7f19c6ee57702ea2`. No files changed, no
commands run against the collector, no provider/API contact. Maturity is assessed from
wiring and behaviour, never from file counts.

## Answer to the owner's key question

**The V2 domain model is a substantial, well-formed *type library*, but the implementation is
still centred on crypto/prediction-market aggregated L2 order-book feeds. It does not support
NSE/BSE cash equities or any other cash-equity venue today.** The V2 types are additive
side-car modules; the live collector neither constructs them nor routes its data through them.
The docs are honest about this (`docs/global-data-architecture.md:3-6,118-123`;
`docs/specifications/event-model.md:85-89`; `docs/specifications/data-taxonomy.md:117-126`;
`docs/specifications/instrument-master.md:187-201`;
`docs/specifications/acquisition-framework.md:177-184`).
The risk is that "universal" naming and an *Accepted* ADR are read as delivered capability.

## Implemented vs aspirational (explicit)

**Implemented (live path):** 7 connector selectors in `ExchangeKind`
(`src/governance/config.rs:19-32`): Kalshi, Polymarket, Hyperliquid, Coinbase, Kraken, OKX,
Binance. OKX and Binance are documented as reference-data/offline-discovery only
(`config.rs:26-31`); the other five are websocket adapters (`src/acquisition/exchange/mod.rs:1-15`).
Products are aggregated L2 snapshots/level changes plus lifecycle and reset only:
`ExchangeEvent` has exactly `Snapshot | LevelChange | Lifecycle | StreamReset`
(`src/domain/model.rs:845-878`); SBE has exactly `Snapshot` and `PriceLevelUpdate`
(`sbe_schemas/fieldbook/orderbook.yaml:5-56`; `src/generated/orderbook.rs:860,878`). Books are
keyed by `native_symbol` (`model.rs:589`; ADR-0001 `docs/adr/0001-...md:57-66`). There is **no
NSE, no BSE, no equity venue, no ITCH/MBO/order-level feed, no trades/BBO/corporate actions on
the wire, no multicast, no FIX certification.**

**Implemented as types, not wired to live data:** taxonomy (`src/domain/taxonomy.rs:16-336`),
opaque identity/PIT catalog (`src/domain/identity.rs:31-384`), reference graph
(`src/domain/reference_data.rs:145-359`), typed event/information payloads
(`src/domain/event_model.rs:163-301`; `src/domain/information.rs:21-258`), canonical envelope
(`src/domain/canonical.rs:318-646`), numeric spec (`model.rs:72-514`; `src/domain/fixed.rs:1-207`),
source manifests (`src/governance/adapter.rs:23-282`; `src/domain/products.rs:222-363`).
Verified by usage search: `CanonicalEnvelope<CanonicalEvent>`, `UniversalEnvelope`,
`DomainPayload`, `InformationPayload`, `ReferenceCatalog`, `ReferenceGraph`,
`venue_mappings_from_markets`, `resolve_venue_as_of` appear only in their own modules'
`#[cfg(test)]`
blocks. `DomainPayload` also appears in `src/acquisition/codec.rs:255-316,474-515`, but that
typed-normalization path is reached only via `ProcessingStages::run`
(`src/acquisition/evidence.rs:520-572`) → `EvidencePipeline::with_stages` → `run_stream_slice`
(`src/acquisition/stream_slice.rs:183-203`) → conformance fixtures
(`src/integrity/conformance.rs:383-403,502`) — i.e. **offline replay/verification, not live
ingestion**. `SourceConnector`/`SourceRegistry` (`src/governance/adapter.rs:257,282`) have **no
implementation**; live adapters implement `Exchange` instead
(`src/acquisition/exchange/mod.rs:112-120`). ADR-0001 is explicitly documentation-only: scope
`:8-9`, non-goals "add Rust structs or enums / change `MarketInfo` / change `ExchangeKind` /
change the numeric model" `:625-641`, migration deferred `:606-620`.

## Verified implementation facts (file:line)

- **Identity genuinely modelled in code:** `OpaqueId::derive` uses `0x1f` separators and first
  12 bytes of SHA-256 (`identity.rs:31-46`); eight id newtypes `:87-106`; scoped, effective-dated
  `InstrumentIdentifier` `:125-144`; venue/feed mappings and 4-state `Resolution` `:159-200`.
- **PIT reference data genuinely modelled in code:** `ReferenceCatalog::resolve_venue_as_of`
  and `resolve_feed_as_of` filter on `is_valid_at` (`identity.rs:314-346`); `ReferenceGraph`
  effective-dated specifications and ambiguity `:324-358`; bitemporal valid/transaction time
  (`src/domain/information.rs:21-61`) with `revision_lineage` `:131`.
- **Timestamp roles/clock domain genuinely modelled in code (offline family only):**
  `TimestampRole` six roles, `ClockSynchronization`, `ObservedTimestamp` with
  precision/resolution/accuracy/uncertainty/utc-offset (`event_model.rs:18-95`); fail-closed
  validation `:268-301`.
- **Sequence/continuity genuinely modelled in live code:** `EventMeta` connection_epoch,
  receive_seq, source seq, clock (`model.rs:661-679`); `ContinuityState`/`DiscontinuityReason`/
  `ResetReason` `:740-841`; `SubscriptionLease` epoch/state machine
  (`products.rs:365-550`); `SequenceMetadata` in canonical envelope (`canonical.rs:69-79`).
- **Scales/units genuinely modelled in live numeric model:** signed `Price`, distinct `Quantity`/
  `QuantityDelta`, `PriceUnit`/`QuantityUnit`, `PriceBand`, `Ratio`, `NumericSpec`
  (`model.rs:72-514`); exact decimal parse/rescale with `i128` (`fixed.rs:1-207`).
- **Typed payloads are serde-tagged and distinct:** `DomainPayload` 12 variants
  (`event_model.rs:163-230`), `InformationPayload` 5 variants (`information.rs:233-255`),
  `CanonicalEvent` 8 kinds with a total, non-fabricating `From<&ExchangeEvent>`
  (`canonical.rs:317-414`).

## Missing implementation

- No equity venue connector, NSE/BSE, ITCH/MoldUDP64, L3/order-by-order feed, or order-level
  add/modify/cancel on the live wire.
- No live construction of canonical/universal envelopes or PIT identity resolution; no durable
  `NormalizedStore` — only `InMemoryNormalizedStore` (`canonical.rs:598-646`;
  `docs/canonical-event-model.md:76-88`).
- No implementation of `SourceConnector`; no CFI/DTI/ISIN/FIGI/LEI/FIGI resolution engine;
  code constructors validate shape only (`taxonomy.rs:301-336`;
  `docs/specifications/data-taxonomy.md:86-88`); no UTI transaction identity.
- No per-source clock metadata on live `EventMeta` (`model.rs:661-679`); no clock-source field
  on `CanonicalEnvelope` (`canonical.rs:453-466`).
- ADR-0001 migration not started; Kalshi `exchange_market_id = event_ticker` undercount still
  open (`docs/adr/0001-...md:68-103,601-604`).

## Defects / risks

1. **Heterogeneous payloads lose meaning in `DomainPayload`.** Spec rule 4 "missing is not zero"
   (`event-model.md:23`) is violated: only `BlockchainEvent.transaction_hash` is `Option`
   (`event_model.rs:225-229`); all other fields are non-optional `i64`/`String`, so absent data
   must be fabricated as `0`/empty. Spec rule 5 "original mantissas and scales are retained"
   (`event-model.md:24-25`) holds only for `EquityQuote` (`:167-174`) and the `information.rs`
   detail structs; `Trade`, `OrderEvent`, `FxQuote`, `FutureQuote`, `OptionQuote`,
   `CommodityAssessment`, `EconomicObservation`, `BondQuote` carry bare mantissas with no scale
   or unit (`event_model.rs:175-229`). Mantissas without scales are not comparable across
   products.
2. **`ReferenceGraph::resolve_venue` ignores its observation-time argument**
   (`reference_data.rs:314-321`, `let _ = observed_at_us;`), acknowledged at
   `docs/specifications/instrument-master.md:90-92`. Venue PIT is therefore not applied.
3. **Two incompatible "canonical" payload families.** `domain::canonical::CanonicalEvent`
   (book taxonomy) vs `storage::application::CanonicalBusinessEvent` (entity/kind/value)
   (`src/storage/application.rs:389-394,427-479`), both serialized with
   `CANONICAL_SCHEMA_VERSION`. Consumers must know which they hold; schema version alone does
   not disambiguate.
4. **Capability-inflation risk.** Specs/ADR speak of "universal" coverage while live scope is
   five crypto/prediction websocket feeds; the ADR is "Accepted" yet its acceptance criteria
   (`:643-662`) are documentation assertions, not code. A reader may treat aspirational terms
   as delivered. The docs' non-claims mitigate but do not remove this risk.
5. `ObservedTimestamp::new` defaults `utc_offset_minutes: Some(0)` (`event_model.rs:111`),
   asserting UTC+0 when no offset was declared; should be `None` unless evidenced.
6. Minor doc drift: docs reference `src/canonical.rs`, `src/event_model.rs`, `src/taxonomy.rs`,
   `src/adapter.rs`; files now live under `src/domain/**`/`src/governance/**` and are re-exported
   (`src/lib.rs:24-38`), so paths resolve but are misleading.

## Existing tests and their limits

Unit tests are dense and genuine: taxonomy string stability/distinctness
(`taxonomy.rs:353-387`); identity as-of/reuse/ambiguity/OTC (`identity.rs:403-524`); reference
conflicts/effective dating (`reference_data.rs:383-496`); exact decimal parse/rescale
(`fixed.rs:214-312`); numeric spec/tick/lot/ratio (`model.rs:1017-1184`); canonical conversions,
reader fail-closed, in-memory replay (`canonical.rs:687-1042`); envelope validation and clock
claim checks (`event_model.rs:365-448`); bitemporal revisions (`information.rs:290-378`);
subscription leases and capabilities (`products.rs:556-791`); codec registry and typed
normalization (`codec.rs:588-820`); conformance (`conformance.rs:721-866`). Integration tests are
venue-specific/offline (`tests/kalshi_lifecycle_plane.rs`, `hyperliquid_books.rs`,
`hyperliquid_telemetry.rs`, `e2e_mock_exchange.rs`, `numeric_model.rs`, `evidence_pipeline.rs`,
`source_conformance.rs`, `jsonl_postgres_reference.rs`, `live_kalshi.rs`, `live_polymarket.rs`).

**Limits:** all are contract-level, unit, fixture or mock. None demonstrates NSE/BSE or any
cash-equity venue, ITCH MBO/order identity, L3 depth, multicast capture, FIX certification,
production latency, a durable normalized store, or live use of the typed payload/PIT identity
path. `live_*` tests were **not run** (would require credentials/network). This matches the
docs' explicit non-claims (`docs/canonical-event-model.md:99-103`).

## Required changes (to make the claim true)

1. Wire `ReferenceCatalog`/`ReferenceGraph` into live normalization: resolve `native_symbol` →
   `InstrumentId` as-of event time; populate envelope `instrument_id`; make it durable. Fix
   `resolve_venue` to apply `observed_at_us` (`reference_data.rs:315-316`).
2. Route live events through `UniversalEnvelope`/`DomainPayload` (or extend `ExchangeEvent`)
   with typed trade, BBO and ITCH-style order-level payloads carrying order identity; add the
   corresponding SBE messages/schema-evolution decision.
3. Add `Option`/scale/unit fields uniformly to every `DomainPayload` variant; enforce spec rules
   4 and 5; default `utc_offset_minutes` to `None`.
4. Add per-source clock metadata to live `EventMeta` and map the six `ObservedTimestamp` roles.
5. Implement `SourceConnector`/`SourceRegistry` in at least one live adapter and a durable
   `NormalizedStore`; disambiguate the two canonical payload families under distinct schema ids.
6. Only then onboard an authorized cash-equity venue (e.g. NSE/BSE feed) with ISIN/MIC/CFI/FIGI
   effective-dated resolution; fix the Kalshi selected-market count; add an equity integration
   test exercising typed payloads + as-of resolution.
7. Keep the SBE wire contract backwards-compatible until step 2 is an explicit schema decision
   (ADR-0001 `:622-623`).

## Decision

**CORRECT.** The V2 domain types are a sound foundation and several hard semantics (PIT
resolution, bitemporality, exact numeric scales, clock roles, continuity) are genuinely in code —
but they are a disconnected library with real semantic gaps, and the live implementation remains
a crypto/prediction-market L2 feed with **no NSE/BSE or cash-equity support**. The claim "the
domain model supports NSE/BSE cash equities and a wider set of products" cannot be ACCEPTed as
delivered. **DEFER** live venue onboarding and durable/connector integration until licensing,
schema evolution and the changes above are agreed; do not infer delivered maturity from the
presence or count of V2 type files.
