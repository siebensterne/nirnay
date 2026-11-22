# Engineering Anti-Patterns and Code Admission Standard

**Document ID:** `ENG-ANTI-PATTERNS`  
**Version:** `0.3.0`  
**Status:** Research-enriched draft; repository adoption and gate mapping pending  
**Proposed path:** `engineering/anti-patterns.md`  
**Applies to:** Human contributors, reviewers, maintainers, AI agents, generated-code producers, build tools, bots and deployment automation  
**Repository scope:** Every language, package, service, schema, test, script, build, data contract, operational procedure and release artifact  
**Research review date:** 2026-11-12  
**Policy owner:** Repository engineering owner (to be assigned)  
**Adoption state:** Proposed; do not treat this document as evidence that all controls are implemented

---

## 0. Mandate

This standard defines what must not be introduced, what must be rejected in review, and what must not be merged into protected `main`.

It is not a formatter guide. Formatting is delegated to the repository's pinned formatters. This standard governs correctness, financial semantics, architecture, ownership, concurrency, performance, determinism, data integrity, security, testing, operational resilience, evidence and AI-generated contributions.

The project language and runtime hierarchy is deliberate:

1. **Rust is the primary implementation language** and the default owner of authoritative infrastructure, domain models, data contracts, state machines, durable state, orchestration, control-plane APIs and performance-sensitive systems code.
2. **C and C++ are secondary implementation languages** for measured native kernels, specialised codecs, hardware interfaces, FFI boundaries, external SDKs and subsystems whose requirements justify native implementation. C/C++ is not a default escape hatch from Rust ownership or safety contracts.
3. **Python is the research, scientific-computing, ML, experimentation, test-tooling and automation language.** It may orchestrate Rust capabilities, but must not become a hidden authority for order, risk, accounting or durable market state.
4. **kdb+/q is an optional time-series and analytical plane.** It may provide event capture, columnar analytics, intraday/historical projections and research queries when the product requirement, licence and operating model justify it. It is not automatically the authoritative ledger or OMS database.
5. **SQLite and PostgreSQL are distinct storage choices with distinct operating contracts.** SQLite is suitable for embedded/local-first state under its filesystem and writer assumptions. PostgreSQL is a server database for multi-client transactional workloads when operational ownership, backups, isolation, monitoring and recovery are funded. They are not interchangeable by renaming a connection string.
6. **Shell, Make and CMake orchestrate builds and operations.** They must not become the primary location for domain logic, financial calculations or state transitions.

The default is not "use every language". The default is Rust. A secondary language or technology must earn its place through a concrete boundary, an owner, a contract, tests, measured need and operational evidence.

This standard is intended to prepare the repository for future high-performance trading and market infrastructure. It does not claim that the current project is an HFT system, a regulated exchange, an institutional production platform or equivalent to a trillion-dollar firm. Such claims require measured operation, real data and venue access, hardware evidence, control testing, incident drills, independent review and legal/regulatory authorisation. A language choice, benchmark screenshot or architecture diagram does not establish them.

**No policy can prevent an author from pushing an unreviewed topic branch to a remote.** The enforceable objective is that unqualified changes cannot merge into protected `main`, and that local/CI gates detect defects before review wherever practical. Protected-branch rules and permissions are part of this standard, not an optional administrative detail.

## 1. Normative language, severity and enforcement

### 1.1 Normative words

- **MUST / MUST NOT**: requirement. A violation blocks merge.
- **SHOULD / SHOULD NOT**: default requirement. A deviation needs a written rationale and reviewer acceptance.
- **MAY**: permitted option, not a recommendation.
- **BLOCKER**: a rule violation that can corrupt data, compromise security, bypass financial controls, falsify evidence or violate an accepted contract.

### 1.2 Severity classes

| Class | Meaning | Admission treatment |
|---|---|---|
| `B0` | Blocker: safety, financial correctness, state ownership, data integrity, security, authority bypass, false evidence or frozen-contract violation | Reject. No ordinary waiver. |
| `H1` | High: material reliability, performance, architecture, reproducibility, operability or long-term maintenance defect | Fix before merge. A temporary exception needs independent approval, owner, compensating control and expiry. |
| `M2` | Medium: maintainability, clarity, testability, naming or operational friction | Fix before merge unless a reviewer accepts a tracked follow-up. |
| `S3` | Style: layout, formatting, naming consistency or mechanical hygiene | Automated correction or fix before merge. |

Severity is about impact, not how difficult a fix is. A one-line incorrect unit conversion can be `B0`; a large formatting issue can be `S3`.

An anti-pattern is not a ban on a language feature. Evaluate it in context: a pattern may be appropriate inside a bounded adapter, test, prototype or measured critical path while being unacceptable as a general system contract. A permitted exception must be explicit, local, reviewed and tested; "it is faster/easier" is not sufficient evidence.

### 1.3 Enforcement tags

- `AUTO`: formatter, compiler, linter, schema validator, static checker, dependency scanner or policy script.
- `TEST`: unit, property, fuzz, conformance, protocol, integration, replay, fault-injection or end-to-end test.
- `REVIEW`: semantic human review is required.
- `EVIDENCE`: benchmark, test artifact, source hash, operational proof, signed record or measured result is required.

Every rule carries one or more tags. A passing formatter does not prove a state invariant. A passing test does not prove the oracle is correct. A review comment does not replace an executable check where a deterministic check is possible.

### 1.4 Non-waivable rules

`B0` rules are non-waivable through ordinary PR comments, author approval, AI approval or a temporary suppression. If a `B0` rule is no longer correct, the governing architecture/contract must be changed through a separate accepted decision before implementation can rely on the new rule. Security, law, exchange requirements, data rights, financial correctness and human authorisation are never waived by a coding-style exception.

`H1` and `M2` exceptions must identify rule ID, exact paths/symbols, reason, impact, compensating control, owner, independent reviewer, remediation task and expiry. Expired waivers fail CI. The author or AI agent cannot approve its own waiver.

## 2. Source-of-truth precedence

When rules conflict, stop and resolve the conflict. Do not silently choose whichever rule is easiest to satisfy.

Precedence:

1. Applicable law, regulation, exchange rule, contract, data licence, privacy/security obligation and verified account entitlement.
2. Accepted repository decisions and frozen machine-readable contracts.
3. Repository architecture registries, capability records, operating envelope, verification matrix, compliance records and Git policy.
4. This standard after adoption.
5. Official language specifications and primary documentation.
6. Existing formatter, compiler and linter configuration.
7. External engineering guides, source code, talks, blog posts and forum discussions.

External repositories are references, not authorities. Stars, popularity, marketing claims and reported performance do not establish correctness, licence compatibility or suitability.

### 2.1 Evidence quality and use

Use sources according to what they can prove:

1. **Binding authority:** applicable law, regulation, exchange rules, contract, licence and accepted repository decisions.
2. **Primary technical authority:** official language specifications, standard-library/runtime documentation, protocol specifications, database manuals and vendor/tool documentation. These establish documented semantics and supported behavior, not that the implementation is correct.
3. **Independent engineering standards:** NIST SSDF, SLSA, OWASP ASVS, C++ Core Guidelines, SRE practices and similar guidance. Adopt only the controls applicable to the system and threat model.
4. **Practitioner discussions:** Reddit, issue trackers, mailing lists, Q&A and incident write-ups. These are useful for discovering failure modes and operational surprises; they are anecdotal, may be incomplete or wrong, and MUST NOT independently create a `B0` rule. Corroborate material claims against primary documentation, tests or a documented incident.
5. **Marketing, benchmarks and social posts:** discovery only. Never proof of correctness, security, scale, latency or production fitness.

For every adopted external rule, record the local invariant it protects, the affected boundary, the checker/test/reviewer and the source version or retrieval date. A URL is not a substitute for a test. When official documentation is versioned, reference the version matching the pinned toolchain; avoid copying advice from a different release without checking compatibility.

### 2.2 Existing repository authorities

This document supplements and does not replace:

- `engineering/decisions.json`
- `engineering/index.json`
- `engineering/capabilities.json`
- `engineering/verification-matrix.json`
- `engineering/target-platform.json`
- `engineering/compliance/`
- `engineering/reference-store-contract.json`
- `engineering/git-policy.json`
- `engineering/operating-envelope.json`
- `engineering/reference-projects.json`
- `.editorconfig`, `.gitattributes`, `.clang-format`, `.rustfmt.toml`,
  `clippy.toml`, `.pylintrc`, `rust-toolchain.toml`, `pyproject.toml`,
  `deny.toml`
- `.github/workflows/`

The pinned toolchain and existing formatter/linter files remain the authority for mechanical style. This guide must not create a competing formatter configuration.

A registry entry is not implementation. Implementation is not verification. Verification is not entitlement. A successful connection is not permission to trade. A synthetic fixture is not production market data. A paper fill is not proof of live execution quality.

## 3. Language and system ownership charter

### 3.1 Rust-first charter

Rust is the default for:

- Canonical instrument, order, execution, position, cash, risk, market and venue-domain types.
- Authoritative state machines and transitions.
- Market-data acquisition, sequencing, validation and normalisation where no external SDK requires another language.
- Event journals, durable metadata, immutable histories and replay authorities.
- OMS/EMS, independent risk control, broker/venue adapters and accounting contracts.
- Concurrency, process coordination, operational daemons and local control-plane services.
- Typed FFI boundaries and high-performance components where Rust performance is sufficient.

Anti-patterns:

- `LANG-RUST-001` `B0 / REVIEW`: MUST NOT move an authoritative state machine into Python, q, SQL
  triggers, shell or an unreviewed native library for speed alone.
- `LANG-RUST-002` `H1 / REVIEW`: MUST NOT split one authoritative state across Rust and another language without a formal authority, transaction and reconciliation contract.
- `LANG-RUST-003` `H1 / REVIEW`: MUST NOT introduce a new Rust crate to simulate modularity. Each crate needs a distinct domain/build/API boundary, an owner, a test plan and architecture registration.
- `LANG-RUST-004` `H1 / REVIEW`: MUST NOT expose internal Rust implementation types as a cross-language wire contract without explicit versioning and compatibility policy.
- `LANG-RUST-005` `B0 / REVIEW`: MUST NOT permit Python/q/C++ to mutate Rust-owned state through shared memory or FFI without an explicit command/event boundary and validation.

### 3.2 C/C++ charter

C/C++ is justified for a measured kernel, specialised protocol implementation, hardware/NIC integration, external library, ABI or compatibility need. It is not justified by a vague claim that "C++ is faster".

Anti-patterns:

- `LANG-CPP-001` `H1 / EVIDENCE`: MUST NOT move logic from Rust to C++ without a benchmark or external requirement identifying the bottleneck/boundary.
- `LANG-CPP-002` `B0 / REVIEW`: MUST NOT create a second source of financial semantics, rounding, overflow or state-transition rules in C/C++.
- `LANG-CPP-003` `B0 / REVIEW`: MUST NOT make C++ authoritative for order/risk/ledger state without a separately accepted architecture decision.
- `LANG-CPP-004` `H1 / REVIEW`: MUST NOT leak STL containers, compiler ABI, exceptions, allocator ownership or compiler-specific layout through a stable ABI.
- `LANG-CPP-005` `B0 / TEST`: Cross-language numeric and event contracts MUST have differential tests against the Rust authority.

### 3.3 Python charter

Python is the primary interface for research, data exploration, model training, analytics, experiment automation, backtest orchestration and developer tooling.

Anti-patterns:

- `LANG-PY-001` `B0 / REVIEW`: MUST NOT allow a notebook or Python process to become the unreviewed live order/risk authority.
- `LANG-PY-002` `H1 / REVIEW`: MUST NOT duplicate Rust business rules in Python. Python can call or independently test the authoritative contract; it must not drift into a parallel source of truth.
- `LANG-PY-003` `H1 / REVIEW`: MUST NOT keep production logic solely in notebooks.
- `LANG-PY-004` `H1 / EVIDENCE`: Research outputs MUST include code SHA, dataset hash, schema version, dependency lock, configuration, random seed(s), model parameters and execution command.
- `LANG-PY-005` `H1 / REVIEW`: MUST NOT assume the GIL makes NumPy arrays, extension state, shared memory or free-threaded Python mutations safe.

### 3.4 kdb+/q charter

q may serve as a specialised time-series store/query engine and streaming analytics plane. The choice must be justified by measured workload, licensing, operational skills, recovery requirements and integration cost.

Anti-patterns:

- `LANG-Q-001` `B0 / REVIEW`: MUST NOT move the authoritative order, risk, fill or accounting ledger into q solely because it is a time-series database.
- `LANG-Q-002` `H1 / REVIEW`: MUST NOT make q the unversioned second authority for a Rust-owned event or instrument contract.
- `LANG-Q-003` `H1 / REVIEW`: MUST NOT deploy kdb+ without verifying licensing, host sizing, backup/replay, operations, monitoring and data entitlements.
- `LANG-Q-004` `H1 / TEST`: MUST validate q null, infinity, symbol, temporal, attribute and vector semantics at every interface.

### 3.5 SQL charter

SQLite and PostgreSQL each require an explicit decision:

- SQLite: embedded/local-first, single-node, database file with WAL and filesystem/locking assumptions. Suitable for local metadata and bounded transactional state where its write model is appropriate.
- PostgreSQL: server database for concurrent clients, transactional workflows and shared service state when network, operations, backups, access control, capacity and recovery are explicitly engineered.
- DuckDB/ClickHouse/columnar engines: analytics and scan/aggregation workloads, not automatically order/ledger authorities.
- kdb+: specialised time-series systems; apply its own operational model.

Anti-patterns:

- `LANG-SQL-001` `B0 / REVIEW`: MUST NOT select a database by fashion or benchmark headline. State workload, consistency, concurrency, recovery, retention and operational assumptions.
- `LANG-SQL-002` `B0 / REVIEW`: MUST NOT put an embedded SQLite WAL store on unsupported network/cloud-synced storage.
- `LANG-SQL-003` `B0 / REVIEW`: MUST NOT treat SQLite and PostgreSQL as semantically interchangeable. Locking, MVCC, isolation, replication, backup and operational failure modes differ.
- `LANG-SQL-004` `H1 / REVIEW`: MUST NOT introduce PostgreSQL before there is a concrete multi-client/server requirement and an owner for operations, backups, upgrades, access control and recovery.
- `LANG-SQL-005` `H1 / REVIEW`: MUST NOT turn every analytical workload into a transactional database workload or every transactional state into a columnar store.

### 3.6 Shell, Make and build tools

- `LANG-BUILD-001` `H1 / REVIEW`: MUST NOT put financial calculations, domain state transitions, complex parsers or business rules in Make, shell or CMake.
- `LANG-BUILD-002` `H1 / REVIEW`: MUST NOT duplicate build logic across Makefiles, shell scripts, Cargo xtasks and CI workflows without one canonical entrypoint.
- `LANG-BUILD-003` `H1 / TEST`: Build and release commands MUST fail on missing tools, failed pipelines and missing artifacts.
- `LANG-BUILD-004` `H1 / REVIEW`: Build scripts MUST be idempotent or explicitly state why they are not.

