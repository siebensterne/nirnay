# NDC v1 consumer requirements (Nirnay side) — Proposed

<!-- SPDX-License-Identifier: Apache-2.0 -->

- **Status:** Proposed engineering requirements. Not ratified, not accepted and
  not an authorization for provider contact, collection, dataset acceptance or
  an experiment.
- **Version:** `ndc-consumer-requirements/v1`. A semantic change requires a new
  version and a migration record; a field rename is a breaking change.
- **Governing decision:**
  `engineering/architecture/decisions/ADR-0001-data-ownership.md`. That ADR
  remains **Proposed design decision**; this document does not ratify it.
- **Alignment sources:** Fieldbook's Proposed producer requirements
  (`engineering/ndc-producer-requirements-v1.md`) and its W1.1-C rights
  enforcement record (`docs/rights-enforcement-2026-11-23.md`). Alignment is
  semantic only: neither source is ratified here and no producer declaration
  grants a right to Nirnay.
- **Owner:** Nirnay consumes. The complementary platform publishes and versions
  the release. Nirnay does not acquire source data, hold a competing raw archive
  or re-normalize raw payloads.
- **Scope:** the semantic fields Nirnay requires on a release and on each event
  envelope, its consumer-operation vocabulary, the trust model and time
  semantics, plus consumer validation and rejection behavior.
- **Out of scope:** transport, RPC, serialization framing, storage engine,
  queue, database, credentials, provider endpoints and technology selection.
  Those require a measured requirement and a separate decision.

## 1. Objects, boundary and release declarations

### 1.1 Object chain and artifact boundary

Three distinct objects are in scope. They are not interchangeable and they are
not merged into one artifact.

| Object | Owner | Identity and schema | Rule |
|---|---|---|---|
| Fieldbook NDC source release | Fieldbook | NDC release manifest and event envelopes | Immutable producer artifact; owns source provenance and its publication signature |
| Nirnay accepted research dataset | Nirnay | `schemas/research/dataset-manifest.v1.schema.json` | Built only from an admitted release; separate manifest, separate digest |
| Experiment record | Nirnay | `schemas/research/experiment-record.v1.schema.json` | References both the NDC source release identity and the accepted research dataset manifest digest |

The term "release manifest" in this document means the Fieldbook NDC source
release manifest. The Nirnay research dataset manifest is a different object
with its own schema and digest; it is not an NDC source release. An experiment
references both identities, so the source release and the accepted dataset stay
independently verifiable.

Nirnay does not re-normalize raw Fieldbook payloads and keeps no competing raw
archive. It reads the published release, validates it, and derives one accepted
research dataset. A raw byte digest and archive reference on an event identify
the producer-held source bytes without copying or rewriting them. A derived
value is not substituted for a raw observation.

### 1.2 NDC source release manifest

The release manifest is the immutable producer object an admission decision
references. It is versioned with the release and is the input to the
dataset-admission gate in `research/python/src/quantlab/dataset_admission.py`.

