# ADR-FND-001 — Corporate financial facts and point-in-time validation

- **Status:** Proposed for owner ratification; implementation is a bounded research-data vertical
  only.
- **Decision date:** 2026-11-19.
- **Baseline:** `ea877ab6d784356c2e852412f5c904fd0ca15cc4` (`main`).
- **Domain owner role:** Fundamental-data owner (natural person to be named by repository owner).
- **Scope:** Typed company identity observations, financial facts, provenance references, fact
  revision lineage, exact-decimal arithmetic, point-in-time selection, JSONL validation and CI
  fixtures.

## 1. Context

Nirnay needs an auditable representation of corporate facts before it can perform reproducible
fundamental research or valuation. A raw number without a source document, locator, reporting
period, unit, accounting basis, scope and knowledge timestamps is not research-grade input.

The current repository already contains source archives, India reference-data concepts, and
point-in-time research datasets. This decision adds a narrow domain contract and a validation
application. It does not create source acquisition, a general storage platform, or an HFT runtime.

## 2. Decision

Introduce two active workspace packages:

- `fundamental-domain`: pure Rust value types and validation for source-backed company identity
  observations and reported/derived financial fact revisions.
- `fundamental-tools`: a bounded JSONL validator and fixture CLI; it is an application/composition
  root and owns file I/O.

The domain contract requires:

1. Opaque Nirnay company identity, separate from CIN, ISIN, exchange symbol and legal name.
2. Effective-dated company identity observations with recorded time, source document digest,
   evidence locator and append-only correction revisions.
3. Financial facts with expanded XBRL concept names (absolute namespace URI and the supported
   ASCII NCName subset), instant/duration periods, standalone/consolidated scope, accounting basis,
   explicit dimensions, units and exact base-10 values.
4. Reported and derived facts as distinct origin variants. Reported facts carry source/filing
   identity and a document digest plus XBRL/PDF/JSON locator. Derived facts carry method
   identity/version, input fact IDs and a derivation digest.
5. Immutable logical fact keys across value restatements; contiguous revision number and
   `supersedes` chain.
6. Historical point-in-time selection that excludes reported facts with unknown publication
   time and excludes any fact whose publication (when applicable), observation, ingestion,
   predecessor or derivation-input state was unavailable at the requested time. Internally
   derived facts use their recorded observation/computation time when no public publication time
   exists.
7. Strict JSONL framing, per-record and aggregate input-byte bounds, record-count bounds, exact
   input digest, and closed-set lineage validation.
8. No floating-point representation for reported financial amounts.

The logical identity hashes use separate domain prefixes:
`nirnay:company-identity-key:v1` and `nirnay:financial-fact-key:v1`, each followed by a NUL byte
and the compact JSON serialization of the typed identity fields. The field order is the
declaration order in the domain structs and is part of schema version 1.

## 3. Source and storage boundary

This initial vertical accepts source-backed records whose source documents were acquired through
an independently permitted process. It does not fetch or scrape MCA21, SEBI, NSE, BSE, company
websites or data vendors. The committed fixture records and their document digests are synthetic
conformance data, not evidence about actual companies or filings. The implementation does not
establish data rights, publisher authenticity, completeness, legal identity or correctness of a fact
because a supplied SHA-256 has valid syntax or matches separately supplied document bytes.

The current validator holds decoded typed records in memory to validate cross-record revision and
derivation graphs. Its limits bound line size and record count, not total memory to a production
service level. It is an offline validation tool, not the production ingest store. Durable
publication, indexed query, archival replication, backup/restore and distributed ingestion require
separate measured decisions and acceptance evidence.

## 4. Dependency and composition rules

- `fundamental-domain` depends only on existing workspace foundation libraries (`jiff`, `serde`,
  `serde_json`, `sha2`, `thiserror`). It performs no I/O and reads no clock.
- `fundamental-tools` owns file I/O and depends inward on `fundamental-domain`.
- No new network client, database, broker SDK, service, message bus or execution dependency is
  introduced.
- The CLI validates complete closed datasets. It is not a source adapter and does not mutate input
  files.
- The application can be replaced by other composition roots without changing domain semantics.

## 5. Non-goals

This ADR does not authorize or claim:

