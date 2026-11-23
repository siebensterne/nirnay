# Engineering Anti-Patterns and Code Admission Standard

**Document ID:** `ENG-ANTI-PATTERNS`
**Version:** `0.4.0`
**Status:** consolidated enforceable subset, RRP-005, 2026-11-23. The former
`0.3.0` research-enriched draft (124879 bytes, SHA-256
`311267df7205d8d1f7e915f35858ec2f529eeecbd1867340b3efb677dc96c188`) remains in
Git history; this consolidation changed no gate, no test and no policy decision.
**Applies to:** contributors, reviewers, maintainers, AI agents, generated-code
producers, build tools and deployment automation.
**Repository scope:** every language, package, service, schema, test, script,
build, data contract, operational procedure and release artifact.
**Policy owner:** repository engineering owner (to be assigned).
**Authority:** the binding gates are the scripts, `Makefile`, CI workflows,
accepted ADRs and frozen contracts. This document states the invariants those
gates enforce and the review rules for cases a gate cannot decide. It is not a
second formatter configuration and does not replace a gate with prose.

---

## 0. Mandate

This standard defines what must not be introduced, what must be rejected in
review and what must not be merged into protected `main`. Formatting is delegated
to the repository's pinned formatters. This standard governs correctness,
financial semantics, architecture, ownership, concurrency, performance,
determinism, data integrity, security, testing, operational resilience, evidence
and AI-generated contributions.

The language and runtime hierarchy is deliberate:

1. **Rust** is the primary implementation language and the default owner of
   authoritative infrastructure, domain models, contracts, state machines,
   durable state, orchestration, control-plane APIs and performance-sensitive
   systems code.
2. **C and C++** are secondary languages for measured native kernels, codecs,
   hardware interfaces, FFI boundaries, external SDKs and subsystems whose
   requirements justify native implementation. They are not an escape hatch from
   Rust ownership or safety contracts.
3. **Python** is the research, scientific-computing, ML, experimentation,
   test-tooling and automation language. It may orchestrate Rust capabilities
   but must not become a hidden authority for order, risk, accounting or durable
   market state.
4. **kdb+/q** is an optional time-series and analytical plane. A licence,
   operating model and product requirement must justify it; it is not
   automatically the authoritative ledger or OMS database.
5. **SQLite and PostgreSQL** have distinct operating contracts. SQLite is
   embedded/local-first state under its filesystem and writer assumptions;
   PostgreSQL is a server database for multi-client transactional workloads with
   funded operations, backups, isolation and recovery. Renaming a connection
   string does not exchange them.
6. **Shell, Make and CMake** orchestrate builds and operations. They must not
   hold domain logic, financial calculations or state transitions.

The default is Rust. A secondary language or technology must earn its place with
a concrete boundary, an owner, a contract, tests, measured need and operational
evidence. This document does not claim HFT, regulated-exchange, institutional or
trillion-dollar-firm status; those claims require measured operation, real data,
venue access, hardware evidence, control testing, incident drills, independent
review and legal/regulatory authorisation.

No policy can prevent an author from pushing an unreviewed topic branch to a
remote. The enforceable objective is that unqualified changes cannot merge into
protected `main`, and that local and CI gates detect defects before review.

## 1. Normative language, severity and enforcement

### 1.1 Normative words

- **MUST / MUST NOT**: requirement. A violation blocks merge.
- **SHOULD / SHOULD NOT**: default requirement. A deviation needs a written
  rationale and reviewer acceptance.
- **MAY**: permitted option, not a recommendation.
- **BLOCKER**: a rule violation that can corrupt data, compromise security,
  bypass financial controls, falsify evidence or violate an accepted contract.

### 1.2 Severity classes

| Class | Meaning | Admission treatment |
|---|---|---|
| `B0` | Blocker: safety, financial correctness, state ownership, data integrity, security, authority bypass, false evidence or frozen-contract violation | Reject. No ordinary waiver. |
| `H1` | High: material reliability, performance, architecture, reproducibility, operability or long-term maintenance defect | Fix before merge. A temporary exception needs independent approval, owner, compensating control and expiry. |
| `M2` | Medium: maintainability, clarity, testability, naming or operational friction | Fix before merge unless a reviewer accepts a tracked follow-up. |
| `S3` | Style: layout, formatting, naming consistency or mechanical hygiene | Automated correction or fix before merge. |

