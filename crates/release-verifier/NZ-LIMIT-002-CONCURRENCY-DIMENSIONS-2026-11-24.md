<!-- SPDX-License-Identifier: Apache-2.0 -->

# NZ-LIMIT-002 concurrency dimensions — dated record

Date: 2026-11-24 work-package basis; code completed 2026-11-25 on branch
`fix/nz-limit-002`, base `c24bc29`. No commit is created by this record.

## Confirmed defect and scope

`check_artifact_declarations_streaming` compared the declared artifact total
against both `max_artifacts` and `max_concurrent_artifacts`. A sequential
verifier with four artifacts, `max_artifacts=4` and
`max_concurrent_artifacts=1` was refused at stage five. Total inventory and
in-flight work are separate dimensions.

Changed files: `crates/release-verifier/src/verifier.rs`,
`crates/release-verifier/src/lib.rs`,
`crates/release-verifier/tests/b16_streaming.rs` and this record. No provider,
right, data, experiment, India artefact or live route is changed. The freeze is
unchanged: rights `UNKNOWN`, dataset acceptance `BLOCKED`, budget INR 0,
collection `DISABLED`, experiments `BLOCKED`, live execution `DISABLED`, D11
strict and `PUBLICATION_DURABILITY_VERIFIED` false. The twelve stages, trusted
profile resolver, payload registry, parser bounds, snapshot cleanup with
`CLEANUP_PENDING` and the stage-six rights refusal are unchanged.

## Delivered code

- The declaration gate checks the total against `max_artifacts` only.
  `u64::try_from` guards the count conversion; an empty inventory stays a
  refusal.
- `InFlightArtifacts` is a checked typed counter with `try_acquire` and
  `try_release`. A request made while the count equals the limit is refused.
  Both transitions are checked, with typed `LimitReached`, `Overflow` and
  `Underflow` errors mapping to `REJECT_RESOURCE_LIMIT` naming
  `max_concurrent_artifacts`.
- Stage five acquires one slot before each artifact verification and releases
  it after verification ends, on success and on refusal. Explicit assertions
  record the sequential invariant: no slot stays open between artifacts and
  the count is one while an artifact runs. No parallel verification is added.
- The future concurrency contract is documented on the type: a scheduler must
  acquire before an artifact starts and release after it ends.

## Tests

Seventeen tests were added: twelve verifier unit tests and five streaming
integration tests. They cover the four-artifact pass with one flight slot, the
fifth-artifact total-count refusal with cleanup, the stage-six rights refusal
after four artifacts, twelve observed stages, the empty inventory, acquire and
release, refusal at the limit, refusal at a zero limit, typed underflow on
release at zero, typed overflow at the counter ceiling, the sequential peak
count, error-to-refusal mapping and limit immutability. The four-artifact
integration test was checked against the old gate and failed with a stage-five
`max_concurrent_artifacts` refusal before the fix was restored.

## Gate results

Style order was applied before build gates: `cargo fmt --all`, the Rust, prose,
source and Ruff full-tree sweeps, then strict rustdoc on `release-verifier`.
Findings were corrected and the sweeps repeated before `make check`.

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `check_rust_style.py` vs `c24bc29` | pass |
| `check_prose_style.py`, `check_prose_unified.py` | pass; advisory candidates pre-existing |
| `check_source_style.py`, `check_ruff_added_lines.py` | pass |
| Ruff | pinned added-line gate pass; no changed Python |
| `english_audit.py --strict` | pass (exit 0); pre-existing advisory in an unchanged file |
| strict rustdoc, `release-verifier` | pass |
| `make check` | pass |
| `make freeze-check` | pass; no frozen path changed |
| `cargo deny check` | pass; all four checks ok, pre-existing duplicate warnings |

The release-verifier suite reports 130 unit tests, 37 canonical-wire integration
tests, 40 verifier integration tests and no failure or ignore. The Python suite
reports 383 passed, 182 deselected and 2 subtests passed. No commit was created.

## Non-claims

- No dataset is accepted, no right is granted and no admission receipt is
  issued under rights `UNKNOWN` or `DENY`.
- The fixture limits used in tests are `TEST_ONLY` values; no production limit
  or profile is approved here.
- No parallel scheduler is implemented. The typed counter describes the
  contract a future scheduler must satisfy; it is not a concurrency result.
- No provider contact, hosted CI run, experiment or live route is part of this
  record. `Implemented` is not promoted to `Verified`.

## Commit convention

No commit is created by this work package. The parent session owns the README
`code_commit` update to the last code-bearing commit.
