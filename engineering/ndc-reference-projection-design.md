# Nirnay NDC India reference projection — Proposed design

<!-- SPDX-License-Identifier: Apache-2.0 -->

- **Status:** Proposed design. It fixes consumer-side semantics for a read-only
  projection of Fieldbook NDC India reference releases. It ratifies no contract,
  grants no right, accepts no dataset and authorizes no implementation,
  collection or experiment.
- **Task / branch / base:** W2-E · branch `docs/w2-consumer-projection` ·
  base `800667a`. Documentation only: one new Markdown file in the Nirnay
  worktree. No crate, schema, data, experiment or CI edit.
- **Method:** read-only inspection of the ratified owner decision, the C0
  semantic model, the NDC v2 producer and consumer requirements, the D1
  crosswalk, and the `india-domain` / `india-reference` types at this base.
- **Scope discipline:** no code transfer, shared crate, provider contact,
  collection, data download, dataset acceptance or live routing.

## 1. Governing inputs and exact digests

Each input is pinned by path and SHA-256. Digests were computed with
`shasum -a 256` against the files in the workspace clones on 2026-11-23. The C0,
producer, consumer and D1 values agree with `ENGINEERING-DOC-DIGESTS.json` and
the owner ratification table.

| Document | Path | SHA-256 |
|---|---|---|
| Owner ratification 2026-11-23 | `engineering/decisions/OWNER-RATIFICATION-2026-11-23.md` | `e3cc365b9121c22f114b1b17805cffcf0e0f0d5324d0b65ea5255e3be7d6f3f1` |
| C0 semantic model | `engineering/design/NDC-SEMANTIC-MODEL-V2-PROPOSED.md` | `3dace660c362e683790ba7e56a785ded8af836abda36f6eab06ed284de49eed2` |
| Fieldbook NDC v2 producer requirements | `fieldbook/engineering/ndc-producer-requirements-v2-proposed.md` | `215a5b553c6cd51aca8c55d1150eef7ceb7bb6c4d3ea9540d09eb990f415f77c` |
| Nirnay NDC v2 consumer requirements | `engineering/ndc-consumer-requirements-v2-proposed.md` | `22eaef7de012e4f144299bc7fe82a701670531799e7e340e60a2908786a5a957` |
| Nirnay consumer-use model | `engineering/ndc-consumer-use-v2-proposed.md` | `4b9d1fe7190de08151989fc8895f42e1daa737021e8dae019ddc3325f9603a2c` |
| D1 ownership crosswalk | `engineering/architecture/india-domain-ownership-crosswalk.md` | `a857592dfa415043d5e87e79d930dbd98792f414cab4ec264b2a1c165d5ecd68` |

The ratified ownership decision assigns acquisition, source decoding, raw
storage, normalization, canonical identity, effective-dated reference data,
calendars and release publication to Fieldbook. Nirnay consumes immutable
releases and owns accepted research datasets. `NR-ARCH-ADR-0001` is Accepted by
that decision; NDC v2 semantics are Approved. The decision is not a data
licence, and its approval grants no provider right.

## 2. Release identity recorded by the projection

No Fieldbook India NDC release is published at this base: provider rights remain
`UNKNOWN` and dataset acceptance stays `BLOCKED`. No `release_id` or release
digest value is invented here. The projection records the exact values carried
by a signed manifest, verbatim and without shortening, renaming or deriving
them. A change to a release or to its rights evidence requires a new admission
decision.

| Manifest section | Field | Recorded use |
|---|---|---|
| `release_identity` | `release_id` | primary key of the projection; a correction has a new identity |
| `release_identity` | `contract_version`, `release_version` | version gate; unknown versions are refused |
| `release_identity` | `as_of_ns`, `publication_state` | revision selection basis and admission gate |
| `publication_identity` | `publisher_identity`, `signing_key_id` | trusted-key registry lookup |
| `publication_identity` | `signature_reference`, `archive_reference` | sidecar and immutable archive link |
| `digests` | `manifest_sha256`, `data_sha256`, `digest_algorithm` | byte-level binding of the exact release |
| `digests` | `artifact_digests` | artifact-level binding |
| `schema_versions` | `contract_version`, `normalizer_version`, `payload_schema_version`, `envelope_version` | schema and envelope gate |
| `coverage` | per-dimension universe, denominator, count basis, `as_of_ns`, verdict | denominators carried on the projection |
| `rights` | `permission_grants`, `document_references`, `reviewed_attestations` | admission verdict and evidence identity |
| `lineage` | PROV-O relations and archive digest | reference and instrument lineage |