Severity is about impact, not difficulty. A one-line incorrect unit conversion
can be `B0`; an extensive formatting issue can be `S3`.

### 1.3 Enforcement tags

- `AUTO`: formatter, compiler, linter, schema validator, static checker,
  dependency scanner or policy script.
- `TEST`: unit, property, fuzz, conformance, protocol, integration, replay,
  fault-injection or end-to-end test.
- `REVIEW`: semantic human review is required.
- `EVIDENCE`: benchmark, test artifact, source hash, operational proof, signed
  record or measured result is required.

Every rule carries one or more tags. A passing formatter does not prove a state
invariant. A passing test does not prove the oracle is correct. A review comment
does not replace an executable check where a deterministic check is possible.

### 1.4 Non-waivable rules

`B0` rules are non-waivable through ordinary PR comments, author approval, AI
approval or a temporary suppression. If a `B0` rule is no longer correct, the
governing architecture or contract must change through a separate accepted
decision before implementation relies on the new rule. Security, law, exchange
requirements, data rights, financial correctness and human authorisation are
not waived by a coding-style exception.

`H1` and `M2` exceptions must identify rule ID, exact paths and symbols, reason,
impact, compensating control, owner, independent reviewer, remediation task and
expiry. Expired waivers fail CI. The author or AI agent cannot approve its own
waiver.

An anti-pattern is not a ban on a language feature. A pattern may fit a bounded
adapter, test, prototype or measured critical path while remaining unacceptable
as a system contract. A permitted exception is explicit, local, reviewed and
tested; "it is faster/easier" is not sufficient evidence.

## 2. Source-of-truth precedence

When rules conflict, stop and resolve the conflict. Do not silently choose the
rule that is easiest to satisfy.

1. Applicable law, regulation, exchange rule, contract, data licence,
   privacy/security obligation and verified account entitlement.
2. Accepted repository decisions and frozen machine-readable contracts.
3. Architecture registries, capability records, operating envelope,
   verification matrix, compliance records and Git policy.
4. This standard after adoption.
5. Official language specifications and primary documentation.
6. Existing formatter, compiler and linter configuration.
7. External engineering guides, source code, talks, blog posts and forums.

External repositories are references, not authorities. Stars, popularity,
marketing claims and reported performance establish neither correctness nor
licence compatibility nor suitability.

### 2.1 Evidence quality and use

- **Binding authority:** law, regulation, exchange rules, contract, licence and
  accepted repository decisions.
- **Primary technical authority:** official specifications, standard-library
  documentation, protocol specifications and vendor/tool documentation establish
  documented semantics, not implementation correctness.
- **Independent standards:** NIST SSDF, SLSA, OWASP ASVS, C++ Core Guidelines
  and SRE practice are adopted only where applicable to the system and threat
  model.
- **Practitioner discussions:** useful for discovering failure modes; anecdotal
  and corroborated against primary documentation, tests or a documented incident
  before creating a `B0` rule.
- **Marketing, benchmarks and social posts:** discovery only, not proof of
  correctness, security, scale, latency or production fitness.

For every adopted external rule, record the local invariant, affected boundary,
checker or reviewer, and source version or retrieval date. A URL is not a test.
Reference the toolchain version that matches the pinned toolchain.

### 2.2 Existing repository authorities

This document supplements and does not replace:

- `engineering/architecture/decisions/` and `engineering/decisions/`
- `claims/ledger.json` — canonical claim record
- `engineering/capabilities.json` — superseded inventory, retained for CAP-xxx
  traceability
- `engineering/source-ingestion-freeze.v1.json` and `engineering/git-policy.json`
- `engineering/verification-policy.md`, `engineering/style-policy.md`,
  `engineering/q-kdb-style.md`, `engineering/supply-chain.md`
- `architecture/layers.toml`, `constraints/`, `deny.toml`, `ruff.toml`,
  `.editorconfig`, `.gitattributes`, `.clang-format`, `.rustfmt.toml`,
  `clippy.toml`, `.pylintrc`, `pyproject.toml`
- `.github/workflows/`

The pinned toolchain and formatter/linter files remain the authority for
mechanical style. The external research catalogue formerly listed here was
removed under RRP-005; external guides are not authorities.

## 3. Technology ownership

