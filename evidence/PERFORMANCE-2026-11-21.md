# Performance evidence — 2026-11-21 (`PERFORMANCE-2026-11-21`)

This record fills `evidence/PERFORMANCE-TEMPLATE.md` with values captured from
actual runs on this working tree. It is a local synthetic observation on one
commit state, not a throughput, latency, capacity or readiness certificate.

Grant rule (verbatim from the template): filling this template grants no latency,
throughput, capacity or readiness claim. Harnesses are opt-in and excluded from
`make check` and CI pass/fail gates. Local synthetic measurements are not
deployment evidence.

## Identity and provenance

| Field | Recorded value |
|---|---|
| Record id | `PERFORMANCE-2026-11-21` |
| Commit | `ec0b216f07e7b7e653e8f784b18c6517d6335696` (base commit; measured tree has the uncommitted changes listed in the environment log) |
| Branch | `feat/repro-evidence` |
| Working tree | Modified: `Makefile`, both Rust `tests/performance.rs` files, `bench.py`, `cli.py`, `comparison.py`, `test_bench.py`, plus new runner, report, tests, schema and job manifest paths. Full `git status --porcelain` in the environment log |
| Toolchain | `rustc 1.99.1 (58cf8f9c3 2026-10-13)`; `cargo 1.99.1 (5f94df478 2026-08-27)`; `Python 3.15.2`; pinned constraint file `constraints/python-research.txt` SHA-256 `e385f9cf1dbd5f06cde53d3cc6939329d03b6dbc1135d6f62e9605f4e4e07996` (records `--python-version 3.12`) |
| Hardware | Host `tourists-MacBook-Air.local`; CPU `Apple M1`; 8 logical / 8 physical CPUs; 8,589,934,592 bytes RAM; macOS arm64, Darwin Kernel 27.0.0 (`xnu-13432.1.9~1/RELEASE_ARM64_T8103`) |
| Environment | Local macOS shell; benchmark process environment is the ambient environment with `PYTHONPATH=research/python/src` set for Python targets. Full `uname`, `sysctl` and `locale` capture in the environment log |
| Date | 2026-11-21 (UTC capture `2026-11-21T04:39:49+00:00`) |

## Commands and outcomes

All five targets exited `0`. Raw output (stdout and stderr combined — the
`BENCH-*` lines are emitted on stderr) is in the listed log files.

| Harness | Exact command | Exit | Repeats | Raw log |
|---|---|---|---|---|
| ingress | `make bench-ingress` | 0 | 2,000 committed frames (single run) | `evidence/performance-2026-11-21-bench-ingress.log` |
| research core | `make bench-research` | 0 | 5 per size (200, 800) | `evidence/performance-2026-11-21-bench-research.log` |
| concurrent experiments | `make bench-experiments` | 0 | 5 rounds x 4 workers | `evidence/performance-2026-11-21-bench-experiments.log` |
| risk / OMS | `make bench-risk-oms` | 0 | 4,000 intents / 200 commits / 2,000 duplicates / 64 reconciliations | `evidence/performance-2026-11-21-bench-risk-oms.log` |
| synthetic latency | `make bench-latency` | 0 | 5 at size 800 | `evidence/performance-2026-11-21-bench-latency.log` |

Failure policy: latency and throughput are recorded, never asserted as gates.
No benchmark target participates in `make check` or CI pass/fail gates.

## Distributions and counters

### Ingress (`make bench-ingress`)

| BENCH line | p50 | p99 | p99.9 | Throughput | Counter provenance |
|---|---|---|---|---|---|
| `BENCH-APPEND` | 4,000.5 µs | 8,043.8 µs | 11,341.0 µs | 244.6 frames/s over 8.178 s | `append_durations` / `frames_per_sec` in `market-archive/tests/performance.rs`; 2,000 committed, 3 dropped, queue depth 2,000, malformed 1, gaps 1, out-of-order 0 from `IngressCheckpoint` |
| `BENCH-APPEND` end-to-end | 4,001.0 µs | 8,044.0 µs | 11,341.0 µs | same run | `e2e_durations` = `unix_nanos()` at commit minus frame receive time |
| `BENCH-REPLAY` | n/a | n/a | n/a | 99,700.7 frames/s over 1.003 s | streamed frames / journal bytes (33,966,685) / peak frames held 1 from `JournalReader` replay |
| `BENCH-RECOVERY` | n/a | n/a | n/a | replay evidence `RECOVERY-COMMITTED 100` | fresh-process child `RECOVERY-COMMITTED` stderr, 0.005 s |

Fault-injection counters on every ingress `BENCH-*` line: `sync_failure_count=0`,
`partial_write_count=0`. The recovery fault is process termination, not a sync
failure or a partial record write; no durable-write fault is injected in the
append or replay workloads.

### Research core (`make bench-research`)

