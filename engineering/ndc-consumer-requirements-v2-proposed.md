# NDC v2 consumer requirements (Nirnay side) — Proposed

<!-- SPDX-License-Identifier: Apache-2.0 -->

- **Status:** Proposed. This document supersedes the operation mapping in
  `engineering/ndc-consumer-requirements-v1.md`. The v1 document keeps its
  path, content and historical digest; nothing in it is edited or re-digested.
  Supersession changes which mapping guides new work, not the history of
  earlier decisions. Nothing here is ratified, is a data licence, or
  authorizes provider contact, collection, dataset acceptance or an experiment.
- **Version:** `ndc-consumer-requirements/v2-proposed`. A semantic change
  requires a new version and a migration record; a field rename is breaking.
- **Governing source:** `NDC-SEMANTIC-MODEL-V2-PROPOSED.md` (C0, Proposed),
  sections 1, 2, 4, 4.1, 5, 6, 7 and 9. C0 is not a final NDC schema.
- **Alignment sources:** `engineering/ndc-consumer-use-v2-proposed.md` and
  `research/python/src/quantlab/consumer_use.py` (C3 declaration-level model),
  `evidence/C3-CONSUMER-USE-PERMISSIONS-2026-11-23.md`, and the read-only
  Fieldbook `engineering/ndc-producer-requirements-v1.md`. Alignment is
  semantic; a producer declaration grants Nirnay no right.
- **Owner and boundary:** Nirnay consumes immutable Fieldbook releases by
  release identity and digest. Nirnay acquires no source data, holds no
  competing raw archive and re-normalizes no raw payload.
- **Scope:** the consumer admission request, use profiles, stage model,
  rejection codes, proof semantics, coverage denominators, time semantics and
  conformance expectations of the proposed NDC v2 model.
- **Out of scope:** a final NDC JSON schema, transport, storage engine, shared
  crate, release publication and data creation. Also out of scope: provider
  contact, market-data collection, dataset acceptance and experiment state.

## 1. Six separable concepts

C0 keeps capability, permission, operation and data object as distinct core
concepts, with purpose and evidence as further independent dimensions. All six
stay separate in a consumer request; none may be inferred from another.

| Concept | Question | Consumer example |
|---|---|---|
| Capability | What can a source technically provide? | intraday or five-level depth claim; not a permission |
| Permission | What does the written agreement allow? | non-display research storage for a bounded term |
| Operation | What does the software perform? | `read_release`, `replay`, `create_derived` |
| Data object | What does the operation act upon? | release manifest, research copy, derived artifact |
| Purpose | Why is it performed? | `proprietary_own_account_trading_research` |
| Evidence | What proves the action or decision? | admission decision, written-record reference, signature receipt |

A capability claim does not create a grant. A grant does not create a capability
claim. An operation on a raw frame is not the same operation on a research
copy. A signature receipt is not a licence, a coverage verdict, a rights grant
or proof of a source observation. No generic default supplies a purpose,
licensee, recipient, territory or scope. The declared purpose is exactly
`proprietary_own_account_trading_research`; `academic_research`,
`non_commercial_research`, `internal_analysis` and `historical_archive` are not
substitutes.

## 2. AdmissionRequest and use profiles

### 2.1 AdmissionRequest

Nirnay submits an admission request; it does not inherit producer operations
and does not reconstruct the request from a proof. The request binds:

| Field | Required content |
|---|---|
| `release_identity` | `release_id`, `manifest_sha256`, `data_sha256`, `publisher_identity` of the immutable Fieldbook release |
| `actor` | named consumer actor, equal on each matching grant |
| `use_profile` | one versioned profile identifier from section 2.2 |
| `requested_actions` | explicit actions, each a member of the declared profile |
| `requested_objects` | object class for each requested action |
| `purpose` | exact reviewed value `proprietary_own_account_trading_research` |
| `licensee`, `recipient` | named legal entity and exact recipient class from the written permission |
| `scope` | provider, product, venue, segment; named instrument, field and modality sets; data-time window |
| `requested_retention` | bounded maximum duration and explicit purge rule |
| `as_of` | point-in-time instant; the implementation carries `as_of_ns` as positive epoch nanoseconds |
| `declared_capabilities` | capability claims only; these are not permissions |

