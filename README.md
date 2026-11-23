# Nirnay Quant — research-to-paper trading workspace

**Scope:** India-first systematic quantitative research, event-driven simulation, portfolio
construction and risk engineering with controlled execution contracts. Market-data
acquisition, raw storage, normalization and point-in-time reference data are owned by the
complementary platform under `engineering/architecture/decisions/ADR-0001-data-ownership.md`.

**Release status:** research/paper engineering candidate. This repository is not a certified broker-connected trading system, not a SEBI/exchange-approved algo product, and not evidence of profitable alpha. Live order routing is hard-disabled. Every capability statement below distinguishes code present from external verification still required.

<!-- CURRENT-STATE
code_commit: 6206052e0e89de33130e03165063ee53aabbe348
verified_on: 2026-11-21
ci_run: 45062286601
scope: make check + hosted CI 45062286601 on f250fee (12/12); active suite 352 passed, 90 market-shaped-fixture tests deselected under owner directive
-->

**Active regression scope:** under the owner directive of 2026-11-22, tests that consume
checked-in synthetic market-shaped fixtures are registered `market_shaped_fixture` and
deselected in `pytest.ini`; they no longer run in `make check` or CI. Historical test counts in
dated notes below are historical records, not current coverage. See
`evidence/POLICY-BOUNDARY-REGRESSION-WITHDRAWAL-2026-11-22.md`.

Withdrawal is now enforced at collection time by
`research/python/tests/conftest.py` and the checked-in
`research/python/tests/market_shaped_test_inventory.json`; overriding the
marker expression cannot restore a prohibited test. Counts and withdrawn
coverage are recorded in
`evidence/SYNTHETIC-TEST-POLICY-NIRNAY-2026-11-23.md`.

## Implemented scope

## Language and authority ownership

Python (`quantlab`) is the research and statistical-validation environment:
feature construction, strategy families, walk-forward experiments, simulation
research and reporting. Rust is the authority for event identity, books,
portfolio state, risk decisions, the OMS and the account journal. KDB-X is an
owner-local analytics tool, not an execution or account authority. No Python or
q process runs across the real-time execution boundary. Claims below are
 Implemented unless a dated record explicitly states Verified.

The provider capture, normalization and reconciliation rows below describe legacy tooling
preserved for review. They are blocked pending rights and owner approval; acquisition
ownership moves to the complementary platform under ADR-0001.

