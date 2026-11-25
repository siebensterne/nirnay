<!-- SPDX-License-Identifier: Apache-2.0 -->

# NZ-LIMIT-001 parser bounds and counters — dated record

Date: 2026-11-24 work-package basis; code completed 2026-11-25 on branch
`fix/nz-limit-001`, base `2f35e2a`. No commit is created by this record.

## Authority and scope

The confirmed defect was in the pre-parse structural scan of one record line.
`check_line_bounds` counted commas, so a container with M + 1 members produced M
commas and passed a bound of M. The two externally influenced counters in
`RecordFramer::push` and `OneRecordParser::accept` used unchecked addition.

Changed area: `crates/release-verifier/src/streaming.rs` and this record. No
provider record, dataset, right, India artefact, experiment, live route,
release schema byte, approved digest or CI configuration is changed. The freeze
is unchanged: rights `UNKNOWN`, dataset acceptance `BLOCKED`, budget INR 0,
collection `DISABLED`, experiments `BLOCKED`, live execution `DISABLED`, D11
strict and `PUBLICATION_DURABILITY_VERIFIED` false. The twelve-stage pipeline,
first refusal, payload registry, trusted profile resolver and snapshot cleanup
with `CLEANUP_PENDING` are untouched.

## Delivered code

- Each `BoundFrame` now carries `has_element`. A string, scalar token or nested
  container marks the current frame as holding a member; `count_member` counts
  the member at each comma and counts the final member at the closing bracket
  when content was present. Empty containers have no content and count zero.
  Per-frame counting handles nested objects, nested arrays, arrays of objects
  and objects of arrays without sharing state between levels. Bytes inside
  strings, including escaped quotes and punctuation, stay in the string branch
  and cannot be counted as separators.
- `count_member` reads the bound for the frame's kind and uses `checked_add`;
  overflow or a count above the bound returns `REJECT_RESOURCE_LIMIT` naming the
  same limit field as before. String and token byte counters also use
  `checked_add` with a typed refusal.
- Stored bytes, expanded bytes, framer record count and parser record count use
  `checked_add`; overflow returns `REJECT_RESOURCE_LIMIT` for the corresponding
  limit field. The expanded-byte emission cap is evaluated once on the checked
  total before hashing or framing, preserving the first-refusal slot.

## Tests

Fourteen streaming unit tests were added. They cover empty object and array,
the member and element bound, one past each bound, nested objects, nested
arrays, array of objects, object of arrays, escaped strings with punctuation,
member-counter overflow, framer record-counter overflow, parser record-counter
overflow and expanded-byte-counter overflow. Seven of the bound tests fail
against the comma-only scan and were checked against that behaviour before the
fix was restored. The existing refusals for deep nesting, long tokens, long
strings, BOM, CR, blank lines, missing LF, record-count mismatch, registry miss
and registry declaration mismatch remain in force.

## Gate results

Style order was applied before build gates: `cargo fmt --all`, then the Rust,
prose, source and Ruff full-tree sweeps, then strict rustdoc on
`release-verifier`. Findings were corrected and the sweeps repeated.

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `check_rust_style.py` vs `2f35e2a` | pass |
| `check_prose_style.py`, `check_prose_unified.py` | pass; whole-tree unified sweep clean with pre-existing advisory candidates |
| `check_source_style.py`, `check_ruff_added_lines.py` | pass |
| Ruff | pinned added-line gate pass; whole-tree sweep advisory with no changed Python |
| `english_audit.py --strict` | pass; no finding in changed files |
| strict rustdoc, `release-verifier` | pass |
| `make check` | pass |
| `make freeze-check` | pass; no frozen path changed |
| `cargo deny check` | pass; `advisories ok, bans ok, licenses ok, sources ok`, pre-existing duplicate warnings |

The release-verifier suite reports 113 unit tests, 37 canonical-wire integration
tests, 40 verifier integration tests and no failure or ignore. The Python suite
reports 383 passed, 182 deselected and 2 subtests passed. No commit was created.

## Non-claims

- No dataset is accepted, no right is granted and no admission receipt is
  created under rights `UNKNOWN` or `DENY`.
- The fixture limits used in unit tests are `TEST_ONLY` values; no production
  limit or profile is approved here.
- No provider contact, hosted CI run, experiment or live route is part of this
  record. `Implemented` is not promoted to `Verified`.

## Commit convention

No commit is created by this work package. The parent session owns the README
`code_commit` update to the last code-bearing commit.