`NOT_APPLICABLE`, `NAMED` and `ALL` are distinct scope states. A runtime request
uses named, non-empty subject sets; an empty `NAMED` set is not `ALL`. Time
containment uses mathematical infinity: a bounded grant cannot cover an
unbounded request, and `(None, None)` as a request requires an unbounded grant.
Data time and operational time are independent intervals, each with
`from <= to`. Retention is a typed policy with a maximum duration and a purge
rule; a narrower request may be permitted, and an indefinite request cannot
exceed a bounded grant. Every applicable dimension must be contained by the
grant. Ambiguous or conflicting grants deny.

### 2.2 Versioned use profiles and exact operation mapping

A job declares its profile and cannot omit a required action. The C0 section 4
profile table and its exact `CONSUMER_OPERATIONS` mapping are authoritative:

| Profile | Required consumer actions (C0 wording) | Required actions (`CONSUMER_OPERATIONS`) |
|---|---|---|
| `reference_lookup` | verify release, read manifest, read event payloads for reference fields | `read_release` |
| `historical_research` | verify, read, store research copy, replay, analyze | `read_release`, `store_permitted_research_copy`, `replay`, `internal_analytics` |
| `intraday_replay_research` | verify, read, store, replay, analyze with intraday capability | same four actions plus an `intraday_capability` claim |
| `derived_research_dataset` | verify, read, store, analyze, create derived, retain derived | `read_release`, `store_permitted_research_copy`, `internal_analytics`, `retain_permitted_derived_data`, plus separate `create_derived` |
| `portfolio_simulation` | verify, read, replay, analyze within an admitted dataset | `read_release`, `replay`, `internal_analytics` |

The exact phrase-to-value mapping is:

| Descriptive phrase | Nirnay `CONSUMER_OPERATIONS` value |
|---|---|
| verify release / read manifest | `read_release` |
| read event payloads | covered by `read_release`; payload access is a separate stage in section 3, not a new right |
| store research copy | `store_permitted_research_copy` |
| replay | `replay` |
| analyze | `internal_analytics` |
| create derived / retain derived | `retain_permitted_derived_data` for retention; creation requires an explicit grant and is not implied by retention |

`CONSUMER_OPERATIONS` stays the authoritative five-value enumeration:
`read_release`, `store_permitted_research_copy`, `replay`,
`internal_analytics`, `retain_permitted_derived_data`. This document invents no
enum value. Creation of derived data is a separate requested action
(`create_derived`) that must appear in the use request and in a matching grant;
it is not a value of `CONSUMER_OPERATIONS` and retention alone does not imply
it. An extra grant never authorizes an unrequested action. An unpermitted
requested action blocks admission.

## 3. Consumer admission stages and typed refusals

Admission is ordered and stops at the first refusal. A later check does not
soften an earlier failure. The 12 stages follow C0 section 4.1:

| # | Stage | Declaration-level check today | Typed refusal |
|---|---|---|---|
| 1 | Parse manifest; validate schema and contract version | positive integer schema and envelope versions | `REJECT_SCHEMA_ENVELOPE` |
| 2 | Verify canonical manifest bytes | named signature receipt and manifest digest binding; canonical bytes later | `REJECT_SIGNATURE_INTEGRITY` |
| 3 | Verify detached signature with an out-of-band trusted key | presence of a named signature receipt; signature verification later | `REJECT_SIGNATURE_INTEGRITY` |
| 4 | Verify publisher key validity and revocation | publisher identity named and equal; key registry check later | `REJECT_PUBLISHER_IDENTITY`, `REJECT_SIGNATURE_INTEGRITY` |
| 5 | Verify data and artifact digests | digest presence and shape; byte comparison later | `REJECT_ARTIFACT_DIGESTS`, `REJECT_SIGNATURE_INTEGRITY` |
| 6 | Verify rights evidence and scope | named written evidence and grant identity fields | `REJECT_RIGHTS_EVIDENCE`, `REJECT_USE_SCOPE` |
| 7 | Verify coverage and denominators | named passing coverage certificate | `REJECT_COVERAGE` |
| 8 | Verify producer activity and operation authorization | producer operations are declarations; consumer grants evaluated separately | `REJECT_RIGHTS_EVIDENCE` |
| 9 | Verify event envelopes | envelope version declaration; no envelope inspection here | `REJECT_SCHEMA_ENVELOPE` |
| 10 | Verify temporal and correction lineage | source lineage reference and PIT basis and as-of instant | `REJECT_SOURCE_LINEAGE`, `REJECT_PIT_CONSISTENCY` |
| 11 | Evaluate requested consumer use | profile coverage, scope, retention and a unique matching grant | `REJECT_USE_SCOPE`, `REJECT_RETENTION`, `REJECT_AMBIGUOUS_GRANTS` |
| 12 | Issue an immutable admission receipt | scalar decision with a typed code on every refusal; receipt later | typed code on failure |