## 4. Repository-wide design and correctness anti-patterns

### 4.1 Complexity and abstractions

- `UNI-001` `B0 / REVIEW`: MUST NOT prefer clever, implicit or surprising behaviour over a direct implementation.
- `UNI-002` `H1 / REVIEW`: MUST NOT introduce an abstraction without naming the invariant, coupling or substitution requirement it removes.
- `UNI-003` `H1 / REVIEW`: MUST NOT generalise beyond a demonstrated second use case unless a stable contract requires it.
- `UNI-004` `H1 / REVIEW`: MUST NOT hide control flow in macros, decorators, reflection, metaprogramming, dynamic evaluation or callback chains without a demonstrated need.
- `UNI-005` `H1 / REVIEW`: MUST NOT rely on undefined, unspecified, implementation-defined or undocumented behaviour.
- `UNI-006` `B0 / TEST`: MUST NOT silently coerce, clamp, wrap, truncate, default, sort or discard invalid financial, temporal, identity or protocol data.
- `UNI-007` `B0 / TEST`: MUST NOT replace a failure with an empty list, zero, `None`, `null`, false or a success-shaped result unless that value is the documented domain meaning.
- `UNI-008` `H1 / REVIEW`: MUST NOT create generic frameworks, plugin systems, registries, dependency-injection containers, service layers or database abstractions without a concrete requirement.
- `UNI-009` `M2 / REVIEW`: Modules MUST have cohesive responsibilities; public APIs MUST be smaller than their implementation surfaces.
- `UNI-010` `H1 / REVIEW`: MUST NOT add hidden global mutable state or implicit cross-module ownership.
- `UNI-011` `H1 / REVIEW`: MUST NOT introduce cyclic architectural dependencies.
- `UNI-012` `H1 / REVIEW`: MUST NOT create duplicate sources of truth for IDs, decisions, policies, schemas, capabilities or ownership.
- `UNI-013` `M2 / REVIEW`: Comments MUST explain intent, invariants, constraints or trade-offs—not restate syntax.
- `UNI-014` `H1 / REVIEW`: A name, comment, README, metric or status MUST NOT claim behaviour that implementation and evidence do not establish.
- `UNI-015` `M2 / REVIEW`: MUST NOT leave dead code, commented-out code, fake methods, placeholder outputs, unused flags or stale scaffolding.
- `UNI-016` `H1 / REVIEW`: Every deferred production TODO MUST name a task/owner and explain why deferral is safe. Critical TODOs block merge.
- `UNI-017` `H1 / REVIEW`: MUST NOT mix unrelated refactors, formatting or generated changes into a narrow correctness change.
- `UNI-018` `H1 / REVIEW`: MUST make units, precision, timezone, ownership, mutability and lifetime visible in types or names.
- `UNI-019` `B0 / TEST`: MUST bound queues, retries, buffers, concurrency, recursion, file growth, memory and waiting time.
- `UNI-020` `H1 / REVIEW`: MUST NOT use a hidden environment-specific branch as a replacement for an explicit platform contract.
- `UNI-021` `H1 / REVIEW`: MUST fail closed for authoritative state when a required invariant is unknown or violated.
- `UNI-022` `H1 / REVIEW`: MUST NOT let filesystem iteration, map order, thread completion or wall-clock jitter become implicit deterministic ordering.
- `UNI-023` `H1 / REVIEW`: MUST NOT optimise correctness away for a performance improvement without measurement and an approved contract.
- `UNI-024` `H1 / EVIDENCE`: Every performance claim MUST name workload, source data, build, hardware, compiler, flags, warm-up, measurement method and distribution.

### 4.2 Error and failure handling

- `ERR-001` `B0 / TEST`: MUST NOT swallow errors with empty catch blocks, ignored results, ignored exit codes or log-and-continue on authoritative state.
- `ERR-002` `H1 / REVIEW`: Errors MUST preserve safe context: operation, resource, identifier, venue, sequence, transaction, file or version.
- `ERR-003` `H1 / REVIEW`: MUST distinguish malformed input, unsupported capability, transient failure, permanent failure, stale state, conflict, timeout, cancellation and uncertain outcome.
- `ERR-004` `B0 / TEST`: MUST NOT retry a non-idempotent action without idempotency, reconciliation or duplicate-prevention semantics.
- `ERR-005` `B0 / TEST`: MUST NOT report success when a commit, order submission, cancel, replace, transfer or external side effect has an unknown outcome.
- `ERR-006` `H1 / REVIEW`: Every network/database/filesystem boundary MUST define deadline, timeout, retry budget, backoff, cancellation and terminal failure.
- `ERR-007` `H1 / REVIEW`: MUST NOT retry permanent errors without a state change.
- `ERR-008` `H1 / REVIEW`: Cleanup/rollback failure MUST NOT erase the original failure. Report both.
- `ERR-009` `B0 / REVIEW`: MUST NOT leak secrets, personal data, private order state or licensed market data in logs and errors.
- `ERR-010` `B0 / TEST`: An uncertain durable or external action MUST be reconciled against authoritative state before replay.
- `ERR-011` `H1 / REVIEW`: MUST NOT use one catch-all error type where callers need to distinguish retryable, permanent, conflict and safety failures.
- `ERR-012` `H1 / TEST`: Error handling MUST itself be tested: rollback failure, cleanup failure, cancellation during I/O and secondary failure paths.
- `ERR-013` `B0 / TEST`: A local timeout, cancellation or lost response MUST NOT be interpreted as proof that a remote command did not complete. Preserve the idempotency key and reconcile the remote/authoritative state before issuing a duplicate command.

### 4.3 Data and time semantics

- `DAT-001` `B0 / REVIEW`: MUST NOT identify an instrument solely by symbol, ticker, venue token, broker token, row number or display name.
- `DAT-002` `B0 / REVIEW`: MUST represent price, quantity, fee, rate, ratio, multiplier, unit and currency explicitly.
- `DAT-003` `B0 / TEST`: MUST NOT route authoritative monetary values through binary floating point unless the domain model explicitly requires it and its error is bounded and independently verified.
- `DAT-004` `B0 / REVIEW`: MUST distinguish event/exchange time, receive time, processing time, publication time, valid time and knowledge/recorded time.
- `DAT-005` `B0 / TEST`: MUST NOT compare timestamps with different precision, timezone or clock semantics without explicit conversion and validation.
- `DAT-006` `B0 / TEST`: MUST reject sequence gaps, regressions, duplicates, stale snapshots and discontinuities according to the feed contract. Do not sort away an ordering defect.
- `DAT-007` `H1 / REVIEW`: MUST distinguish absent, unknown, null, not applicable, zero, empty and false.
- `DAT-008` `B0 / TEST`: MUST validate ranges, scale, sign, precision, currency, lot, tick, multiplier, date, calendar and market-specific semantics at ingress.
- `DAT-009` `H1 / REVIEW`: MUST retain provenance, source/version, retrieval time, licensing and transformation lineage for reproducible data.
- `DAT-010` `B0 / REVIEW`: MUST NOT rewrite raw captured source bytes. Corrections are new immutable records or versioned transformations.
- `DAT-011` `H1 / REVIEW`: MUST NOT rely on physical row order, natural database order, map iteration or thread completion order.
- `DAT-012` `H1 / REVIEW`: MUST NOT use locale-sensitive number/date parsing for machine contracts.
- `DAT-013` `H1 / TEST`: MUST define empty, late, duplicate, malformed and unsupported-version behaviour.
- `DAT-014` `B0 / REVIEW`: MUST NOT substitute today's instrument, calendar, corporate-action or contract state into historical queries without point-in-time justification.
- `DAT-015` `H1 / REVIEW`: MUST record whether a timestamp is exchange-sourced, locally clocked, estimated or reconstructed.
- `DAT-016` `B0 / TEST`: MUST NOT conflate a sequence number with a timestamp or use timestamp sorting as a substitute for exchange sequencing.

## 5. Architecture, boundaries and authoritative state

### 5.1 Components and dependencies

- `ARC-001` `B0 / REVIEW`: MUST NOT let a component mutate another component's authoritative state via shared DB handles, shared mutable objects, filesystem side channels or undocumented callbacks.
- `ARC-002` `B0 / REVIEW`: MUST NOT bypass declared contracts by importing internal modules or reaching through an adapter.
- `ARC-003` `H1 / REVIEW`: MUST NOT create a service mesh or distributed deployment for an in-process system without measured need and an operations plan.
- `ARC-004` `H1 / REVIEW`: MUST NOT duplicate a domain rule in adapters, clients, scripts and services. Put it at its authority boundary and test mappings.
- `ARC-005` `H1 / REVIEW`: MUST NOT expose storage implementation details as the domain API unless storage is the contract.
- `ARC-006` `H1 / REVIEW`: MUST NOT create generic repositories, caches, event buses or state stores without concrete implementations/use cases and semantics.
- `ARC-007` `H1 / REVIEW`: MUST NOT use one type for distinct states when fields match.
- `ARC-008` `H1 / REVIEW`: MUST separate transport, decode, normalization, validation, state transition and persistence.
- `ARC-009` `B0 / REVIEW`: MUST keep trading-firm OMS/risk/accounting authority separate from venue-operator private order, matching, participant and surveillance state.
- `ARC-010` `B0 / REVIEW`: MUST keep matching authority, clearing authority, settlement state, accounting state and broker order state distinct.
- `ARC-011` `B0 / TEST`: Architecture edges MUST be classified as authority, command, read-only data, event notification or integration. An unclassified edge touching authority boundaries fails the gate.
- `ARC-012` `H1 / TEST`: Every component graph node MUST resolve to exactly one component ID or an explicitly declared structural node.
- `ARC-013` `H1 / REVIEW`: MUST NOT create modules, crates, services, databases or deployment units to decorate an architecture diagram.
- `ARC-014` `H1 / REVIEW`: MUST NOT use `common`, `misc`, `manager`, `utils`, `helpers` or `shared` as an unbounded dumping ground.
- `ARC-015` `H1 / REVIEW`: MUST NOT let test helpers become production dependencies.
- `ARC-016` `B0 / REVIEW`: MUST NOT activate a capability by registry entry alone. Activation requires code, entitlement, verification and owner approval.
- `ARC-017` `H1 / REVIEW`: MUST NOT create multiple implementations of the same adapter contract without a conformance suite proving equivalence or declared divergence.
- `ARC-018` `H1 / REVIEW`: MUST NOT place venue-specific quirks in generic domain code if the venue adapter can map them at the boundary without losing semantics.

### 5.2 State, events and transactions

- `STA-001` `B0 / REVIEW`: Identify one authoritative writer/owner for each mutable state.
- `STA-002` `B0 / TEST`: MUST NOT assume network/process delivery is exactly once. Design for duplicates, gaps, uncertain outcomes and reconciliation.
- `STA-003` `B0 / TEST`: State transitions MUST be explicit; impossible transitions are rejected.
- `STA-004` `B0 / REVIEW`: MUST NOT update immutable financial, market or audit history in place. Use a new event, revision, correction or compensation.
- `STA-005` `H1 / REVIEW`: Events MUST carry stable identity, source identity, sequence/version and causality where replay/audit depends on them.
- `STA-006` `B0 / TEST`: MUST NOT perform cross-resource dual writes and report atomic success without an outbox, idempotency, compensation or reconciliation protocol.
- `STA-007` `H1 / TEST`: MUST inject crashes before, during and after publication.
- `STA-008` `H1 / REVIEW`: MUST specify state rebuild, recovery, reconciliation and verification after restart.
- `STA-009` `B0 / TEST`: MUST NOT let stale cache state override durable authority.
- `STA-010` `H1 / REVIEW`: Queues MUST have capacity, backpressure, overflow, replay, drop/coalesce and shutdown semantics.
- `STA-011` `H1 / TEST`: Concurrent revision/ID allocation MUST be serialized and conflict-checked.
- `STA-012` `H1 / REVIEW`: MUST NOT collapse event time and processing time, or valid time and knowledge time.
- `STA-013` `B0 / REVIEW`: MUST NOT use a mutable in-memory cache as the only record of an accepted order, fill, risk decision or accounting event.
- `STA-014` `H1 / TEST`: MUST test idempotency across retry, process restart, duplicate delivery and lost response.

## 6. HFT and low-latency engineering standard

This section sets stricter requirements than ordinary application engineering. Low latency is not a substitute for correctness. A trading path that is fast but cannot prove sequencing, risk, recovery or order state is defective.

### 6.1 Establish the latency budget before optimising

Define a named end-to-end path, for example:

`NIC receive -> kernel/driver -> packet decode -> feed sequence validation -> book update -> strategy input -> decision -> risk check -> OMS -> order encode -> NIC transmit -> venue acknowledgement -> execution report -> portfolio update`.

For every boundary, define:

- timestamp source and semantic meaning;
- clock domain and synchronization method;
- unit and precision;
- queueing and batching behaviour;
- worst-case work and maximum queue depth;
- allocations, copies, syscalls and locks;
- CPU/thread/NUMA ownership;
- loss/reorder/reconnect behaviour;
- metric and trace points;
- failure and overload policy.

Anti-patterns:

- `HFT-001` `B0 / REVIEW`: MUST NOT call an operation "low latency" without identifying the measured path and clock boundaries.
- `HFT-002` `H1 / EVIDENCE`: MUST NOT optimise a microbenchmark while the end-to-end critical path is unknown.
- `HFT-003` `H1 / EVIDENCE`: MUST NOT use mean latency alone. Report percentiles and tail maxima, sample count, workload, warm/cold state and measurement method.
- `HFT-004` `H1 / EVIDENCE`: MUST NOT compare numbers from different hardware, compiler versions, flags, kernel versions, clock settings or data distributions as if directly comparable.
- `HFT-005` `H1 / REVIEW`: MUST NOT subtract timestamps from unrelated clock domains without a measured synchronization/calibration error bound.
- `HFT-006` `H1 / REVIEW`: MUST NOT treat application timestamps as wire timestamps or NIC hardware timestamps.
- `HFT-007` `H1 / REVIEW`: MUST NOT use a single benchmark run to set a performance gate. Establish distributions across repeated runs and noise conditions.
- `HFT-008` `H1 / EVIDENCE`: MUST disclose coordinated omission, sampling bias, queueing, warm-up and percentile algorithm in latency tests.

### 6.2 Clocks and timestamp discipline