Every projection row carries the `release_id`, `manifest_sha256` and
`data_sha256` of the release it came from. A missing, malformed or mismatched
field stops admission before a row is materialized.

## 3. Verify, then project

Stages are ordered. The first refusal stops admission; a subsequent stage does
not soften an earlier failure. A refused release is not consumed in part. The
existing declaration-level admission validator is not a full release verifier
and must not be described as one.

| # | Stage | Check | Refusal |
|---|---|---|---|
| 1 | Parse and version gate | positive integer schema and envelope versions; known contract version and declared IDs | `REJECT_SCHEMA_ENVELOPE` |
| 2 | Canonical bytes | recompute RFC 8785 JCS manifest bytes and `manifest_sha256` | `REJECT_SIGNATURE_INTEGRITY` |
| 3 | Detached signature | Ed25519 sidecar, key id bound to publisher identity, key validity and revocation | `REJECT_SIGNATURE_INTEGRITY`, `REJECT_PUBLISHER_IDENTITY` |
| 4 | Data and artifact digests | compare `data_sha256` and `artifact_digests` against published bytes | `REJECT_ARTIFACT_DIGESTS` |
| 5 | Rights evidence | one unique `PERMIT` grant per requested action, exact purpose, scope and retention | `REJECT_RIGHTS_EVIDENCE`, `REJECT_AMBIGUOUS_GRANTS` |
| 6 | Coverage | declared universe, denominator, count basis, as-of instant and verdict per dimension | `REJECT_COVERAGE` |
| 7 | Lineage and corrections | provenance on each assertion, correction chain, as-of and knowledge-time selection | `REJECT_SOURCE_LINEAGE`, `REJECT_PIT_CONSISTENCY` |
| 8 | Projection build | stable Fieldbook IDs, effective intervals, mapping uniqueness | projection hold; no row emitted |
| 9 | Admission receipt | immutable decision carrying exact release identity and digests | typed code; no receipt on refusal |

### 3.1 Publication identity and signature

Signature verification uses RFC 8785 JSON Canonicalization Scheme over the
manifest object with a detached Ed25519 sidecar. The signature is not a manifest
property. Canonicalization keeps the `I-JSON` subset, no duplicate property names,
UTF-8 strings preserved as-is, no whitespace between tokens, recursive property
name sorting by UTF-16 code units, preserved array order, deterministic ordering
for unordered sets, and exact decimals and arbitrary-precision integers as JSON
strings. NaN, Infinity and lone surrogates terminate the operation. The sidecar
stays outside the canonical bytes and does not enter the manifest digest. The key id resolves to
one public key in an out-of-band trusted registry; unknown, malformed, expired
or revoked keys fail closed.

A verified signature proves publisher-key control and integrity of the exact
signed bytes. It establishes no source authenticity, source completeness,
coverage, right or research entitlement. Those are separate stages with
separate trust relationships.

### 3.2 Rights and coverage

Rights evidence is a reviewed written record with an exact purpose string,
named licensee and recipient, scope, effective interval and document digest.
Each requested action needs one matching `PERMIT` grant covering actor, purpose,
licensee, recipient, action, object, scope, retention and interval. A denial, a
missing grant, two grants for one action or an ambiguous grant denies. The
declared purpose is `proprietary_own_account_trading_research`, preserved
verbatim; nearby labels do not substitute.

Each coverage dimension carries its own declared universe and denominator.

| Dimension | Denominator |
|---|---|
| Instrument coverage | declared point-in-time instrument universe |
| Listing / identity coverage | declared listing and identifier assertions |
| Contract coverage | declared derivative or commodity contract set |
| Corporate-action coverage | declared action universe per instrument and period |
| Session / time coverage | declared venue calendar and session intervals |
| Trade-event coverage | declared trades in scope |
| Quote / depth coverage | declared BBO or MBP levels and snapshots |
| MBO coverage | declared order lifecycle events |
| Sequence continuity | declared sequence ranges and gap policy |

