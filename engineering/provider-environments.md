# Provider Python environments

**Status:** enforced by CI on 2026-11-13.  
**Scope:** `research/python`, `apps/kite-ops`, `apps/fyers-recorder`.

The three Python distributions do not share one dependency environment. This is
deliberate and is a release gate, not an installer inconvenience.

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
use a fake socket.

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

## Live capture runbook (owner-local, read-only)

Verified on 2026-11-20 after the M3.1 programme. All commands run from the
repository root. No command in this section places an order or writes to a
broker.

### Credentials in Keychain (metadata only)

| Service | Purpose |
|---|---|
| `NIRNAY_UPSTOX_ANALYTICS_TOKEN` | Upstox V3 market-data feed, read-only |
| `NIRNAY_FYERS_APP_ID`, `NIRNAY_FYERS_APP_SECRET` | FYERS v3 app, used only to mint a token |
| `NIRNAY_FYERS_ACCESS_TOKEN` | FYERS Data Socket access token (short-lived) |
| `NIRNAY_KITE_API_KEY`, `NIRNAY_KITE_API_SECRET` | Kite login inputs; no order capability in Nirnay |

Never print these values. The recorder reads them into the process environment.

### Provider status

- **Upstox: READY.** WebSocket smoke connected and decoded 2/2 frames with 0
  errors on 2026-11-20. Real 90-second Full capture produced 37 frames, 0
  decode errors and 36 five-level observations. Token expires 2027-11-09.
- **FYERS: credential-ready, market-timing blocked.** `profile` and `depth`
  REST calls return `s=ok`; the Data Socket capture on 2026-11-20 evening
  produced zero callbacks because NSE was `CLOSED`/`POSTCLOSE_CLOSED`. The
  access token expires overnight, so regenerate it on the morning of capture
  with the operator-local v3 helper under `.local/perf/`.
- **Zerodha: login inputs present, daily token missing.** Generate the Kite
  daily access token from the dashboard before any read-only reconciliation.
  Nirnay keeps live order submission disabled.

### Monday sequence

1. Confirm NSE is open (09:15-15:30 IST). The FYERS token must be freshly
   minted that morning; Upstox remains valid.
2. Run the bounded read-only recorders into `.local/live-sessions/YYYY-MM-DD/`.
   Raw journals and manifests stay outside Git and outside the source tree.
3. Decode and project each session with the existing tools:
   `upstox-decode-export`, `upstox-kernel-session`, `session-replay`.
   FYERS callbacks use the committed `fyers-importer` typed projection.
4. Run `session-replay` over both sessions to produce per-source books and the
   cross-source reconciliation report. Never merge or sum displayed depth.
5. Results are written under `.local/perf/`; the full evidence record is
   `evidence/M3-1-REAL-SOURCE-STATUS-2026-11-20.md`.

### KDB-X analysis

KDB-X 5.0 is installed at `/Users/tourist/.kx/bin/q`. The checked-in q contract
is `analytics/kdb/mbp_metrics.q`; the repo-native harness is `apps/kdb-lab`
(`cargo run -p kdb-lab --bin nirnay-kdb`) and is driven by `make kdb-analyze`.
It validates the header-prefixed CSV input, runs q with absolute paths, parses
the typed `metric,value` output, and writes `metrics.csv` plus a digest-bearing
`report.json` (schema `schemas/analytics/kdb-metrics-report.v1.schema.json`).
D11 retired the synthetic golden corpus, its `check-golden` command and the
golden-dependent tests; only analysis of an explicitly supplied input remains.

The q contract accepts one source at a time and never merges books. Price and
spread metrics use the input price unit (rupees as exported by
`upstox-decode-export`); no metric name implies paise. 2026-11-20 real Upstox
result: 36 rows, 87.07 s, 0 capture gaps, mean spread 0.0219 rupees, mean
imbalance -0.3034, minimum depth 5 levels.