- `CLK-001` `B0 / REVIEW`: MUST distinguish wall-clock time (UTC/epoch) from monotonic elapsed time.
- `CLK-002` `B0 / TEST`: MUST NOT use wall-clock time for timeout/deadline measurement when a monotonic clock is available.
- `CLK-003` `B0 / REVIEW`: MUST NOT assume NTP, PTP, hardware NIC timestamping, TSC or application clocks have equivalent precision or semantics.
- `CLK-004` `H1 / REVIEW`: MUST define timestamp precision at every interface. Truncating nanoseconds to microseconds without an explicit contract is forbidden.
- `CLK-005` `H1 / REVIEW`: MUST NOT assume timestamps are globally monotonic across hosts, CPU cores or clock adjustments.
- `CLK-006` `H1 / TEST`: MUST test leap seconds/smear policy, DST/timezone boundaries, epoch conversion, overflow, negative/pre-epoch values and precision truncation where relevant.
- `CLK-007` `H1 / EVIDENCE`: PTP offset, clock drift, holdover and loss-of-sync MUST be monitored where time precision is used for market ordering or latency claims.
- `CLK-008` `H1 / REVIEW`: MUST NOT read CPU cycle counters directly as nanoseconds without calibrating invariant TSC behaviour, frequency, cross-core synchronisation, migration and platform support.
- `CLK-009` `H1 / REVIEW`: MUST NOT silently fall back from exchange/hardware timestamp to user-space receive time; label and expose fallback quality.

### 6.3 CPU, cache, memory and NUMA

- `HW-001` `H1 / EVIDENCE`: MUST profile before introducing CPU pinning, NUMA binding, huge pages, custom allocators, cache-line padding, busy-spin loops or kernel bypass.
- `HW-002` `H1 / REVIEW`: MUST NOT use `volatile` for inter-thread synchronization.
- `HW-003` `B0 / REVIEW`: MUST NOT invent memory-ordering rules or rely on x86 ordering when the code is expected to run on ARM or another architecture.
- `HW-004` `B0 / TEST`: Atomics MUST use documented acquire/release/relaxed/SeqCst semantics and tests/model checking where the invariant warrants it.
- `HW-005` `H1 / REVIEW`: MUST NOT label an algorithm lock-free, wait-free or obstruction-free without a proof of progress properties and memory reclamation design.
- `HW-006` `H1 / REVIEW`: MUST NOT introduce hazard pointers, epochs, RCU or custom reclamation until ownership and reclamation safety are formally understood and tested.
- `HW-007` `H1 / EVIDENCE`: MUST investigate false sharing only after profiling identifies cache-line contention; pad based on target cache-line/hardware assumptions, not folklore.
- `HW-008` `H1 / REVIEW`: MUST NOT assume a 64-byte cache line on every future target without declaring the hardware support envelope.
- `HW-009` `H1 / REVIEW`: MUST NOT assume NUMA locality or CPU affinity on a laptop, cloud VM or arbitrary runner. Discover and verify runtime topology.
- `HW-010` `H1 / REVIEW`: MUST NOT allocate/free, touch page-fault-prone memory or perform unbounded memory growth in a latency-critical path without measured acceptance.
- `HW-011` `H1 / EVIDENCE`: Huge pages, locked memory, prefaulting and core isolation require deployment-level tests and privilege/operational documentation.
- `HW-012` `H1 / REVIEW`: MUST NOT assume data structures are cache-friendly because they are contiguous; measure access pattern, stride, working set, prefetch and mutation locality.
- `HW-013` `H1 / REVIEW`: MUST NOT use shared mutable cache-line state across independent workers where ownership partitioning is possible.
- `HW-014` `H1 / REVIEW`: MUST NOT introduce a memory pool that can exhaust, fragment or retain memory without explicit size, alignment, lifetime and recovery semantics.

### 6.4 Threads, queues and event loops

- `CON-001` `B0 / REVIEW`: Every mutable hot-path state MUST have an explicit owner: thread, actor, shard or lock/transaction boundary.
- `CON-002` `H1 / REVIEW`: Prefer single-writer ownership and message passing over multi-writer mutation when it simplifies correctness and meets throughput.
- `CON-003` `B0 / REVIEW`: MUST NOT access the same mutable data concurrently without a valid synchronization/ownership protocol.
- `CON-004` `H1 / REVIEW`: MUST NOT use an unbounded queue in a persistent process.
- `CON-005` `B0 / TEST`: Every bounded queue MUST define full behaviour: block, reject, shed, overwrite, aggregate or disconnect. The policy must be domain-specific.
- `CON-006` `B0 / REVIEW`: MUST NOT drop order, fill, risk, accounting or control events to protect latency. If they cannot be processed, fail closed and reconcile.
- `CON-007` `H1 / REVIEW`: Market-data events MAY be coalesced/dropped only under a declared data-quality policy that preserves sequence-gap detection and forces resynchronization when required.
- `CON-008` `H1 / REVIEW`: MUST NOT use hot spinning as the default wait strategy. Spin/yield/block strategies require workload and CPU-budget evidence.
- `CON-009` `H1 / REVIEW`: MUST NOT assume SPSC queue semantics for MPSC/MPMC use. Producer/consumer cardinality is part of the contract.
- `CON-010` `B0 / TEST`: MUST test queue wraparound, full/empty transition, producer crash, consumer crash, sequence rollover, cancellation and shutdown.
- `CON-011` `H1 / REVIEW`: MUST NOT detach threads/tasks without lifecycle ownership, cancellation, join/error observation and shutdown behaviour.
- `CON-012` `H1 / REVIEW`: MUST NOT hold global locks across network I/O, database calls, logging, callbacks or await points.
- `CON-013` `H1 / REVIEW`: MUST NOT spawn one thread/task per event, symbol or message without explicit resource and backpressure bounds.
- `CON-014` `H1 / EVIDENCE`: MUST measure queue wait separately from service time and end-to-end latency.
- `CON-015` `H1 / REVIEW`: MUST NOT add a multi-stage queue pipeline when a single owner/event loop is clearer and fast enough.
- `CON-016` `H1 / REVIEW`: MUST NOT use a custom lock-free ring buffer because it appears in HFT presentations; prove the chosen design against simpler alternatives.

### 6.5 Network, NIC and kernel path

- `NET-001` `B0 / TEST`: MUST validate frame length, header, version, endian, alignment, sequence, checksum/integrity and variable-length fields before decoding.
- `NET-002` `B0 / REVIEW`: MUST NOT reinterpret packet bytes as native structs unless layout, alignment, padding, endian and bounds are contractually identical and validated.
- `NET-003` `H1 / REVIEW`: MUST separate socket lifecycle, transport framing, protocol decoding, message validation and domain event creation.
- `NET-004` `H1 / REVIEW`: MUST define TCP reconnect, partial read/write, timeout, keepalive, session reset, retransmission and duplicate handling.
- `NET-005` `H1 / REVIEW`: MUST define UDP multicast loss, gap detection, retransmission, snapshot recovery, channel reset, packet reordering and duplicate semantics where applicable.
- `NET-006` `H1 / REVIEW`: MUST NOT equate a successful send syscall with venue receipt, acceptance or execution.
- `NET-007` `H1 / REVIEW`: MUST NOT introduce DPDK, kernel-bypass, AF_XDP, RDMA, Solarflare/OpenOnload or vendor NIC SDKs before a measured bottleneck and deployment target exist.
- `NET-008` `H1 / EVIDENCE`: DPDK/PMD adoption MUST measure per-core throughput, packet loss, burst sizing, queue locality, NUMA placement, interrupt/poll mode, CPU consumption and backpressure.
- `NET-009` `H1 / REVIEW`: MUST NOT share a NIC RX/TX queue across cores without verifying driver capabilities, locking and queue ownership requirements.
- `NET-010` `H1 / REVIEW`: MUST NOT store packet capture and decoded events as if they were the same representation. Preserve raw capture, parser version and normalised record lineage.
- `NET-011` `H1 / REVIEW`: MUST NOT silently treat socket receive order as exchange event order.
- `NET-012` `H1 / TEST`: MUST test partial frames, coalesced frames, malformed lengths, disconnect at every boundary, reconnect and replay.

### 6.6 Memory allocation, syscalls and hot-path discipline

- `HOT-001` `H1 / EVIDENCE`: Hot-path status MUST be based on profiles and latency/throughput budget, not file location or developer intuition.
- `HOT-002` `H1 / REVIEW`: MUST avoid unbounded allocation, string formatting, JSON parsing, dynamic dispatch, syscalls, disk I/O and blocking log writes in a measured hot path unless the design explicitly requires them.
- `HOT-003` `H1 / REVIEW`: MUST NOT claim zero-copy if any boundary copies, serializes, deserializes or reallocates the data.
- `HOT-004` `H1 / REVIEW`: MUST NOT batch blindly. Batch size trades throughput, latency, fairness and memory; declare the policy.
- `HOT-005` `H1 / REVIEW`: MUST NOT use a global allocator override or custom allocator without full lifetime, alignment, thread-safety, OOM and measurement analysis.
- `HOT-006` `H1 / TEST`: Allocation-failure and bounded-memory paths must be tested where allocation is critical.
- `HOT-007` `H1 / REVIEW`: MUST NOT use synchronous logging or formatted error construction in a high-frequency path without a non-blocking/bounded strategy and a loss policy.
- `HOT-008` `H1 / REVIEW`: MUST NOT assume compiler optimisations are stable across versions or architectures; inspect generated assembly only for a measured question and keep semantic tests.
- `HOT-009` `H1 / EVIDENCE`: Profile CPU, cache misses, branch misses, context switches, page faults, syscalls and off-CPU wait before low-level changes.

### 6.7 HFT performance acceptance

Any claimed low-latency optimisation must provide:

- A baseline commit and a changed commit.
- Identical input data or an exact reproducible generator/seed.
- Hardware/VM model, CPU topology, kernel, NIC/driver, power and frequency policy.
- Compiler, linker, flags, LTO/PGO settings, allocator and dependency versions.
- Warm-up, repetitions, sample count, p50/p90/p99/p99.9/max where useful, variance and confidence/uncertainty.
- Queue wait, service time and end-to-end latency when relevant.
- Correctness, loss, reorder, tail-stall and failure tests under the same workload.
- A decision explaining the extra complexity and regression budget.

`HFT-009` `B0 / EVIDENCE`: A speedup claim is rejected if correctness, completeness, data quality or event loss has regressed.  
`HFT-010` `H1 / EVIDENCE`: MUST benchmark under overload and recovery, not steady-state low load
  alone.
`HFT-011` `H1 / REVIEW`: MUST NOT use average throughput to hide queue growth or p99 latency collapse.  
`HFT-012` `H1 / REVIEW`: MUST NOT call a system HFT because it uses Rust/C++, q, atomics, DPDK, a ring buffer or microsecond timers.

## 7. Market data, feed handlers and book reconstruction

- `MD-001` `B0 / REVIEW`: MUST define feed scope, product, venue, channel, sequence domain, symbol mapping, timestamp fields, snapshot semantics and entitlement before implementing a handler.
- `MD-002` `B0 / TEST`: MUST distinguish full snapshot from incremental update and define which sequence or epoch anchors the book.
- `MD-003` `B0 / TEST`: MUST reject or quarantine an incremental update that cannot be applied because of a sequence gap or unknown state.
- `MD-004` `B0 / TEST`: MUST NOT apply deltas across a session reset, channel reset, symbol remap or book epoch boundary without a protocol-defined recovery.
- `MD-005` `B0 / TEST`: MUST detect duplicates, out-of-order messages, stale updates, crossed/locked states and invalid quantities with venue-specific policy.
- `MD-006` `H1 / REVIEW`: MUST preserve raw payloads or a lossless canonical capture sufficient to reproduce parser decisions, subject to data licence/retention rights.
- `MD-007` `H1 / REVIEW`: MUST NOT normalise away venue-specific order types, flags, participant IDs, conditions or sequencing semantics needed by downstream logic.
- `MD-008` `H1 / REVIEW`: MUST distinguish last trade, quote, indicative price, auction uncross, settlement price and theoretical value.
- `MD-009` `H1 / TEST`: MUST validate price/quantity scale and null sentinel semantics from each feed specification.
- `MD-010` `H1 / REVIEW`: MUST NOT use local receive time as a substitute for source time without labelling it.
- `MD-011` `H1 / REVIEW`: MUST NOT assume every venue emits a complete book or that every book update is executable. A top-of-book feed does not establish depth, queue position or executable liquidity.
- `MD-012` `H1 / REVIEW`: MUST expose feed health: last event, age, sequence gap, reconnect count, snapshot age, decode failures, recovery state and entitlement status.
- `MD-013` `H1 / TEST`: MUST test recovery after a gap at the beginning, middle and end of a batch; test packet loss during snapshot transition.
- `MD-014` `H1 / REVIEW`: MUST NOT share a single sequence counter across independent channels unless the protocol defines a shared sequence domain.
- `MD-015` `H1 / REVIEW`: MUST NOT sort updates by timestamp to hide exchange sequence defects.
- `MD-016` `H1 / EVIDENCE`: Replay fidelity MUST be labelled by data level: bars, trades, top of book, L2 depth, L3/order events or full packet capture. Do not infer a richer level.

## 8. Trading, order lifecycle, risk, accounting and venue semantics

### 8.1 Instrument and contract modelling

- `FIN-001` `B0 / REVIEW`: MUST distinguish venue, market, segment, product, instrument, listing, contract, account, broker identifier and display symbol; MUST separate instrument identity from each venue/broker mapping.
- `FIN-002` `B0 / TEST`: MUST NOT flatten distinct asset classes into a generic instrument record that omits their financial semantics.
- `FIN-003` `B0 / REVIEW`: Derivatives must model underlying, expiry, strike, option type, multiplier, exercise, assignment, settlement and contract rules as applicable.
- `FIN-004` `B0 / REVIEW`: Bonds/fixed income must model face value, coupon, day-count, maturity, accrued interest, yield convention, settlement and calendar as applicable.
- `FIN-005` `B0 / REVIEW`: FX must distinguish currency pair orientation, base/quote, pip/precision, spot/forward date, fixing, NDF and settlement terms.
- `FIN-006` `B0 / REVIEW`: Commodities must model grade, delivery point, unit, contract month, lot, expiry, physical/financial settlement and quality/location basis.
- `FIN-007` `B0 / REVIEW`: Digital assets must model chain/network, token contract, wallet/custody, nonce, finality, reorg, funding and fee asset where relevant.
- `FIN-008` `B0 / REVIEW`: Prediction/event contracts must model resolution source, resolution time, dispute, void, cancellation, payout and event rules.
- `FIN-009` `B0 / REVIEW`: MUST NOT force all venues into a central limit order book abstraction. CLOB, auction, RFQ/RFS, dealer/principal, OTC, dark pool/crossing, internalization and AMM/DEX differ; pool reserves, indicative quotes and RFQ responses are not equivalent to executable CLOB depth.
- `FIN-010` `H1 / TEST`: MUST model market calendars, phases, special sessions, cutoffs, expiry, settlement and holidays from explicit venue data.
- `FIN-011` `B0 / REVIEW`: MUST NOT silently roll futures, synthesize option chains, apply corporate actions or backfill contract terms.
- `FIN-012` `B0 / REVIEW`: MUST NOT equate feed access with data licence, account entitlement or trade permission.
- `FIN-013` `H1 / REVIEW`: OTC/dealer workflows MUST preserve quote validity, counterparty, negotiation, confirmation, amendment, settlement and bilateral status; do not retrofit them into a simple exchange-order model.

### 8.2 Research and backtesting