| Group | Required fields | Consumer meaning |
|---|---|---|
| Dataset identity and source scope | `dataset_id`, `dataset_version`, `dataset_origin`, `source_scope` (`provider`, `product`, `venue`, `segment`) | Named release identity; source and venue are declared, not inferred from a symbol |
| Rights | `rights.record_reference`, `rights.permission_source`, `rights.operations`, `rights.scope`, `rights.review_date`, `rights.valid_until`, `rights.retention_period` | Written permission reference and exact operations; review date and optional validity bound must not precede the as-of UTC date; registry `UNKNOWN` is not permission |
| Rights evidence | the fields of section 4 | Full typed evidence for the permission decision; a partial or placeholder record is a refusal |
| Coverage and completeness | `coverage.certificate_reference`, `coverage.valid_until`, `coverage.verdict`, `coverage.completeness`, `coverage.scope`, `coverage.universe_id`, `coverage.as_of_ns` | Passing, unexpired certification over the declared scope and as-of instant; gaps and rejects are declared, not invented |
| Coverage denominator | `coverage.denominator` and `coverage.universe_id` | Named universe and the explicit basis of the completeness claim (instrument and event counts at `coverage.as_of_ns`); completeness is measured against that denominator |
| Production operations | `release.producing_operations` | Typed producer `Operation` values that produced the release; declares acquisition and processing acts, not consumer entitlements |
| Schema versions | contract schema version, normalizer version, payload schema version per event type | Version identity for each transformation applied before publication |
| Retention | `rights.retention_period`, archive closure and deletion basis | Bounded ISO-8601 duration or positive integer days; a missing or open-ended term is a denial |
| Digest | `release.manifest_sha256`, `release.data_sha256`, `release.digest_algorithm` | Immutable digests of the canonical release manifest and canonical dataset content |
| Publication identity | `release.release_id`, `release.publication_identity`, `release.publication_method`, `release.archive_reference` | Immutable release identity and the named publisher of that identity |
| Publication signature | the detached object of section 5 | Verifiable publisher-key control and manifest integrity; not a rights grant |
| Point-in-time lineage | `lineage.basis`, `lineage.universe_version`, `lineage.universe_source_reference`, `lineage.instrument_identity_source_reference`, `lineage.corporate_action_source_reference`, matching `*_sha256` fields, `lineage.as_of_ns`, `lineage.knowledge_time_ns` | Named universe, instrument-identity and corporate-action provenance with digests and one as-of basis |

The coverage verdict is a claim over the declared denominator. A release that
omits the denominator, or whose completeness, scope, universe or as-of instant
disagrees with the manifest, is inconsistent. Nirnay rejects an inconsistent or
partially valid release; it does not consume the valid part of one.

### 1.3 Event envelope

The envelope carries only facts tied to one observation. It references the
release manifest for dataset-wide rights, coverage, retention and publication
identity.

| Group | Required fields | Consumer meaning |
|---|---|---|
| Source and instrument identity | provider key, venue, segment, product, canonical instrument id, valid and known intervals | The producer that emitted the event and the canonical identity it maps to |
| Typed payload | event type, typed payload, units, scale, missing-value semantics | Defined payload per event type; null is not zero and a snapshot is not a tape |
| Source and receive timestamps | source timestamp, source timestamp unit, receive timestamp, clock domain and clock identity | Original resolution is preserved; a unit change is not added precision |
| Sequence and continuity | provider sequence where supplied, local sequence, partition scope, gap status | Continuity is scoped to a named producer and partition; a local gap is not a provider continuity claim |
| Raw digest and archive reference | raw byte digest, immutable archive object reference | Link to the exact source bytes without re-reading or re-normalizing them |
| Correction and as-of lineage | revision, supersession reference, knowledge time, availability time, as-of basis | Select the view known at the as-of instant; a subsequent correction does not rewrite that view |
| Schema versions | contract schema version, normalizer version, payload schema version | Version identity for the record and its payload |

## 2. Consumer operations and the admission decision

Nirnay decides rights for the operation it is about to perform. The decision is
not taken from a producer label, a grant for a different operation, arrival
order, or the existence of a release. Each operation is evaluated against its
own declaration in the written record and its own effective interval.

| Consumer operation | Operation Nirnay is about to perform |
|---|---|
| `read_release` | Read and validate the immutable NDC release manifest, its signature and its event envelopes to decide whether the release may be admitted |
| `store_permitted_research_copy` | Persist the Nirnay accepted research dataset derived from an admitted release, under a bounded retention term, in Nirnay's own store |
| `replay` | Replay the admitted, accepted research dataset through Nirnay research code |
| `internal_analytics` | Compute research statistics and features from the admitted dataset inside Nirnay |
| `retain_permitted_derived_data` | Keep derived research artifacts produced from the admitted dataset inside the declared retention term |

