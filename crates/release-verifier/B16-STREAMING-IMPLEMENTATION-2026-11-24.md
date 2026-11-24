<!-- SPDX-License-Identifier: Apache-2.0 -->

# B16 bounded streaming verifier — implementation record, 2026-11-24 basis

Work package: WP-5 / B16. Branch `feat/b16-streaming-verifier`, base `d3b66f52`.
The record name retains the 2026-11-24 work-package basis; implementation under
the ratified contract was completed on 2026-11-25. No commit is created by this
work package. This record reports code and local gate results; it promotes no
capability state from `Implemented` to `Verified`.

## 1. Authority and scope

Authority: owner-ratified
`engineering/decisions/WS-STREAM-001-b15-b16-architecture-ratification.md`
(2026-11-25) and the design
`engineering/design/b16-streaming-verifier-design.md`. The ratification approves
architecture and interface contracts A-01..A-12 and resolves B16 D1..D10. It
approves no numeric maximum, no schema version, no codec identifier, no right,
no dataset, no release, no key and no capability promotion.

Changed area: `crates/release-verifier` only. No provider record, dataset,
right, India artefact, experiment, live route, schema byte, approved digest,
rejection code or CI configuration is changed. The freeze is unchanged: rights
`UNKNOWN`, dataset acceptance `BLOCKED`, budget INR 0, collection `DISABLED`,
experiments `BLOCKED`, live execution `DISABLED`, D11 strict and
`PUBLICATION_DURABILITY_VERIFIED` false.

## 2. Public API and compatibility

`ReleaseVerifier::verify_release` and `verify_release_with_diagnostics` keep
their signatures and their caller-policy behavior. A new entry point is added:

```rust
pub fn verify_release_streaming(
    &self,
    locator: &ReleaseLocator,
    request: &AdmissionRequest,
    policy: &StreamingResourcePolicy,
) -> Result<AdmissionReceipt, RefusalDiagnostic>
```

`ReleaseLocator`, `VerifierConfig`, `VerifierResourcePolicy`,
`RefusalDiagnostic`, `AdmissionReceipt` and `ArtifactBinding` remain usable by
existing callers and conformance binaries. The twelve stage numbers, completion
observer callbacks and first-refusal rule are retained. No shared runtime crate
is introduced; the verifier imports no Fieldbook module.

## 3. Implemented contracts

### 3.1 Bounded streaming path

`src/streaming.rs` implements a single pass per artifact:

1. Descriptor-relative artifact open with `O_NOFOLLOW`, regular-file and
   hard-link checks, and pre/post `fstat` identity comparison.
2. `StoredStream`: fixed `io_chunk_bytes` reads; incremental stored SHA-256;
   each chunk is handed to the snapshot writer and the decoder; declared and
   effective stored-byte bounds are checked before the next chunk is accepted.
3. `ExpandedSink`: the effective emission cap is
   `min(max_decompression_budget_bytes, max_expanded_bytes_per_artifact,
   declared uncompressed_bytes)`; the cap is checked on decoder output before
   any further processing. Gzip uses `flate2::write::GzDecoder`, which validates
   the trailer and CRC at `finish`. Compression `"none"` passes stored chunks
   through the same cap path.
4. `RecordFramer`: a reusable line buffer bounded by `max_record_bytes`; one
   LF-terminated line at a time; BOM, CR, blank-line and missing-trailing-LF
   refusals; record count checked before the next record is parsed.
5. `OneRecordParser`: strict JCS parse of one line, canonical-byte identity,
   structural depth/string/token/member/element bounds, required fields,
   envelope-version check at detection (D3), event-registry membership with
   `REJECT_PAYLOAD_REGISTRY` (D6), byte-derived ordering key and comparison with
   a single predecessor. No record, key vector or line text is retained.
6. `ArtifactState`: fixed-size stored and expanded digests, byte and record
   counters, one predecessor key and the envelope-agreement flag. Stages nine
   through twelve consume bindings, digests and the flag; no stage after five
   reads a record body or an artifact byte vector.

Stored, expanded and dataset SHA-256 values are updated over exact bytes as
they pass through. The dataset digest remains SHA-256 over concatenated
uncompressed canonical record streams in canonical artifact order.

### 3.2 Resource profile contract

`src/resource_profile.rs` defines `ResourceProfileId`, `ProcessingRole`,
`LimitsInput`, `ProfileLimits`, `ResourceProfile`, `EffectiveLimits` and
`StreamingResourcePolicy`. All fields of `ProfileLimits` and `ResourceProfile`
are private and populated by validating constructors. Validation refuses
non-finite values, zeros where a positive value is required, inconsistent
per-artifact/per-release relations, chunk, string, token and queue bounds above
the record bound, a staging disk bound below the stored release bound, a memory
budget below the fixed buffers, and a descriptor bound below the parallel set
of artifact descriptor, staging writer and verify descriptor.

The canonical digest is domain-separated SHA-256 over identifier name, version,
role token and the complete limits set. A declared digest that disagrees fails
closed. Role selection is local: the streaming entry point accepts only a
`VERIFIER` profile plus a `SNAPSHOT` profile; an optional producer declaration
must carry a `PRODUCER` profile. The effective limit is the stricter of the
local ceiling, the selected role profile and the producer declaration.
A producer declaration lowers allowances but cannot raise them.

### 3.3 Snapshot API

