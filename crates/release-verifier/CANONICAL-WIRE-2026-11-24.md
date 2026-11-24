# Canonical NDC v2 Profile 1 wire mapping — dated record

<!-- SPDX-License-Identifier: Apache-2.0 -->

Date: 2026-11-24. Work package: WP-B10 on branch `feat/ndc-wire-mapping`, based on
`2dad4962b584c557237d219c1c5356fc58bc4b69`.

## Authority

Owner-ratified `WS-NDC-2026-11-24-003` and the canonical schema package in the Workspace
engineering directory `engineering/conformance/ndc-v2-schema-profile-1/` govern this work. The
package digest registered here is
`6e8c1e48e27289b8c3d3e959a06a805e736d0759b4fb244edbb074f65e107bab`. The schema package is the
shared authority; validator implementations remain independent and no shared runtime crate,
Fieldbook import or provider path is introduced.

## Delivered code

- `src/canonical_wire.rs` — independent strict serde types for the canonical manifest, sidecar
  inputs, record envelope, typed rights evidence, typed coverage, complete PROV lineage, tagged
  artifact and record order unions, registered string compression identifiers, single root
  `contract_version` and the per-event payload schema registry. Decimal-string instants and
  counts use grammar-checked newtypes; every object denies unknown members and requires explicit
  members. Explicit mapping functions convert validated canonical values into the existing
  internal verifier types.
- Byte-derived ordering — `derive_key_from_bytes` parses each received canonical record twice,
  checks JCS byte identity, validates the record mode and domain/epoch against the artifact
  profile, recomputes the key from raw parsed members and rejects disagreement with the typed
  envelope or any caller-supplied key. `validate_order_sequence` enforces strictly ascending keys
  with visible gaps and no event-time fallback.
- `src/verifier.rs` — optional canonical-wire mode, byte-derived record-stream checks and a
  stage-completion observer that does not relax first-refusal behavior.
- `src/schema.rs` — internal performed-operation records carry an explicit `operation_ref` so
  evidence binding is checked against the operation reference rather than the action label.
- `src/bin/ndc-conformance-verifier.rs` — `--release <dir> --out <diagnostic.json>` executable.
  The test-only trusted registry entry carries only the public key bytes derived from the fixed
  conformance seed `[0x11; 32]`; no private key object, production key or release key is used.

## Diagnostic format

The diagnostic is a JSON object with `schema_version`, `record_date`, `tool`, `release`,
`outcome`, `stages`, `phase_summary`, `refusal`, `admission_receipt`, `receipt_written`,
`private_snapshot_survives`, `rights_state`, `dataset_accepted`, `collection_enabled` and
`live_execution_enabled`. Each of the twelve `stages` entries records `stage`, `name`, `status`
(`PASS`, `REFUSED` or `NOT_RUN`) and a typed `code`. Under the current freeze the expected
`outcome` is `RIGHTS_ADMISSION_REFUSED`, `refusal.stage` is 6, `refusal.code` is
`REJECT_RIGHTS_EVIDENCE`, `admission_receipt` is null and
`private_snapshot_survives` is false. Stages 1 through 5 pass; stages 6 through 12 are recorded
as `REFUSED` at stage 6 and `NOT_RUN` afterwards.
The executable exits successfully only for that exact conformance result.

## Status and freeze

This record reports implementation and abstract tests with arbitrary structured scalars. It does
not claim `SCHEMA_CONFORMANT`, `CROSS_EXECUTED`, `CRYPTOGRAPHICALLY_VERIFIED`, `RIGHTS_ADMITTED`
or `DATASET_ACCEPTED`. Rights remain `UNKNOWN`, dataset acceptance remains blocked, collection
remains disabled, experiments remain blocked, live execution remains disabled and D11 remains
strict. No right is granted, no dataset is accepted and no admission receipt is created.

## Commit convention

No commit is created by this work package. Under the repository convention, README
`code_commit` is updated in a following prose commit to the last code-bearing commit. The parent
session owns that commit sequence and the accompanying README update.
