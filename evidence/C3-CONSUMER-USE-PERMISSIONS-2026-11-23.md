# C3 consumer-use and permission-evaluation record — 2026-11-23

<!-- SPDX-License-Identifier: Apache-2.0 -->

- **Status:** Implemented at declaration level; not Verified. No hosted CI run
  is claimed for this worktree.
- **Scope:** the C3 consumer-use profile model, AdmissionRequest structure,
  permission evaluation and typed rejection codes of the proposed NDC v2
  semantic model. No NDC schema is finalized.
- **Worktree:** `/private/var/folders/7c/m_f7hs9n445174c0x3gsy5kw0000gn/T/opencode/nv-w12c3`,
  branch `fix/w12-c3-consumer`, base commit `d73e145`. No commit or push was
  made in this worktree.

## Files changed

| Path | Change |
|---|---|
| `research/python/src/quantlab/consumer_use.py` | new versioned five-profile model, AdmissionRequest, grants, evaluation and 11 typed rejection codes |
| `research/python/tests/test_consumer_use.py` | new abstract scalar tests; no market observation, fixture or experiment path |
| `research/python/src/quantlab/dataset_admission.py` | module documentation only: explicit statement that this gate is not a full release verifier |
| `engineering/ndc-consumer-use-v2-proposed.md` | new proposed model document |
| `evidence/C3-CONSUMER-USE-PERMISSIONS-2026-11-23.md` | this dated record |
| `README.md` | two document-map rows for the new documents; no current-state edit |

## Commands and results

| Command | Exit | Result |
|---|---|---|
| `make check` before edits | 0 | Python 251 passed, 199 deselected, 2 subtests passed; Rust 73 passed plus 1 doc test |
| `python3 -m pytest -q research/python/tests/test_consumer_use.py` | 0 | 41 passed |
| `make docs-check source-style python-ruff-style prose-style` | 0 | all binding gates pass; prose audit reports advisory review candidates only |
| `make check` after edits | 0 | Python 292 passed, 199 deselected, 2 subtests passed; Rust 73 passed plus 1 doc test |
| `cargo test --doc --workspace --locked` | 0 | 1 passed, 24 targets with 0 tests |

The W1.1-B Python baseline (251 passed, 199 deselected, 2 subtests) and the
W1.2-B Rust baseline (73 plus 1 doc test) are unchanged apart from 41 added
abstract scalar tests. No market-shaped test was enabled or created.

## Invariants and non-goals

- `CONSUMER_OPERATIONS` remains the exact five-value set. Derived-data creation
  is a separate requested action with its own required grant; retention alone
  does not authorize it.
- Every requested action is evaluated against a unique matching `PERMIT` grant.
  A missing grant, a denial, an identity mismatch or an ambiguous grant blocks
  admission. Extra grants do not authorize unrequested actions.
- The declared purpose is `proprietary_own_account_trading_research` only. No
  generic purpose, recipient, licensee or scope default is supplied.
- All 21 source-product rights remain `UNKNOWN`. No provider contact, market
  data, release, research dataset, data object, synthetic observation or
  experiment was created. No live-order path is touched.
- `dataset_admission.py` validation logic is unchanged; only its module
  documentation was extended. No CLI path consumes the new model.
- Full detached-signature verification, canonical-byte recomputation, key
  validity and revocation checks, and actual artifact-digest checks remain a
  separate implementation stage. The new evaluator checks named declaration
  receipts and is not a release verifier.

## Handover note

The README `CURRENT-STATE` block is deliberately left at its existing
`code_commit`. The parent must update that field in a final prose commit after
the implementation commit exists, with the exact commit, command, exit and CI
state. This record does not promote Implemented to Verified.
