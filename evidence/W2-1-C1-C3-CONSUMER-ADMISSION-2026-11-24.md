# W2.1 steps 7-9 C1-C3 consumer admission conformance — 2026-11-24

<!-- SPDX-License-Identifier: Apache-2.0 -->

- **Status:** Implemented at declaration level; not Verified. No hosted CI run
  is claimed for this worktree.
- **Scope:** C1-C3 consumer admission conformance for Nirnay against the
  approved NDC v2 consumer requirements and the approved C0 semantic model.
  The separate `ndc-consumer-use-v2-proposed.md` stays Proposed; this record
  neither ratifies nor amends it.
- **Worktree:** `nv-w21c1`, branch `feat/w21-c1`, base commit `92ee49b`.
  No commit, push, rebase or history rewrite was made in this worktree.
- **Boundary:** evaluator reads named declarations, grants and release
  evidence. No signature check, canonical-byte recomputation, file-digest
  comparison or event-envelope inspection is performed. No final NDC schema,
  transport or storage choice is selected.

## Files changed

| Path | Change |
|---|---|
| `research/python/src/quantlab/consumer_use.py` | additive W2.1 preflight layer: six time axes, exact pair coverage, four capability facts, 32 typed codes, durable `PreflightDecision` |
| `research/python/tests/test_consumer_preflight.py` | new abstract scalar suite; no market observation, fixture, provider call, dataset or experiment |
| `evidence/W2-1-C1-C3-CONSUMER-ADMISSION-2026-11-24.md` | this dated record |
| `README.md` | one document-map row for the new dated evidence; no current-state edit |

## Invariants enforced

- **C1:** `data_as_of`, `source_data_time`, `producer_operation_time`,
  `decision_at`, `release_available_at` and `knowledge_time` are separate typed
  axes. Grant operational validity is evaluated only at `decision_at`. A 2024
  selection cannot revive a grant that expired in 2025 for a 2026 decision.
  Written post-expiry conditions stay on a separate grant field and do not
  extend validity.
- **C2:** each profile exposes `R_p` as exact `(action, object-kind)` pairs.
  A `RequestedUse` binds action, object kind, object identity, object version
  and scope. Every required pair needs a unique applicable `PERMIT` grant
  matching actor, object identity, action, source, dataset, release, recipient
  class and named recipient, licensee, purpose, territory, usage mode,
  delivery method, subject sets, data time, retention and operational validity.
  Conflict resolution is order-independent: no grant rejects, explicit `DENY`
  rejects, multiple applicable grants reject, and grants for other objects or
  unrequested actions are irrelevant. No "most specific wins" rule is used;
  simultaneous dimension failures use a fixed declared precedence.
- **C3:** source, dataset, release, actor, object, territory, usage mode,
  delivery method, licensee, recipient class and identity, purpose, data time
  and operational validity are bound on the request and grant. Four capability
  facts stay separate. Nirnay declares only its profile requirement; producer
  declarations and release availability come from release evidence; a
  non-empty verified-capability fact is refused because this stage performs no
  verification.
- Every refusal carries a typed `PreflightRejectionCode`. `PreflightDecision`
  is immutable, has a deterministic content-derived id, and records
  `is_admission_receipt=False` and `release_verified=False`.

## Commands and results

| Command | Exit | Result |
|---|---|---|
| `python3 -m pytest -q research/python/tests/test_consumer_use.py` | 0 | 41 passed; earlier model unchanged |
| `python3 -m pytest -q research/python/tests/test_consumer_preflight.py` | 0 | 36 passed |
| `make docs-check source-style python-ruff-style prose-style` | 0 | all binding gates pass; unified prose audit reports advisory review candidates only |
| `make check` | 0 | Rust 129 passed; Python 345 passed, 182 deselected, 2 subtests passed; contracts 56 JSON, 43 TOML, 25 schemas |

The prior Python baseline was 309 passed, 182 deselected, 2 subtests; the 36
added abstract scalar tests account for the increase. No market-shaped test was
enabled or created.

## Hashes at record time

| Path | SHA-256 |
|---|---|
| `research/python/src/quantlab/consumer_use.py` | `e810dae644742dd8306159b99ec32c59d1b3ac5427fccb6c2dff56cac1e25623` |
| `research/python/tests/test_consumer_preflight.py` | `8baa303296e10c68deb3ebdcd7832b5b63120393da75c55a7498a2897dafdb10` |

## Invariants and non-goals

- All source-product rights remain `UNKNOWN`; dataset acceptance stays BLOCKED.
- No provider was contacted; no market data, release, dataset, data object,
  synthetic observation or experiment was created. No live-order path was
  touched, and no entitlement, capability record or claim record was changed.
- The legacy `evaluate_consumer_use` declaration model is unchanged; the new
  entry point is `preflight_consumer_admission`.
- No CLI path, research job, registry, schema, transport or storage choice
  consumes the new model. This record does not promote Implemented to
  Verified.

## Handover note

The README `CURRENT-STATE` block keeps its existing `code_commit`. The parent
updates that field in the final prose commit once the implementation commit
exists, with the exact commit, command, exit and CI state.