`SnapshotEntry` is metadata only: private name, stored byte count, stored
SHA-256 and verified state. No byte vector exists in metadata, in the builder
or in the sealed snapshot. `SnapshotBuilder` offers `begin_entry`,
`write_chunk`, `finalize_entry` and `verify_entry`. Names are tracked at create
time before any write, sync or verification. `finalize_entry` syncs, reopens
with `O_NOFOLLOW`, streams the copy through an incremental digest and byte
counter, and marks the entry verified. `verify_entry` re-streams the sealed copy
at snapshot binding; no bytes are materialized. Both verification points run.
The NZ #144 cleanup contract is retained: `discard_partial` and `discard`
return `Result<CleanupReport, CleanupPending>`; removal is descriptor-relative
with no-follow stat before unlink; no deletion error is discarded; a successful
report requires empty re-enumeration after a directory sync. `CLEANUP_PENDING`
is reported beside the first refusal.

### 3.4 Stage and code mapping

Stage six rights refusal keeps `REJECT_RIGHTS_EVIDENCE`, stage six and
`receipt_written` false; cleanup completes or reports `CLEANUP_PENDING`. Under
D3 a record envelope-version mismatch is refused at stage five with the stable
code `REJECT_SCHEMA_ENVELOPE` at detection. Under D6 a registry miss is refused
at stage five with `REJECT_PAYLOAD_REGISTRY`. The stage-nine declaration check
and all other stage numbers and codes are unchanged.

## 4. Deferred numerics and test-only profiles

Every numeric maximum remains DEFERRED (WS-STREAM-001 DEF-01..DEF-04, DEF-06).
This implementation registers no `STREAM-RES-1` digest, approves no production
limit, sets no fixed internal chunk constant and registers no gzip codec
identifier. The chunk size comes from the selected role profile's
`io_chunk_bytes`. Unknown compression declarations fail closed under the
retained declaration check; no byte-pattern classification is introduced.

Test fixtures construct finite `TEST_ONLY` profiles whose identifiers carry the
`TEST_ONLY` marker. The streaming entry point refuses such identifiers in a
production invocation; a harness opts in through
`ReleaseVerifier::with_test_only_profiles`. Those values exist to drive
adversarial and infrastructure tests only. They are not approved maxima, not evidence for `PERF-001`, not RSS
or throughput measurements and not a basis for operational qualification.
`STREAM_RESOURCE_PROFILE_RATIFIED` remains `PENDING`; `PERF_001_QUALIFIED` and
`OPERATIONALLY_QUALIFIED` remain `BLOCKED`.

Codec window, queue capacity, worker count and processing duration are bound by
the validated role profile. The single-threaded path holds one record, so its
queue occupancy cannot exceed the validated queue capacity by construction.
The processing-duration budget is checked while artifacts are processed. Role
memory budget must cover the fixed chunk and line buffers at profile
validation; no resident-memory number is claimed.

## 5. Tests

Unit tests cover profile validation, digest sensitivity, role and digest
mismatch, stricter-of selection, bounded reader chunking and incremental
digest, framer limits and boundary faults, structural string/token/depth
bounds, decoder emission cap, registry hit and miss, snapshot entry lifecycle,
finalize-time and binding-time verification faults, and cleanup outcomes.
Integration tests use abstract scalar records and cover gzip and uncompressed
acceptance, incremental digest agreement, high expansion, truncated gzip,
invalid CRC, oversized records, deep nesting, record-count overflow, ordering
violations, exact artifact inventory, rights refusal with no surviving
snapshot, stage-five envelope refusal, metadata-only entries, refusal of a
pre-existing staged entry, producer declarations that cannot raise local
limits, test-only profiles refused without harness opt-in, first-refusal
precedence and constant-size state across one-, two- and four-fold input
scaling. The scaling test is a structural bound check on state
size and record flow; it is not measured RSS evidence.

## 6. Non-claims

- No dataset is accepted, no right is granted and no admission receipt is
  created under rights `UNKNOWN` or `DENY`.
- No Fieldbook fixture/e2e suite, hosted CI run, cross-execution, provider
  contact or collection command is part of this work package.
- No schema version, registered digest, codec identifier, platform,
  filesystem or device tier is approved here.
- No measured memory plateau, throughput, latency or CPU result is claimed.
  `Implemented` is not promoted to `Verified` by this record.
- The batch path remains a compatibility adapter with no deprecation date.
  This record makes no memory-independence claim for that path.

## 7. Local gate results

Style order was applied before build gates: `cargo fmt --all`, then
`check_rust_style.py`, `check_prose_style.py`, `check_prose_unified.py`,
`check_source_style.py`, Ruff added-line gates and `english_audit.py --strict`
with strict rustdoc on `release-verifier`. Findings were corrected and the
sweeps repeated; the final sweeps report zero findings and zero advisory
review candidates.

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `make check` | pass |
| `make freeze-check` | pass |
| `cargo deny check` | pass; `advisories ok, bans ok, licenses ok, sources ok` |
| `cargo xtask test` | pass, including doc tests |
| strict rustdoc, `release-verifier` | pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | pass |
| `python3 -m pytest -q` | 383 passed, 182 deselected, 2 subtests passed |

The release-verifier suite ran 42 unit tests, 20 new streaming integration
tests, 25 canonical-wire integration tests and 40 existing verifier
integration tests, with no failures and no ignores. `cargo xtask docs` on the
whole workspace reports pre-existing private intra-doc links in
`crates/market-archive/src/journal.rs`; that crate is outside this work
package and was not edited. Strict rustdoc for `release-verifier` passes.

The two public batch entry points remain compatibility adapters with their
previous bounded in-memory behavior. The bounded streaming path is the new
entry point; no memory-independence claim is made for the batch adapters.
No commit was created.