These names are consumer activities. They are distinct from Fieldbook's
producer operations. The consumer set need not equal the producer set, and a
producer operation is not a consumer entitlement. A grant for another operation
does not authorise an unapproved activity: no consumer operation is inferred from
another consumer operation, from a producer operation, or from an extra entry
in a rights record. The admission gate requires the exact five names of this
section, each `PERMITTED` and citing the same written record; an extra operation
is a mismatch, not an added entitlement.

For each operation the decision input is: the rights evidence of section 4 for
that operation, a `PERMITTED` state with the same written reference, an
effective interval containing the operation instant, a verified publication
signature under section 5, a passing coverage certificate, matching digests,
consistent scope and a bounded retention term. A missing or denied input refuses
that operation. No operation is authorised by a whole-release verdict.

## 3. Canonical rights vocabulary and producer mapping

Nirnay declares five independent dimensions. Each maps to the typed vocabulary
used by Fieldbook's W1.1-C rights implementation. The mapping aligns evidence;
it does not transfer a right or widen the declared purpose.

| Dimension | Canonical value Nirnay declares | Fieldbook typed type | Producer value | Rule |
|---|---|---|---|---|
| operation | `read_release` | `Operation` | no source-data operation | Consumer-side validation of a published release; no acquisition act is performed by Nirnay, so no producer grant is inferred |
| operation | `store_permitted_research_copy` | `Operation` | `retention` | The stored object is the accepted research dataset, not a raw archive; producer `raw_storage` and `backup` stay with the producer |
| operation | `replay` | `Operation` | `replay` | Exact value match |
| operation | `internal_analytics` | `Operation` | `internal_analytics` | Exact value match |
| operation | `retain_permitted_derived_data` | `Operation` | `derived_data_creation`, `retention` | Both producer values must appear in `permitted_operations` |
| usage mode | `non_display` | `UsageMode` | `non_display` | Research use is not display; a display-only grant does not cover replay or analytics |
| purpose | `proprietary own-account trading research` | `Purpose` | `internal_analysis` | The written record must state proprietary own-account trading research explicitly; `internal_analysis` is the nearest producer value and does not widen it |
| recipient | `internal` | `Recipient` | `internal` | Nirnay research use only; `public`, `participant`, `customer` and `affiliate` do not cover it |
| licensee | named Nirnay legal entity | `Licensee` | non-blank declared name | The name belongs to the written record, not to a command, process or operator flag |

The producer `Operation` enum is the 14-value lifecycle vocabulary
`source_contact`, `reference_data_retrieval`, `historical_data_retrieval`,
`real_time_capture`, `raw_storage`, `normalization`, `replay`, `backup`,
`internal_analytics`, `derived_data_creation`, `display`,
`external_redistribution`, `retention` and `deletion`. Values with no mapping in
the table are producer acts, not activities Nirnay performs on a release. Their
presence in rights evidence neither adds a consumer operation nor removes the
requirement for the exact five.

A research use is not labelled academic, non-commercial, personal or an
unqualified `research` purpose. A record carrying one of those purpose labels
is refused for a proprietary own-account trading research operation. Verdict
vocabulary stays fail-closed: `UNKNOWN`, `PENDING`, `FAILED`, `BLOCKED`,
`PROHIBITED`, `DENIED`, `EXPIRED`, `REVOKED` and `NO` are denials.

## 4. Rights evidence fields

The written permission record published with the release carries every field
below with a declared type. A field that is absent, empty, a placeholder or a
denial refuses the operation that depends on it.

