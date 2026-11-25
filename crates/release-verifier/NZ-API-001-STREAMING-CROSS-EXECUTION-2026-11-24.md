# NZ-API-001 — streaming cross-execution harness, 2026-11-24 basis

<!-- SPDX-License-Identifier: Apache-2.0 -->

Date: 2026-11-24 work-package basis. Work package: NZ-API-001 on branch
`fix/nz-api-001`, base commit `c24bc299e102c3f2f9ef46d5eb6eb477e5cd7d01`.
No commit is created by this work package; the parent owns the commit sequence
and the README `code_commit` field. This record reports code and local gate
results. It promotes no capability state from `Implemented` to `Verified`.

## 1. Scope and freeze

Added area: a `cfg(test)` cross-execution harness under
`crates/release-verifier/tests/nz_api001_cross_execution.rs`, a workspace driver
`scripts/nz_api001_streaming_cross_execution.sh`, local negative tests in
`crates/release-verifier/tests/b16_streaming.rs`, and LEGACY_BUFFERED
classification in the verifier API docs.

The harness invokes a Fieldbook B15 streaming producer binary when one is
supplied, then feeds the produced release to the real
`ReleaseVerifier::verify_release_streaming` method with a `TEST_ONLY` approved
profile resolver. The buffered verifiers are not invoked by the harness. A
missing binary yields an explicit skip with a reason; producer bytes are not
simulated.

Freeze is unchanged: rights `UNKNOWN`, dataset acceptance `BLOCKED`, budget
INR 0, collection `DISABLED`, experiments `BLOCKED`, live execution `DISABLED`,
D11 strict and `PUBLICATION_DURABILITY_VERIFIED` false. No provider, market
observation, India artefact, experiment or live route was created or read. No
schema byte, wire byte, admission receipt shape, signing key for a real
publisher or rejection code was changed.

## 2. Exact binaries, commits and digests

Nirnay worktree:
`/private/var/folders/7c/m_f7hs9n445174c0x3gsy5kw0000gn/T/opencode/nv-api001`,
branch `fix/nz-api-001`, HEAD and base
`c24bc299e102c3f2f9ef46d5eb6eb477e5cd7d01`, tracked modifications listed by
`git status --short` and no commit created.

Fieldbook local clone: `/Users/tourist/Documents/siebensterne-workspace/fieldbook`,
branch `main`, HEAD `e0d065e0298da3a97a191eab08f69ae53bff0865`, clean working
tree. B15 streaming producer first landed at
`8216197124d33466fbe651e417b7c19505e4155e` (merge of
`feat/b15-streaming-producer`). Later Phase 3A fixes available locally:
`7e0f74a567e906d656da1341ed117e239bf31a0f` (plan/source pairing through
sorting) and `7008e1c8ff47035563011ab4c731aa1bd4665b35` (producer expansion
ratio against stored bytes). A further streaming quarantine fix is available
at `b07be1c82d99b4107d365fa927b87b54ea32b970` in the local
`fix/fb-stream-002` worktree.

Producer method invoked by the harness contract: Fieldbook
`distribution::ndc_stream::publish_streaming` with `StreamingBuildRequest`, a
bounded `ArtifactRecordSource`, `TEST_ONLY` `ResourceProfile` and
`InvocationContext::TestHarness`. The `InvocationContext::TestHarness` variant
is `cfg(test)` in Fieldbook, so an external binary cannot select it from the
published library API. A producer executable that performs this call inside a
Fieldbook test build must be supplied by the owner; none is present locally.

Binaries inspected in the Fieldbook clone:
`target/debug/fieldbook` and `target/debug/ndc-conformance-producer`. The
conformance producer calls `ndc_release::build_release`, a buffered producer,
and is not used as a B15 streaming source. No executable matching
`*stream*producer*` exists under `target/`.

File digests recorded with `shasum -a 256` in the Nirnay worktree:

| File | SHA-256 |
|---|---|
| `crates/release-verifier/tests/nz_api001_cross_execution.rs` | `0f505151b1ab083f564a60a81874e5292076e07da7fa5ac81004a17cd91fa266` |
| `crates/release-verifier/tests/b16_streaming.rs` | `158dc08daaa342ca88109e7cc132ec829990078cf036525115846075b7b2f810` |
| `crates/release-verifier/src/verifier.rs` | `1f360062178c18e691655deda8d8103b363b9c93245a1f167eb9d6e681b82207` |
| `crates/release-verifier/src/lib.rs` | `ad12f181c635fff9ac4d1ba140e1bbab5b6b0e2c0c8d77172ec7913710fb43fe` |
| `scripts/nz_api001_streaming_cross_execution.sh` | `51801762a3041512252141494d6021a3d4325fad9b817778a740144398c79991` |

## 3. Harness contract

Environment variables:

- `NIRNAY_B15_PRODUCER_BIN`: absolute or relative path to a producer
  executable. The harness checks that the path is a regular file with an
  execute bit. The executable accepts `--out DIR` and writes a release directory
  containing `manifest.json`, `signature.json` and `artifacts/` beneath `DIR`.
- `NIRNAY_B15_PRODUCER_COMMIT`: Fieldbook commit reported in the harness log.
  Absence is recorded as `not-supplied`.