- MCA, SEBI, NSE or BSE scraping, APIs, entitlements or complete company coverage.
- PDF or XBRL parsing, taxonomy resolution or a complete Ind AS ontology.
- Accounting policy resolution, automatic restatement or automatic financial statement repair.
- Production persistence, zero-data-loss, replicated durability or horizontal scaling.
- DCF, multiples or investment recommendations.
- Market-data, order-book, strategy, OMS, risk or broker changes.
- Colocated HFT, nanosecond execution, FPGA/SBE/kernel-bypass or live order routing.
- Trading readiness, regulatory approval or profitability.

## 6. Failure semantics

Malformed identifiers, unsupported schema versions, non-canonical exact decimals, invalid units or
periods, malformed document digests, missing evidence, duplicate record IDs, broken revision
chains, missing derivation inputs, derived-input cycles, future-knowledge dependencies, invalid
JSONL framing, overlong records and input I/O errors fail closed. The validator emits no success
report on failure.

Unknown publication timestamps are never inferred from reporting periods, ingestion time or a
later document observation. Such facts are excluded from point-in-time selection and counted in
the summary.

## 7. Verification plan

The repository's existing Rust format/check/test/Clippy, architecture, schema, source-style and CI
jobs apply. The synthetic fundamental fixture files and the `make fundamental-fixture-check`
target were retired under D11; `scripts/check_fundamental_contract_vectors.py` and the Rust
contract-vector tests continue to validate deterministic schema vectors through the domain
validator. Independent verification must record the exact commit, Rust toolchain, target,
commands, exit statuses and test counts; a status field in JSON is not evidence.

## 8. Future decisions

Separate ADRs are required before adding:

- Each regulatory/provider acquisition adapter and its legal/technical access contract.
- XBRL and PDF parser implementations and taxonomy versions.
- Filing/document archive and immutable dataset publication implementation.
- An indexed durable fundamental store and backup/restore model.
- Financial statement reconciliation rules by reporting framework.
- Valuation models and model governance.
- Integration with strategy, portfolio or execution domains.
- Any low-latency or HFT runtime profile.

## 9. Acceptance boundary

A passing build or fixture test establishes only the checked software contract. It does not
authorize M1 exit, unrestricted M2 implementation, HFT operation, broker connectivity, live
trading, use of unlicensed data, external distribution or platform-readiness claims.
Repository-owner ratification and separate milestone authorization remain required under
`AGENTS.md` and the accepted M1 decisions.

## 10. Schema-v1 fact semantics (Nirnay-owned decisions)

These decisions prevent the v1 contract from silently discarding source
semantics when a taxonomy-versioned XBRL or PDF parser is introduced later.
They do not authorize a parser, source acquisition or external distribution.

- **Reported accuracy (`decimals`):** v1 stores the exact canonical value only;
  reported precision is not represented. A future version must add accuracy
  metadata rather than reinterpreting the stored coefficient.
- **Nil facts:** not representable in v1. A nil requires a new explicit variant
  in a later schema version; an absent value is never encoded as zero.
- **Original lexical value:** v1 commits to the source document digest and an
  evidence locator, not to the original lexical token. Preserving the token is
  an ingestion requirement for a future versioned parser.
- **Taxonomy identity and version:** not fields on a v1 fact. They belong in
  the source evidence and the derived method identity of a future parser; a
  fact cannot claim a taxonomy it does not record.
- **Units and dimensions:** represented in `FactIdentity` (currency, magnitude,
  dimension set). They are part of the logical fact key, not free text.
- **Derivation digest:** `derivation_sha256` is recomputed by
  `expected_derivation_sha256` over a domain-separated canonical object
  containing `method_id`, `method_version`, sorted unique `input_fact_ids`,
  the exact output value and the output `fact_id`. A self-declared digest that
  does not match is rejected. This makes derivation lineage verifiable before
  any durable fundamental store exists.
- **Schema/Rust equivalence:** shared vectors under
  `research/fundamentals/contract-vectors/` are validated by both the published
  JSON Schemas and the Rust domain rules. Numeric range checks beyond the
  schema's digit bound remain Rust-only and are documented as such; no second
  definition of a valid fact is published without a vector that exercises it.