The current `research/python/src/quantlab/dataset_admission.py` is a declaration
validator, **not** a full release verifier, and must not be described as one.
The C3 evaluator reads named declarations and grants: it verifies no signature,
recomputes no canonical bytes, compares no digest with file bytes and inspects
no event envelope. Full detached-signature verification, trusted-key validity
and revocation checks, canonical-byte and digest recomputation, and
event-envelope verification come later. Stage codes are declaration-level
classifications today; a code does not claim that the matching cryptographic or
byte-level check ran.

### 3.1 Typed rejection codes

| Code | Declaration-level failure |
|---|---|
| `REJECT_PUBLISHER_IDENTITY` | publisher identity absent or different |
| `REJECT_SIGNATURE_INTEGRITY` | no named signature receipt or manifest digest binding |
| `REJECT_SOURCE_LINEAGE` | source lineage reference absent |
| `REJECT_RIGHTS_EVIDENCE` | evidence absent, no grant, denied grant or identity mismatch |
| `REJECT_USE_SCOPE` | profile coverage, purpose, recipient, scope or object failure |
| `REJECT_COVERAGE` | no passing named coverage certificate |
| `REJECT_PIT_CONSISTENCY` | as-of basis or instant differs |
| `REJECT_RETENTION` | unbounded or uncovered retention |
| `REJECT_ARTIFACT_DIGESTS` | artifact digests absent or malformed |
| `REJECT_SCHEMA_ENVELOPE` | schema or envelope version is not a positive integer |
| `REJECT_AMBIGUOUS_GRANTS` | duplicate, conflicting or non-unique grants |

Each requested action requires one matching `PERMIT` grant covering actor,
purpose, licensee, recipient, action, object, scope, retention and effective
interval. A missing grant, a denial or an identity mismatch blocks admission.

## 4. Three independent proofs

| Proof | Establishes | Does not establish |
|---|---|---|
| Publisher integrity | publisher-key control and canonical manifest integrity for the exact signed bytes | source authenticity, source completeness, coverage, a rights grant or a research entitlement |
| Source lineage | raw archive reference, digests and transformation record | truth of the observations |
| Rights evidence | reviewed written agreement, exact scope and document digest | permission for an action not listed in the agreement |

The publication key and the rights-review authority are separate trust
relationships. A verified signature does not substitute for lineage or rights
evidence, and a rights record does not substitute for signature verification.

### 4.1 RFC 8785 canonical bytes

Manifest signing uses a detached signature: the signature is a sidecar, not a
property inside the manifest. RFC 8785 JCS is applied to the manifest object
itself; no property removal is needed because no signature property exists in
that object. The rules applied here are:

- `I-JSON` subset; no duplicate property names; Unicode strings preserved as-is.
- No whitespace between tokens; recursive property-name sorting by UTF-16 code
  units.
- Array order preserved; semantically unordered sets require a defined
  deterministic ordering before canonicalization.
- UTF-8 output; exact financial decimals and large integers are JSON strings,
  not IEEE 754 numbers.
- NaN, Infinity and lone surrogates terminate the operation.
- The detached signature sidecar is outside the canonicalized bytes and does
  not enter the manifest digest.

A verified signature proves publisher-key control and manifest integrity for
the exact bytes. It does not prove source authenticity, source completeness,
coverage, a rights grant or a research entitlement. Data content is covered by
a separate data digest, and rights evidence is evaluated independently.

## 5. Per-dimension coverage denominators

One count cannot establish all coverage. Each dimension has its own universe
and denominator, as declared in C0 section 6:

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