- `BT-001` `B0 / TEST`: MUST prevent look-ahead, survivorship, selection, corporate-action, constituent, symbol mapping and publication-time leakage.
- `BT-002` `B0 / TEST`: MUST use point-in-time instrument identity and effective-dated contract/calendar/fee/borrow/margin data.
- `BT-003` `B0 / TEST`: MUST NOT random-split temporal data where chronology or overlap matters. Use temporal validation, purge and embargo where necessary.
- `BT-004` `B0 / REVIEW`: MUST NOT equate daily bars, OHLC, trade prints, quotes, L2 or L3 data. Fill and queue models must match the available evidence.
- `BT-005` `B0 / REVIEW`: MUST NOT fill orders at a price that was unavailable after decision, latency, queue priority, spread, market impact or venue constraints.
- `BT-006` `B0 / REVIEW`: MUST model order types, partial fills, lot sizes, ticks, price bands, sessions, auctions, halts and rejection rules that apply to the target market.
- `BT-007` `B0 / REVIEW`: MUST include all applicable fees, taxes, duties, borrow, funding, spread, slippage, impact, latency and capacity assumptions.
- `BT-008` `H1 / EVIDENCE`: Every experiment MUST retain dataset hashes, point-in-time policy, code SHA, config, seeds, dependency versions and full trial count.
- `BT-009` `H1 / REVIEW`: MUST NOT present a single Sharpe, CAGR, win rate or profit figure as proof of robust alpha.
- `BT-010` `H1 / REVIEW`: Evaluate multiple testing, selection bias, probability of backtest overfitting, deflated Sharpe, regime stability, sensitivity, turnover and capacity where appropriate.
- `BT-011` `B0 / REVIEW`: MUST NOT tune repeatedly on the holdout and still call it out-of-sample.
- `BT-012` `H1 / REVIEW`: MUST separate candidate generation, screening, validation, final holdout and paper/live confirmation.
- `BT-013` `H1 / REVIEW`: AI-generated strategies MUST record rejected trials and search process. Retain the experiment ledger; do not only publish winners.
- `BT-014` `B0 / REVIEW`: MUST NOT infer persistent live alpha from synthetic fills, a single market regime or one favourable interval.
- `BT-015` `H1 / EVIDENCE`: Strategy results MUST be reproducible from immutable data, versioned transformations, model code and execution assumptions.

### 8.3 OMS, EMS, risk and accounting

- `ORD-001` `B0 / REVIEW`: Strategy, model, AI agent, adapter and execution algorithm MUST NOT bypass independent risk authority.
- `ORD-002` `B0 / TEST`: Model new, pending, acknowledged, rejected, partially filled, filled, cancel pending, canceled, replace pending, replaced, expired, busted and corrected states distinctly where supported.
- `ORD-003` `B0 / TEST`: MUST NOT equate HTTP success, socket write, broker request acceptance or venue acknowledgement with execution.
- `ORD-004` `B0 / TEST`: Unknown acknowledgement MUST be reconciled before resubmitting, cancelling or replacing.
- `ORD-005` `B0 / REVIEW`: MUST NOT mutate a fill/trade to correct it. Use cancel/correct/bust or compensating entries, preserving the original.
- `ORD-006` `B0 / REVIEW`: MUST NOT derive accounting truth from mutable order state. Use immutable trade, fee, cash, corporate action, settlement and reconciliation events.
- `ORD-007` `B0 / TEST`: Risk must fail closed on missing, stale, inconsistent or unknown limits, prices, positions, buying power, margin and account state.
- `ORD-008` `B0 / REVIEW`: A `risk_ok` boolean without timestamp, input version, policy version, scope and reason is forbidden.
- `ORD-009` `B0 / REVIEW`: Risk overrides require human authority, explicit scope, reason, expiry, independent approval and audit. AI cannot approve its own override.
- `ORD-010` `B0 / TEST`: Test duplicate/out-of-order execution reports, cancel/replace races, partial fills, disconnect, restart and reconciliation.
- `ORD-011` `H1 / REVIEW`: Define stable client order ID, venue order ID, execution ID, idempotency key, parent/child and replace chain semantics.
- `ORD-012` `B0 / REVIEW`: Broker-neutral API MUST NOT erase venue-specific order semantics, margin, limits, rate limits, restrictions or settlement.
- `ORD-013` `B0 / REVIEW`: Adapters translate transport/protocol; they do not become alternate risk, order or accounting authorities.
- `ORD-014` `B0 / REVIEW`: Venue-operator state MUST NOT mutate trading-firm OMS, risk or accounting authority.
- `ORD-015` `B0 / REVIEW`: Venue integration does not imply clearing or settlement authority/finality.
- `ORD-016` `B0 / TEST`: Reconcile positions, cash, fees, collateral and obligations to independent statements/sources; breaks remain visible until resolved.
- `ORD-017` `B0 / REVIEW`: Live submission MUST remain disabled until explicit risk, broker entitlement, compliance, operations and human approval gates pass.
- `ORD-018` `H1 / REVIEW`: Rate limiting MUST be scoped to broker, account, venue, endpoint and rolling-window contract. Do not use one global guessed limit.

### 8.4 Quantitative model and ML governance

- `ML-001` `H1 / REVIEW`: Every production model/strategy MUST have a named owner, intended use, supported instruments/markets, training-data scope, approved operating envelope, known limitations and retirement/rollback owner.
- `ML-002` `B0 / TEST`: MUST NOT train, select features, tune thresholds, calibrate or impute using information unavailable at the decision time. Preserve event time and actual publication/availability time for every feature and label.
- `ML-003` `H1 / EVIDENCE`: A model release MUST bind the immutable model artifact hash to source revision, training-data and feature hashes, transformation/schema versions, parameter set, seed, environment lock, evaluation report and approval record.
- `ML-004` `B0 / REVIEW`: MUST NOT promote a research model directly into a live decision path. Require independent validation against the approved model-risk criteria, signed/versioned artifact, compatibility checks, shadow or controlled rollout where appropriate, and an independent promotion approval.
- `ML-005` `H1 / TEST`: Production monitoring MUST cover input freshness/missingness, schema drift, feature distribution, prediction distribution, calibration/quality where labels become available, latency, resource saturation and risk/exposure outcomes. Thresholds and owners must be explicit.
- `ML-006` `B0 / TEST`: MUST fail closed or enter a documented safe mode when required model inputs are stale, missing, out of schema, outside the validated domain or inconsistent with the model's declared operating envelope. Do not silently impute a safety-critical feature.
- `ML-007` `H1 / REVIEW`: MUST NOT treat a backtest, offline metric, paper result or a single champion/challenger interval as proof of production suitability. Preserve rejected trials, selection decisions, drift/market-regime assumptions and live/paper separation.
- `ML-008` `H1 / TEST`: Revalidate models after a material change to training data, feature definitions, label construction, execution assumptions, runtime, dependencies, market structure or instrument universe. Define rollback and disable criteria before deployment.

## 9. Rust rules

Pinned Rust toolchain is authoritative. Do not upgrade compiler or edition as a side effect of unrelated work.

- `RS-001` `B0 / AUTO`: No `unwrap`, `expect`, `panic!`, `unreachable!` or unchecked indexing for untrusted input, ordinary operational failures or externally mutable state in production paths. A panic for a violated internal invariant may be acceptable only when the process-level failure policy is explicit, the invariant is not user/data-controlled, and the impact is reviewed. Test-only use follows local lint policy.
- `RS-002` `B0 / REVIEW`: MUST NOT use `unwrap_or_default`, `unwrap_or(0)`, `unwrap_or(false)` or fallback values to hide parse, missing-state or validation errors.
- `RS-003` `H1 / REVIEW`: MUST NOT blanket-allow Clippy lints or compiler warnings at crate/workspace scope to make a gate green.
- `RS-004` `B0 / REVIEW`: Every unsafe block needs a `SAFETY` argument: invariants, preconditions, aliasing, validity, lifetime and why the operation is sound.
- `RS-005` `B0 / TEST`: Unsafe code must have boundary, lifetime, aliasing, concurrency and target tests. Prefer safe code where adequate.
- `RS-006` `B0 / REVIEW`: Use checked conversions and arithmetic for money, quantities, IDs, timestamps, lengths and counters.
- `RS-007` `H1 / REVIEW`: MUST NOT clone large buffers, allocate, format, parse dynamic data or block in a measured hot path without evidence.
- `RS-008` `H1 / REVIEW`: MUST NOT block an async runtime worker with synchronous file/database/network I/O or unbounded computation.
- `RS-009` `H1 / REVIEW`: Async tasks need cancellation, join/error observation, backpressure and shutdown semantics. A dropped `JoinHandle` is not equivalent to aborting the task; define who owns and joins long-lived tasks.
- `RS-010` `H1 / REVIEW`: MUST NOT hold a synchronous mutex guard across `.await`.
- `RS-011` `B0 / REVIEW`: Evaluate each `Send`, `Sync`, `Copy`, `Clone`, `Default` and `Deserialize` derivation as a change to the invariant/capability surface.
- `RS-012` `H1 / REVIEW`: Public types SHOULD encode invariants with private fields, validated constructors, enums and newtypes.
- `RS-013` `H1 / REVIEW`: MUST NOT expose `serde_json::Value`, generic maps or strings where a stable typed domain API is required.
- `RS-014` `B0 / REVIEW`: Persistent/untrusted data requires explicit versioned decoding, strict field policy and semantic validation before exposure.
- `RS-015` `H1 / REVIEW`: `allow`/`expect` attributes require a local reason, scope and expiry/task where material.
- `RS-016` `H1 / REVIEW`: MUST NOT use `static mut` or global mutable singletons for domain state.
- `RS-017` `H1 / REVIEW`: Preserve typed errors where callers branch on them. Do not reduce stable library APIs to formatted strings.
- `RS-018` `H1 / REVIEW`: Do not expose `anyhow::Error` as a public stable library contract when callers need machine-readable variants.
- `RS-019` `B0 / REVIEW`: Do not rely on destructors alone for durability. Explicitly flush, sync, commit and verify.
- `RS-020` `H1 / REVIEW`: No detached task/thread without lifecycle owner and shutdown policy.
- `RS-021` `H1 / REVIEW`: New crates require task/component registration, API intent, owner, tests and dependency-boundary justification.
- `RS-022` `H1 / REVIEW`: Feature flags must not create untested combinations or silently alter financial semantics.
- `RS-023` `H1 / TEST`: Test panic/error safety around transient invariant violations and cancellation.
- `RS-024` `H1 / REVIEW`: FFI boundaries must translate panic/error explicitly; no unwind across incompatible ABI.
- `RS-025` `H1 / REVIEW`: Do not use `catch_unwind` as routine control flow or as a way to claim memory/state recovery after panic.
- `RS-026` `H1 / AUTO`: Run pinned formatting, Clippy with warnings denied, the relevant test/feature matrix, dependency and licence checks, and repository gate suite. Use the repository's configured dependency checker (for example `cargo deny check`); do not assume an unconfigured tool exists.
- `RS-027` `B0 / TEST`: Every async operation that may be cancelled MUST document whether dropping its future is cancellation-safe. Do not use `select!`, timeout or task abort around a multi-step operation unless partial progress, resource release and replay/reconciliation are correct. Prefer explicit operation state and a cancellation protocol for non-cancel-safe work.
- `RS-028` `H1 / TEST`: Use Miri for applicable unsafe-code tests and a concurrency model checker such as Loom for small synchronization protocols when supported. These tools explore bounded models; passing them does not prove the whole program correct.

## 10. C and C++ rules

- `CPP-001` `B0 / REVIEW`: Every owning pointer needs explicit ownership and deallocation. Prefer RAII and standard ownership types.
- `CPP-002` `B0 / REVIEW`: No UB: out-of-bounds access, use-after-free, double-free, uninitialized read, invalid pointer arithmetic, data race or lifetime violation.
- `CPP-003` `B0 / REVIEW`: Check arithmetic overflow and narrowing conversions at protocol and financial boundaries.
- `CPP-004` `B0 / REVIEW`: `volatile` is not thread synchronization.
- `CPP-005` `B0 / REVIEW`: Shared mutable state needs explicit synchronization and ownership.
- `CPP-006` `H1 / REVIEW`: Prefer typed functions/constants to public macros; macros must not surprise control flow or evaluate arguments multiple times.
- `CPP-007` `B0 / REVIEW`: No exception crossing Rust/C ABI boundaries. Translate exceptions into explicit result/error contracts.
- `CPP-008` `B0 / TEST`: Never cast raw network/file bytes to native structs without validating endian, alignment, padding, bounds and version.
- `CPP-009` `H1 / REVIEW`: Avoid static initialisation order dependency and side-effecting global constructors.
- `CPP-010` `H1 / REVIEW`: Headers must be self-contained, guarded and minimal.
- `CPP-011` `H1 / REVIEW`: Avoid non-trivial implementation in commonly included headers unless templates/inline contract requires it.
- `CPP-012` `H1 / REVIEW`: Avoid advanced templates/concepts/SFINAE when direct overloads or explicit types are clearer.
- `CPP-013` `H1 / EVIDENCE`: Custom allocator, SIMD, lock-free, cache padding, branchless algorithm or prefetch requires benchmark and correctness evidence.
- `CPP-014` `B0 / REVIEW`: Define ownership/lifetime at every FFI boundary. No borrowed pointer may outlive owner.
- `CPP-015` `H1 / REVIEW`: Do not leak compiler flags, packing rules, STL types or allocator details through stable ABI.
- `CPP-016` `H1 / TEST`: Sanitizer and static-analysis findings cannot be suppressed without explicit local justification and independent review.
- `CPP-017` `H1 / AUTO`: Run pinned formatting, build, CTest, conformance and sanitizer gates on supported platforms.
- `CPP-018` `H1 / EVIDENCE`: Compare cache/container/allocation strategies on representative inputs before changing them.
- `CPP-019` `H1 / REVIEW`: Packed structs are not a wire format. Use explicit encoders/decoders.
- `CPP-020` `B0 / TEST`: Numeric kernels must match Rust domain semantics for rounding, overflow, scaling and edge cases.
- `CPP-021` `H1 / REVIEW`: Use standard library facilities before introducing local replacements; custom containers require evidence and maintenance ownership.
- `CPP-022` `H1 / REVIEW`: Do not depend on unspecified evaluation order, signed overflow or aliasing assumptions.
- `CPP-023` `H1 / REVIEW`: Make ownership visible at call sites; avoid APIs where a function silently retains caller pointers/references.
- `CPP-024` `H1 / AUTO`: Use target-scoped CMake include paths, definitions, compile options and link dependencies; avoid global mutable build settings.
- `CPP-025` `H1 / TEST`: Tests must compile public headers independently and test ABI boundary failure paths.

## 11. Python, scientific computing and ML rules