| Field | Consumer meaning |
|---|---|
| `permission_id` | Unique identifier of the written permission decision |
| `provider` | Source provider; equal to `dataset.source_scope.provider` |
| `source_product` | Source product; equal to `dataset.source_scope.product` |
| `venue` | Venue; equal to `dataset.source_scope.venue` |
| `segment` | Segment; equal to `dataset.source_scope.segment` |
| `instrument_scope` | Named instruments or scope set; the dataset instruments must be inside it |
| `field_modality_scope` | Named fields and modalities; the event payload fields and modalities must be inside it |
| `date_range` | Source event-time window; source timestamps of used observations must fall inside it |
| `territory` | Territory of use; an absent territory or a wildcard is a scope failure |
| `licensee` | Named legal entity holding the licence; non-blank and stable across revisions |
| `recipient` | Recipient class; `internal` for every consumer operation |
| `purpose` | Explicit proprietary own-account trading research |
| `permitted_operations` | Typed producer `Operation` values the record grants; must cover each mapped value of section 3, and extra values are not consumer entitlements |
| `retention` | Bounded ISO-8601 date duration or positive integer days |
| `effective_interval` | Interval containing the instant of the operation being decided |
| `document_reference` | Immutable reference to the written record; shared by all five operations |
| `document_sha256` | SHA-256 digest of the written record bytes |
| `contract_revision` | Revision identity of the rights contract and its decision lineage |
| `review_date` | Dated review; not before the as-of UTC date |
| `recording_authority` | Named authority that recorded the decision in the written record |

`document_reference` and `document_sha256` bind the decision to exact bytes.
`contract_revision`, `review_date` and `recording_authority` make the decision
auditable without contact with a provider. Rights evidence is independent of
the publication signature: a verified signature does not substitute for any
field in this section.

## 5. Trust model and detached publication signature

The publication signature reuses `schemas/research/dataset-signature.v1.schema.json`
without extending it. The schema is a detached object with `schema_version`
equal to 1, `algorithm` equal to `Ed25519`, a non-empty `key_id`, a lowercase
64-hex `manifest_sha256` and a base64 `signature_b64` holding one 64-byte
Ed25519 signature.

| Term | Definition |
|---|---|
| Producer identity | Fieldbook as the named `release.publication_identity`; naming the publisher does not assert source authenticity |
| Key id | Stable identifier of the signing key in Nirnay's explicit trusted-public-key map; a key outside that map is unknown |
| Canonical manifest digest | `manifest_sha256`: SHA-256 of the canonical NDC release manifest bytes, recomputed by the consumer |
| Signature | Detached Ed25519 signature over the domain-separated digest bytes, verified with the trusted public key named by `key_id` |
| Trusted public key | 32-byte Ed25519 public key pinned out of band under `key_id`; a key supplied inside the release is not trusted |
| Key validity and revocation | Each trusted key carries a validity interval and revocation state; an expired, revoked or unknown key refuses verification |
| Verification result | A named result for the exact release: verified when digest, key and signature agree; otherwise a typed refusal and no consumption |

The signed message is the fixed domain-separation constant followed by the
32 raw digest bytes, not the manifest text. Nirnay verifies over hashed bytes:
it recomputes the canonical manifest digest and checks both that digest against
the declaration and the Ed25519 signature against the pinned key. Canonical
bytes use sorted JSON object keys, UTF-8 text, one number encoding and no
insignificant whitespace. Arrays that are semantically sets, including artifact
digests, provider lists and hash lists, are placed in deterministic sorted order
before hashing. The detached signature object and any self-referential digest
field are excluded from the hashed manifest, so the digest can be recomputed
without circularity; the signature lives outside those bytes. Dataset content
is covered separately by `release.data_sha256` and the artifact digests.

A verified signature proves publisher-key control and manifest integrity for
the exact bytes. It does not prove source authenticity, source completeness,
coverage, a rights grant or a research entitlement. Rights evidence of section
4 is evaluated independently, and a failed signature stops admission even when
the rights record is complete. An unknown, expired or revoked key, a malformed
signature object, or a digest mismatch is a `REJECT_SIGNATURE` refusal. A signed
manifest does not substitute for rights evidence.

## 6. Time semantics

Each time field has one owner and one clock domain. A consumer does not convert
a resolution change into added precision.