Passing one dimension implies no other. A release is refused when an applicable
dimension lacks a declared universe, denominator, scope, as-of instant or
verdict, when a denominator disagrees with the manifest, or when one count is
offered for multiple dimensions. Gaps and rejects stay attached to their
dimension; no value is filled or interpolated from a local file.

### 3.3 Reference and instrument lineage

Lineage follows PROV-O entities, activities and agents with
`wasDerivedFrom`, `wasRevisionOf`, `hadPrimarySource`, `generatedAtTime` and
`invalidatedAtTime`. Canonical instrument, venue, listing and provider IDs come
from the release. Venue identifiers carry valid and known intervals. A
corporate-action correction is a new revision with a new release identity; the
prior revision and its knowledge time stay in lineage. Selection uses the as-of
instant and knowledge time, not arrival order. PROV-O describes lineage; it
confers no permission and proves no truth of an observation.

### 3.4 Rejection matrix

| Condition | Outcome |
|---|---|
| Unknown schema, contract version, envelope version or declared ID | refuse at stage 1 |
| Recomputed canonical bytes or signature sidecar unavailable or invalid | refuse at stage 2 or 3 |
| Publisher identity, key id, validity or revocation mismatch | refuse at stage 3 |
| Data or artifact digest mismatch | refuse at stage 4 |
| Rights evidence absent, denied, duplicated, ambiguous or out of scope | refuse at stage 5 |
| Denominator absent, inconsistent with the manifest or reused across dimensions | refuse at stage 6 |
| Missing provenance, broken correction chain, arrival-order selection | refuse at stage 7 |
| Conflicting mapping or overlapping effective interval in the projection | hold at stage 8 |
| No matching `PERMIT` grant or incomplete admission decision | refuse at stage 9 |

Design-level holds at stage 8 are projection-specific outcomes. They do not add
a value to the ratified NDC refusal code set.

## 4. Hard exclusions

- Nirnay does not re-normalize raw Fieldbook payloads. Normalization belongs to
  Fieldbook; a projection consumes published records.
- Nirnay does not ingest raw source files. Raw bytes, provider files and source
  decoders stay in the producer.
- Nirnay does not allocate canonical IDs. Local prefix-and-sequence allocation
  is frozen and consumed IDs come from the release.
- Nirnay does not maintain a competing raw archive. Archive references and
  digests are carried as lineage, not rebuilt from local bytes.
- `india-reference` is legacy frozen. It is not a second archive, an identity
  authority or a source to extend.
- `india-domain` may remain only as a trading projection that consumes
  Fieldbook stable IDs. Its local identity allocation and self-contained
  provenance helper are migration items.
- No NDC JSON schema, transport, storage engine or shared crate is selected by
  this design. No code is transferred between repositories.

## 5. Projection mapping of existing constructs

The four constructs below are described as replacements to be built from an
admitted release. Their present shapes stay in place for wire compatibility and
are frozen against new authority. Finding numbers refer to the D1 crosswalk.

| Current construct | Defect in the crosswalk | Projection rule | Rejection condition |
|---|---|---|---|
| `IdentityRegistry` (`india-reference/src/identity.rs`) | F2: `(Venue, symbol)` key with no effective interval; local ID allocation | key by release-supplied canonical `InstrumentId`; venue identifiers with valid and known intervals; symbol text is an attribute | symbol-only lookup, absent validity, duplicate owner, locally minted ID |
| `ProviderListingHistory` (`india-domain/src/reference_history.rs`) | F3: bitemporal shape is sound, but `instrument_id` is locally allocated and provider identity is not release-supplied | retain the valid/known query surface; canonical side and provider/feed identity from the release | overlapping valid or known intervals, provider mapping conflict, local canonical ID |
| `CorporateActionLedger` (`india-domain/src/corporate_actions.rs`) | F4, F10: no `supersedes` chain; `append` rejects a corrected slot; embedded array is a snapshot | consume release revisions with `wasRevisionOf`, knowledge time and a new release id per correction; retain prior view | invalid correction chain, missing revision ID, locally corrected action |
| `TradingCalendar` (`india-domain/src/lib.rs`) | F7: date-only `jiff::civil::Date`, no session intervals; `covers_month` needs an entry per date | consume effective-dated session and calendar assertions with timezone-aware intervals | date-only entry offered as session coverage; no release calendar artifact |
| `provenance_for_source` (`india-domain/src/provenance.rs`) | F5: physical `String::lines()` count, not a logical record count | denominators from the release manifest per coverage dimension | local record count offered as a coverage denominator |
| `InstrumentType` / `Segment` / `Venue` (`india-domain/src/instrument.rs`) | F8, F9, F11: memory-only variants, cash-only segment, closed venue enum; `deny_unknown_fields` | v2 wire surface stays Equity/Etf and cash; new families need a versioned Fieldbook contract and paired validators | derivative variant presented under v2, widened enum or schema |