| Plane | Owner | Boundary |
|---|---|---|
| Authoritative state, domain models, contracts, durable state | Rust | One writer per state; typed errors; checked arithmetic |
| Native kernels, codecs, FFI, external SDKs | C/C++ | Measured need, pinned toolchain, sanitizers, ABI contract |
| Research, experiments, statistics, tooling | Python | No order, risk, accounting or durable market authority |
| Time-series analytics | kdb+/q | Licensed runtime, explicit source, one source at a time |
| Embedded state | SQLite | Filesystem and writer assumptions declared |
| Multi-client transactional state | PostgreSQL | Backups, isolation, monitoring and recovery funded |

## 4. Universal correctness rules

| ID | Severity / tag | Rule |
|---|---|---|
| `UNI-001` | `B0 / REVIEW` | No clever, implicit or surprising behaviour over a direct implementation. |
| `UNI-006` | `B0 / TEST` | No silent coercion, clamp, wrap, truncation, default, sort or discard of invalid financial, temporal, identity or protocol data. |
| `UNI-007` | `B0 / TEST` | No replacement of a failure with an empty list, zero, `None`, `null`, false or success-shaped result unless that is the documented domain meaning. |
| `UNI-012` | `H1 / REVIEW` | No duplicate source of truth for IDs, decisions, policies, schemas, capabilities or ownership. |
| `UNI-014` | `H1 / REVIEW` | A name, comment, README, metric or status must not claim behaviour that implementation and evidence do not establish. |
| `UNI-018` | `H1 / REVIEW` | Units, precision, timezone, ownership, mutability and lifetime are visible in types or names. |
| `UNI-019` | `B0 / TEST` | Bound queues, retries, buffers, concurrency, recursion, file growth, memory and waiting time. |
| `UNI-021` | `H1 / REVIEW` | Fail closed for authoritative state when a required invariant is unknown or violated. |
| `UNI-024` | `H1 / EVIDENCE` | A performance claim names workload, source data, build, hardware, compiler, flags, warm-up, method and distribution. |
| `ERR-001` | `B0 / TEST` | No swallowed errors, ignored results, ignored exit codes or log-and-continue on authoritative state. |
| `ERR-004` | `B0 / TEST` | No retry of a non-idempotent action without idempotency, reconciliation or duplicate prevention. |
| `ERR-005` | `B0 / TEST` | No success report when a commit, order submission, cancel, replace, transfer or side effect has an unknown outcome. |
| `ERR-009` | `B0 / REVIEW` | No secrets, personal data, private order state or licensed market data in logs and errors. |
| `ERR-013` | `B0 / TEST` | A local timeout, cancellation or lost response is not proof that a remote command did not complete; preserve the idempotency key and reconcile before duplicating. |
| `DAT-001` | `B0 / REVIEW` | No instrument identity from symbol, ticker, venue token, broker token, row number or display name alone. |
| `DAT-002` | `B0 / REVIEW` | Price, quantity, fee, rate, ratio, multiplier, unit and currency are explicit. |
| `DAT-003` | `B0 / TEST` | No binary floating point for authoritative monetary values unless the domain model requires it with bounded, independently verified error. |
| `DAT-004` | `B0 / REVIEW` | Distinguish event/exchange time, receive time, processing time, publication time, valid time and knowledge time. |
| `DAT-005` | `B0 / TEST` | No timestamp comparison across precision, timezone or clock semantics without explicit conversion and validation. |
| `DAT-006` | `B0 / TEST` | Reject sequence gaps, regressions, duplicates, stale snapshots and discontinuities per feed contract; do not sort away an ordering defect. |
| `DAT-010` | `B0 / REVIEW` | No rewriting of raw captured source bytes; corrections are new immutable records or versioned transformations. |
| `DAT-014` | `B0 / REVIEW` | No present-day instrument, calendar, corporate-action or contract state in historical queries without point-in-time justification. |
| `DAT-016` | `B0 / TEST` | No conflation of sequence number and timestamp; timestamp sorting is not exchange sequencing. |

## 5. Architecture and authoritative state