| Time | Owner | Definition and consumer rule |
|---|---|---|
| Source event time and original resolution | Producer, from source | When the source says the observation occurred; the source timestamp unit and original resolution are preserved |
| Receive time | Producer | When the producer received the source bytes; recorded with its clock domain and identity, distinct from event time |
| Valid time | Producer | Business effective time of a fact, including corporate-action effective dates; used for the business view, not for knowledge selection |
| Knowledge time | Producer | When the fact was known to the producer; the as-of basis for point-in-time selection |
| Availability time | Producer | When the record became available in the release; at or after knowledge time when both are declared |
| Corporate action times | Producer | Announcement, ex-date, record date and effective date are declared separately; none is inferred from another |
| Revision lineage | Producer | Each correction carries a revision id, a supersession reference and ordered lineage; a subsequent correction does not rewrite the earlier view |

The admission decision uses `as_of_basis` equal to `knowledge_time` and the
single `dataset.as_of_ns` instant. A view is selected by as-of instant and
knowledge time, not by arrival order. Knowledge time after the as-of instant is
a `REJECT_ASOF` refusal. Source event time, valid time, receive time
and availability time are kept as declared facts; a consumer does not replace
one with another to repair a scope or rights failure.

## 7. Consumer validation

Validation is ordered and stops at the first refusal. A following check is not
run to soften an earlier failure.

1. **Contract version.** Require an exact match on `contract_version`. An
   unknown major version is a rejection, not a migration attempt.
2. **Required fields.** Require every field in sections 1.2 and 1.3 with the
   declared type, including the rights evidence of section 4, the production
   operations, the coverage denominator and the signature object. Reject
   missing, empty, redirected or placeholder references.
3. **Signature and key.** Verify the detached signature under section 5 against
   a trusted, valid, non-revoked key and the recomputed canonical manifest
   digest. Refuse an unknown key or a digest, key or signature mismatch.
4. **Verdicts.** Require `PERMITTED` for each listed operation, `PASS` and
   `COMPLETE` for coverage, and an immutable release flag. `UNKNOWN`,
   `PENDING`, `FAILED`, `BLOCKED`, `PROHIBITED`, `DENIED`, `EXPIRED`, `REVOKED`
   and `NO` are denials in verdicts and references.
5. **Validity.** Convert `dataset.as_of_ns` to a UTC calendar date. Refuse a
   rights review date, a rights validity bound or a certificate expiry that
   precedes that date. Refuse an unbounded retention term; accept a positive
   integer day count or a bounded ISO-8601 date duration.
6. **Exact consumer operations.** Require the rights record to cover the five
   operations of section 2, each `PERMITTED`, citing the same written record
   and carrying the section 4 evidence. An extra operation is a mismatch.
7. **Scope consistency.** Require rights, coverage, lineage and dataset source
   scope to name the same provider, product, venue and segment. Require one
   declared universe id and one coverage denominator across those sections.
8. **Release digest.** Recompute the manifest and data digests and compare them
   with the release declaration and the dataset manifest. Refuse a mismatch.
9. **Production declaration.** Require `release.producing_operations` to name
   typed producer `Operation` values and the coverage denominator to name its
   universe and count basis. Refuse a release whose declaration is absent or
   inconsistent with the event envelopes.
10. **As-of semantics.** Require one as-of basis and instant. Refuse lineage
    knowledge time after the as-of instant. Do not select a revision by arrival
    order.
11. **Typed payload.** Refuse an unregistered event type, an untyped payload, a
    null presented as zero, or a timestamp whose unit is undeclared.
12. **Continuity.** Report gaps against the declared provider and partition
    scope. A declared gap is a bounded coverage claim, not a filled observation.
13. **Archive link.** Require a raw digest and an archive reference on every
    event. Do not create a substitute observation when either is absent.
14. **Admission.** Refuse research use, experiment registration or a research
    job until the release passes the dataset-admission gate against the exact
    published identity and digests. The job runner takes an explicit published
    dataset directory and builds no dataset of its own.

Validation reads declarations and digests. It does not acquire provider data,
replay provider traffic or produce a market observation.

