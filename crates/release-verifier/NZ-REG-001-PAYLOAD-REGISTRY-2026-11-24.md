<!-- SPDX-License-Identifier: Apache-2.0 -->

# NZ-REG-001 per-event payload schema registry — dated record

Date: 2026-11-24 work-package basis; code completed 2026-11-25 on branch
`fix/nz-reg-001`, base `5f114e3`. No commit is created by this record.

## Authority and scope

WS-STREAM-001 D6 requires the ratified per-event payload schema registry to be
enforced at stage five with `REJECT_PAYLOAD_REGISTRY`. The confirmed defect was
that `canonical_wire.rs` reduced `PayloadSchemaEntry` declarations to an event
type set and the canonical record paths checked membership only; no local
payload registry or validator existed.

Changed area: `crates/release-verifier` only. No provider record, dataset,
right, India artefact, experiment, live route, release schema byte, approved
digest or CI configuration is changed. The freeze is unchanged: rights
`UNKNOWN`, dataset acceptance `BLOCKED`, budget INR 0, collection `DISABLED`,
experiments `BLOCKED`, live execution `DISABLED`, D11 strict and
`PUBLICATION_DURABILITY_VERIFIED` false. The NZ-RES-001 profile resolver and its
stricter-of rule are untouched.

## Delivered code

- `src/payload_registry.rs` defines the immutable `PayloadSchemaRegistry` with
  `registry_id`, `registry_version`, `registry_digest` and `schemas`. Each
  installed schema binds `event_type`, `schema_id`, `schema_version`,
  `schema_bytes_digest` and `validator_identity`. `load` verifies the local
  registry version, a domain-separated SHA-256 digest over identity, version and
  every binding, each schema byte digest, the validator identity and binding
  uniqueness. No byte is read from the untrusted release to build a schema.
- `src/canonical_wire.rs` carries the publisher-declared per-event schema values
  on `ParsedManifest` instead of a membership set. Those values are comparison
  inputs only.
- `src/streaming.rs` and `src/verifier.rs` resolve the record event type at
  stage five, compare the declared schema identity, version and digest against
  the installed entry, and validate the payload through the installed
  validator. Missing, mismatched or invalid bindings refuse with
  `REJECT_PAYLOAD_REGISTRY` at stage five. The twelve stage numbers, first
  refusal, absence of an admission receipt and snapshot cleanup with
  `CLEANUP_PENDING` are retained.
- `src/bin/ndc-conformance-verifier.rs` installs a local `TEST_ONLY` registry
  fixture so the conformance diagnostic still ends at the stage-six rights
  refusal. The fixture schema bytes are local and are not loaded from a release.

## Validator choice

The ratified contract names no schema language. This record selects an explicit
local structural validator, `nirnay-structural-payload-v1` with validator
identity `structural-v1`, driven by registry metadata. It checks canonical
payload object members, scalar kinds, required fields, duplicate names and
scalar grammar. The implementation uses the crate's existing JSON parsing and
hashing dependencies; no dependency is added and no silent runtime binding is
introduced. Adopting JSON Schema requires a separate Workspace amendment. D6 is
not weakened to event-type membership.

## Tests

Unit tests cover registry load and digest sensitivity, unknown version,
corrupted and non-canonical schema bytes, schema byte digest mismatch, unknown
validator identity, duplicate event or schema identity, and structural payload
faults including wrong kind, undeclared, missing and duplicate fields. Streaming
unit tests cover registry hit and miss plus wrong declared identity, version,
digest and invalid payload as stage-five typed refusals. Canonical-wire
integration tests cover a registered valid payload reaching the rights stage,
unknown event, each declaration mismatch, invalid and unregistered payload
fields, unknown registry version, corrupted local registry, registry digest
mismatch, local schema digest mismatch and an absent registry. Each refusal
asserts stage five, `REJECT_PAYLOAD_REGISTRY`, no receipt and a completed or
pending cleanup.

## Gate results

Style order was applied before build gates: `cargo fmt --all`, then the Rust,
prose, source, Ruff and English audit sweeps, then strict rustdoc on
`release-verifier`. Findings were corrected and the sweeps repeated; the final
sweeps report no binding findings in changed lines.

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `check_rust_style.py` vs `5f114e3` | pass |
| `check_prose_style.py`, `check_prose_unified.py` | pass |
| `check_source_style.py`, `check_ruff_added_lines.py` | pass |
| `english_audit.py --strict` | pass; no finding in changed files |
| strict rustdoc, `release-verifier` | pass |
| `make check` | pass |
| `make freeze-check` | pass; no frozen path changed |
| `cargo deny check` | pass; `advisories ok, bans ok, licenses ok, sources ok` |
| `cargo xtask test` | pass, including doc tests |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | pass |
| `python3 -m pytest -q` | 383 passed, 182 deselected, 2 subtests passed |

The release-verifier suite reports 99 unit tests, 37 canonical-wire integration
tests, 40 verifier integration tests and 4 doc tests, with no failure or ignore.
This work added 20 registry unit tests, 4 streaming unit tests and 12 stage-five
integration tests. No commit was created.

## Non-claims

- No dataset is accepted, no right is granted and no admission receipt is
  created under rights `UNKNOWN` or `DENY`.
- No schema version, registry identity, digest, codec identifier or platform is
  approved as a production value here; the local registry fixtures carry
  `TEST_ONLY` identities.
- No provider contact, cross-execution, hosted CI run, experiment or live route
  is part of this record. `Implemented` is not promoted to `Verified`.

## Commit convention

No commit is created by this work package. The parent session owns the README
`code_commit` update to the last code-bearing commit.