| ID | Severity / tag | Rule |
|---|---|---|
| `ARC-001` | `B0 / REVIEW` | No component mutates another component's authoritative state through shared handles, mutable objects, filesystem side channels or undocumented callbacks. |
| `ARC-002` | `B0 / REVIEW` | No bypass of a declared contract by importing internals or reaching through an adapter. |
| `ARC-004` | `H1 / REVIEW` | No duplicate domain rule in adapters, clients, scripts and services; put it at its authority boundary and test mappings. |
| `ARC-008` | `H1 / REVIEW` | Separate transport, decode, normalization, validation, state transition and persistence. |
| `ARC-009` | `B0 / REVIEW` | Trading-firm OMS, risk and accounting authority stay separate from venue-operator private order, matching, participant and surveillance state. |
| `ARC-010` | `B0 / REVIEW` | Matching, clearing, settlement, accounting and broker order state stay distinct. |
| `ARC-011` | `B0 / TEST` | Classify architecture edges as authority, command, read-only data, event notification or integration; an unclassified edge touching authority fails the gate. |
| `ARC-016` | `B0 / REVIEW` | No capability activation by registry entry alone; activation requires code, entitlement, verification and owner approval. |
| `STA-001` | `B0 / REVIEW` | Identify one authoritative writer or owner for each mutable state. |
| `STA-002` | `B0 / TEST` | Do not assume one-time delivery; design for duplicates, gaps, uncertain outcomes and reconciliation. |
| `STA-003` | `B0 / TEST` | State transitions are explicit; impossible transitions are rejected. |
| `STA-004` | `B0 / REVIEW` | No in-place update of immutable financial, market or audit history; use a new event, revision, correction or compensation. |
| `STA-006` | `B0 / TEST` | No cross-resource dual write reported as atomic success without outbox, idempotency, compensation or reconciliation. |
| `STA-009` | `B0 / TEST` | Stale cache state must not override durable authority. |
| `STA-010` | `H1 / REVIEW` | Queues MUST have capacity, backpressure, overflow, replay, drop/coalesce and shutdown semantics. |
| `STA-013` | `B0 / REVIEW` | No mutable in-memory cache as the only record of an accepted order, fill, risk decision or accounting event. |
| `STA-014` | `H1 / TEST` | Test idempotency across retry, process restart, duplicate delivery and lost response. |

## 6. Market data, trading, risk and research

| ID | Severity / tag | Rule |
|---|---|---|
| `MD-001` | `B0 / REVIEW` | Define feed scope, product, venue, channel, sequence domain, symbol mapping, timestamp fields, snapshot semantics and entitlement before implementing a handler. |
| `MD-003` | `B0 / TEST` | Reject or quarantine an incremental update that cannot be applied after a sequence gap or from unknown state. |
| `MD-004` | `B0 / TEST` | No delta applied across session reset, channel reset, symbol remap or book epoch without protocol-defined recovery. |
| `MD-015` | `H1 / REVIEW` | No sorting updates by timestamp to hide exchange sequence defects. |
| `MD-016` | `H1 / EVIDENCE` | Label replay fidelity by data level: bars, trades, top of book, L2, L3 or full capture; do not infer a richer level. |
| `FIN-001` | `B0 / REVIEW` | Distinguish venue, market, segment, product, instrument, listing, contract, account, broker identifier and display symbol. |
| `FIN-009` | `B0 / REVIEW` | Do not force all venues into a central limit order book abstraction; CLOB, auction, RFQ, dealer, OTC, dark pool and AMM differ. |
| `FIN-011` | `B0 / REVIEW` | No silent future roll, synthesized option chain, applied corporate action or backfilled contract term. |
| `FIN-012` | `B0 / REVIEW` | Feed access is not a data licence, account entitlement or trade permission. |
| `BT-001` | `B0 / TEST` | Prevent look-ahead, survivorship, selection, corporate-action, constituent, symbol-mapping and publication-time leakage. |
| `BT-002` | `B0 / TEST` | Use point-in-time identity and effective-dated contract, calendar, fee, borrow and margin data. |
| `BT-004` | `B0 / REVIEW` | Do not equate daily bars, OHLC, trade prints, quotes, L2 and L3 data; fill and queue models must match the evidence. |
| `BT-008` | `H1 / EVIDENCE` | Every experiment retains dataset hashes, point-in-time policy, code SHA, config, seeds, dependency versions and full trial count. |
| `BT-011` | `B0 / REVIEW` | No repeated tuning on the holdout while still calling it out-of-sample. |
| `BT-014` | `B0 / REVIEW` | Do not infer persistent live alpha from synthetic fills, one regime or one favourable interval. |
| `ORD-001` | `B0 / REVIEW` | Strategy, model, AI agent, adapter and execution algorithm do not bypass independent risk authority. |
| `ORD-003` | `B0 / TEST` | HTTP success, socket write, broker acceptance and venue acknowledgement are not execution. |
| `ORD-005` | `B0 / REVIEW` | No mutating a fill or trade to correct it; use cancel, correct, bust or compensating entries. |
| `ORD-007` | `B0 / TEST` | Risk fails closed on missing, stale, inconsistent or unknown limits, prices, positions, buying power, margin and account state. |
| `ORD-008` | `B0 / REVIEW` | A `risk_ok` boolean without timestamp, input version, policy version, scope and reason is forbidden. |
| `ORD-009` | `B0 / REVIEW` | Risk overrides require human authority, scope, reason, expiry, independent approval and audit; an AI cannot approve its own override. |
| `ORD-013` | `B0 / REVIEW` | Adapters translate transport and protocol; they are not alternate risk, order or accounting authorities. |
| `ORD-017` | `B0 / REVIEW` | Live submission stays disabled until risk, broker entitlement, compliance, operations and human approval gates pass. |
| `ML-002` | `B0 / TEST` | No training, feature selection, tuning, calibration or imputation using information unavailable at decision time. |
| `ML-004` | `B0 / REVIEW` | No direct promotion of a research model into a live decision path; independent validation, signed artifact and approval are required. |
| `ML-006` | `B0 / TEST` | Fail closed or enter a documented safe mode when required model inputs are stale, missing, out of schema or outside the validated domain. |