## 8. Compatibility and rejection behavior

| Code | Trigger | Behavior |
|---|---|---|
| `REJECT_CONTRACT_VERSION` | Unsupported major version | Stop; request a published migration record |
| `REJECT_INCOMPLETE` | Missing, empty or mistyped required field, including rights evidence | Stop; list the field path |
| `REJECT_SIGNATURE` | Signature object, key validity, key trust or canonical manifest digest fails | Stop; do not use any event from the release |
| `REJECT_VERDICT` | A rights or coverage verdict is not passing | Stop; name the field and observed value |
| `REJECT_VALIDITY` | Review, validity or expiry bound precedes the as-of UTC date | Stop; treat denied, expired and revoked references as refusals |
| `REJECT_RETENTION` | Retention is missing, open-ended or unbounded | Stop; require bounded days or an ISO-8601 duration |
| `REJECT_RIGHTS` | Exact consumer operation set, evidence, scope or written reference differs | Stop; no operation is inferred from another operation |
| `REJECT_DIGEST` | Manifest or data digest mismatch | Stop; do not use any event from the release |
| `REJECT_SCOPE` | Source scope, denominator or universe id differs between sections | Stop; do not merge scopes |
| `REJECT_PRODUCTION` | Production operations or coverage denominator is absent or inconsistent | Stop; do not consume a partially valid release |
| `REJECT_ASOF` | As-of basis or instant differs, or knowledge time leaks | Stop; do not substitute a subsequent revision |
| `REJECT_PAYLOAD_TYPE` | Unknown event type, untyped payload or unit error | Stop; do not coerce a missing value into zero |
| `REJECT_CONTINUITY` | Undeclared gap or sequence discontinuity | Stop; do not interpolate |
| `REJECT_ARCHIVE` | Raw digest or archive reference absent | Stop; do not treat a derived value as raw |
| `REJECT_ADMISSION` | No valid admission receipt for the exact release | Stop; no research use and no experiment registration |

Rules for change over time:

- A new optional field does not change the major version. Nirnay ignores it
  only after the field is reviewed and recorded; unreviewed fields are refused.
- A renamed, removed or retyped required field, a changed verdict vocabulary, a
  changed consumer operation set or vocabulary mapping, or a changed digest
  algorithm requires a new major version and a migration record.
- A corrected release receives a new release identity and a new signature over
  its new canonical manifest. The prior identity is retained; a consumer
  selects a view by as-of instant and knowledge time.
- A rejected release is not partially consumed. No observation, digest or
  verdict from it enters a research dataset or an experiment record.
- A synthetic or self-attested artifact is not a fallback for a rejected
  release. Absence of an alternative is a blocked state, not a substitution.

## 9. Relation to the repository gate

`dataset_admission.py` enforces the four evidence sections on scalar
declarations, including validity bounds and bounded retention. Its named
`CONSUMER_OPERATIONS` constant is the exact five-operation set of section 2;
the gate still requires that exact set with matching `PERMITTED` states and
written references. Abstract scalar tests cover the constant and the
per-operation refusal behavior. The full rights evidence field set of section 4
is a proposed contract requirement; the gate validates the declared scalar
subset and does not substitute for review of the written record. No admission
path is broadened and no validation is weakened by this alignment.

The `admit-dataset`, `verify-dataset --for-research` and
`run-research-job --dataset --admission` CLI paths fail closed with a named
reason before any artifact write; the job runner binds its command and registry
record to the admitted release and builds no dataset. No data is ingested by
these checks. The dataset acceptance contract in
`engineering/research/dataset-acceptance-contract.md` remains BLOCKED while all
21 source-product rights remain `UNKNOWN`.

This document changes no right, no capability record, no claim record, no
experiment state and no source-product rights value. It selects no transport
and no storage technology. ADR-0001 and the Fieldbook producer requirements
stay Proposed until owner ratification. No provider was contacted, no data was
collected and no release was created by this documentation change.
