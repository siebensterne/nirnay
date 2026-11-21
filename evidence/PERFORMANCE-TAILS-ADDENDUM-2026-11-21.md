# Performance tail-statistics addendum — 2026-11-21

Addendum to `evidence/PERFORMANCE-2026-11-21-PINNED.md`. The original record is
unchanged. This addendum responds to the review finding that 1,200 repeats do
not support a precise p99.9 estimate and that the ingress benchmark was not
lossless.

## Additional independent runs

Three independent runs were executed under Python 3.12.13 on the same Apple M1
host, using a different seed per run. Each used `--suite latency --repeats 1200
--size 800`. Commands are in the pinned evidence document; only the seed
changed. No explicit warm-up phase was used; load average at measurement time
was 32.71 / 17.10 / 9.27, memory 8 GiB, no CPU pinning.

| Seed | Historical query p50 / p99 / p999 (µs) | E2E p50 / p99 / p999 (µs) | Peak RSS (B) |
|---|---|---|---|
| 8 | 1.584 / 10.584 / 109.000 | 73,291.458 / 145,388.083 / 190,606.875 | 44,613,632 |
| 9 | 1.584 / 8.459 / 48.917 | 70,793.833 / 91,441.667 / 142,812.000 | 45,481,984 |
| 10 | 1.584 / 15.500 / 159.458 | 71,448.500 / 568,686.291 / 1,150,723.708 | 45,219,840 |

Historical-query sample count per run: 1,600 lookups. End-to-end sample count
per run: 800. At p99.9 the E2E series has far below one expected observation in
the upper 0.1% tail; the run-to-run p999 spread (142,812 to 1,150,724 µs) shows
the estimate is unstable. A p99.9 estimate with roughly 100 tail observations
would need about 100,000 independent samples, plus repeat-run variation and a
confidence interval. That was not performed.

**Interpretation:** p50 and p99 are reported above with their sample counts.
p99.9 is reported only as an unstable tail estimate. No latency threshold,
capacity claim or HFT claim is granted. Target-host and pinned-container runs
remain outstanding.

## Ingress loss and continuity

The ingress benchmark recorded `ingested=2003 committed=2000 dropped=3`, with
`gaps=1 malformed=1 out_of_order=0`. In the harness, `dropped` is the sum of
conflicts rejected, malformed rejected and duplicates skipped; it is not a
queue-overflow count. The result is therefore **not lossless ingress**. The
benchmark does not measure the effect of those rejections on continuity state
or demonstrate that the risk authority blocked new risk as a consequence;
those are correctness properties covered by separate continuity/replay and
health-to-risk tests, not by this throughput run.

## Scope

Local synthetic workloads on one macOS host, Rust 1.99.1 and Python 3.12.13.
Not Linux, not exchange-feed, not broker, not production, not HFT. Benchmarks
remain opt-in and outside CI pass/fail gates.

## Log digests (SHA-256)

| Log | Digest |
|---|---|
| `latency-run1.log` | `80ceb79c92e08df9632d2b48eb9a697302fce1a3c72c2676f50640eaa90ef56e` |
| `latency-run2.log` | `7ca9d7125b12973430b1e76f1a7fb9713ab04e7c5f86e3f6390ca463e3d40748` |
| `latency-run3.log` | `f3d9dd63284c435845ba8120cc779ea99dec5a1215014717db7d20debd7c2149` |