## 7. Language, build and test rules

| ID | Severity / tag | Rule |
|---|---|---|
| `RS-001` | `B0 / AUTO` | No `unwrap`, `expect`, `panic!`, `unreachable!` or unchecked indexing on untrusted input or ordinary operational failure in production paths. |
| `RS-002` | `B0 / REVIEW` | No `unwrap_or_default`, `unwrap_or(0)` or `unwrap_or(false)` to hide parse, missing-state or validation errors. |
| `RS-003` | `H1 / REVIEW` | No blanket Clippy or compiler warning suppression at crate or workspace scope. |
| `RS-004` | `B0 / REVIEW` | Every `unsafe` block carries a `SAFETY` argument covering invariants, aliasing, validity, lifetime and soundness. |
| `RS-006` | `B0 / REVIEW` | Checked conversions and arithmetic for money, quantities, IDs, timestamps, lengths and counters. |
| `RS-014` | `B0 / REVIEW` | Persistent or untrusted data needs versioned decoding, strict field policy and semantic validation before exposure. |
| `RS-019` | `B0 / REVIEW` | Do not rely on destructors alone for durability; flush, sync, commit and verify explicitly. |
| `RS-021` | `H1 / REVIEW` | New crates require registration, API intent, owner, tests and dependency-boundary justification. |
| `NET-001` | `B0 / TEST` | Validate frame length, header, version, endian, alignment, sequence, checksum and variable-length fields before decoding. |
| `NET-002` | `B0 / REVIEW` | No reinterpretation of packet bytes as native structs unless layout, alignment, padding, endian and bounds are contractually identical and validated. |
| `NET-006` | `H1 / REVIEW` | A successful send syscall is not venue receipt, acceptance or execution. |
| `NET-010` | `H1 / REVIEW` | Keep raw capture, parser version and normalized record lineage distinct. |
| `HOT-003` | `H1 / REVIEW` | No zero-copy claim if any boundary copies, serializes, deserializes or reallocates the data. |
| `TST-001` | `B0 / TEST` | A regression test fails against the prior defective behavior and names the invariant it protects. |
| `TST-002` | `H1 / TEST` | Test malformed, missing, stale, duplicate, reordered and unknown input. |
| `TST-003` | `H1 / TEST` | Test crash, restart, disk full, read-only filesystem, network loss, timeout and uncertain commit paths. |
| `SEC-001` | `B0 / REVIEW` | No secrets, credentials or account identifiers in Git; rights stay `UNKNOWN` until written evidence exists. |
| `SEC-002` | `B0 / REVIEW` | No scraping, ban evasion, CAPTCHA bypass, identity rotation, rate-limit circumvention or undocumented endpoints. |
| `SEC-003` | `H1 / AUTO` | Pin actions, review advisories and licences, and record provenance for every published artifact. |
| `AI-001` | `H1 / REVIEW` | AI-generated changes name the invariant, owner and falsifier; the agent cannot approve its own waiver or experiment. |
| `AI-002` | `B0 / REVIEW` | No AI claim of profitability, live readiness, zero loss, nanosecond precision or data rights without a dated, independently reviewed evidence package. |