- `PY-001` `H1 / REVIEW`: Reusable APIs need type annotations where they define or clarify contracts.
- `PY-002` `B0 / REVIEW`: Do not use `assert` for runtime validation of input, policy, account, market or configuration state.
- `PY-003` `H1 / REVIEW`: Broad exception catches must translate/re-raise with a deliberate failure policy; never swallow.
- `PY-004` `H1 / REVIEW`: Importing a module must not perform network requests, database writes, process mutation or expensive hidden work.
- `PY-005` `M2 / AUTO`: Avoid wildcard imports, mutable defaults, builtin shadowing and implicit namespace pollution.
- `PY-006` `B0 / REVIEW`: Do not use `pickle` for untrusted or cross-boundary artifacts; do not use unsafe YAML loaders.
- `PY-007` `B0 / REVIEW`: Avoid shell=True and interpolated shell strings where argument vectors are available.
- `PY-008` `B0 / REVIEW`: Do not use binary float for ledger/order values; use exact integer or decimal contracts.
- `PY-009` `H1 / REVIEW`: Do not mutate shared NumPy arrays or DataFrames across concurrent boundaries without explicit ownership/synchronization.
- `PY-010` `H1 / TEST`: Do not rely on implicit pandas/Polars row order, index alignment, dtype coercion, timezone-naive timestamps or automatic NA fill.
- `PY-011` `H1 / REVIEW`: Notebooks are exploration records, not the only location for reusable production logic.
- `PY-012` `H1 / TEST`: Inject clocks, RNGs, network clients, file roots and databases into deterministic tests.
- `PY-013` `H1 / REVIEW`: Do not share a global RNG across independent tests or experiments.
- `PY-014` `H1 / REVIEW`: Do not mutate `sys.path`, monkey-patch packages or change process-global state in imported libraries.
- `PY-015` `H1 / REVIEW`: Do not globally suppress warnings; they may identify numeric instability or deprecation.
- `PY-016` `H1 / AUTO`: Follow pinned Ruff configuration and project test commands.
- `PY-017` `H1 / EVIDENCE`: Research results require package versions, data hash, code revision, config, seed and reproducible command.
- `PY-018` `H1 / REVIEW`: Avoid row-wise `apply` or Python loops on large tabular workloads without measuring and considering vectorised/native kernels.
- `PY-019` `H1 / REVIEW`: Do not use multiprocessing/thread pools without explicit worker bounds, process lifecycle, cancellation and memory budget.
- `PY-020` `H1 / REVIEW`: Do not assume the GIL prevents races in C extensions/NumPy or remains present in future free-threaded Python. Free-threaded CPython exists from 3.13 as an optional build; built-in container locking is an implementation detail, not a substitute for application-level synchronization.
- `PY-021` `H1 / TEST`: Verify timezone, dtype, null, categorical and join semantics before/after dataframe transformations.
- `PY-022` `H1 / REVIEW`: Do not train/fit scalers, encoders, feature selection or imputation on data that includes validation/test/holdout periods.
- `PY-023` `B0 / REVIEW`: Do not use future labels or post-event data in features; retain publication/availability time, not only event date.
- `PY-024` `H1 / EVIDENCE`: ML experiments require environment lock, hardware, data lineage, training code, model artifact hash, hyperparameters and full trial record.
- `PY-025` `H1 / TEST`: If free-threaded Python is in the supported envelope, test that mode explicitly and audit every C extension, native dependency, shared iterator, global and mutable buffer for its documented thread-safety guarantees. Do not infer extension compatibility from a successful import.
- `PY-026` `H1 / TEST`: Async cancellation and timeout paths MUST preserve invariants and clean up resources. A timeout may cancel the awaited task; do not translate `TimeoutError` into proof that the remote operation did not complete. Reconcile uncertain side effects.

## 12. SQL, SQLite and PostgreSQL rules

### 12.1 Universal SQL rules

- `SQL-001` `B0 / REVIEW`: Use parameterised queries for values. Never concatenate untrusted input into SQL.
- `SQL-002` `H1 / REVIEW`: Avoid `SELECT *` in application contracts and persistent queries; list columns explicitly.
- `SQL-003` `B0 / REVIEW`: Do not store exact monetary values in binary float columns.
- `SQL-004` `B0 / REVIEW`: Enforce authoritative invariants in the database where possible: PK, FK, unique, check and immutability constraints.
- `SQL-005` `B0 / REVIEW`: Application-side validation alone does not protect a database from other processes/future code paths.
- `SQL-006` `B0 / REVIEW`: Multi-row state changes need explicit transaction boundaries and isolation semantics.
- `SQL-007` `B0 / TEST`: Do not mix external side effects inside a DB transaction unless dual-write recovery is defined.
- `SQL-008` `H1 / EVIDENCE`: Unbounded scans require representative query-plan and workload evidence.
- `SQL-009` `H1 / REVIEW`: Index measured predicates/orderings; inspect query plans and write amplification.
- `SQL-010` `B0 / REVIEW`: Schema changes require versioning, compatibility plan, backup, recovery and migration tests.
- `SQL-011` `B0 / REVIEW`: Do not rewrite append-only history in a migration.
- `SQL-012` `H1 / REVIEW`: Define `NULL`, zero, missing and not-applicable separately.
- `SQL-013` `H1 / REVIEW`: Define timestamp UTC/precision/timezone rules explicitly.
- `SQL-014` `H1 / REVIEW`: A transaction does not span multiple databases, filesystems, message brokers or APIs unless a protocol explicitly implements that boundary.
- `SQL-015` `B0 / TEST`: Test duplicate writes, locks, deadlocks/busy, cancel, crash, restart, disk-full and permission failures for persistence.
- `SQL-016` `B0 / TEST`: Verify connection-scoped settings on every connection.
- `SQL-017` `H1 / REVIEW`: Do not rely on row order without `ORDER BY`.
- `SQL-018` `H1 / REVIEW`: Do not hide schema mismatch with fallback casts/default columns.
- `SQL-019` `H1 / REVIEW`: Do not turn a database into a generic bus/cache without an accepted design.
- `SQL-020` `H1 / REVIEW`: Document isolation, locking, timeout, retry, checkpoint, vacuum/maintenance, retention and backup semantics.

### 12.2 SQLite-specific rules

- `SQLITE-001` `B0 / REVIEW`: SQLite WAL MUST NOT be used on filesystems where locking/shared-memory/sync semantics are not supported and tested.
- `SQLITE-002` `B0 / REVIEW`: Do not put authoritative local WAL stores in network shares, cloud-sync folders or replicated folders without a supported and tested design.
- `SQLITE-003` `B0 / REVIEW`: Never copy only the live main DB file while WAL may contain committed data. Use SQLite online backup or `VACUUM INTO` and verify.
- `SQLITE-004` `B0 / TEST`: Explicitly set and verify `journal_mode`, `synchronous`, `foreign_keys`, busy timeout and any required auto-checkpoint settings.
- `SQLITE-005` `B0 / REVIEW`: Do not assume WAL means multiple concurrent writers; write transactions are serialised.
- `SQLITE-006` `H1 / REVIEW`: Do not hold a write transaction while performing network/filesystem operations or long-running domain computation.
- `SQLITE-007` `B0 / TEST`: `SQLITE_BUSY`, `SQLITE_LOCKED`, `SQLITE_FULL`, `SQLITE_IOERR`, corruption and uncertain commit outcomes need different handling.
- `SQLITE-008` `B0 / REVIEW`: Do not replay a publication after an uncertain commit until its idempotency key and durable payload are verified.
- `SQLITE-009` `H1 / REVIEW`: Do not let a busy-timeout setting silently change between open and operation.
- `SQLITE-010` `H1 / TEST`: Test checkpoint, long readers, WAL growth, restart recovery, file permission and disk exhaustion.
- `SQLITE-011` `B0 / REVIEW`: Immutable tables require DB-level protection if other processes or SQL clients can write.
- `SQLITE-012` `H1 / REVIEW`: A backup is not proven until restore validates schema, rows, metadata, hashes, archive dependencies and application-level reads.
- `SQLITE-013` `B0 / TEST`: Restore must compare manifest metadata, actual database head and all version/hash/parent relationships before atomic promotion.
- `SQLITE-014` `H1 / REVIEW`: Atomic directory swap semantics and crash recovery must be documented per supported OS/filesystem. A rename sequence is not automatically crash-atomic across all failure points.
- `SQLITE-015` `H1 / REVIEW`: Do not delete old backup/retired restore copies automatically without a retention and recovery contract.
- `SQLITE-016` `H1 / TEST`: Test torn append-only manifest recovery without rewriting committed prefixes; recovery itself must be interruption-safe.

### 12.3 PostgreSQL-specific rules

- `PG-001` `B0 / REVIEW`: Do not assume `READ COMMITTED`, `REPEATABLE READ` and `SERIALIZABLE` have equivalent semantics. Select isolation based on invariants.
- `PG-002` `B0 / TEST`: Applications using serializable transactions must handle serialization failures by retrying the entire transaction with bounded retry and idempotency.
- `PG-003` `B0 / TEST`: Deadlocks, lock timeout, statement timeout, connection loss during commit and failover must have separate recovery paths.
- `PG-004` `H1 / REVIEW`: Do not use advisory locks as hidden business ownership; name lock keys, scope, timeout and unlock semantics.
- `PG-005` `H1 / REVIEW`: Do not run long transactions/idle-in-transaction sessions that retain old MVCC snapshots and impede vacuum.
- `PG-006` `H1 / EVIDENCE`: Use `EXPLAIN (ANALYZE, BUFFERS)` on representative data to validate plans; do not infer index use from schema appearance.
- `PG-007` `H1 / REVIEW`: Do not add indexes without considering write amplification, bloat, vacuum and lock effects.
- `PG-008` `H1 / REVIEW`: Do not use sequences as gapless financial numbering; sequence values can be consumed without transaction commit.
- `PG-009` `B0 / REVIEW`: Do not treat a replica as current authority without defining replication lag, consistency and failover promotion policy.
- `PG-010` `B0 / REVIEW`: Do not claim PITR until base backup, WAL archive, restore target and recovery timeline are tested.
- `PG-011` `H1 / REVIEW`: Do not use connection pooling without accounting for server-side prepared statements, transaction/session state and pooler mode.
- `PG-012` `H1 / TEST`: Test connection loss before, during and after commit; reconcile unknown commit outcomes by idempotency key or durable business identifier.
- `PG-013` `H1 / REVIEW`: Schema migrations need lock duration, expand/contract compatibility, backfill strategy, rollback/forward-repair and concurrent application-version plan.
- `PG-014` `H1 / REVIEW`: Do not assume `CREATE INDEX`, `ALTER TABLE`, vacuum or reindex are operationally free; account for lock and I/O impact.
- `PG-015` `H1 / REVIEW`: Partitioning must match query and retention patterns; do not partition
  on size alone.
- `PG-016` `H1 / REVIEW`: Do not use a generic JSONB blob for core typed order/ledger relations when constraints, joins and query semantics are required.
- `PG-017` `H1 / REVIEW`: Do not assume MVCC eliminates locks, deadlocks, serialization failures or the need for consistent business-level conflict detection.
- `PG-018` `H1 / EVIDENCE`: Capacity plans must include WAL generation, checkpoint spikes, autovacuum, backup bandwidth, restore time, replication lag and connection count.

## 13. kdb+/q rules

- `Q-001` `B0 / REVIEW`: Do not use `value`, `eval` or dynamically evaluated strings on untrusted or interpolated input.
- `Q-002` `B0 / REVIEW`: Do not intern unbounded external identifiers as symbols without cardinality and lifecycle control.
- `Q-003` `B0 / TEST`: Distinguish typed nulls, infinities, NaN, zero, empty lists and missing values; never coerce missing market data to zero.
- `Q-004` `B0 / REVIEW`: Define date, timestamp, timespan, timezone, precision and epoch conversion at each boundary.
- `Q-005` `H1 / REVIEW`: Do not assume physical table order; sort explicitly using event/sequence/time keys.
- `Q-006` `B0 / REVIEW`: Do not use float columns for exact financial ledger/order state where scaled integers/decimal contract are required.
- `Q-007` `H1 / REVIEW`: Shared global tables need explicit single-writer, partition or serialisation semantics.
- `Q-008` `H1 / REVIEW`: Do not use `peach`, `par`, `each`, `over`, `scan` or joins as performance decoration. Define shape, side effects, threading and order.
- `Q-009` `M2 / REVIEW`: Do not use cryptic one-letter names for public functions or persistent domain fields. Idiomatic short local names are acceptable in small scopes.
- `Q-010` `H1 / TEST`: Test empty tables, typed nulls, duplicate keys, timestamp boundaries, attributes, schema drift and cardinality.
- `Q-011` `H1 / REVIEW`: Apply `sorted`, `parted`, `unique` and `grouped` attributes only when continuously enforced.
- `Q-012` `H1 / REVIEW`: Do not perform unbounded HDB scans in intraday capture/streaming paths.
- `Q-013` `H1 / REVIEW`: Do not change return shape/type without versioned cross-language contract update.
- `Q-014` `H1 / REVIEW`: Do not mix feed capture, tickerplant publication, RDB serving and HDB analytics into one process without operational rationale.
- `Q-015` `H1 / REVIEW`: Tickerplant log/replay semantics, subscriber recovery, message ordering and end-of-day rollover must be explicit.
- `Q-016` `H1 / REVIEW`: Do not allow analytic queries to starve or block feed capture. Isolate query workloads or use replicas/secondary consumers.
- `Q-017` `H1 / EVIDENCE`: Benchmark q with representative symbol cardinality, table sizes, partition counts, compression, cold/warm cache and query mix.
- `Q-018` `H1 / REVIEW`: Do not let q own a second version of the Rust domain contract; schema and units must be versioned and tested across bindings.
- `Q-019` `H1 / REVIEW`: q production use requires licensing, process supervision, log retention, recovery, disk capacity and operator runbooks.

## 14. JSON, YAML, schemas and configuration

- `CFG-001` `B0 / REVIEW`: Every persistent/exchanged schema has explicit version and compatibility policy.
- `CFG-002` `B0 / TEST`: Reject duplicate keys in authoritative JSON/YAML; parser-dependent last-key-wins behaviour is forbidden.
- `CFG-003` `B0 / REVIEW`: Define unknown-field policy. Persistent financial/security/policy payloads default to strict rejection. Forward-compatible protocol extensions require an explicit protocol rule and tests.
- `CFG-004` `B0 / REVIEW`: Distinguish absent, null, empty, zero and false.
- `CFG-005` `H1 / REVIEW`: Do not encode integers outside the exact representable range of any consuming language without string/typed encoding contract.
- `CFG-006` `H1 / REVIEW`: Do not rely on object key order except in explicitly canonical encoding.
- `CFG-007` `B0 / REVIEW`: Hashed/signed/content-addressed payloads require canonical bytes and an encoder-version policy.
- `CFG-008` `B0 / TEST`: Schema validation does not replace semantic domain validation.
- `CFG-009` `H1 / REVIEW`: Avoid YAML aliases, implicit scalar typing, custom tags and merge keys in critical config unless parser semantics are fixed and tested.
- `CFG-010` `B0 / REVIEW`: Use safe YAML loaders and explicit size/depth limits.
- `CFG-011` `H1 / REVIEW`: Do not duplicate capability/component/decision registries.
- `CFG-012` `H1 / AUTO`: Every ID/reference must resolve exactly once or be declared external/structural.
- `CFG-013` `H1 / AUTO`: Duplicate IDs, dangling references, forbidden cycles, invalid enums and unowned paths fail CI.
- `CFG-014` `H1 / REVIEW`: Generated schemas/code must be regenerated from authoritative source, not hand-edited.
- `CFG-015` `H1 / TEST`: Include negative fixtures for unknown fields, wrong types, missing required fields, unsupported versions, duplicate keys and invalid semantic relationships.
- `CFG-016` `H1 / REVIEW`: Configuration defaults must be explicit, safe and visible in effective-config output. Do not rely on undocumented defaults.
- `CFG-017` `B0 / REVIEW`: Risk, live mode, venue, account and entitlement configuration must not silently default to an enabled state.