The Rust harness discovers the release directory beneath `--out`, parses the
manifest through `canonical_wire::parse_manifest`, derives the internal
admission request with `canonical_wire::into_internal_request`, builds a
`TrustedKeyRegistry` from the published B15 test seed `[0x51; 32]`, installs a
`TEST_ONLY` `ProfileResolver`, and calls `verify_release_streaming`. Exact
command, binary path, binary SHA-256, commit and producer stdout are printed
under `--nocapture`.

When a binary is present the harness asserts: stages 1 through 5 observed in
order, stage 6 `REJECT_RIGHTS_EVIDENCE` under `UnknownRights`, no admission
receipt, cleanup complete or `CLEANUP_PENDING`, and an empty staging directory.
It repeats with one-byte mutations of the manifest, sidecar, stored artifact
and expanded artifact and asserts typed codes at stages 1, 3, 5 and 5. It adds a
payload-registry hit case and a miss case using `PayloadSchemaRegistry`.

## 4. Commands and results

Commands run in the Nirnay worktree with no commit:

| Command | Outcome |
|---|---|
| `cargo fmt --all` | applied |
| `cargo test -p release-verifier --lib --locked` | 125 passed, 0 failed |
| `./scripts/nz_api001_streaming_cross_execution.sh` | 5 harness tests passed; 3 producer cases reported `SKIP` with the reason below |
| `cargo test -p release-verifier --lib --locked nz_api001_cross_execution_tests -- --nocapture` | 5 passed; skip lines recorded |
| `git -C /Users/tourist/Documents/siebensterne-workspace/fieldbook log --oneline -12` | commits in section 2 |
| `find target -maxdepth 2 -type f -name '*stream*producer*'` (Fieldbook clone) | no match |

## 5. Actual cross-execution status: NOT_RUN

Skip reason, emitted by the driver and by each producer test:
`NIRNAY_B15_PRODUCER_BIN is unset or does not name an executable Fieldbook B15`
streaming producer; no producer output was fabricated.

No Fieldbook B15 streaming producer binary is installed in either local clone.
The Fieldbook `InvocationContext::TestHarness` variant and
`ResourceProfile::new_test_only` are `cfg(test)` only, so a separate executable
cannot select them from the published library. Building such an executable
requires a paired Fieldbook change, which is outside this work package. The
local public conformance producer uses the buffered `build_release` path and is
not substituted. The three producer-dependent tests therefore report an
explicit skip and write no release. Nine local `TEST_ONLY` tests ran and passed:
four one-byte mutation cases, two legacy-classification cases, one stage-observer
case and two payload-registry cases. They use Nirnay-local fixtures and are
recorded as such, not as producer output.

## 6. LEGACY_BUFFERED classification and non-fallback

`verify_release` and `verify_release_with_diagnostics` are documented as
LEGACY_BUFFERED. Their public signatures and caller-owned policy behavior are
unchanged. `verify_release_streaming` is the only streaming entry point. A
streaming refusal returns directly to its caller with no receipt; the buffered
verifiers are not attempted after a streaming failure. Two local tests confirm
that an empty streaming resolver yields `NoApprovedProfile` while the buffered
entry point keeps its caller policy.

## 7. Preserved contracts

The NZ-RES-001 approved profile resolver, NZ-REG-001 payload registry,
NZ-LIMIT-001 parser bounds and the private snapshot cleanup contract are
unchanged. The `TEST_ONLY` helpers stay `cfg(test)`. Effective limits remain
the stricter-of combination; a producer declaration can lower a limit and
cannot raise one. No new public method or signature was added to
`ReleaseVerifier` outside the existing streaming entry point.

## 8. Gate record

Style order was observed: `cargo fmt --all`, the four style sweep scripts,
Ruff, `english_audit.py --strict` and strict crate rustdoc ran before the
deterministic gates. Findings in newly added Rust lines were corrected (one
Clippy `collapsible_if` and three assertion-context findings); the full-tree
`check_prose_style.py --all` sweep reports pre-existing review findings in
owner-maintained `BUILD-RESULTS.md` and `engineering/references/`, which are
outside this work package. No finding named a file added by this work package.

| Command | Outcome |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `python3 scripts/check_rust_style.py` | PASS on changed lines |
| `python3 scripts/check_prose_style.py` and `python3 scripts/check_prose_unified.py` | PASS on changed lines, zero advisory review candidates |
| `python3 scripts/check_source_style.py` | PASS on changed lines |
| `python3 scripts/ruff_gate.py ...` and `python3 scripts/check_ruff_added_lines.py` | PASS |
| `python3 scripts/assurance/english_audit.py --repo . --strict --exclude 'references/*'` | exit 0 |
| `RUSTDOCFLAGS="-D warnings -D rustdoc::broken_intra_doc_links" cargo doc -p release-verifier --no-deps --locked` | exit 0 |
| `make check` | exit 0; includes docs-check, freeze-check, D11 policy, shellcheck, generated-paths, pollution, cargo fmt, `cargo xtask check`, `cargo deny check`, style, Rust tests, schema-check and Python tests |
| `make freeze-check` | exit 0 |
| `cargo deny check` | exit 0; advisories, bans, licenses and sources ok |
| `cargo clippy -p release-verifier --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test -p release-verifier --lib --locked` | 125 passed, 0 failed |
| `cargo test -p release-verifier --doc --locked` | 4 compile-fail doc tests passed |
| `python3 -m pytest -q` (inside `make check`) | 383 passed, 182 deselected, 2 subtests passed |

No gate result above is promoted to a cross-execution result.
