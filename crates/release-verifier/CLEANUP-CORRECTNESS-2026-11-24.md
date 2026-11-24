# Private-snapshot cleanup correctness — 2026-11-24

<!-- SPDX-License-Identifier: Apache-2.0 -->

Date: 2026-11-24. Work package: PR 4 / NZ-SNAP-001 on branch `fix/nz-snapshot-cleanup`,
base `70716eb`. This record describes code and abstract tests. It grants no right,
accepts no dataset and creates no admission receipt.

## Contract

- `SnapshotBuilder::add` tracks the staging name immediately after the `O_EXCL |
  O_NOFOLLOW` create, before write, file sync, reopen and copy verification. A
  failure after create leaves a tracked name for cleanup.
- `SnapshotBuilder::discard_partial` and `PrivateSnapshot::discard` return
  `Result<CleanupReport, CleanupPending>`. Removal uses `unlinkat` relative to the
  owned staging descriptor with no-follow semantics. `CleanupPending` carries the
  exact staging path, tracked names not confirmed absent or removed, and the
  failing operation.
- A successful report requires every tracked entry removed or already absent, a
  directory `fsync` after removals, and a final re-enumeration with no entries.
  `CleanupReport::is_complete` encodes that conjunction. No deletion error is
  discarded.
- `VerifierError::CleanupPending` maps to `CLEANUP_PENDING` at stage 5. It is
  reported through `RefusalDiagnostic::cleanup` beside the original refusal. A
  rights refusal keeps `REJECT_RIGHTS_EVIDENCE` and stage 6; `receipt_written`
  stays false.
- No `Drop` implementation is used as the proof signal. Callers invoke cleanup on
  every refusal path; the typed result is the observable state.

## Tested-run observation

In the recorded `cargo test -p release-verifier` runs, nine injected failure points
were exercised: create, partial write, file sync, reopen, copy verification, first
removal, removal after two prior removals, directory sync after deletion and final
enumeration. Successful cleanup reports showed `directory_empty` and
`directory_sync_completed` true, and the staging directory held no entries at
assertion time. Each injected failure showed `cleanup_pending` true with the exact
remaining names. A rights refusal paired with a cleanup failure kept stage 6 and
`REJECT_RIGHTS_EVIDENCE`.

## Scope of the record

That observation applies to the recorded runs on the stated base and toolchain. It
is not a statement about every filesystem, scheduling order or caller. Directory
`fsync` semantics remain a platform property. An abrupt process stop before an
explicit cleanup call leaves caller-side recovery outside this result. An operator
facing `CLEANUP_PENDING` uses the preserved path and remaining names for manual
inspection; the verifier makes no claim that staged bytes were removed in that
case.

## Commit convention

No commit is created by this work package. The parent owns the commit sequence and
updates README `code_commit` in a following prose commit to the last code-bearing
commit.