## 15. Build, shell, Make, CMake and CI

- `OPS-001` `B0 / REVIEW`: No untrusted interpolation into `eval`, `sh -c`, SQL, paths or shell command strings.
- `OPS-002` `H1 / REVIEW`: Quote shell variables, use arrays for args, set deliberate options and inspect pipeline exit status.
- `OPS-003` `B0 / REVIEW`: No `curl | sh`, unpinned remote scripts or unverified downloaded tools in trusted build/release paths.
- `OPS-004` `H1 / REVIEW`: Lock GitHub Actions and transitive workflow dependencies with a supported dependency lockfile where available; otherwise pin each third-party action to a verified full-length commit SHA and comment the upstream release/tag. A mutable tag alone is not an immutable dependency identity.
- `OPS-005` `B0 / REVIEW`: Use least privilege workflow permissions. No write token or secret where not required.
- `OPS-006` `B0 / REVIEW`: Never check out/execute untrusted PR code in a privileged `pull_request_target` workflow with secrets.
- `OPS-007` `B0 / REVIEW`: No `continue-on-error`, `|| true`, warning-only required gates or broad test skips to mask failure.
- `OPS-008` `B0 / TEST`: Verify the checked-out commit equals the expected event SHA before tests/evidence.
- `OPS-009` `B0 / EVIDENCE`: Never reuse benchmark/evidence artifacts from another commit as proof for the current commit.
- `OPS-010` `H1 / REVIEW`: Pin toolchain and dependency versions where reproducibility/build integrity requires it.
- `OPS-011` `H1 / REVIEW`: Do not treat `latest`, `stable`, `master`, `main`, `ubuntu-latest` or unpinned packages as immutable build inputs.
- `OPS-012` `H1 / REVIEW`: Cache is an optimisation, not source of truth for dependencies, generated files or evidence.
- `OPS-013` `B0 / REVIEW`: Do not execute untrusted cache/artifact output without verifying provenance and trust boundary.
- `OPS-014` `H1 / REVIEW`: Build must fail on missing tools; no silent compiler fallback.
- `OPS-015` `H1 / REVIEW`: Linux pass does not replace required macOS pass or vice versa.
- `OPS-016` `H1 / REVIEW`: Distinguish infrastructure failure, code failure, skipped check, unsupported platform and missing evidence.
- `OPS-017` `H1 / REVIEW`: Bound build/test duration, parallelism, retries and artifact retention.
- `OPS-018` `H1 / REVIEW`: CMake uses explicit targets, target-scoped flags/includes/defs and declared dependencies.
- `OPS-019` `H1 / REVIEW`: Avoid source globbing when explicit source membership is required for traceability.
- `OPS-020` `H1 / REVIEW`: Do not write generated outputs into tracked source directories unless contract requires it.
- `OPS-021` `B0 / REVIEW`: No secrets in source, fixtures, logs, command history or artifacts.
- `OPS-022` `H1 / REVIEW`: Recursive deletion requires a validated bounded path and guard; never construct destructive paths from untrusted input.
- `OPS-023` `H1 / AUTO`: Maintained shell scripts must pass ShellCheck and script tests.
- `OPS-024` `B0 / REVIEW`: Never disable dependency, licence, vulnerability or secret scanning to force merge.
- `OPS-025` `H1 / REVIEW`: Make targets must declare dependencies and outputs; avoid hidden order dependencies and non-idempotent build steps.
- `OPS-026` `H1 / REVIEW`: Build scripts must not mutate the developer's global environment or package manager state without explicit opt-in.
- `OPS-027` `B0 / TEST`: CI gates must not be skipped by untested path filters when affected source paths are changed.
- `OPS-028` `H1 / REVIEW`: Infrastructure outage is not a repository pass. Keep required evidence pending or blocked.
- `OPS-029` `B0 / REVIEW`: Build a release artifact once and promote the same immutable artifact by cryptographic digest through environments. Do not rebuild separately for staging and production and assume the outputs are identical. Environment-specific configuration and secrets MUST be injected separately and MUST NOT mutate the artifact.
- `OPS-030` `B0 / EVIDENCE`: A release MUST identify source commit, artifact digest, builder identity, toolchain, dependency closure and provenance/attestation. Verify the attestation and artifact digest before promotion; a checksum without a trusted origin does not establish provenance.
- `OPS-031` `H1 / REVIEW`: Do not use mutable tags (`latest`, floating image tags or overwritten package versions) as release identity. Deploy by immutable digest or an equivalently immutable, verified identifier.
- `OPS-032` `B0 / REVIEW`: A deployment MUST NOT silently apply destructive database/schema changes before the new application version is compatible. Use expand/contract, staged backfills or an explicit forward-repair plan; define rollback compatibility before rollout.
- `OPS-033` `H1 / TEST`: Exercise deployment rollback or forward recovery using the exact release artifact and realistic state. A documented command that has never been exercised is not a recovery capability.
- `OPS-034` `H1 / REVIEW`: Use progressive rollout/canary only with explicit health signals, abort
  thresholds, an owner and a tested stop/rollback path. A canary is not safe on low traffic alone.
- `OPS-035` `H1 / REVIEW`: Release builds MUST NOT depend on undeclared workstation state, ambient credentials, mutable downloads, untracked generated files or an unpinned toolchain. Record build inputs that vary by design.
- `OPS-036` `H1 / EVIDENCE`: Produce a machine-readable SBOM (SPDX or CycloneDX, as selected by policy) for each releasable artifact. Include direct and transitive dependencies, relevant base images/runtime components and exact versions; retain it with the artifact digest and scan results.
- `OPS-037` `B0 / EVIDENCE`: Release artifacts and provenance attestations MUST be signed/attested by an approved build identity and independently verifiable before deployment. Signing keys MUST NOT be exposed to untrusted build steps; record verification identity, trust root and result.

## 16. Tests, fuzzing, fault injection and verification

A test is useful only if its oracle is correct and it would fail for the defect it claims to prevent.

- `TST-001` `B0 / TEST`: Every behaviour change needs tests for intended and relevant failure behaviour.
- `TST-002` `B0 / TEST`: Every bug fix needs a regression test that fails on the pre-fix implementation.
- `TST-003` `H1 / REVIEW`: Do not test only a helper when the public contract/integration path is the real behaviour.
- `TST-004` `H1 / REVIEW`: Do not mock away the behaviour the test claims to verify.
- `TST-005` `H1 / REVIEW`: Isolate temp dirs, ports, databases, clocks, RNG and environment between tests.
- `TST-006` `H1 / TEST`: No arbitrary sleeps as a concurrency oracle. Use barriers, channels, state predicates and bounded timeouts.
- `TST-007` `B0 / REVIEW`: Do not rerun flaky tests until green, suppress failure or expand tolerance without diagnosing cause.
- `TST-008` `H1 / REVIEW`: Skips require scope, platform, reason and owner; promised platforms cannot be broadly skipped.
- `TST-009` `H1 / TEST`: Test malformed, boundary, empty, duplicate, reordered, stale, unsupported-version and conflicting inputs.
- `TST-010` `B0 / TEST`: Persistence requires process kill, restart, uncertain commit, disk full, permission, lock, corruption, backup and restore tests.
- `TST-011` `H1 / TEST`: Concurrency tests must include interleaving, cancellation, failure and shutdown.
- `TST-012` `H1 / TEST`: Protocol decoders need captured fixtures, malformed frames, length/endian boundaries, sequence/epoch recovery and differential tests.
- `TST-013` `H1 / TEST`: Numeric code needs overflow, underflow, rounding, scale, negative values, boundaries and cross-language parity tests.
- `TST-014` `H1 / TEST`: Use property testing for invariants, fuzzing for parsers and sanitizers for memory/UB paths.
- `TST-015` `H1 / REVIEW`: One integration test does not replace unit/property tests; unit tests do not replace integration tests.
- `TST-016` `H1 / EVIDENCE`: Performance tests record source data, seed, machine, compiler, flags, workload, warm-up and method.
- `TST-017` `H1 / REVIEW`: Do not set hard performance gates from one noisy run.
- `TST-018` `B0 / REVIEW`: Do not claim production readiness from unit tests, synthetic benchmarks, mock venues or CI alone.
- `TST-019` `B0 / TEST`: Architecture rules that can be checked from registries/graphs must have executable gates.
- `TST-020` `H1 / REVIEW`: Test oracle must state expected contract, not current output alone.
- `TST-021` `H1 / TEST`: Test deterministic replay across restart, code/schema version and process boundaries where replay is required.
- `TST-022` `H1 / TEST`: Test graceful shutdown during active I/O, queued work, partial writes and in-flight transactions.
- `TST-023` `H1 / TEST`: Test restore into a new location and over an existing location, failed swap and interruption.
- `TST-024` `H1 / TEST`: Evidence artifacts must identify commit, toolchain, platform, task, gate and result.
- `TST-025` `H1 / REVIEW`: Negative tests must prove the guard rejects bad input, not that a
  helper returns an error for a hand-picked string.
- `TST-026` `H1 / REVIEW`: Fuzz corpus and regression fixtures must be retained when they identify a real bug.
- `TST-027` `H1 / TEST`: Cross-language bindings require both directions where data can round-trip: Rust->C++/Python/q and consumer->Rust.
- `TST-028` `H1 / TEST`: Test crash recovery at every durability boundary, including failure during recovery itself.
- `TST-029` `H1 / REVIEW`: Test suites must not rely on test execution order or shared mutable global fixtures.
- `TST-030` `H1 / EVIDENCE`: A claimed gate must run at the exact commit SHA. An earlier SHA or branch state is not evidence for a later commit.
- `TST-031` `H1 / REVIEW`: Do not make flaky tests green by blind retries, increased timeouts or permanent quarantine. A retry may gather diagnostics, but the original failure remains visible until root cause, scope, owner and expiry are recorded.
- `TST-032` `H1 / REVIEW`: Separate fast deterministic admission tests from slower
  integration/soak suites where useful, but do not move a required correctness/security/financial
  control out of the merge gate to shorten CI.

### Required test ladder

Select the relevant layers and state why omitted layers do not apply:

1. Constructors and type invariants.
2. Unit tests for deterministic operations.
3. Property tests for domain/state invariants.
4. Schema, ABI, protocol and component contract tests.
5. Integration with real local persistence/processes.
6. Crash, fault injection and restart/replay.
7. Fuzzing and sanitizers for untrusted parsers, unsafe and memory-sensitive code.
8. End-to-end tests against broker/venue test environments when access exists.
9. Performance/capacity tests with reproducible workloads.
10. Operational drills: restore, failover, credential rotation, reconciliation and incident recovery.

A tiny change does not automatically require every layer. The task must identify applicable layers and justify omissions. Financial, persistence, authority, protocol and security changes have higher default coverage requirements.

## 17. Security, privacy, licences and supply chain

- `SEC-001` `B0 / REVIEW`: Network data, user input, market feeds, broker messages, archive objects, model outputs and generated code are untrusted until validated.
- `SEC-002` `B0 / REVIEW`: Do not log secrets, tokens, account identifiers, personal data, private orders or licensed market data outside allowed retention.
- `SEC-003` `B0 / REVIEW`: No secrets in Git, fixtures, crash dumps, golden data or CI artifacts.
- `SEC-004` `B0 / REVIEW`: Do not invent cryptography, signatures, RNGs, key derivation or secret storage. Use maintained reviewed primitives.
- `SEC-005` `B0 / REVIEW`: Do not disable TLS/certificate verification.
- `SEC-006` `B0 / REVIEW`: Untrusted deserialisation requires size, depth, version, field and semantic validation.
- `SEC-007` `H1 / REVIEW`: File storage requires path containment, permission, symlink and atomic-publication policy.
- `SEC-008` `H1 / REVIEW`: Research/AI agents must have minimum filesystem, network, process and credential access.
- `SEC-009` `H1 / REVIEW`: Review dependency maintenance, licence, advisories, transitive graph, source, platform, performance and exit cost.
- `SEC-010` `B0 / REVIEW`: Do not copy substantial external code without origin, licence, attribution and obligation review.
- `SEC-011` `H1 / REVIEW`: Popularity, stars and author benchmarks are not security/quality review.
- `SEC-012` `H1 / EVIDENCE`: Dependency waivers require exact advisory/package, scope, reason, owner, mitigation and expiry.
- `SEC-013` `B0 / REVIEW`: Do not grant agents write access to risk, evidence, live permits, secrets or protected branches without independent need and approval.
- `SEC-014` `H1 / REVIEW`: No Python pickle for untrusted/cross-boundary artifacts.
- `SEC-015` `H1 / REVIEW`: Validate downloaded release artifacts with trusted origin and checksum/signature where available.
- `SEC-016` `H1 / REVIEW`: Classify data ownership, rights, retention, redistribution, privacy and permitted model use before storing/training.
- `SEC-017` `H1 / REVIEW`: Threat models must include compromised provider credentials, malicious feed payloads, replay, stale entitlement, dependency compromise and malicious generated code.
- `SEC-018` `H1 / TEST`: Test auth expiry, token rotation, permission downgrade, replayed request, signature mismatch and revoked credentials.
- `SEC-019` `B0 / REVIEW`: Authorisation MUST be checked at the server/authority boundary for every sensitive operation. Hiding a UI control, validating only in a client, or trusting an agent-supplied role is not access control.
- `SEC-020` `B0 / REVIEW`: Treat file names, archive members, URLs, redirects, environment variables and process arguments as untrusted. Enforce path containment, SSRF/network egress policy, allowlists where appropriate, and bounded resource consumption.
- `SEC-021` `H1 / REVIEW`: Separate production, staging, research and developer credentials/data. Do not reuse privileged production credentials in notebooks, local tests, demos or general-purpose agents.

## 18. Observability, production operations and SRE

- `OBS-001` `H1 / REVIEW`: Do not log every event on hot paths without measured budget, sampling and redaction.
- `OBS-002` `H1 / REVIEW`: Metrics labels must be bounded; no unbounded instrument/order/account IDs or arbitrary error strings.
- `OBS-003` `H1 / REVIEW`: Logs, metrics, traces, audit events and durable business events are different systems; do not substitute one for another.
- `OBS-004` `B0 / REVIEW`: Operational logs are not the accounting/audit ledger.
- `OBS-005` `H1 / REVIEW`: Expose queue depth, drops, lag, sequence gaps, retries, rejections, latency distribution and resource saturation.
- `OBS-006` `H1 / TEST`: Define graceful shutdown and durable state for every daemon.
- `OBS-007` `B0 / EVIDENCE`: No claim of continuity, HA, failover, DR, HFT, production readiness or venue operation without corresponding tests and measured evidence.
- `OBS-008` `H1 / REVIEW`: Define operating envelope: supported scale, hardware, throughput, tail latency, recovery time, memory, disk and known limits.
- `OBS-009` `H1 / REVIEW`: Never erase forensic evidence during repair/recovery. Preserve original state and record each repair.
- `OBS-010` `H1 / TEST`: Test restore, not backup. Record date, source, target and verification
  result.