Passing one dimension implies no other. Nirnay refuses a release when an
applicable dimension lacks a declared universe, scope, as-of instant or
denominator, when a denominator disagrees with the manifest, or when one count
is offered for several dimensions. Gaps and rejects are declared against their
dimension; Nirnay does not fill or interpolate them, and does not consume a
partially valid release. The typed refusal is `REJECT_COVERAGE`.

## 6. Time semantics, revision lineage and derived dataset chain

| Time | Definition and consumer rule |
|---|---|
| Source event time | when the source says the observation occurred; original resolution and unit preserved |
| Receive time | when the producer received the source bytes, with its own clock domain and identity |
| Valid time | business effective time of a fact, including corporate-action effective dates |
| Knowledge time | when the fact became known to the producer; the point-in-time selection basis |
| Availability time | when the record became available in the release, at or after knowledge time when both are declared |

Each time has one owner and clock domain; a unit change is not added precision.
The admission basis is `knowledge_time` at one `as_of` instant. A knowledge
time after the as-of instant is a `REJECT_PIT_CONSISTENCY` refusal. One time
field does not substitute for another to repair a scope or rights failure.

Corporate actions carry announcement, record, ex and effective dates plus a
revision knowledge time, each with its own knowledge time. A correction creates
a new revision and, where published, a new release identity; the earlier view
and its knowledge time are retained. Selection uses the as-of instant and
knowledge time, not arrival order. PROV-O vocabulary describes this lineage;
it does not establish permission or truth.

The derived research dataset chain follows C0 section 7:

```text
Fieldbook NDC release (release_id, manifest_sha256, data_sha256)
  -> Nirnay admitted dataset (admission receipt id, use profile, as_of)
    -> Nirnay research dataset (research dataset id + digests)
      -> experiment (references research dataset and exact NDC release)
```

Every link carries the exact release identity and digests of the Fieldbook
release being consumed; the research dataset also records its own manifest
identity and digests as a separate object. Nirnay stores a derived projection,
performs no re-normalization of raw Fieldbook payloads and keeps no competing
raw archive. A change to the release or to its rights evidence requires a new
admission decision. This document creates no release and no dataset.

## 7. Consumer conformance matrix

| Area | Required consumer behavior | Position today | Later work |
|---|---|---|---|
| C0 section 1 concepts | capability, permission, operation, data object, purpose and evidence bound separately in the request | declaration-level separation in `AdmissionRequest` and grants | none for this document |
| C0 section 4 profiles | five profiles, exact `CONSUMER_OPERATIONS` mapping, explicit `create_derived` grant | declared in `USE_PROFILES` | runtime job profile declaration |
| C0 section 4.1 stages | 12 ordered stages, first refusal stops admission | declaration checks and scalar decision | signature, digest and envelope verification |
| C0 section 5 proofs | three independent proofs with stated non-claims | named receipts only | JCS recomputation and key checks |
| C0 section 6 coverage | per-dimension denominators; incomplete denominator refused | named passing certificate | denominator verification against bytes |
| C0 section 7 chain | exact release identity and digests on every chain link | admission decision binds the release identity | receipt issuance |
| Time semantics | `knowledge_time` basis at one as-of instant | PIT consistency of declarations | correction-lineage verification |
| Extra grants | an extra grant never authorizes an unrequested action | grants matched by requested action | none for this document |
| Technology | no schema, transport, storage or shared crate selected | not selected | separate owner decision |

A grant for an action outside the admission request has no effect on the
decision. No consumer operation is inferred from another consumer operation,
from a producer operation, from an extra entry in a rights record or from the
existence of a release. A rejected release is not partially consumed.

## 8. Change control and non-goals

This document changes no right, capability record, claim record, experiment
state, CLI path or source-product rights value. All source rights remain
`UNKNOWN`; dataset acceptance stays BLOCKED while that evidence is absent. No
provider was contacted, no market data was collected, no release was published
and no data object was created by this documentation change.

No final NDC JSON schema, transport, storage engine, shared crate, release or
data is created or selected by this document. A ratified schema, a transport
choice and a storage choice each need a measured requirement and a separate
owner decision. The v1 documents and the Fieldbook producer requirements keep
their own status and digests.
