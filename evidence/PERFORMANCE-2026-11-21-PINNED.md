# Pinned-interpreter benchmark re-run — 2026-11-21

Dated evidence for the qualified performance baseline requested after review of
`evidence/PERFORMANCE-2026-11-21.md`. That earlier record is a real measurement,
but it used Python 3.15.2 on macOS from a topic branch and its five-repeat tail
percentiles could not support p99.9. This record supersedes it as the local
pinned-environment baseline. It is still not a Linux, exchange-feed, broker or
HFT result.

## Environment

| Field | Value |
|---|---|
| Commit | `0a726a6b4b2a2b78220a68fffcf5c0f737173235` (clean `main` after PR #107) |
| Python | 3.12.13 (`requires-python >=3.12`; constraints pin the research environment) |
| Rust | rustc/cargo 1.99.1 |
| Hardware | Apple M1, 8 cores, Darwin 27.0.0 arm64 |
| Workspace | clean tracked tree; opt-in bench targets only, outside `make check` and CI |

## Re-run commands and results

All commands exited 0 after the test-harness policy amplitudes were corrected.
The correction is confined to `crates/risk-authority/tests/performance.rs`; no
production risk limit was changed.

### Research (100 repeats, sizes 200 and 800)

```
PYTHONPATH=research/python/src python3 -m quantlab.bench --suite core --repeats 100 --sizes 200,800 --seed 7
```

Size 800, 100 repeats: BUILD p50/p99/p999 28.529/42.135/61.171 ms;
WRITE 13.765/24.214/29.796 ms; VERIFY 25.827/52.968/54.964 ms;
BACKTEST 3.591/11.800/82.744 ms; PORTFOLIO 3.091/7.406/17.916 ms;
ALPHA 70.260/214.893/312.021 ms; peak RSS 45,924,352 B.
`BENCH-RESEARCH-OUTPUT` reports `deterministic=true` for both sizes with
output SHA-256 `6453f36c9043d7b6f08078432cab66298305bf287806d323b2df53469ed0448d`
and dataset SHA-256 `1454ddadc63fe128db2f2a99fe6baf364c09821f78ba5bb6bd0b1056e641e981`.

### Concurrent experiments (100 repeats, 4 workers)

```
PYTHONPATH=research/python/src python3 -m quantlab.bench --suite experiments --workers 4 --repeats 100 --size 800 --seed 7
```

400 worker samples: p50/p99/p999 111.023/243.594/560.381 ms;
13.463/s; max worker RSS 44,400,640 B; output SHA-256
`a0e5944e5e056802cb8483f64cf60e024d14777c2d8e5a4211dd186ca476cf8d`.

### Latency (1200 repeats — enough observations for p99.9)

```
PYTHONPATH=research/python/src python3 -m quantlab.bench --suite latency --repeats 1200 --size 800 --seed 7
```

`BENCH-HISTORICAL-QUERY`: 1600 lookups, p50/p99/p999 1.041/23.916/958.958 µs,
output SHA-256 `a181f22c211284a17e7991750cecf7949b89f89f95faa8670c9fe397a3190572`.

`BENCH-E2E-LATENCY`: 800 samples, p50/p99/p999 46,108.334/70,165.625/97,403.791 µs,
output SHA-256 `05eda5dee60270dd207dec33283ba9f865dafab9e9ce3bea8c1688d2c644288f`.

### Ingress

`make bench-ingress`: append 2003 frames / 2000 committed / 3 dropped,
245.7 frames/s, p50/p99/p999 3991.0/8099.9/10508.7 µs;
replay 100,000 frames at 94,910.5 frames/s; recovery 0.005 s.

### Risk/OMS

`make bench-risk-oms`: 4000 intents / 8 workers / 4000 decisions,
31,766.5 authorizations/s, p50/p99/p999 33.2/4359.5/11430.0 µs;
OMS duplicate handling 15,996.4/s; reconciliation 64/64 complete, 50.3/s;
recovery 201/201 commits at 161.0/s with one injected `SyncFailure`.

## Log digests (SHA-256)

| Log | Digest |
|---|---|
| `research.log` | `bd6398e5fce083259202ebb9f6d85902d84fde89b93e75db903e23c1415f9d33` |
| `experiments.log` | `140361eb3f0df57f510b265eb982e9eb07c6d05e5cf709bac9aedcd26de75997` |
| `latency.log` | `1635b613d8ac97eb267d22f9d2d4982e40a3eae963ded3d0ed4d0e7d131c124d` |
| `ingress.log` | `ad151c5a10b51e3492b1a3f215a744330833fba02049ef5858231295af2176cc` |
| `risk-oms.log` | `1783747ceda02f208e01160a3b43d33bd537cf7c89e1f7aeff442b6fb9cdacdf` |

## Scope limits

These are local synthetic-fixture measurements on one macOS host. They are not
pinned-container, Linux deployment, exchange-feed, broker, real-market or HFT
latency evidence. No threshold is asserted and the benchmarks remain outside
CI pass/fail gates. The earlier five-repeat Python 3.15 record remains a valid
historical measurement but is not the pinned baseline.