| Capability | Active implementation | Boundary / non-claim |
|---|---|---|
| Durable raw journal | `crates/market-archive` | A single-writer, newline-delimited journal with `sync_data()` per append, poisoned-writer behaviour after an uncertain commit, and a bounded incremental scanner. Serialized records are capped at 1 MiB (512 KiB decoded payload); oversized, torn or corrupt records fail closed. |
| Raw Upstox V3 capture and Protobuf decode | `crates/upstox-feed-proto`, `apps/upstox-tools`, `schemas/upstox/v3/` | `upstox-decode-export` verifies raw-frame hashes and exports full decoded message envelopes. It now reads the journal through the shared bounded `JournalReader`, so record limits, torn-tail detection and EOF semantics match the archive. It requires a verified session manifest, rechecks it after decoding, and publishes no output on any failure. Live provider capture remains unverified. |
| FYERS Data Socket capture | `apps/fyers-recorder` | Captures decoded SDK callback JSON, not original wire packets. Local sequence/gap detection does not establish exchange sequence continuity. |
| Canonical Upstox/FYERS market events | `research/python/src/quantlab/market.py`, `providers.py` | Preserves provider IDs, timestamp semantics, local sequence, source payload and capture-frame hashes. LTP changes are observations, not invented trade prints or aggressor-side labels. |
| Cross-feed reconciliation | `quantlab/reconcile.py` | Compares fresh, comparable provider state for one canonical instrument. It never merges/sums duplicate liquidity or constructs an authoritative consolidated tape. |
| Point-in-time reference data and datasets | `quantlab/point_in_time.py` | Bitemporal listing/universe membership, immutable corporate-action revision lineage, exact split/bonus view, source/frame/event lineage, atomic immutable JSONL publication, optional Zstandard Parquet, semantic Parquet verification and optional detached Ed25519 manifest signature through AWS KMS. A valid signature authenticates the publisher key, not the upstream data source or its completeness. |
| Corporate financial fact contracts | `crates/fundamental-domain`, `apps/fundamental-tools` | Exact decimal financial facts, source-document digests and evidence locators, effective-dated company identity, append-only restatement lineage, derived-fact dependency checks and point-in-time selection. The current CLI validates bounded JSONL inputs; it does not acquire filings, parse XBRL/PDF, or provide a production store. |
| Event-driven backtest | `quantlab/backtest.py`, `quantlab/portfolio_backtest.py` | `EventDrivenBacktester` is single-instrument; `MultiInstrumentPortfolioBacktester` is a separate multi-instrument, single-provider, long-only NSE cash portfolio model with independent per-symbol orders, cash/exposure reservations, latency, partial fills, participation caps, fees, spread/impact and conservative L2 market-by-price queue approximation. Neither is an exchange matching engine; queue/impact parameters need calibration. |
| Strategy and alpha research | `quantlab/strategies/`, `alpha_search.py`, `quantlab/validation/` | Streaming deterministic EMA-cross fixture and bounded predeclared candidate screen with train/validation/untouched holdout, embargo, flat segment portfolios, explicit net costs, Holm/HAC screening, CSCV/PBO and Deflated Sharpe diagnostics. This is a research screen, not arbitrary code generation, full execution-cost proof or validated alpha. |
| Order management and live authorization contracts (Python research path) | `quantlab/oms.py`, `quantlab/durable_oms.py`, `risk_evidence.py`, `risk_service.py`, `risk_gateway.py`, `risk_daemon.py`, `kms_signing.py`, `live_auth.py` | Local SQLite WAL/FULL transactional intent/outbox store, hash-chained journal, leases, idempotent broker-event ingestion, partial fills, cancel/unknown-outcome handling and paper-only dispatcher. This is a research/service path; the Rust OMS and account journal are the execution-state authority. A separately runnable HTTPS/mTLS risk-evidence daemon obtains eight independently signed control observations and signs scoped runtime evidence via AWS KMS Ed25519. The daemon and KMS adapter have tests but no deployed KMS key, collectors or cloud service; no broker OMS adapter is certified and live mode always rejects. |
| C++ numeric oracle | `verification/cpp-oracle/` | Isolated C++20 pure-numeric verification target, not linked to production. D11 retired the order-book oracle and market-shaped corpus; no differential equivalence is claimed. |
| Typed research simulation | `quantlab/sim/` | Frozen lineage-bearing events, a pure state reducer, first-class execution reports, seeded research mechanics and content-addressed replay. The conservative MBP baseline declares queue truth unavailable and refuses FIFO claims; cost terms are assumption-labelled. No calibrated, venue-truth or profitability claim. |
| Experiment registry and research jobs | `quantlab/experiments/`, `quantlab/research_jobs.py`, `quantlab/validation_report.py` | Append-only, hash-chained experiment records with atomic holdout claims, disjoint validation/holdout periods, local opt-in job manifests and integrity-only validation reports. No holdout reuse, no alpha certification, no scheduler service. |
| Rust account ledger and reconciliation | `crates/authoritative-oms/src/account_projection.rs`, `crates/authoritative-oms/src/reconciliation.rs`, `crates/account-journal` | Content-addressed OMS fill events project idempotently into the append-only account journal with an evidenced opening balance, exact-paise folds, reversal chains and a replay cursor. OMS-vs-ledger mismatch is a typed, sticky block on new risk. Implemented and locally tested, not broker-reconciled. |
| Account-wide health authority | `crates/risk-authority`, `apps/paper-runner` | New risk is denied unless six versioned, fresh observations from registered in-process owners pass inside the reservation mutex. Typed denials, an evidenced kill switch with persisted owner-local state, and observation-free risk-reducing paths are present. No deployed independent risk service. |
| Provider rights enforcement | `configs/providers/source-products.v1.json`, `crates/market-archive/src/capability.rs`, recorder CLIs | Seven tri-state operations with provenance. All 21 rights remain `UNKNOWN`; recorders and replay paths require explicit registry authorization and fail closed. Written evidence is required before any right changes. |
| Opt-in performance harnesses | `Makefile` (`bench-*`), `evidence/PERFORMANCE-2026-11-21.md` | Measured synthetic local workloads with distributions, RSS and digests, kept outside `make check` and CI pass/fail gates. The published run used Python 3.15 on macOS; it is not a Python 3.12, Linux, exchange-feed or HFT baseline. |

| Broker operations | `apps/kite-ops` | macOS Keychain-backed local Kite OAuth login and read-only profile/orders/positions/instrument operations are implemented. Order submission is unconditionally hard-blocked in this build; this adapter is not connected to the durable OMS and is not a production broker adapter. OAuth must be completed by the account owner. |

## Quick start

Python 3.12+ is required. Install the research package and test dependencies:

```bash
python3 -m pip install -e 'research/python[dev,research,security]'
python3 -m pip install -e 'apps/fyers-recorder[test]'
python3 -m pip install -e 'apps/kite-ops[test]'
```

Run the complete local gate (documentation lifecycle, tool isolation,
generated-path audit, ShellCheck where shell scripts exist, formatting, style
checks, Rust and Python tests, Clippy, dependency policy and contract
validation):

```bash
make check
```

Run the local verification suite (Python tests, compile/policy gates, active JSON/TOML and schema validation, and the isolated C++ oracle):

```bash
make verify-local
```