Rule IDs are permanent across the consolidation. The enumerated long-form rules
of the `0.3.0` draft remain in Git history at the digest in the header; this
table keeps the invariant and severity of each retained rule. A retired ID keeps
its place with a `RETIRED` marker and a replacement link.

## 8. Enforcement roadmap

This guide does not claim every rule is automated. Implement enforcement in
stages, with an owner, checker, positive fixture, negative fixture and
false-positive policy for each automated rule.

| Stage | Gate | Scope |
|---|---|---|
| `E0` | Existing format/compiler gates | Pinned rustfmt, Clippy, Ruff, clang-format, compiler warnings |
| `E1` | Hygiene | Forbidden paths, generated noise, placeholders, TODO ownership, secrets, diff scope |
| `E2` | Registry/schema integrity | Strict JSON/YAML, duplicate keys, schema, IDs, paths, dangling refs, cycles, task mapping |
| `E3` | Language static analysis | Rust, C/C++, Python, shell, SQL and q checks |
| `E4` | Contract/conformance | Cross-language numeric/encoding parity, protocol, versioning, authority boundaries |
| `E5` | Failure/resilience | Crash, ENOSPC, read-only, locking, duplicate/reorder, restart, replay and restore |
| `E6` | Security/supply chain | Secrets, dependency audit, licences, provenance, permissions and threat model |
| `E7` | Performance/capacity | Reproducible benchmarks, tail latency, memory, disk, throughput and operating envelope |
| `E8` | Operational certification | Soak, incident drill, recovery, failover, restore, entitlement and live gate evidence |

Do not implement this with fragile regexes pretending to understand every
language. Prefer compiler/linter APIs, syntax trees, type checks, schema
validators, dependency tools, architecture graphs and targeted tests. High-risk
semantic rules require review and domain tests.

Every machine-enforced rule record contains: stable ID, severity, subsystem
scope, enforcement type, checker or test ID, positive fixture, negative fixture,
owner, exception policy, version and review date.

### 8.1 Canonical gate wiring

The enforcement stages map to unchanged repository entry points:

| Entry point | Scope |
|---|---|
| `make check` | full gate: `check-static`, Rust tests and Clippy, schema validation, Python tests |
| `make check-static` | `docs-check`, `freeze-check`, `shellcheck-check`, `generated-paths-check`, `pollution-check`, `cargo fmt --check`, `cargo xtask check`, `cargo deny check`, `style` |
| `make style` | `rust-style`, `source-style`, `python-ruff-style`, `q-style`, `prose-style`, `references-check`, `standards-check`, `style-gate` |
| `make freeze-check` | RRP-004 frozen-path guard with self-test |
| `make docs-check` | canonical and dated document presence and local links |
| `make python-ruff-style` | pinned Ruff check on added Python lines |
| `make sbom` | CycloneDX SBOM build artifact via `scripts/supply_chain.py` |

Removing or weakening an entry in the `Makefile`, a CI workflow or the style
gate integrity checker fails the gate. `scripts/check_style_gate.py` verifies the
wiring and self-test inventory in both `make check` and CI.

## 9. Review protocol and branch admission

### 9.1 Before coding

The author identifies one coherent task, reads its source-of-truth context,
identifies the owner of every changed state, defines data, time and units, lists
failure modes, identifies test layers, reviews new dependencies, licences and
entitlements, and defines acceptance gates.

### 9.2 During coding

Keep commits scoped. Add tests with code. Preserve old contracts until approved
migration. Stop when a frozen contract is contradicted. Do not change evidence
or policy to make work pass.

### 9.3 Before PR

Run relevant format, lint, build, test, architecture, dependency, security,
schema and evidence gates. Read the complete diff. Search for placeholders,
unsafe defaults, ignored errors, generated noise, secrets, unrelated changes and
unsupported claims. Confirm the exact head SHA.

### 9.4 Reviewer questions

