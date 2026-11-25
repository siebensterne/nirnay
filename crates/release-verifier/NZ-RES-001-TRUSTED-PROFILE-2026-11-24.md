# Trusted resource-profile selection — 2026-11-24

<!-- SPDX-License-Identifier: Apache-2.0 -->

Date: 2026-11-24. Work package: NZ-RES-001 on branch `fix/nz-res-001`, base
`c1c2f0d`. This record describes code and abstract tests. It approves no profile,
accepts no dataset and creates no admission receipt.

## Trust boundary

`ProfileResolver` owns the approved registry inside the verifier. The streaming
entry point takes only a locator and a request; it holds no policy parameter.
`ResourceProfile`, `StreamingResourcePolicy` and producer-declaration
construction are crate-private, and the two public profile types are opaque
handles. A caller cannot supply a ceiling, a role profile or a producer
declaration at enforcement time.

`ApprovedProfileRegistry::new` is empty. No production digest, numeric maximum,
codec identifier or capability state is registered by this work package. A local
ceiling is derived from the approved consumer profiles; no value is invented.
The only registry mutations are `cfg(test)` helpers that require a `TEST_ONLY`
identifier. The former public `with_test_only_profiles` opt-in is removed.

## Fail-closed behavior

Resolution precedes every release-directory and source open. An empty registry
returns `VerifierError::NoApprovedProfile` at stage 5 with code
`REJECT_RESOURCE_LIMIT`, before any descriptor is opened or staging is written.
Each entry is rechecked for its approved role binding, canonical digest over
identifier, role and all limits, and approved version. An ambiguous role
binding, wrong role, digest mismatch or version mismatch is refused.

Effective limits keep the stricter-of rule: the derived local ceiling, the
role profile and an optional approved producer declaration are combined by
minimum, so a declaration lowers an allowance and cannot raise one. The twelve
ordered stages, first-refusal rule, cleanup contract and rights refusal at
stage 6 are unchanged.

## Verification scope

Rust tests cover an empty registry before any filesystem open, a missing root,
wrong role, wrong digest, wrong version, ambiguous binding, the stricter-of
rule, opaque construction through compile-fail documentation tests, and the
existing B16 acceptance harness with finite `TEST_ONLY` profiles. The recorded
`cargo test -p release-verifier --all-targets --locked` run passed 140 tests and
`cargo test -p release-verifier --doc --locked` passed 4 compile-fail tests. No
result here claims a production profile, a ratified maximum or performance
evidence.

## Commit convention

No commit is created by this work package. The parent owns the commit sequence
and updates the README `code_commit` in a following prose commit.