Style is enforced on added lines for Rust, Python, C/C++, Shell, Makefile,
JSON/JSONL and Markdown; see `engineering/style-policy.md`.

The root `pytest.ini` collects only the active test suites and excludes
`.local/extras/`, `target/`, virtual environments and generated caches. The
Parquet round-trip runs when PyArrow is installed from the `research` extra;
CI installs it and fails if the test is skipped.

Check dependency integrity — workflow SHA pins, resolved Python constraints and the SBOM:

```bash
make supply-chain
```

Regenerate the constraint sets with `make constraints` after a deliberate dependency change. Known provider-SDK advisories are waived with an owner and expiry in `constraints/pip-audit-allowlist.toml`; see `engineering/supply-chain.md`.

Check the packaging contract — three clean wheels, no cache entries, each installed and executed in a fresh environment:

```bash
make packaging-test
```

Build a release bundle and verify it case-sensitively the way a Linux consumer would:

```bash
make bundle BUNDLE_OUT=../nirnay-quant-bundle.zip BUNDLE_ARTIFACTS=../artifacts
mkdir -p /tmp/unpacked && cd /tmp/unpacked && unzip -q /path/to/bundle.zip
python3 /path/to/repo/scripts/verify_bundle.py --bundle . --zip /path/to/bundle.zip
```

`scripts/make_bundle.py` writes the manifest and the archive from one file
list, so a manifest path and an archive member name cannot diverge; both are
compared as exact strings, because a case-insensitive filesystem cannot tell
`sbom.cdx.json` from `SBOM.cdx.json`. The bundle job stages compiled output
under `$RUNNER_TEMP/release`, outside the checkout, and the bundler rejects an
artifacts directory inside the source tree so wheels and binaries cannot be
packed twice.

### Synthetic demonstrations retired

The former synthetic `quantlab demo` smoke test and its archived fixture directory
`research/python/fixtures/` were retired from the active tree under owner directive
D11. No synthetic market-data demonstration is runnable from the normal workflow.
The dated path/ha256 record is in the Nirnay retirement manifest and the RRP-002
retirement record under `evidence/`; historical withdrawal notes remain unchanged.

## Corporate financial fact validation

The first fundamental-data vertical defines source-backed company identity
and financial fact records.
It is offline by design: source acquisition and data entitlements remain separate decisions.
The CLI validates record framing, per-record and aggregate-byte limits, identity and fact keys,
append-only revisions, derivation dependencies and historical availability. The checked-in
example records are archived synthetic conformance data under the owner directive of 2026-11-22;
no runnable command consuming them is provided in this quickstart.

The JSON Schema contracts are in `schemas/fundamentals/`. The historical fixture records were
synthetic conformance data, not records fetched from MCA or an exchange; their document digests
are test values, not source evidence. The domain stores exact decimal coefficients as canonical
signed `i128` strings plus decimal scale; it does not use binary floating point for financial
facts. Each reported fact includes a source ID, filing ID, source-document SHA-256 and an XBRL,
PDF-table or JSON-pointer locator. This CLI validates digest and locator syntax; it does not fetch
or re-hash the source document. A digest proves byte identity only when checked against the bytes;
it does not prove who published the document or whether the source is complete or licensed. The
validator checks record
framing, per-record and aggregate-byte limits, record count, identity and fact keys, append-only
revisions, derivation dependencies and historical availability. It retains decoded facts in memory
for closed-set lineage validation and is not a production ingestion store.

The governing design and non-goals are recorded in
`engineering/decisions/ADR-FND-001-corporate-financial-facts.md`. MCA/SEBI/NSE/BSE source
adapters, taxonomy-aware XBRL parsing, PDF extraction, durable fundamental storage, statement
reconciliation by accounting framework and valuation remain separate work items with independent
contracts and verification.

## Research pipeline

### 1. Build a point-in-time dataset

Reference data must provide bitemporal provider listings, immutable corporate-action source
records and (for historical universe analysis) dated universe-membership records. The former
quickstart example built a dataset from checked-in synthetic canonical events and reference
data. It has been withdrawn under the owner directive of 2026-11-22; no runnable command
consuming those assets is provided here.

`build-dataset` emits immutable outputs and requires a destination that does not already exist.
Optional Parquet output requires PyArrow. Optional detached manifest signing requires an AWS KMS
Ed25519 key and narrowly scoped `kms:DescribeKey`, `kms:GetPublicKey` and `kms:Sign` permissions.
These capabilities remain available for an accepted dataset; their previous synthetic
demonstration was retired from the active tree under D11.

`verify-dataset` establishes internal manifest/artifact/hash consistency; with a trusted key,
it additionally verifies the publisher's detached Ed25519 signature over a domain-separated
manifest digest. It does not establish source authenticity, entitlements, or source
completeness. It cannot certify the source was official, complete, licensed, correct, or
independently captured. The builder preserves raw prices and emits exact split/bonus adjustment
factors as a separate point-in-time view. Dividends remain cash-flow events; rights, mergers,
demergers, delistings and other complex actions fail closed until a reviewed transform exists.
If the selected universe membership source is incomplete, survivorship bias remains possible
and the manifest does not claim otherwise.