1. What invariant is protected?
2. Who owns the changed state?
3. What happens with malformed, missing, stale, duplicate, reordered or unknown input?
4. What happens on process crash, power loss, disk full, network loss, timeout or uncertain commit?
5. Can the result be replayed and independently reconstructed?
6. Are units, time semantics, rounding, identity and provenance explicit?
7. Are cross-language and external contracts preserved?
8. Do tests cover failure paths and would they fail against the old defect?
9. Does evidence prove this exact SHA and workload?
10. Is the change simpler than the alternatives?
11. What capability or claim does it establish, and what does it not establish?
12. What is rollback, restore or compensation?
13. Does a performance claim justify added complexity?
14. Does a new language, crate, database or service have a real ownership boundary?

### 9.5 Protected main

Configure GitHub rulesets and branch protection to:

- Require PRs and independent approval; require code-owner review for sensitive paths.
- Require exact-head CI, architecture, C/C++, sanitizers, security/dependency and aggregate evidence checks as defined by the project.
- Require conversation resolution and current checks.
- Enforce squash merge and linear history per Git policy.
- Restrict direct push, force push and deletion.
- Disable ordinary bypass for administrators, bots and automation.
- Protect `.github/workflows/`, `engineering/`, persistence, risk, OMS,
  broker/venue adapters, compliance and live execution paths.
- Required checks must run; obsolete-SHA checks and skipped required jobs do not
  satisfy admission.

A green check from an earlier SHA is not evidence for the current head. A
platform outage is not a code failure, but it is also not a pass. Keep required
evidence blocked until it exists or a formally approved policy change changes
the requirement.

## 10. Minimum pre-merge checklist

- [ ] One coherent task and explicit scope.
- [ ] Relevant decisions, contracts, capability state and component ownership read.
- [ ] No unowned crate, module, service, registry or dependency.
- [ ] No hidden state owner, unclassified edge or authority bypass.
- [ ] No ambiguous financial values or units.
- [ ] No point-in-time leakage or unproven fill assumptions.
- [ ] Input, schema, version and time semantics validated.
- [ ] Error, timeout, retry and uncertain-outcome handling explicit.
- [ ] Queue, resource, memory and latency bounds defined.
- [ ] Persistence, crash and restore paths tested when relevant.
- [ ] Regression tests fail against prior defective behaviour.
- [ ] Property, fuzz, conformance, sanitizer and performance checks considered; omissions justified.
- [ ] Dependency, security, licence and data-rights review complete.
- [ ] Docs, architecture, capability, evidence and verification matrix updated where necessary.
- [ ] No fake adapter, placeholder, no-op, hidden skip or dead code.
- [ ] Formatting, lint, build, tests and required gates pass.
- [ ] Exact-head evidence current.
- [ ] Independent review complete; no unresolved required conversations.
- [ ] No unsupported alpha, HFT, live, production or venue claim.
- [ ] Human approval recorded for sensitive operation.

## 11. Official verification profiles

This section maps the policy to tool families. It does not assert that every
tool is installed or approved in this repository. The pinned files and existing
CI workflow define the actual toolchain. Before enabling a command, verify its
compatibility with the repository's compiler or runtime version, target
platforms and configuration. Do not introduce a second formatter or conflicting
linter policy.

### 11.1 Rust and Cargo

- The pinned `rust-toolchain.toml`, `Cargo.lock`, workspace manifests and Clippy
  configuration are authoritative. Do not follow `stable` or `nightly` because a
  blog post uses it.
- Admission checks come from repository policy: `cargo fmt --all -- --check`,
  Clippy with warnings denied over the supported matrix, and `cargo test` over
  the required matrix. `--all-features` is not automatically correct when
  features are mutually exclusive or change semantics.
- Use `cargo deny check` or the configured dependency scanner for licence,
  advisory, source and duplicate-dependency review. Keep the lockfile current
  with intentional dependency changes.
- Miri, Loom and fuzzers are targeted evidence, not universal proofs.
- Do not globally suppress Clippy or rustc warnings. A local suppression needs a
  reason, narrow scope, owner and expiry or task.
- Each `unsafe` block documents aliasing, alignment, initialization, provenance,
  lifetime and thread-safety assumptions.

### 11.2 C and C++ toolchain

- Use the pinned compiler, standard library, language mode, formatter, CMake and
  dependency versions. One compiler is not portability evidence.
- Use `clang-format`, the configured static analyser, CTest and independently
  compiled public headers.