Each latency is the p50 / p99 / p999 over 5 repeats, in milliseconds. Peak RSS
is process high-water `ru_maxrss` from `resource.getrusage`.

| Stage | size=200 p50 / p99 / p999 ms | size=800 p50 / p99 / p999 ms | Peak RSS (200 / 800) |
|---|---|---|---|
| `BUILD` | 7.875 / 10.483 / 10.483 | 25.591 / 25.957 / 25.957 | 36,454,400 / 47,497,216 bytes |
| `WRITE` | 3.506 / 3.577 / 3.577 | 12.031 / 12.729 / 12.729 | 36,454,400 / 47,497,216 bytes |
| `VERIFY` | 6.262 / 6.388 / 6.388 | 25.202 / 26.399 / 26.399 | 36,454,400 / 47,497,216 bytes |
| `BACKTEST` | 0.905 / 0.963 / 0.963 | 3.423 / 3.515 / 3.515 | 36,454,400 / 47,497,216 bytes |
| `PORTFOLIO` | 0.826 / 0.860 / 0.860 | 3.145 / 3.253 / 3.253 | 36,454,400 / 47,497,216 bytes |
| `ALPHA` | 17.427 / 17.583 / 17.583 | 68.309 / 68.854 / 68.854 | 36,454,400 / 47,497,216 bytes |

Counter provenance: each median/percentile comes from the timed stage in
`benchmark_core_once` in `research/python/src/quantlab/bench.py`; the dataset,
backtest, portfolio and alpha digests come from the executed functions, not
from literals. Both `BENCH-RESEARCH-OUTPUT` lines report
`deterministic=true` and `verify_status=verified_internal_integrity`.

### Concurrent experiments (`make bench-experiments`)

| Metric | Value | Provenance |
|---|---|---|
| Worker p50 / p99 / p999 | 84.279 / 91.960 / 91.960 ms | 20 worker samples (5 rounds x 4 workers) in `run_experiments_suite` |
| Throughput | 17.851 experiments/s | `worker_samples / wall_seconds` |
| Max worker RSS | 45,645,824 bytes | `max_worker_rss_bytes` across `ProcessPoolExecutor` workers |
| Integrity | `verified_internal_integrity` | `verify_published_dataset` before and after the concurrent reads |

### Risk / OMS (`make bench-risk-oms`)

| BENCH line | p50 / p99 / p999 | Throughput | Counter provenance |
|---|---|---|---|
| `BENCH-RISK-AUTH` | 31.5 / 3,011.2 / 11,942.1 µs | 34,984.2 authorizations/s | 4,000 accepted / 0 rejected intents over 0.114 s, seed 20261121 |
| `BENCH-OMS-DUPLICATE` | 61.1 / 74.4 / 82.9 µs | 16,038.4 duplicates/s | 2,000 duplicate deliveries, all ignored, one persisted fill |
| `BENCH-OMS-RECON` | 19,994.9 / 23,991.5 / 24,024.8 µs | 50.1 reconciliations/s | 64 injected UNKNOWN orders, 64 resolved, `complete=true` |
| `BENCH-OMS-RECOVERY` | n/a | 174.5 replays/s (201 commits in 1.152 s) | 201/201 replayed after one injected `SyncFailure` |

Fault-injection counters: `BENCH-OMS-RECOVERY` reports
`fault=SyncFailure sync_failure_count=1 partial_write_count=0`; the duplicate
and reconciliation lines report `sync_failure_count=0 partial_write_count=0`
(no durable-write fault injected in those paths).

### Synthetic latency paths (`make bench-latency`)

These are the new opt-in timed paths added for historical-query latency and
end-to-end data latency on synthetic point-in-time fixtures. They emit no gate
or threshold.

| BENCH line | p50 / p99 / p999 | Sample definition | Peak RSS |
|---|---|---|---|
| `BENCH-HISTORICAL-QUERY` | 0.917 / 1.584 / 4.125 µs | 1,600 point-in-time `resolve` / `known_actions` lookups over 800 published rows, 5 repeats | 45,432,832 bytes |
| `BENCH-E2E-LATENCY` | 41,003.542 / 44,142.333 / 44,185.708 µs | 800 ingest-to-consumer deltas (synthetic event build + publish + JSONL read), 5 repeats | 45,498,368 bytes |

## Hashes and integrity