### 2. Run a bounded alpha-search screen

The CSV interface requires strictly increasing `timestamp_ns` and positive `close` columns. The
former quickstart example consumed a deterministic synthetic bar series. It has been withdrawn
under the owner directive of 2026-11-22; no runnable command consuming that asset is provided
here. No experiment parameter may be chosen from synthetic data; see
`research/protocols/NSE-CASH-MOMENTUM-PROTOCOL-v1-AMENDMENT-1.md`.

The search family is finite and predeclared. It selects on validation only, applies Holm
correction across the candidate family, then reports one untouched chronological holdout.
Each segment starts flat and pays exit/liquidation turnover cost; one return interval at each
fold boundary is embargoed. The report additionally includes development-only CSCV/PBO and
validation-only Deflated Sharpe diagnostics. Sharpe annualisation is controlled by the explicit
`--periods-per-year`; it is not inferred. The screen uses bar closes and simplified basis-point
costs, not full event-level fills or all exchange charges. A positive holdout is a research
screen, not validated alpha. Use licensed point-in-time data and the event simulator before any
strategy promotion.

## Synthetic research assets retired

All checked-in synthetic market-shaped assets — price bars, canonical events,
reference data, market calendars, fabricated India listings, KDB MBP goldens and
order-book corpora — were removed from the active tree under owner directive D11.
They must not be regenerated, extended, transformed, reused as research evidence,
or cited as proof of source completeness, rights or real-market verification. The
dated path/SHA-256 record is in the retirement manifest and the RRP-002 retirement
record under `evidence/`; historical withdrawal notes remain unchanged.

## Legacy provider tooling — blocked pending rights and owner approval

The Upstox and FYERS capture, normalization and reconciliation tooling below is preserved as
implemented code and historical evidence. It is **not** an active quickstart. All provider
rights remain `UNKNOWN`; collection, local storage, replay, backup and analytics are not
authorized. Do not execute any command in this section. Ownership of source acquisition and
raw data belongs to the complementary platform under ADR-0001; this section is retained only
until its architecture has been reviewed.

Upstox V3 captures original binary Protobuf payloads in `raw-frames.jsonl`. The
companion diagnostic projection is useful for inspection but is not treated as
complete depth/quote data. The decoder uses the bounded `JournalReader` from
`market-archive`; a record without a final newline is a torn tail and a failure
anywhere in the journal prevents publication of decoded output. Verify the
completed session before decoding; the decoder checks the manifest hash, frame
count and terminal status before and after reading, and rejects a journal that
differs from the one the manifest names. This establishes file-to-manifest
consistency, not source authenticity or data rights. Verify, then decode:

```bash
cargo run --release -p upstox-tools --bin upstox-verify-session -- \
  --manifest data/market-sessions/upstox/<session>/manifest.json

cargo run --release -p upstox-tools --bin upstox-decode-export -- \
  --input data/market-sessions/upstox/<session>/raw-frames.jsonl \
  --manifest data/market-sessions/upstox/<session>/manifest.json \
  --output data/market-sessions/upstox/<session>/decoded-full.jsonl
```

Copy and edit the example maps using an independently reconciled instrument master (do not treat the example entries as live mappings):

```bash
cp configs/providers/upstox-instrument-map.example.json data/upstox-instrument-map.json
cp configs/providers/upstox-venue-map.example.json data/upstox-venue-map.json
```

Normalize the full decoded export using an explicit mapping from provider instrument IDs to canonical security IDs:

```bash
quantlab normalize --provider upstox \
  --input data/market-sessions/upstox/<session>/decoded-full.jsonl \
  --instrument-map data/upstox-instrument-map.json \
  --venue-map data/upstox-venue-map.json \
  --output data/normalized/upstox/<session>.jsonl
```

Prepare the FYERS map separately:

```bash
cp configs/providers/fyers-instrument-map.example.json data/fyers-instrument-map.json
```

The FYERS Data Socket recorder defaults to a conservative 100-symbol cap. Current official documentation lists 100 symbols/user for Standard and 5,000 for Prime across Data Socket connections. The recorder cannot verify your account tier; raise the cap only after confirming the entitlement:

```bash
python3 -m fyers_recorder.cli \
  --watchlist configs/providers/fyers-watchlist.txt \
  --max-symbols 5000 \
  --output data/market-sessions/fyers
```

This path captures `SymbolUpdate` and five-level `DepthUpdate`. FYERS separately documents a 50-level Protobuf Tick-by-Tick (TBT) socket with different limits; the TBT socket is not implemented by this recorder. The active archive is SDK callback JSON, not raw WebSocket bytes.