### 5.1 IdentityRegistry as a read-only identity projection

The registry currently mints `{prefix}{sequence:08x}` IDs and resolves by
`(Venue, String)` symbol with no validity parameter. A reuse or change of a
symbol cannot be represented as a separate interval, and the minted ID is a
competing canonical identity. In the projection the canonical ID is copied from
the admitted release. Venue symbols and tokens are effective-dated attributes
with knowledge time and provenance from the release. Symbol text is not a
lookup key and does not create an identity. A conflicting mapping or an
unbounded effective interval blocks the projection.

### 5.2 ProviderListingHistory as a mapping projection

The bitemporal query shape is retained: resolution on both `valid_on` and
`known_at`, with ambiguity rejected. The canonical `instrument_id` and the
provider and feed identity are copied from the release rather than allocated or
read from Nirnay configuration. Two mappings covering the same provider key,
venue and overlapping effective or knowledge interval are a conflict and block
the projection. The projection does not reconstruct a provider identity from a
symbol or token.

### 5.3 CorporateActionLedger as a revision projection

The present ledger rejects a second record with the same
`(instrument_id, effective_date, kind)`, so a corrected action cannot replace
its predecessor, and `known_as_of` has no invalidation basis. In the projection
the release owns revisions and supersession: each correction is a new revision
with its own release identity, and the earlier view and knowledge time remain
available. Selection uses as-of and knowledge time. An action without a
Fieldbook revision ID, a broken `wasRevisionOf` chain or a locally corrected
value blocks the projection. The embedded corporate-action array stays frozen
for v2 compatibility.

### 5.4 TradingCalendar as a session projection

The present calendar stores whole dates and derives trading-day answers from
date entries; it has no session start, end or timezone-aware interval. Session
coverage for research requires a release calendar artifact with effective-dated
intervals and its own denominator. Nirnay consumes those intervals and does not
infer session times from a sparse holiday file. A date-only calendar presented
as session coverage, or a calendar without a declared denominator, blocks the
projection.

## 6. Derived research dataset chain

```text
Fieldbook NDC release (release_id, manifest_sha256, data_sha256)
  -> admitted dataset (admission receipt id, use profile, as_of_ns)
    -> research dataset (dataset_id, manifest_sha256, data_sha256)
      -> experiment (research dataset digests + exact NDC release id/digests)
```

Every link carries the exact `release_id`, `manifest_sha256` and
`data_sha256` of the Fieldbook release being consumed. The research dataset is
a separate object with its own manifest identity and digests. An experiment
references both the exact release identity and the research dataset digests; a
reference to one without the other does not close the chain. A change to a
release or to rights evidence requires a new admission decision. Nirnay stores
a derived projection, performs no re-normalization of raw Fieldbook payloads
and keeps no competing raw archive. This design creates no release, dataset or
experiment.

## 7. Non-claims and current state

- No provider contact, market-data API call, download, scrape or collection
  command is authorized or performed.
- No right is granted. All source rights remain `UNKNOWN`; the stated budget is
  INR 0. A signature, a checksum or a public endpoint grants nothing.
- No dataset is accepted. Dataset acceptance stays `BLOCKED` while rights
  evidence is absent.
- No experiment is registered. Synthetic market-shaped tests stay disabled;
  only abstract scalar checks are in scope.
- No live execution is enabled. No code or CI task in this design authorizes
  order routing.
- No code, data, schema, capability, claim or experiment record changes. No
  commit is made in any repository.
- Signature integrity is not source authenticity; provenance is not truth; a
  capability claim is not a permission.
- This design does not ratify an ADR, finalize an NDC schema, select a
  transport or select a storage engine.