| Artifact | SHA-256 |
|---|---|
| `performance-2026-11-21-bench-ingress.log` | `5e9bddc38dedd48bc0676846a40010e2f738eb781655e2f2fddb8c416a1552b9` |
| `performance-2026-11-21-bench-research.log` | `bf9b26fec280887469c193e9ceaae6e5e8d2cefd09dd2d8d32974107a5989e58` |
| `performance-2026-11-21-bench-experiments.log` | `b867c26d95174e5e59b8d6749d8a4bf901b8c5f8492d57ca90714de2cd2b3cb0` |
| `performance-2026-11-21-bench-risk-oms.log` | `4d083df20165ef6f6059427b654ad6f02d7f10b28293b8bc3ed5fe7269e5324e` |
| `performance-2026-11-21-bench-latency.log` | `9be0592c4991651986ba91d59b0b298adfc0eb491ca362f36b144969c5e773d9` |
| `performance-2026-11-21-environment.log` | `a66f8137a6fb5beda8f5853df26534bb8fdcbee66d56e2681102cb20000b93e2` |
| `performance-2026-11-21-commands.txt` | `d5b8ca9ce61d8f356431209eddecb116ab5d825e886e692dedabfad15261ea39` |
| Exercised code digest (definition below) | `116be6ec65115cf9fa52936656df7ad147e2f2ba778d37e72377b3f3de54a713` |

### Exercised code digest (reproducible definition)

The exercised code digest is an aggregate over exactly these nine tracked
paths (relative to the repository root):

- `Makefile`
- `crates/authoritative-oms/tests/performance.rs`
- `crates/market-archive/tests/performance.rs`
- `crates/risk-authority/tests/performance.rs`
- `research/python/src/quantlab/bench.py`
- `research/python/src/quantlab/cli.py`
- `research/python/src/quantlab/experiments/store.py`
- `research/python/src/quantlab/point_in_time.py`
- `research/python/src/quantlab/reporting/comparison.py`

The digest is SHA-256 of the newline-terminated `sha256sum` lines emitted for
those files in `git ls-files` order. Reproduce from the repository root with:

```bash
git ls-files -z -- Makefile \
  crates/authoritative-oms/tests/performance.rs \
  crates/market-archive/tests/performance.rs \
  crates/risk-authority/tests/performance.rs \
  research/python/src/quantlab/bench.py \
  research/python/src/quantlab/cli.py \
  research/python/src/quantlab/experiments/store.py \
  research/python/src/quantlab/point_in_time.py \
  research/python/src/quantlab/reporting/comparison.py \
  | xargs -0 sha256sum | sha256sum
```

Observed output:
`116be6ec65115cf9fa52936656df7ad147e2f2ba778d37e72377b3f3de54a713  -`.
On systems without `sha256sum`, the equivalent `shasum -a 256` pipeline yields
the same value. Per-file SHA-256 values, the exact command and this note are
also recorded in `performance-2026-11-21-environment.log` and
`performance-2026-11-21-commands.txt`. This disclosure replaces an earlier
unpublished aggregate that a reviewer could not reproduce; no benchmark
measurement, raw `BENCH-*` line or benchmark log hash was changed.

Research output hashes (from `BENCH-RESEARCH-OUTPUT`): size 200
`output_sha256=3b3721bc4c489d8a1b7e56ab61731a56d90935f25987f2960f45624bf46ff876`,
`dataset_sha256=53fd9c411cf03d5d00295a2bb3876994268080ea5539acc06a7778a093522d42`;
size 800
`output_sha256=ed9d4b6274578f50e02c1390d872b6b8b1d7a971ff343b7d5c024023f93d190f`,
`dataset_sha256=1454ddadc63fe128db2f2a99fe6baf364c09821f78ba5bb6bd0b1056e641e981`.
Concurrent experiments `output_sha256=`:
`c92c920cc7b88a01f5a3ff9af30fa6f193724bc1e2388cf7f3c9c08d7782552f`.
Synthetic latency `output_sha256=a181f22c211284a17e7991750cecf7949b89f89d95faa8670c9fe397a3190572`
(historical query) and
`05eda5dee60270dd207dec33283ba9f865dafab9e9ce3bea8c1688d2c644288f` (end-to-end).
Determinism: each harness asserted identical output digests across every
repeat; no mismatch was observed.

## Scope limits (copy verbatim unless a narrower limit applies)

- All fixtures are synthetic and local; no licensed provider feed was used.
- Measured latencies include local process and filesystem overhead.
- No broker, deployed risk service, live adapter or target infrastructure was involved.
- No rights, entitlement, profitability or live-readiness claim is implied.
- The `BENCH-*` lines are observations, not thresholds, certificates or gates.

Additional boundary notes specific to this run: the Python harness executed on
the host interpreter `Python 3.15.2`, while the repository constraint file pins
Python 3.12 for CI; no Python-3.12-only run was performed. The measured tree is
the base commit plus the uncommitted implementation described in the
environment log; it is not a claim about any other commit. Target-infrastructure
measurement remains owner-gated and unimplemented.

## Acceptance checklist

- [x] Commit, branch, toolchain and hardware fields are filled from the run.
- [x] Command, exit status and repeat count are recorded verbatim.
- [x] p50, p99 and p99.9 are present for each reported latency.
- [x] Every counter names its measured source; no literal stands in for a counter.
- [x] Output and dataset hashes matched across all repeats.
- [x] README document map points at this dated file when it is published.