FYERS callbacks are SDK-decoded JSON, not WebSocket frame bytes:

```bash
quantlab normalize --provider fyers \
  --input data/market-sessions/fyers/<session>/events.jsonl \
  --instrument-map data/fyers-instrument-map.json \
  --output data/normalized/fyers/<session>.jsonl
```

By default, explicit FYERS journal gaps stop normalization. `--allow-gaps` is
an explicit degraded mode; the quality metadata marks the output as unsuitable
for a continuous/research-grade tape. Both providers accept
`--reference-revision <file>` to bind a projection to one exact reference
content revision. The embedded maps must match the supplied maps exactly,
unknown provider instruments remain unresolved, and the quality report records
the binding hash plus normalizer and event-schema versions. The binding proves
consistency, not authenticity of the reference file. Keep provider instrument
maps versioned and dated; do not map on ticker text alone.

Cross-check comparable Upstox/FYERS quote or depth state:

```bash
cat data/normalized/upstox/<session>.jsonl data/normalized/fyers/<session>.jsonl \
  > /tmp/provider-events.jsonl
quantlab reconcile --events /tmp/provider-events.jsonl \
  --now-utc-ns <trusted-current-utc-nanoseconds>
```

Reconciliation is diagnostic. A quote-state comparison does not establish that both providers delivered every event, nor does it justify summing their displayed size. Use a single selected provider for any backtest replay unless an independently designed feed arbitration layer selects one authoritative path.

## Market and execution model caveats

- Upstox `currentTs` is retained as feed-envelope/received-tick time; `ltpc.ltt` is represented separately as last-trade time. A change in an LTP snapshot is not evidence of a discrete trade print with aggressor side.
- FYERS symbol/depth callbacks are decoded SDK messages. Their local callback order is not an exchange sequence. The current recorder captures the Data Socket five-level depth, not the separate 50-level Protobuf TBT socket. Official Data Socket Standard/Prime symbol ceilings are 100/5,000; the configured cap is operator-declared and broker entitlement remains unverified.
- Upstox V3 subscription limits vary by mode and plan; the official docs distinguish LTPC, Option Greeks, Full and Plus Full D30. Do not assume published ceilings apply to the connected account. The current normalizer is not a full CAS/auction-session model.
- Aggregate L2 depth does not expose individual queue priority. The backtester models a conservative queue ahead and only allows passive queue consumption from an explicitly classified trade event with known aggressor side. Current normalizers do not manufacture those trade events from LTP-only updates.
- Taker spread is embedded in crossing the selected book; square-root impact is a separate modeled price displacement. It is not calibrated without real venue-specific data. Impact is an attribution of cost and is not debited twice.
- `quantlab.backtest.EventDrivenBacktester` is single-instrument. The separate `quantlab.portfolio_backtest.MultiInstrumentPortfolioBacktester` supports multiple NSE cash instruments but remains long-only, single-provider, one-active-order-per-instrument and cash-funded. Neither models margin, short borrow, derivatives, auctions, corporate-action cash settlement, multi-venue routing, broker throttles, exact order-by-order priority or exchange-certified matching.
- The strategy emits order intents only. It never calls brokers, risk authorities or OMS mutation methods.

## OMS, live risk and regulatory boundary

The active OMS components are `quantlab.oms` (domain state machine) and `quantlab.durable_oms.DurableOMSStore` (single-node SQLite WAL/FULL transactional store with atomic intent+outbox acceptance, leases, hash-chain event journal, idempotent broker event/execution IDs, monotonic broker sequence, partial fills, cancel request/ack separation and unknown-outcome quarantine). `PaperOutboxDispatcher` admits only paper adapters. SQLite is not replicated HA or distributed exactly-once delivery. There is no certified Upstox/FYERS/Kite execution adapter behind this OMS; Kite operations are read-only and its submission method is hard-blocked.

The permit verifier expects an externally root-signed trust registry, independent issuer signature, distinct approver roles, separate risk-service runtime attestation, bounded scope/time and one-time consumption. Runtime evidence can only be constructed from eight fresh, independently signed source observations (account risk, kill switch, market data, reconciliation, broker entitlement, session, clock and deployed artifact); direct caller-supplied `RiskFacts` signing is rejected. `AwsKmsEd25519Signer` supports a customer-managed AWS KMS Ed25519 key, but no key or separate risk service is deployed here. Private keys must stay in separately administered KMS/HSM/offline services, not in Git, fixtures or the OMS process. Test signing keys are ephemeral. The OMS rejects every live submission, even when a test permit verifies.

Before live use, the organization needs (at minimum) legally reviewed broker/exchange/SEBI applicability, account and data entitlements, certified broker adapters, independent real-time risk service, operational kill switch, static-IP/session checks where applicable, clock/feed health, reconciliation, key management, audit retention, deployment controls, controlled canaries and incident/restore drills. A signed permit is necessary policy evidence, not a substitute for those controls. Consult `engineering/market-compliance-and-entitlements.md` and qualified compliance counsel; this README is not legal advice.