- `OBS-011` `H1 / REVIEW`: Before deployment, assign ownership for alert, runbook, upgrade, retention, incident and recovery.
- `OBS-012` `B0 / REVIEW`: Do not enable live execution/venue operation because a feature flag exists. Require independent risk, entitlement, operational and human gates.
- `OBS-013` `H1 / REVIEW`: Alerts must be actionable and tied to an owner/runbook; avoid alert storms and unbounded cardinality.
- `OBS-014` `H1 / REVIEW`: Define SLOs from user/system outcomes and error budgets; do not use CPU or process-up as the only health measure.
- `OBS-015` `H1 / REVIEW`: Incident postmortems must record impact, timeline, contributing conditions, detection gaps, recovery, action owners and due dates. Avoid blame and vague actions.
- `OBS-016` `H1 / TEST`: Exercise overload, dependency outage, clock degradation, disk full, network partition, corrupted input, stale credentials and process restart.

## 19. AI-agent and generated-code rules

AI agents are untrusted contributors. They are not maintainers, risk authorities, release authorities or evidence authorities.

- `AI-001` `B0 / REVIEW`: Agents MUST read task context, source-of-truth docs, decisions, ownership and relevant tests before editing.
- `AI-002` `B0 / REVIEW`: Agents MUST NOT invent files, APIs, symbols, CI results, benchmark results, licences, entitlements, approvals or production capabilities.
- `AI-003` `B0 / REVIEW`: Agents MUST NOT say a command passed unless it ran and exit status/output were observed.
- `AI-004` `B0 / REVIEW`: Agents MUST NOT modify claims, capabilities, evidence, compliance, entitlements, risk limits, live permits or verification status to make their own change pass.
- `AI-005` `B0 / REVIEW`: Agents MUST NOT approve, merge, deploy, enable trading or promote a capability without explicit human authorisation.
- `AI-006` `H1 / REVIEW`: AI-generated code is untrusted and reviewed as external contribution.
- `AI-007` `H1 / REVIEW`: Compile success or model-generated tests do not prove correctness.
- `AI-008` `B0 / REVIEW`: Agents MUST NOT weaken, skip, mask or rewrite a gate as a response to a failure without proving a gate defect and receiving independent review.
- `AI-009` `H1 / REVIEW`: No speculative wrappers, placeholder crates, fake adapters, no-op methods or TODO-only features.
- `AI-010` `H1 / REVIEW`: No broad formatting/rename/architecture rewrite outside task scope.
- `AI-011` `H1 / REVIEW`: Preserve public contracts unless approved migration/decision changes them.
- `AI-012` `H1 / REVIEW`: Handoff claims must cite exact path, line, test, commit, run or output.
- `AI-013` `H1 / REVIEW`: External code copy requires provenance and licence review.
- `AI-014` `H1 / REVIEW`: Agents SHOULD flag destructive, financial, persistence, security and public-API risks before editing.
- `AI-015` `B0 / REVIEW`: Agents MUST NOT alter this policy/enforcement in the same change whose compliance they are establishing.
- `AI-016` `H1 / REVIEW`: Review AI tests for oracle quality and mutation sensitivity; do not accept tautological tests.
- `AI-017` `H1 / REVIEW`: Handoff lists changed paths, base/head SHA, commands run, skips,
  failures, assumptions, risks and remote actions.
- `AI-018` `B0 / REVIEW`: Human approval required for live trading, risk policy, credentials, entitlement, destructive migration, remote deletion, merge and release promotion.
- `AI-019` `B0 / REVIEW`: Agents MUST NOT create evidence for a test they did not execute or alter raw evidence after capture.
- `AI-020` `H1 / REVIEW`: Agents MUST distinguish retrieved source evidence, repository-local evidence, inference and proposal.

### Required AI handoff

1. Objective and task/decision IDs.
2. Base SHA and exact head SHA.
3. Changed paths by responsibility.
4. Contracts and state ownership affected.
5. Commands run and exact pass/fail/skip status.
6. Tests added, including negative/failure cases.
7. CI run IDs and exact-head verification.
8. Dependency/licence/advisory changes and generated artifacts.
9. Assumptions and unresolved questions.
10. Known limitations and human approval gates.

## 20. Naming, comments and source layout

- `NAM-001` `M2 / REVIEW`: Use domain names, not implementation accidents.
- `NAM-002` `H1 / REVIEW`: Encode units where ambiguity matters: `_ns`, `_us`, `_ms`, `_utc`, `_paise`, `_ticks`, `_lots`, `_bps`, `_pct`, `_bytes`, `_seq`.
- `NAM-003` `M2 / REVIEW`: Boolean names read as predicates: `is_valid`, `has_gap`, `can_submit`, `requires_reconcile`.
- `NAM-004` `H1 / REVIEW`: Distinguish ID, revision, sequence, version, epoch, token, symbol, key and hash.
- `NAM-005` `M2 / REVIEW`: Avoid vague public names such as `data`, `info`, `manager`, `helper`, `common`, `misc`, `thing` and `temp`.
- `NAM-006` `M2 / REVIEW`: Keep acronym casing, singular/plural and domain terminology consistent.
- `NAM-007` `H1 / REVIEW`: Do not encode mutable state in names that become stale.
- `NAM-008` `M2 / REVIEW`: Comments explain invariants, rationale, units, boundaries and references.
- `NAM-009` `H1 / REVIEW`: Comments for safety, finance, protocol, persistence and concurrency SHOULD link to contract/test.
- `NAM-010` `H1 / REVIEW`: TODOs name issue/task and owner; TODO is not a substitute for a design.
- `NAM-011` `H1 / REVIEW`: File placement follows source ownership and architecture mapping.
- `NAM-012` `M2 / AUTO`: Use existing editor/format tools. No repository-wide formatting as part of a narrow fix.
- `NAM-013` `H1 / REVIEW`: Public API names must distinguish command from query and mutation from observation.
- `NAM-014` `M2 / REVIEW`: Avoid abbreviations except established domain/protocol terms (`OMS`, `EMS`, `ISIN`, `FIX`, `UTC`, `VWAP`).

## 21. Performance and profiling beyond HFT

- `PERF-001` `H1 / EVIDENCE`: Do not optimise on intuition. Profile representative workload.
- `PERF-002` `H1 / EVIDENCE`: Report p50/p90/p99/p99.9/max where tail matters; averages alone hide stalls.
- `PERF-003` `H1 / REVIEW`: No lock-free, busy-spin, CPU pinning, NUMA pinning, custom allocator or kernel bypass without evidence.
- `PERF-004` `H1 / REVIEW`: Bound memory, queue, batch and worst-case work.
- `PERF-005` `H1 / REVIEW`: Do not claim zero-copy when any boundary copies or serialises.
- `PERF-006` `H1 / REVIEW`: A microbenchmark is not end-to-end throughput proof.
- `PERF-007` `H1 / REVIEW`: Consider allocator/GC, scheduler, page faults, caches, context switches, clock, NIC/driver and storage effects where material.
- `PERF-008` `H1 / REVIEW`: Do not sacrifice determinism, correctness, traceability or recovery for unmeasured latency.
- `PERF-009` `H1 / EVIDENCE`: Compare same data, hardware class, toolchain, flags, warm-up, concurrency and statistics.
- `PERF-010` `H1 / REVIEW`: Do not call a platform HFT because it is written in C++/Rust/q or reports microseconds.
- `PERF-011` `H1 / EVIDENCE`: Include overload and recovery scenarios in performance evaluation.
- `PERF-012` `H1 / REVIEW`: Do not tune a hot path based solely on profiler sampling when instrumentation perturbs the target; triangulate with low-overhead counters and external measurements.

## 22. Review protocol and branch admission

### 22.1 Before coding

The author must identify one coherent task, read its relevant source-of-truth context, identify the owner of every changed state, define data/time/units, list failure modes, identify test layers, review new dependencies/licences/entitlements, and define acceptance gates.

### 22.2 During coding

Keep commits scoped. Add tests with code. Preserve old contracts until approved migration. Stop
  when a frozen contract is contradicted. Do not change evidence or policy to make work pass.

### 22.3 Before PR

Run relevant format, lint, build, test, architecture, dependency, security, schema and evidence gates. Read the complete diff. Search for placeholders, unsafe defaults, ignored errors, generated noise, secrets, unrelated changes and unsupported claims. Confirm the exact head SHA.

### 22.4 Reviewer questions

1. What invariant is protected?
2. Who owns the changed state?
3. What happens with malformed, missing, stale, duplicate, reordered or unknown input?
4. What happens on process crash, power loss, disk full, network loss, timeout or uncertain commit?
5. Can the result be replayed and independently reconstructed?
6. Are units, time semantics, rounding, identity and provenance explicit?
7. Are cross-language/external contracts preserved?
8. Do tests cover failure paths and would they fail against the old defect?
9. Does evidence prove this exact SHA and workload?
10. Is the change simpler than the alternatives?
11. What capability/claim does it establish—and not establish?
12. What is rollback, restore or compensation?
13. Does any performance claim justify added complexity?
14. Does a new language/crate/database/service have a real ownership boundary?

### 22.5 Protected main

Configure GitHub rulesets/branch protection to:

- Require PRs and independent approval; require code-owner review for sensitive paths.
- Require exact-head CI, architecture, C/C++, sanitizers, security/dependency and aggregate evidence checks as defined by the project.
- Require conversation resolution and current checks.
- Enforce squash merge and linear history per Git policy.
- Restrict direct push, force push and deletion.
- Disable ordinary bypass for administrators, bots and automation.
- Protect `.github/workflows/`, `engineering/`, persistence, risk, OMS, broker/venue adapters, compliance and live execution paths.
- Ensure required checks run; obsolete-SHA checks and skipped required jobs do not satisfy
  admission.

A green check from an earlier SHA is not evidence for the current head. A platform outage is not a code failure, but it is also not a pass. Keep required evidence blocked until it exists or a formally approved policy change changes the requirement.

## 23. Enforcement roadmap

This guide is a policy proposal. It does not claim every rule is already automated. Implement enforcement in stages, with an owner, checker, positive fixture, negative fixture and false-positive policy for each automated rule.

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

Do not implement this with fragile regexes pretending to understand every language. Prefer compiler/linter APIs, syntax trees, type checks, schema validators, dependency tools, architecture graphs and targeted tests. High-risk semantic rules require review and domain tests.

Every machine-enforced rule record must contain: stable ID, severity, subsystem scope, enforcement type, checker/test ID, positive fixture, negative fixture, owner, exception policy, version and review date.

## 24. Minimum pre-merge checklist

- [ ] One coherent task and explicit scope.
- [ ] Relevant decisions, contracts, capability state and component ownership read.
- [ ] No unowned crate/module/service/registry/dependency.
- [ ] No hidden state owner, unclassified edge or authority bypass.
- [ ] No ambiguous financial values or units.
- [ ] No point-in-time leakage or unproven fill assumptions.
- [ ] Input/schema/version/time semantics validated.
- [ ] Error, timeout, retry and uncertain-outcome handling explicit.
- [ ] Queue/resource/memory/latency bounds defined.
- [ ] Persistence, crash and restore paths tested when relevant.
- [ ] Regression tests fail against prior defective behaviour.
- [ ] Property/fuzz/conformance/sanitizer/performance checks considered; omissions justified.
- [ ] Dependency/security/licence/data-rights review complete.
- [ ] Docs, architecture, capability, evidence and verification matrix updated where necessary.
- [ ] No fake adapter, placeholder, no-op, hidden skip or dead code.
- [ ] Formatting, lint, build, tests and required gates pass.
- [ ] Exact-head evidence current.
- [ ] Independent review complete; no unresolved required conversations.
- [ ] No unsupported alpha/HFT/live/production/venue claim.
- [ ] Human approval recorded for sensitive operation.

## 25. Official language and tool-specific verification profiles

This section maps the policy to tool families. It does **not** assert that every tool is installed or approved in this repository. The pinned files and existing CI workflow define the actual toolchain. Before enabling a command, verify its compatibility with the repository's compiler/runtime version, target platforms and configuration. Do not introduce a second formatter or conflicting linter policy.

### 25.1 Rust and Cargo

- The pinned `rust-toolchain.toml`, `Cargo.lock`, workspace manifests and Clippy configuration are
  authoritative. Do not follow `stable`/`nightly` because a blog post uses it.
- Typical admission checks are `cargo fmt --all -- --check`, Clippy over the supported workspace/target/feature matrix with warnings denied, and `cargo test` over the required matrix. The exact flags and feature combinations must come from repository policy; `--all-features` is not automatically correct if features are mutually exclusive or change semantics.
- Use `cargo deny check` or the repository's explicitly configured dependency scanner for licence, advisory, source and duplicate-dependency review. Keep the lockfile current with intentional dependency changes.
- Use Miri for supported unsafe-code tests, Loom for small concurrency protocols and cargo-fuzz or an equivalent fuzzer for parsers. Treat these as targeted evidence, not universal proofs.
- Do not globally suppress Clippy/rustc warnings. Any local lint suppression needs a reason, narrow scope, owner and expiry/task.
- For `unsafe`, make each block's proof local enough to review; document aliasing, alignment, initialization, provenance, lifetime and thread-safety assumptions. Check `unsafe_op_in_unsafe_fn` under the pinned edition/lint policy.

### 25.2 C and C++ toolchain

- Use the repository-pinned compiler, standard library, language mode, formatter, CMake and dependency versions. A build with one compiler is not portability evidence unless that is the only supported target.
- Use `clang-format` for formatting and `clang-tidy`/Clang Static Analyzer or the configured equivalent for static analysis. Run CTest and compile public headers independently.
- Run AddressSanitizer, UndefinedBehaviorSanitizer and ThreadSanitizer in compatible dedicated configurations. Do not claim that one sanitizer detects the bug classes covered by another; sanitizers have platform and instrumentation limits.
- Prefer target-scoped CMake properties (`target_include_directories`, `target_compile_options`, `target_compile_definitions`, `target_link_libraries`) over global directory state. Generated build directories must remain separate from source.
- C++ Core Guidelines and platform style guides are guidance, not substitutes for the language standard, local ABI contract or sanitizer evidence.

### 25.3 Python, notebooks and native extensions

