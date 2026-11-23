# Provider Python environments and ingestion ownership

**Status:** environment isolation enforced by CI on 2026-11-13; ingestion
ownership metadata frozen under RRP-004 and consolidated under RRP-005 on
2026-11-23.
**Scope:** `research/python`, `apps/kite-ops`, `apps/fyers-recorder`, and the
frozen source-ingestion paths listed below.

The three Python distributions do not share one dependency environment. This is
deliberate and is a release gate, not an installer inconvenience. Provider
acquisition paths in Nirnay are frozen and owned by Fieldbook; see the second
half of this document.

## The conflict

| Distribution | Requirement |
|---|---|
| `nirnay-kite-ops` | `requests>=2.32,<3` |
| `nirnay-fyers-recorder` | `fyers-apiv3>=3.1.0`; the current SDK release (`3.2.1`) pins `requests==2.31.0` |

`requests==2.31.0` and `requests>=2.32,<3` cannot both be satisfied. Installing
both provider packages into one virtual environment makes `pip check` fail:

```text
nirnay-kite-ops 0.4.0 has requirement requests<3,>=2.32, but you have requests 2.31.0.
```

Do not relax either bound to hide this. Use isolated environments and test the
combined deployment boundary explicitly.

## Supported environments

Run each command from the repository root.

### Research

```bash
python3 -m venv .venv-research
. .venv-research/bin/activate
python -m pip install -e 'research/python[dev,research,security]'
python -m pytest -q research/python/tests scripts
python scripts/validate_contracts.py
```

`research` includes PyArrow. The Parquet round-trip is a required test, not an
optional skip.

### Kite operations (read-only)

```bash
python3 -m venv .venv-kite
. .venv-kite/bin/activate
python -m pip install -e 'apps/kite-ops[test]'
python -m pip check
python -m pytest -q apps/kite-ops/tests
```

### FYERS recorder

```bash
python3 -m venv .venv-fyers
. .venv-fyers/bin/activate
python -m pip install -e 'apps/fyers-recorder[test]'
python -m pip check
python -m pytest -q apps/fyers-recorder/tests
```

The FYERS SDK is imported only when a live socket is started; the unit tests
use a fake socket. The recorder source is a frozen migration item; no new socket,
callback schema or recorder output may be added under RRP-004.

## Combined boundary

The combined install remains unsupported until the FYERS SDK removes
its exact `requests==2.31.0` pin or a compatible release is tested.

CI runs a canary (`provider-boundary`) that performs:

```bash
python -m pip install --dry-run -e apps/fyers-recorder -e apps/kite-ops
```

The job passes only while the known conflict is present. If the install starts
resolving, CI fails and requires this document, the isolation jobs and the
dependency bounds to be revisited together. This prevents a silent change from
turning a documented incompatibility into an untested deployment assumption.

## CI mapping

| CI job | Environment | Proof |
|---|---|---|
| `python-research` | research only | PyArrow import, Parquet round-trip, research tests, schema validation |
| `python-provider` (`kite`) | Kite only | `pip check`, Kite tests |
| `python-provider` (`fyers`) | FYERS only | `pip check`, FYERS tests |
| `provider-boundary` | both, dry-run | Known conflict is still present |

No provider credential, Keychain item or live session is used by these jobs.

## Frozen source-ingestion paths and Fieldbook ownership

NR-ARCH-ADR-0001 (Accepted 2026-11-23) assigns source acquisition, raw storage,
normalization, point-in-time reference data and the raw-data lifecycle to the
future complementary platform, Fieldbook. Nirnay owns research, experiments,
simulation, portfolio, risk and eventual execution under separate authorization.
Nirnay runs no second collector and holds no competing raw archive.

The following Nirnay paths are frozen metadata-only under RRP-004. This section
is an ownership index; the full responsibility, no-new-feature rule and
migration order for each path are in `evidence/SOURCE-INGESTION-FREEZE-2026-11-23.md`
and `engineering/source-ingestion-freeze.v1.json`.

| Frozen path | Frozen responsibility | Fieldbook target owner | Replacement condition |
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

Binding rules:

1. `make freeze-check` runs `scripts/check_source_ingestion_freeze.py` with its
   self-test and is part of `make check` through `check-static`. A change to a
   frozen path needs an approved migration record naming an evidence file, the
   owner, the replacement commit and the decision date. No such record exists at
   this base.
2. Migration order: Fieldbook adapter, then NDC projection, then verification,
   then removal of the old importer or archive. No stage softens an earlier
   refusal. No bulk `git rm -r` and no history rewrite.
3. No new feature, provider, provider map, watchlist, collection path, decoder,
   archive writer or reference authority may be added under a frozen path.
   Fixes that keep existing behavior and documentation metadata updates still
   need an approved migration record to pass the gate. No replacement synthetic
   market observation may be created under these paths.
4. Rights, acceptance and experiment state are unchanged: every source product
   is `UNKNOWN`, dataset acceptance is `BLOCKED`, provider collection is
   `DISABLED`, experiment registration is blocked and live order submission is
   disabled. A checksum or a successful connection grants no right.
5. The historical real-source observation record in
   `evidence/M3-1-REAL-SOURCE-STATUS-2026-11-20.md` is dated evidence and is not
   edited by this rewrite. No command in this document contacts a provider or
   authorizes capture.

## KDB-X analysis (research plane, no ingestion)

KDB-X 5.0 is installed at `/Users/tourist/.kx/bin/q`. The checked-in q contract
is `analytics/kdb/mbp_metrics.q`; the repo-native harness is `apps/kdb-lab`
(`cargo run -p kdb-lab --bin nirnay-kdb`) and is driven by `make kdb-analyze`.
It validates the header-prefixed CSV input, runs q with absolute paths, parses
the typed `metric,value` output, and writes `metrics.csv` plus a digest-bearing
`report.json` (schema `schemas/analytics/kdb-metrics-report.v1.schema.json`).
D11 retired the synthetic golden corpus, its `check-golden` command and the
golden-dependent tests; only analysis of an explicitly supplied input remains.

The q contract accepts one source at a time and never merges books. Price and
spread metrics use the input's declared price unit; no metric name implies
paise. Source decoding and raw capture are Fieldbook-owned frozen
responsibilities; q analysis is a Nirnay research plane over an already admitted
input. The 2026-11-20 real Upstox result (36 rows, 87.07 s, 0 capture gaps,
mean spread 0.0219 rupees, mean imbalance -0.3034, minimum depth 5 levels) is a
dated observation recorded in
`evidence/M3-1-REAL-SOURCE-STATUS-2026-11-20.md`, not a current capability claim.