## Build and verification

The Rust workspace pins Rust 1.99.1 in `rust-toolchain.toml`, edition 2024. Run with the pinned toolchain:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo deny check
cargo xtask check
```

Run the promoted C++ verification oracle independently:

```bash
cmake -S verification/cpp-oracle -B /tmp/nirnay-cpp-oracle -DCMAKE_BUILD_TYPE=Release
cmake --build /tmp/nirnay-cpp-oracle --parallel
ctest --test-dir /tmp/nirnay-cpp-oracle --output-on-failure
```

The oracle is not wired into production Rust. On 2026-11-18 the pinned Rust
1.99.1 gates were rerun on `main` at `c91ad3f`: local `make check` passed with
410 Rust test cases across the then-active workspace and archived reference
targets, 156 Python tests and 2 subtests, plus formatting, style, Clippy
`-D warnings` for that baseline, `cargo deny check`, `cargo xtask check` and
contract validation.
Hosted CI run `44199935228` passed 11/11 jobs at that baseline. CI also builds the active C++ target
under GCC and runs schema validation and the required Parquet round-trip. The
Parquet test executes rather than skips. Live Upstox/FYERS/Kite accounts were
not accessed; provider tests use deterministic fixtures and fake clients/SDK
objects. Earlier 2026-11-13 counts (278 root tests, 96 extras tests) are
historical. See `BUILD-RESULTS.md`,
`evidence/VERIFICATION-2026-11-17.md` and
`engineering/provider-environments.md`.

## Extras policy

`.local/extras/**` is an operator-local archive of the predecessor reference
workspace (`nirnay-extras`). It is outside version control and excluded from
all active build, test and CI graphs. On 2026-11-13 the useful
subset was trimmed from 325 to 61 files and promoted from `legacy/` to
`.local/extras/`: the India ingestion/durable-store crate and tests, the C++
SBE/conformance probes, synthetic fixtures and challenge/internal schemas. The
old monolith, prediction-market material, q experiments, archived measurements,
historical registries, duplicate active-crate copies, old locks and old deploy
workflows were moved to the external snapshot
`../legacy-snapshot-2026-11-13/legacy-full-backup-2026-11-13.tar.gz` (SHA-256
`a49e864308a7e58d667a86049dd80e8ff2c631cd762e50a74aff60007464f9b1`). The
predecessor's useful Upstox decoder and protocol schema are active; selected
C++ numeric/book code is promoted as an isolated oracle; corporate-action and
provenance concepts are reimplemented behind current bitemporal contracts.
The archived workspace and its C++ probes are no longer part of the active
gates. Historical counts from 2026-11-13 and 2026-11-17 are
retained in the dated verification documents; the current baseline is the
2026-11-19 state note below. See `engineering/extras-audit.md`,
`engineering/legacy-audit-2026-11-13.md` and `engineering/migration-map.csv`.
Promotion of the retained India code into the active workspace still requires a
new crate, ADR, owner and tests.

## Compile performance

Development and test profiles emit line-table debug information, preserving
source-line backtraces while keeping the debug target small. Release quality is
unchanged. Measured scope, size and gate improvements are recorded in
`evidence/COMPILE-PERFORMANCE-2026-11-20.md`.

```bash
make perf    # cold compile measurement with enforced budgets
```

The budget job runs in CI in parallel with the other gates.

## Documentation map and maintenance

Canonical documents are updated with the code. Dated evidence is immutable
once published; a new run adds a new dated file instead of rewriting history.
`scripts/check_docs.py` enforces this split in `make check` and CI.

| Class | Document | Purpose |
|---|---|---|
| Canonical | `README.md` | current state, scope and this map |
| Canonical | `AGENTS.md` | agent and contributor rules |
| Canonical | `CONTRIBUTING.md` | change and evidence workflow |
| Canonical | `SECURITY.md` | disclosure and secret handling |
| Canonical | `engineering/verification-policy.md` | claim-state vocabulary |
| Canonical | `engineering/q-kdb-style.md` | q / kdb+ (KDB-X) style rules and their authoritative sources |
| Canonical | `engineering/research/compute-and-validation.md` | local opt-in research jobs, validation reports and owner-gated compute gaps |
| Canonical | `engineering/research/dataset-acceptance-contract.md` | evidence required before a licensed dataset is accepted for research |
| Canonical | `engineering/ndc-consumer-requirements-v1.md` | proposed NDC v1 consumer requirements: release manifest, event envelope, validation and rejection behavior; no transport or storage choice |
| Proposed | `engineering/ndc-consumer-use-v2-proposed.md` | versioned consumer-use profiles, AdmissionRequest, permission evaluation and typed rejection codes; declaration-level model, no schema finalization |
| Protocol | `research/protocols/NSE-CASH-MOMENTUM-PROTOCOL-v1.md` | frozen proposed real-data-only NSE cash-equity momentum protocol; not registered or run, dataset-blocked |
| Protocol | `research/protocols/NSE-CASH-MOMENTUM-PROTOCOL-v1-AMENDMENT-1.md` | amendment 1 supersedes the v1 synthetic-fixture sentence and the nearest-price rule; v1 unchanged; SHA-256 sidecar beside it |
| Research | `engineering/research/procurement/m43-source-qualification-dossier.md` | M4.3 NSE source qualification: coverage, rights, tariffs and inquiry pack |
| Research | `engineering/research/procurement/zero-cost-source-availability-2026-11-22.md` | source-availability report at INR 0 from the existing dossier and cited official documentation; SOURCE_DOCUMENTATION_ONLY |
| Research | `engineering/research/procurement/source-qualification-upstox-nse-2026-11-22.md` | Phase 1 source qualification for Upstox V3 candles and NSE public reports; rights UNKNOWN, collection BLOCKED |
| Architecture | `engineering/architecture/decisions/ADR-0001-data-ownership.md` | Phase 2 ADR: complementary platform owns acquisition, raw storage, normalization and point-in-time reference data; Nirnay owns research; no second collector |
| Architecture | `engineering/architecture/decisions/ADR-INDEX.md` | canonical ADR ID namespace |
| Review | `engineering/architecture/reviews/FIELDBOOK-ARCHITECTURE-REVIEW-2026-11-22.md` | read-only Fieldbook architecture review, ownership map, seven findings and NDC v1 semantic compatibility; no code or provider contact |
| Review | `engineering/architecture/reviews/FIELDBOOK-REVIEW-REFRESH-2026-11-23.md` | baseline refresh at Fieldbook 28eb877, W1 remediation status, ownership and NDC reconciliation; not a ratification |
| Review evidence | `engineering/architecture/reviews/fieldbook-audits/` | five full read-only audits: architecture, durability, semantics, governance and distribution at Fieldbook `aee91de` |
| Test policy | `research/python/tests/conftest.py` | collection-time gate that deselects inventoried market-shaped tests and blocks uninventoried market-domain modules |
| Test policy | `research/python/tests/market_shaped_test_inventory.json` | checked-in authority listing prohibited test modules, categories and test names; D11 retirement record in `evidence/TEST-RETIREMENT-NIRNAY-2026-11-23.md` |
| Dated evidence | `evidence/TEST-RETIREMENT-NIRNAY-2026-11-23.md` | RRP-002 removal of synthetic market-data assets and function-level tests; removed paths, hashes, preserved historical counts, withdrawn coverage and residual gates |
| Dated evidence | `evidence/SYNTHETIC-TEST-POLICY-NIRNAY-2026-11-23.md` | enforced inventory counts, inspection commands, withdrawn coverage and current test figures |
| Dated evidence | `evidence/C3-CONSUMER-USE-PERMISSIONS-2026-11-23.md` | C3 consumer-use and permission-evaluation implementation record, gate counts and explicit non-goals |
| Dated evidence | `evidence/RUST-TEST-POLICY-NIRNAY-2026-11-23.md` | gated Rust targets and src modules, inventory and enforced default counts, withdrawn coverage and historical-count disclaimer |
| Dated evidence | `evidence/OWNER-DIRECTIVE-2026-11-22.md` | owner data-freeze directive: no synthetic market data, INR 0 data budget, STOP on missing evidence or authorization |
| Dated evidence | `evidence/M4-2-1-CLAIM-SCOPE-2026-11-22.md` | M4.2.1 narrowed to implemented, not real-market verified; calendar provenance, coverage-bound and bar-anchor findings |
| Dated evidence | `evidence/POLICY-BOUNDARY-REGRESSION-WITHDRAWAL-2026-11-22.md` | runtime-audit inventory of the 90 deselected market-shaped-fixture tests and the regression coverage withdrawn from active CI |
| Dated evidence | `BUILD-RESULTS.md` | owner-maintained bundle record; do not edit |
| Dated evidence | `evidence/VERIFICATION-2026-11-17.md` | executed checks and boundaries |
| Dated evidence | `evidence/UPGRADE-VERIFICATION-2026-11-13.md` | implementation inventory and upgrade gates |
| Dated evidence | `evidence/BUILD-INVENTORY-2026-11-13.json` | machine-readable release inventory |
| Dated evidence | `evidence/M2-AUTHORIZATION-2026-11-20.md` | signed M1 exit and bounded M2 scope |
| Dated evidence | `evidence/KDB-ANALYTICS-2026-11-20.md` | checked-in KDB-X contract, golden pin and real-capture measurements |
| Dated evidence | `evidence/DATA-RIGHTS-2026-11-20.md` | tri-state data rights and Kite semantics correction |
| Dated evidence | `evidence/M3-6-0-FAIL-CLOSED-2026-11-21.md` | fail-closed risk authority startup |
| Dated evidence | `evidence/M3-5-0-OMS-DURABILITY-2026-11-21.md` | exclusive-writer OMS journal and poison-on-uncertain-commit |
| Dated evidence | `evidence/M3-5-1-FILL-EVENTS-2026-11-21.md` | typed durable execution events |
| Dated evidence | `evidence/Q-TESTS-STYLE-2026-11-20.md` | q unit harness, style guide, added-lines gate and measured results |
| Dated evidence | `evidence/A0-ARCHIVE-BUNDLE-2026-11-20.md` | archive Git-bundle defect, correction and verification |
| Dated evidence | `evidence/PROGRAMME-2026-11-21.md` | external-review remediation programme and verified results |
| Dated evidence | `evidence/TASK-CLOSURE-2026-11-21.md` | exhaustive Task.md closure, commits, gates and owner-gated residual items |
| Dated evidence | `evidence/PERFORMANCE-2026-11-21-PINNED.md` | pinned Python 3.12 benchmark re-run with higher-repeat tail statistics |
| Dated evidence | `evidence/PERFORMANCE-TAILS-ADDENDUM-2026-11-21.md` | three independent latency runs; unstable p99.9 and ingress-loss caveats |
| Dated evidence | `evidence/PERFORMANCE-2026-11-21.md` | local synthetic benchmark observations on `ec0b216`, with toolchain, hardware, commands, distributions, hashes and scope limits |
| Dated evidence | `evidence/RISK-CONTROLS-RECOVERY-DRILL-2026-11-21.md` | typed risk-control fixtures, permit binding and in-process recovery drill |
| Dated evidence | `evidence/M4-RESEARCH-EVIDENCE-2026-11-21.md` | first synthetic pre-registered experiment and registry evidence |
| Dated evidence | `evidence/M4-CADENCE-CORRECTION-2026-11-21.md` | immutable addendum correcting the M4 cadence and annualisation registration |
| Dated evidence | `evidence/M4-2-1-CALENDAR-CADENCE-ADDENDUM-2026-11-22.md` | generalized calendar-based cadence validation; implemented, not verified |

The current-state block above names the verified commit, date, CI run and
scope. Update it in the PR that changes the verified baseline. Do not add a
new root-level status, brief or inventory file; extend a canonical document
or add a dated file under `evidence/`.

**Governing principle:** a capability is admitted by contract, executable
tests and external evidence—not by a successful demo, strategy backtest,
architecture diagram or a green check from a different commit.

### Historical 2026-11-18 state note (superseded)

The verification for commit `c91ad3f` is recorded in hosted CI run
`44199935228`, which passed 11/11 jobs; the pinned-toolchain local `make
check` passed all project gates (410 Rust test cases, 156 Python tests, 2
subtests). Dated documents retain older counts and commit hashes as
historical records. These results verify the named commit only and do not
grant M1 exit approval, M2 authorization, platform readiness or live trading.
Capture-session verification is implemented; the canonical-event, dataset,
replay and paper stages of the vertical slice are still outstanding.

### 2026-11-19 current-state note

The reviewed baseline is merge commit
`30ee424b62217a4b40680a9843f130af76c4a0bf` (PR #33, real-source conformance).
Hosted CI run `44366085320` on that merge commit passed 11/11 jobs; local
`make check` passed all project gates (455 Rust test cases, 162 Python tests,
2 subtests). Details and the evidence boundary are recorded in
`evidence/STATE-2026-11-19.md`.

These results verify the named commit only and do not grant M1 exit approval,
M2 authorization, platform readiness or live trading. PR #33 is complete for
the declared `nse-cash-equity-v1` and `bse-security-master-v2` versions and the
two dated source files only; it is not a claim for every exchange product.

The earlier "stages outstanding" sentence described the 2026-11-18 baseline
and no longer holds: capture-session verification, canonical-event
normalization, point-in-time dataset building and the paper backtest/OMS
stages are implemented (`engineering/capabilities.json` CAP-004, CAP-005,
CAP-008, CAP-013, CAP-016). Dataset/build replay and raw-frame lineage
verification are implemented for synthetic/internal fixtures; replay of real
archived provider sessions remains unverified.

The shared ingress **library** (`crates/market-archive/src/ingress.rs`) is
implemented with Upstox and FYERS adapters and streaming replay. PR #44
corrected delivery identity, sequence scope, continuity state and bounded
replay before application wiring. `upstox_recorder` now writes its authoritative
raw journal through `DurableIngestor`. The FYERS SDK recorder keeps its own
Python hash-chain journal as the capture authority; `apps/fyers-importer`
verifies a completed session's manifest and chain and imports it offline into
the same raw journal. No live FYERS bridge, provider entitlement verification
or canonical event projection is claimed.
