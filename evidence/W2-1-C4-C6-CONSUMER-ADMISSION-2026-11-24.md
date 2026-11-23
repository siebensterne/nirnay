# W2.1 steps 10-12 C4-C6 consumer admission completion — 2026-11-24

<!-- SPDX-License-Identifier: Apache-2.0 -->

- **Status:** Implemented at declaration level; not Verified. No hosted CI run
  is claimed for this worktree.
- **Scope:** C4 typed retention, C5 per-dimension coverage and C6 preflight
  separation for Nirnay against the approved `ndc-consumer-requirements-v2-proposed.md`
  consumer requirements. `engineering/ndc-consumer-use-v2-proposed.md` stays
  separately Proposed; this record neither ratifies nor amends it.
- **Worktree:** `nv-w21c4`, branch `feat/w21-c4`, base commit `a64d4f5`.
  No commit, push, rebase or history rewrite was made in this worktree.
- **Boundary:** this code checks named declarations, grants, typed policies and
  dimension reports. It does not cryptographically verify a signature,
  recompute RFC 8785 canonical bytes, compare an artifact digest with file
  bytes, check key validity or revocation, or inspect an event envelope. A full
  verifier is a later milestone. No schema, transport or storage choice is made.

## Files changed

| Path | Change |
|---|---|
| `research/python/src/quantlab/dataset_admission.py` | shared typed retention (`NoRetention`, `Bounded`, `Indefinite`, `DeleteOnExpiry`, `PurgeAfter`, `LegalHold`) with explicit `retention_covered`; nine-dimension coverage reports and typed certificate validation; explicit non-verifier documentation |
| `research/python/src/quantlab/consumer_use.py` | typed retention and coverage adopted from `dataset_admission`; `evaluate_consumer_use` now returns `PreflightDecision`; 40 typed codes including the C6 distinct refusals; explicit non-verifier documentation |
| `research/python/src/quantlab/research_jobs.py` | `run_job` refuses a `PreflightDecision` (or preflight-kind mapping) in place of an admitted release record |
| `research/python/tests/test_dataset_admission.py` | abstract typed-retention containment and nine-dimension coverage tests |
| `research/python/tests/test_consumer_use.py` | abstract tests for the preflight verdict, distinct codes, typed coverage and purge containment |
| `research/python/tests/test_consumer_preflight.py` | abstract tests for field/modality, weaker purge, declaration codes, incomplete coverage and the research-job guard |
| `evidence/W2-1-C4-C6-CONSUMER-ADMISSION-2026-11-24.md` | this dated record |
| `README.md` | one document-map row for the new dated evidence; no current-state edit |

## Invariants enforced

- **C4:** a retention request carries both a typed duration and a typed purge
  duty. Containment requires `duration <= grant` **and** purge duty no weaker
  than the grant (`DeleteOnExpiry > PurgeAfter > LegalHold`). A shorter duration
  with a weaker purge is rejected. `None` is never interpreted as either
  absent or unlimited; only explicit `NoRetention` and `Indefinite` types carry
  those meanings.
- **C5:** the single `coverage_verdict == PASS` is replaced by nine W2
  dimensions: instrument, listing/identity, contract, corporate action,
  calendar, trades, quotes/depth, MBO and sequence continuity. Each applicable
  dimension declares scope, denominator identity and value, as-of, numerator,
  missing count, rejected count, gaps, verdict and evidence reference. N/A
  carries a reason. An absent or inconsistent denominator yields REJECT or
  UNKNOWN and never PASS. No aggregate percentage is accepted.
- **C6:** `evaluate_consumer_use` produces an explicitly named `PreflightDecision`
  with `kind=PREFLIGHT_DECISION`, `is_admission_receipt=False` and
  `release_verified=False`. `run_job` explicitly refuses that verdict as an
  admission record. Distinct typed codes cover wrong object id/version, wrong
  actor, recipient, purpose and source, out-of-scope field and modality,
  data-time beyond grant, grant expired at decision, weaker purge, missing
  producer capability, incomplete coverage, invalid correction lineage,
  unknown contract version, invalid signature and digest mismatch.
- `dataset_admission.py` owns the shared typed declarations and dataset record
  validation; `consumer_use.py` owns consumer request and grant evaluation.
  Neither module creates a receipt, dataset, release or data object.

## Commands and results

| Command | Exit | Result |
|---|---|---|
| `make check` before edits | 0 | Python 345 passed, 182 deselected, 2 subtests; Rust 129 passed plus 1 doc test |
| targeted C4-C6 tests before edits | 0 | 137 passed |
| `make check` after edits | 0 | Python 383 passed, 182 deselected, 2 subtests; Rust 129 passed; contracts 56 JSON, 43 TOML, 25 schemas |
| `make docs-check source-style python-ruff-style prose-style` | 0 | all binding gates pass; prose audits advisory only |

The Python baseline rose from 345 to 383: 38 new abstract scalar tests. No
market-shaped test was enabled or created. One cargo build-cache repair
(`cargo clean -p libsqlite3-sys`) was needed after a stale `bindgen.rs` output;
no source or lockfile change was involved.

## Hashes at record time

| Path | SHA-256 |
|---|---|
| `research/python/src/quantlab/dataset_admission.py` | `6dc551e89921a2e22c987a028c8f49f08a076b3243a5f3e2560da14f89659010` |
| `research/python/src/quantlab/consumer_use.py` | `d7d786725d0e100d3764b0c9baea8c346942c31a9a6d0678ab091e9fb8769b78` |
| `research/python/src/quantlab/research_jobs.py` | `fcbdb061b490f6bbf179bcd10c7e2b641dad744778fc4a1b043465546813c0ef` |
| `research/python/tests/test_dataset_admission.py` | `1c56b2248be6c7ada5e3606095b25d6f3559114938d62cc37ff5ff5f4dc42ba7` |
| `research/python/tests/test_consumer_use.py` | `fca763b5c20fdc51fae0bf9c0bb1876ccd576ad47959787015b2a09314c310fb` |
| `research/python/tests/test_consumer_preflight.py` | `9f1f3f132c8ace4f467753ba78ea53bd0ce18539fea95dfea0bbce89d5e9ff53` |

## Invariants and non-goals

- All source-product rights remain `UNKNOWN`; dataset acceptance stays BLOCKED.
- No provider was contacted; no market data, release, dataset, data object,
  synthetic observation or experiment was created. No live-order path was
  touched, and no entitlement, capability record or claim record was changed.
- No CLI behaviour change beyond the defensive research-job refusal of a
  preflight verdict. No schema, transport or storage choice was finalized.
- This record does not promote Implemented to Verified.

## Handover note

The README `CURRENT-STATE` block keeps its existing `code_commit`. The parent
updates that field in the final prose commit once the implementation commit
exists, with the exact commit, command, exit and CI state.