- Run AddressSanitizer, UndefinedBehaviorSanitizer and ThreadSanitizer in
  dedicated configurations; do not claim one sanitizer covers another's classes.
- Prefer target-scoped CMake properties over global directory state. Generated
  build directories stay separate from source.

### 11.3 Python, notebooks and native extensions

- Use the pinned Python runtime and lockfile. `pyproject.toml` defines the
  Ruff, type-check and test configuration. Do not run competing formatters on
  the same files.
- Imports stay side-effect-light; explicit entry points own I/O, process
  creation, environment changes and network access.
- For asyncio, define task ownership and cancellation. Cancellation is not proof
  that a remote side effect did not happen.
- Test the exact interpreter build and native extensions for free-threaded
  CPython; do not rely on incidental GIL serialization.
- Notebooks record runtime, package lock, data hash, parameters, seeds and
  source revision; reusable reviewed logic lives in importable modules.

### 11.4 SQL, SQLite and PostgreSQL

- Use explicit migrations, named constraints, parameter binding and the target
  engine's real isolation and locking semantics. Dialect differences are part of
  the contract.
- For SQLite, set and read back connection pragmas, verify foreign keys and the
  journal, synchronous and busy policy. WAL is not a network-filesystem protocol.
  Use the online backup API or `VACUUM INTO`, then restore-test the result.
- For PostgreSQL, test isolation, serialization and deadlock retry, transaction
  timeouts, connection-loss-at-commit reconciliation and replica policy. Use
  `EXPLAIN (ANALYZE, BUFFERS)` and test base backup plus WAL or PITR recovery.
- A local SQLite result is not proof of PostgreSQL behavior, or a reverse.

### 11.5 kdb+/q and time-series systems

- Verify the licensed runtime and version, schema, data types, symbol
  cardinality and temporal semantics at every boundary. Typed nulls, infinities,
  symbol interning, attributes and table ordering differ from SQL and Python.
- Keep ingestion, intraday and historical responsibilities explicit where that
  architecture is used; test log replay, subscriber recovery, rollover,
  retention and query isolation.
- Do not promote vendor examples into a production architecture without
  validating the exact product, licence, deployment and version.

### 11.6 Shell, Make, CMake and GitHub Actions

- Run ShellCheck on maintained shell scripts. Quote expansions, use arrays for
  argument vectors, avoid `eval`, check pipeline failures explicitly and test
  cleanup paths. `set -e` is not a substitute for error handling.
- Use one canonical build entry point. Make targets declare dependencies and
  outputs; CMake configuration is target-scoped and reproducible.
- Pin third-party actions by verified full commit SHA where no lockfile exists.
  Minimise job permissions and keep secrets out of untrusted pull-request
  execution. A privileged `pull_request_target` job must not execute PR code
  that it checks out with secrets.
- Validate that branch protection enforces required review, exact-head checks,
  conversation resolution, restricted bypass and protected-path ownership.
  Screenshots and policy documents are not proof of effective settings.

### 11.7 Toolchain change policy

A language, runtime, compiler, database or CI tool upgrade is its own reviewed
change. It identifies old and new versions, release notes and advisories,
changed defaults or semantics, the supported target matrix, lockfile, ABI and
schema effects, benchmark or compatibility evidence, a rollback plan and the
owner who maintains the new toolchain. A broad toolchain upgrade is not smuggled
into an unrelated fix.

## 12. Maintenance and governance

- The engineering owner maintains the standard with language, quantitative,
  data, security and operations reviewers.
- Changes to this standard require independent review.
- Changes to `B0` rules require an accepted decision explaining the invariant change.
- Rule IDs are permanent. Retire a rule with a `RETIRED` marker and replacement
  link; do not reuse its ID.
- Review after architecture, toolchain, language, database, asset-class, venue,
  security or incident changes, and at least once per release cycle.
- New languages and subsystems need a dedicated section before production use.
- Add rules from incidents and real defects, not speculative prohibitions.
- Keep this policy aligned with actual tools; policy that CI cannot enforce and
  reviewers do not apply becomes documentation theatre.
- A missing tool is not permission to skip the invariant. Record a compensating
  test or review control and an owner for adding the tool when warranted.

**Governing principle:** code is admitted by contract and evidence, not author
confidence, model confidence, language fashion, code volume, cleverness,
popularity or a green check that did not test the changed behaviour.