- Use the pinned Python runtime and lockfile. `pyproject.toml` defines the authoritative Ruff/type-check/test configuration. Run `ruff check` and `ruff format --check` where Ruff is configured; do not run multiple competing formatters on the same files.
- Run the repository's configured test and type-check tools. Do not introduce a type checker as a merge blocker until its scope, baseline and owner are configured.
- Importing a module must remain side-effect-light. Use explicit entry points for I/O, process creation, environment changes and network access.
- For asyncio, understand task ownership and cancellation. Timeouts can cancel a task; cancellation is not a guarantee that a remote side effect did not happen.
- For optional free-threaded CPython, test the exact interpreter build and third-party native extensions. Do not rely on incidental GIL serialization or current built-in container locks. NumPy/pandas/Polars thread safety and mutation behavior must be checked in their current docs.
- Treat notebooks as reproducible research artifacts: record runtime, package lock, data hash, parameters, seeds and source revision; move reusable, reviewed logic into importable modules.

### 25.4 SQL, SQLite and PostgreSQL

- Use explicit migrations, named constraints, parameter binding and the target engine's real isolation/locking semantics. SQL dialect differences are part of the contract.
- For SQLite, set and read back required connection pragmas rather than trusting defaults. In particular, verify foreign-key enforcement and the chosen journal/synchronous/busy policy on every connection where applicable. WAL requires participating processes on the same host and is not a network-filesystem protocol. Use the online backup API or `VACUUM INTO` for live backups, then restore-test the result.
- For PostgreSQL, test the selected isolation level, serialization/deadlock retry behavior,
  transaction timeouts, connection-loss-at-commit reconciliation and replica lag/failover policy.
  Use representative `EXPLAIN (ANALYZE, BUFFERS)` plans. Test base backup plus WAL/PITR recovery,
  not backup-job success.
- Do not turn a successful local SQLite test into proof of PostgreSQL behavior, or a PostgreSQL benchmark into proof of SQLite semantics.

### 25.5 kdb+/q and time-series systems

- Verify the licensed runtime/version, schema, data types, symbol cardinality and temporal semantics at every boundary. q typed nulls, infinities, symbol interning, attributes, vector operations and table ordering are not interchangeable with ordinary SQL/Python assumptions.
- Keep ingestion/tickerplant, intraday/RDB and historical/HDB responsibilities explicit where that architecture is used. The log/replay, subscriber recovery, EOD rollover, disk retention and query-isolation contracts must be tested.
- Do not promote KX examples into a production architecture without validating the exact kdb+/KDB-X product, licence, deployment and version.

### 25.6 Shell, Make, CMake and GitHub Actions

- Run ShellCheck on maintained shell scripts. Quote expansions, use arrays for argument vectors, avoid `eval`, and check pipeline failures explicitly. `set -e` has context-dependent exceptions in Bash and is not a substitute for explicit error handling. Test destructive and cleanup paths.
- Use one canonical build entry point. Make targets must declare dependencies and outputs; CMake configuration must be target-scoped and reproducible. Build scripts should be safe to re-run.
- Prefer GitHub Actions workflow dependency lockfiles that pin actions and transitive dependencies. Where unavailable, pin third-party actions by verified full commit SHA. Minimise each workflow/job's `permissions`, and keep secrets out of untrusted pull-request execution. A privileged `pull_request_target` job must not check out or execute attacker-controlled PR code with secrets.
- Validate branch protection/rulesets enforce required review, exact-head checks, conversation
  resolution, restricted bypass and protected-path ownership. Configuration screenshots or a
  policy document are not proof; test the effective repository settings.

### 25.7 Toolchain change policy

A language/runtime/compiler/database/CI tool upgrade is its own reviewed change. It must identify: old and new versions; release notes/security advisories; changed defaults or semantics; supported target matrix; lockfile/ABI/schema effects; benchmark or compatibility evidence; rollback plan; and the owner who will maintain the new toolchain. Never smuggle a broad toolchain upgrade into an unrelated bug fix.

---

## 26. External research and engineering references

**Research boundary:** reviewed 2026-11-12. Official sources below are primary documentation or standards. Versioned documentation must be rechecked against the project's pinned version before use. Community threads appear in a separate non-normative subsection. This is a curated reference set, not a claim of exhaustive audit of every listed project.

### 26.1 Language specifications, official guidance and tool docs

- Rust Reference: <https://doc.rust-lang.org/reference/>
- Rust Edition Guide, including `unsafe_op_in_unsafe_fn`: <https://doc.rust-lang.org/edition-guide/rust-2024/unsafe-op-in-unsafe-fn.html>
- Rustonomicon, `Send` and `Sync`: <https://doc.rust-lang.org/nomicon/send-and-sync.html>
- Rust Clippy usage and lint levels: <https://doc.rust-lang.org/clippy/usage.html>
- Rust Clippy configuration: <https://doc.rust-lang.org/clippy/configuration.html>
- Tokio `select!` cancellation-safety guidance: <https://docs.rs/tokio/latest/tokio/macro.select.html>
- Tokio `JoinHandle` task lifetime, cancellation and join semantics: <https://docs.rs/tokio/latest/tokio/task/struct.JoinHandle.html>
- Miri: <https://github.com/rust-lang/miri/>
- Loom: <https://docs.rs/loom/latest/loom/>
- C++ Core Guidelines: <https://isocpp.github.io/CppCoreGuidelines/CppCoreGuidelines>
- LLVM Coding Standards: <https://llvm.org/docs/CodingStandards.html>
- Clang Static Analyzer: <https://clang.llvm.org/docs/ClangStaticAnalyzer.html>
- Clang AddressSanitizer: <https://clang.llvm.org/docs/AddressSanitizer.html>
- Clang UndefinedBehaviorSanitizer: <https://clang.llvm.org/docs/UndefinedBehaviorSanitizer.html>
- Clang ThreadSanitizer: <https://clang.llvm.org/docs/ThreadSanitizer.html>
- CMake target include directories: <https://cmake.org/cmake/help/latest/command/target_include_directories.html>
- Python 3.14 asyncio tasks/timeouts: <https://docs.python.org/3.14/library/asyncio-task.html>
- Python free-threading HOWTO: <https://docs.python.org/3/howto/free-threading-python.html>
- Python thread-safety guarantees for free-threaded builds: <https://docs.python.org/3/library/threadsafety.html>
- NumPy thread safety: <https://numpy.org/doc/stable/reference/thread_safety.html>
- Ruff configuration: <https://docs.astral.sh/ruff/configuration/>
- Ruff formatter: <https://docs.astral.sh/ruff/formatter/>
- ShellCheck wiki: <https://www.shellcheck.net/wiki/>
- Bash `set` builtin and `errexit` semantics: <https://www.gnu.org/s/bash/manual/html_node/The-Set-Builtin.html>

### 26.2 Market data, low latency and protocol standards

- Linux kernel memory barriers: <https://docs.kernel.org/core-api/wrappers/memory-barriers.html>
- Linux `perf`: <https://perf.wiki.kernel.org/>
- DPDK Programmer's Guide: <https://doc.dpdk.org/guides/prog_guide/>
- LMAX Disruptor technical paper: <https://lmax-exchange.github.io/disruptor/disruptor.html>
- Aeron: <https://github.com/aeron-io/aeron>
- FIX Trading Community standards: <https://www.fixtrading.org/standards/fix-protocol/>
- FIX Simple Binary Encoding: <https://github.com/FIXTradingCommunity/fix-simple-binary-encoding>
- KX q documentation: <https://code.kx.com/q/>
- kdb+ architecture and tickerplant recovery: <https://code.kx.com/q/architecture/>
- KDB-X tick architecture examples: <https://code.kx.com/kdb-x/examples/tick.html>
- Databento documentation: <https://databento.tech/docs>

### 26.3 Database and durability documentation

- SQLite Write-Ahead Logging and same-host constraint: <https://www.sqlite.org/wal.html>
- SQLite PRAGMA reference, including `foreign_keys`, `journal_mode`, `synchronous` and `busy_timeout`: <https://www.sqlite.org/pragma.html>
- SQLite online backup API: <https://www.sqlite.org/backup.html>
- SQLite `VACUUM INTO`: <https://www.sqlite.org/lang_vacuum.html>
- PostgreSQL 18 transaction isolation: <https://www.postgresql.org/docs/18/transaction-iso.html>
- PostgreSQL 18 MVCC: <https://www.postgresql.org/docs/18/mvcc.html>
- PostgreSQL 18 query planning and `EXPLAIN`: <https://www.postgresql.org/docs/18/using-explain.html>
- PostgreSQL 18 backup, WAL archiving and PITR: <https://www.postgresql.org/docs/18/backup.html>

### 26.4 Security, supply chain and repository controls

- NIST SP 800-218 Secure Software Development Framework (SSDF) v1.1: <https://csrc.nist.gov/pubs/sp/800/218/final>
- SLSA v1.2 build requirements and provenance: <https://slsa.dev/spec/v1.2/build-requirements>
- OWASP Application Security Verification Standard (ASVS) 5.0.0: <https://asvs.owasp.org/>
- OWASP Top 10 2025: <https://owasp.org/projects/top-ten>
- OWASP Developer Guide: <https://devguide.owasp.org/>
- GitHub protected branches: <https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches>
- GitHub rulesets: <https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/about-rulesets>
- GitHub Actions secure-use guidance (workflow dependency lockfiles, SHA pinning, least privilege): <https://docs.github.com/en/actions/reference/security/secure-use>
- GitHub guidance for privileged `pull_request_target` workflows: <https://docs.github.com/en/actions/reference/security/securely-using-pull_request_target>

### 26.5 Reliability and engineering practice

- Google SRE Workbook: <https://sre.google/workbook/table-of-contents/>
- Google Engineering Practices / Code Review: <https://google.github.io/eng-practices/review/>
- Google Style Guides: <https://google.github.io/styleguide/>
- Microsoft Pragmatic Rust Guidelines: <https://microsoft.github.io/rust-guidelines/guidelines/>
- NautilusTrader developer guide: <https://nautilustrader.io/docs/latest/developer_guide/>
- NautilusTrader architecture: <https://nautilustrader.io/docs/latest/concepts/architecture/>
- Polars user guide: <https://docs.pola.rs/>
- Apache Arrow C++ development: <https://arrow.apache.org/docs/developers/cpp/development.html>

### 26.6 Practitioner discussions and forum signal (non-normative)

These discussions informed the failure-mode review. They are examples of practitioner questions and disagreement, not verified incident reports and not standalone policy authorities. Use them to generate tests and questions, then validate against primary documentation.

- Rust async task lifetime, cancellation and shutdown concerns (r/rust, 2026-01-29): <https://www.reddit.com/r/rust/comments/1qq1xcz/struggling_to_reason_about_task_lifetimes_in/>
- Rust async deadlock and cancellation safety discussion (r/rust, 2026-07-24): <https://www.reddit.com/r/rust/comments/1v54et2/what_are_the_most_absolutely_demonic_patterns_and/>
- Tokio task cancellation and `JoinHandle` behavior discussion (r/rust, 2024-04-19): <https://www.reddit.com/r/rust/comments/1c7xv7l/a_short_blog_post_about_some_tokio_task_cancellation_patterns_ive_found_useful/>
- Code smells and accidental utility/helper dumping grounds (r/ExperiencedDevs, 2024-03-05): <https://www.reddit.com/r/ExperiencedDevs/comments/1b79jgc/what_are_some_common_code_smells_and_antipatterns/>
- Code review quality, context and constructive feedback (r/ExperiencedDevs, 2025-08-18): <https://www.reddit.com/r/ExperiencedDevs/comments/1mtl60u/teach_and_stay_humble_in_code_reviews/>
- Flaky CI suites and loss of trust in test results (r/devops, 2025-12-30): <https://www.reddit.com/r/devops/comments/1pzgupz/qa_tests_blocking_deploys_6_times_today_averaging/>
- Backtesting overfit, execution assumptions and slippage (r/algotrading, 2026-01-05): <https://www.reddit.com/r/algotrading/comments/1q4gpu1/is_overfitting_the_1_reason_most_backtested/>
- Trade-slippage modelling, next-bar fills and data-resolution limits (r/algotrading, 2026-06-09): <https://www.reddit.com/r/algotrading/comments/1tycaha/trade_slippages/>
- Forward-testing after frozen strategy rules, explicit spread/financing/slippage and rejected variants (r/algotrading, 2026-11-06): <https://www.reddit.com/r/algotrading/comments/1xprhrh/update_after_60_backtested_strategies_6_are_now/>
- Practitioner notes on historical backtesting pitfalls (r/quantfinance, 2025-08-22): <https://www.reddit.com/r/quantfinance/comments/1mxlsdy/>

### 26.7 Open-source project discovery (reference-only)

- Awesome Open Source Trading: <https://github.com/WallStreetPete/awesome-open-source-trading>
- NautilusTrader: <https://github.com/nautechsystems/nautilus_trader>
- QuantConnect LEAN: <https://github.com/QuantConnect/Lean>
- HftBacktest: <https://github.com/nkaz001/hftbacktest>
- ABIDES: <https://github.com/abides-sim/abides>
- exchange-core: <https://github.com/exchange-core/exchange-core>
- QuickFIX: <https://github.com/quickfix/quickfix>
- Hummingbot: <https://github.com/hummingbot/hummingbot>
- QuantLib: <https://github.com/lballabio/QuantLib>
- Microsoft Qlib: <https://github.com/microsoft/qlib>
- vectorbt: <https://github.com/polakowo/vectorbt>
- DuckDB: <https://github.com/duckdb/duckdb>
- ClickHouse: <https://github.com/ClickHouse/ClickHouse>
- Apache Arrow: <https://github.com/apache/arrow>
- DataFusion: <https://github.com/apache/datafusion>
- DPDK: <https://github.com/DPDK/dpdk>
- PostgreSQL: <https://github.com/postgres/postgres>
- SQLite: <https://github.com/sqlite/sqlite>

The authoritative local curated set remains `engineering/reference-projects.json`. Its adoption policy is `reference_only` by default. Before copying code or adding a dependency, review active maintenance, exact release/branch, licence, security, test quality, platform support, performance, recovery, data rights and architecture fit. External abstractions must not be transplanted wholesale.

## 27. Maintenance and governance

- The engineering owner maintains the standard with language, quantitative, data, security and operations reviewers.
- Changes to this standard require independent review.
- Changes to `B0` rules require an accepted decision explaining why the invariant changes.
- Rule IDs are permanent. Retire a rule with a `RETIRED` marker and replacement link; never reuse its ID.
- Review after architecture, toolchain, language, database, asset-class, venue, security or incident changes, and at least once per release cycle.
- New languages/subsystems need a dedicated section before production use.
- Add rules from incidents and real defects. Do not add speculative prohibitions solely because a pattern looks unfamiliar.
- Every rule should state harm, applicability, counterexample if permitted, and how it is tested/reviewed.
- Review official source links and pinned-version compatibility at least quarterly and after relevant upstream releases or incidents.
- Keep this policy aligned with actual tools; policy that CI cannot enforce and reviewers do not apply becomes documentation theatre.
- Do not interpret a tool's absence as permission to skip the invariant. Record a compensating test/review control and an owner for adding the missing tool when warranted.

**Governing principle:** code is admitted by contract and evidence, not author confidence, model confidence, language fashion, code volume, cleverness, popularity or a green check that did not test the changed behaviour.
