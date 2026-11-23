# Research compute, scheduled jobs and validation reports

**Status:** local opt-in manifest runner and integrity-only validation report
implemented; all scheduling, distributed, allocation, licensed-data and
target-infrastructure items remain owner-gated and are explicitly absent.
**Owner:** research. **Applies to:** `configs/research/jobs.v1.json`,
`schemas/research/research-job-manifest.v1.schema.json`,
`quantlab.research_jobs`, `quantlab.validation_report`.

## 1. Implemented local opt-in slice

`configs/research/jobs.v1.json` declares the environment pin (code digest,
constraint file and code paths) and the job shape through
`schemas/research/research-job-manifest.v1.schema.json` (Draft 2020-12). The
synthetic job entry bound to `research-job-synthetic-nse-session` was retired in
the W2.1 residual closure; the manifest now declares no runnable job. The
`code_digest` pin was recomputed under the versioned, path-safe code-manifest
rule in `quantlab.experiments.reproducibility` and is recorded with its commit
pin limitations in `evidence/CODE-MANIFEST-INTEGRITY-NIRNAY-2026-11-24.md`.
The runner recomputes the manifest from disk before any dataset is admitted.

`quantlab.research_jobs` validates the manifest, refuses an unpinned environment
or a stale dataset digest, and executes the allowlisted command (`verify-dataset`)
in dry-run mode only against an already admitted, externally published dataset
directory. It builds no dataset and generates no market observation; a manifest
with no job refuses to run. It appends one structured run record (commit,
command, exit, duration, hashes) through the existing append-only
`ExperimentRegistry`. `quantlab validation-report` reads a frozen record and its
published dataset and emits a deterministic, byte-stable JSON report whose
digest recomputes from the report body. It reuses `experiments/verify.py` chain
and dataset-binding checks and the `reporting/comparison.py` digest/summary
surface. Under D11 the generated-dataset integration tests were removed; pure
manifest and digest tests remain active. Both paths are local and opt-in. They
are **implemented, not verified**: the reports assert internal integrity only
and make no latency, throughput, readiness, data-rights or alpha claim.

## 2. BLOCKED_OWNER — by design left unimplemented

The following gaps require an owner ADR, authorization, entitlement, joint
decision or target infrastructure. No agent should implement them by editing
this manifest or adding a service in a follow-up without that gate.

| Gap | Why it is blocked | Required owner action |
|---|---|---|
| Distributed experiment execution | A new compute service and contract are required; the repo has process-local `ProcessPoolExecutor` only | Owner ADR naming the distributed execution contract, trust boundary and measured reason |
| Multi-user resource allocation / quotas | Same as distributed execution; a static `--workers` flag is not an allocator | Owner ADR for allocation policy, tenancy and accounting |
| Scheduled research jobs (daemon, cron, Airflow-style orchestrator) | Running a scheduling service or granting compute is outside the opt-in local runner | Owner authorization for the scheduler service, compute budget and on-call ownership; the manifest's `schedule_hint` is metadata, not a scheduler |
| Real feature store (online/offline serving) | A new database/service is needed; `FeatureLineage` is declarative data only | Owner contract, service boundary and measured reason (CAP-034 remains declarative) |
| Per-experiment pinned container environment | New build/container decision; pinning here is repo-global through the constraint file and toolchain | Owner decision on container build, registry and image provenance |
| Licensed institutional history, complete universes, delisted names, historical corporate actions and corrections | No rights granted; all source products are tri-state UNKNOWN (`CAP-028`) | Written entitlement and entitlement verification from the data owner |
| Shared instrument IDs, event schemas, timestamp/provenance and quality standards with Fieldbook | Joint cross-project decision; Fieldbook is explicitly untouched | Joint Fieldbook/Nirnay agreement; no Fieldbook code or data may be added here |
| Target-infrastructure performance measurement | No deployment or target infrastructure is authorized | Named target environment, deployment authorization and a new dated evidence file |

## 3. Boundary

A frozen experiment record plus a published dataset can prove internal
integrity: append order, digests, dataset binding and an optional trusted head
signature. It cannot prove source completeness, data rights, model correctness,
profitability or live readiness. Historical performance records are dated local
observations of retired synthetic harnesses; the remaining opt-in Rust `BENCH-*`
lines are not gates or certificates.
