# NDC v1 consumer requirements (Nirnay side) — Proposed

- **Status:** Proposed engineering requirements. Not ratified, not accepted and
  not an authorization for provider contact, collection, dataset acceptance or
  an experiment.
- **Version:** `ndc-consumer-requirements/v1`. A semantic change requires a new
  version and a migration record; a field rename is a breaking change.
- **Governing decision:**
  `engineering/architecture/decisions/ADR-0001-data-ownership.md`. That ADR
  remains **Proposed design decision**; this document does not ratify it.
- **Owner:** Nirnay consumes. The future complementary platform publishes and
  versions the release. Nirnay does not acquire source data, hold a competing
  raw archive or re-normalize raw payloads.
- **Scope:** the semantic fields Nirnay requires on a release and on each event
  envelope, plus consumer validation and rejection behavior.
- **Out of scope:** transport, RPC, serialization framing, storage engine,
  queue, database, credentials, provider endpoints and technology selection.
  Those require a measured requirement and a separate decision.

## 1. Contract objects

The release contract is split into two objects. Dataset-wide facts live in the
release manifest; per-observation facts live in the event envelope. A fact is
not repeated in both objects.

### 1.1 Release manifest

The release manifest is the immutable object an experiment references. It is
versioned with the release and is the input to the dataset-admission gate in
`research/python/src/quantlab/dataset_admission.py`.

| Group | Required fields | Consumer meaning |
|---|---|---|
| Dataset identity and source scope | `dataset_id`, `dataset_version`, `dataset_origin`, `source_scope` (`provider`, `product`, `venue`, `segment`) | Named release identity; source and venue are declared, not inferred from a symbol |
| Rights | `rights.record_reference`, `rights.permission_source`, `rights.operations`, `rights.scope`, `rights.review_date`, `rights.retention_period` | Written permission reference and the exact operations covered; a registry `UNKNOWN` is not permission |
| Coverage and completeness | `coverage.certificate_reference`, `coverage.verdict`, `coverage.completeness`, `coverage.scope`, `coverage.universe_id`, `coverage.as_of_ns` | Passing certification over the declared scope and as-of instant; gaps and rejects are declared, not invented |
| Schema versions | contract schema version, normalizer version, payload schema version per event type | Version identity for each transformation applied before publication |
| Retention | `rights.retention_period`, archive closure and deletion basis | Declared retention term; a missing term is a denial |
| Digest | `release.manifest_sha256`, `release.data_sha256`, `release.digest_algorithm` | Immutable digests of the release manifest and canonical dataset content |
| Publication identity | `release.release_id`, `release.publication_identity`, `release.publication_method`, `release.archive_reference` | Immutable release identity and the named publisher of that identity |
| Point-in-time lineage | `lineage.basis`, `lineage.universe_version`, `lineage.universe_source_reference`, `lineage.instrument_identity_source_reference`, `lineage.corporate_action_source_reference`, matching `*_sha256` fields, `lineage.as_of_ns`, `lineage.knowledge_time_ns` | Named universe, instrument-identity and corporate-action provenance with digests and one as-of basis |

The admission gate maps these fields to four evidence sections: written rights
record, coverage certificate, immutable release identity with matching digests,
and point-in-time lineage. Admission refuses a missing, `UNKNOWN`, `PENDING`,
`FAILED` or internally inconsistent field. No right is granted by a manifest.

### 1.2 Event envelope

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

## 2. Consumer validation

Validation is ordered and stops at the first refusal. A following check is not
run to soften an earlier failure.

1. **Contract version.** Require an exact match on `contract_version`. An
   unknown major version is a rejection, not a migration attempt.
2. **Required fields.** Require every field in section 1.1 and the applicable
   fields in section 1.2 with the declared type. Reject missing, empty,
   redirected or placeholder references.
3. **Verdicts.** Require `PERMITTED` for each listed operation, `PASS` and
   `COMPLETE` for coverage, and an immutable release flag. `UNKNOWN`,
   `PENDING`, `FAILED`, `BLOCKED` and `PROHIBITED` are denials.
4. **Exact operations.** Require the rights record to cover the research
   operations `store`, `replay`, `internal_analytics` and `retention`, each
   citing the same written record. An extra operation is a mismatch.
5. **Scope consistency.** Require rights, coverage, lineage and dataset source
   scope to name the same provider, product, venue and segment. Require one
   declared universe id across those sections.
6. **Release digest.** Recompute the manifest and data digests and compare them
   with the release declaration and the dataset manifest. Refuse a mismatch.
7. **As-of semantics.** Require one as-of basis and instant. Refuse lineage
   knowledge time after the as-of instant. Do not select a revision by
   arrival order.
8. **Typed payload.** Refuse an unregistered event type, an untyped payload, a
   null presented as zero, or a timestamp whose unit is undeclared.
9. **Continuity.** Report gaps against the declared provider and partition
   scope. A declared gap is a bounded coverage claim, not a filled observation.
10. **Archive link.** Require a raw digest and an archive reference on every
    event. Do not create a substitute observation when either is absent.
11. **Admission.** Refuse research use, experiment registration or a research
    job until the release passes the dataset-admission gate against the exact
    published identity and digests.

Validation reads declarations and digests. It does not acquire provider data,
replay provider traffic or produce a market observation.

## 3. Compatibility and rejection behavior

| Code | Trigger | Behavior |
|---|---|---|
| `REJECT_CONTRACT_VERSION` | Unsupported major version | Stop; request a published migration record |
| `REJECT_INCOMPLETE` | Missing, empty or mistyped required field | Stop; list the field path |
| `REJECT_VERDICT` | A rights or coverage verdict is not passing | Stop; name the field and observed value |
| `REJECT_RIGHTS` | Exact operation set, scope or written reference differs | Stop; no operation is inferred from another operation |
| `REJECT_DIGEST` | Manifest or data digest mismatch | Stop; do not use any event from the release |
| `REJECT_SCOPE` | Source scope or universe id differs between sections | Stop; do not merge scopes |
| `REJECT_ASOF` | As-of basis or instant differs, or knowledge time leaks | Stop; do not substitute a subsequent revision |
| `REJECT_PAYLOAD_TYPE` | Unknown event type, untyped payload or unit error | Stop; do not coerce a missing value into zero |
| `REJECT_CONTINUITY` | Undeclared gap or sequence discontinuity | Stop; do not interpolate |
| `REJECT_ARCHIVE` | Raw digest or archive reference absent | Stop; do not treat a derived value as raw |
| `REJECT_ADMISSION` | No valid admission receipt for the exact release | Stop; no research use and no experiment registration |

Rules for change over time:

- A new optional field does not change the major version. Nirnay ignores it
  only after the field is reviewed and recorded; unreviewed fields are refused.
- A renamed, removed or retyped required field, a changed verdict vocabulary
  or a changed digest algorithm requires a new major version and a migration
  record.
- A corrected release receives a new release identity. The prior identity is
  retained; a consumer selects a view by as-of instant and knowledge time.
- A rejected release is not partially consumed. No observation, digest or
  verdict from it enters a research dataset or an experiment record.
- A synthetic or self-attested artifact is not a fallback for a rejected
  release. Absence of an alternative is a blocked state, not a substitution.

## 4. Relation to the repository gate

`dataset_admission.py` enforces the four evidence sections on scalar
declarations. The `admit-dataset`, `verify-dataset --for-research` and
`run-research-job --admission` CLI paths fail closed with a named reason. No
data is ingested by these checks. The dataset acceptance contract in
`engineering/research/dataset-acceptance-contract.md` remains BLOCKED while all
21 source-product rights remain `UNKNOWN`.

This document changes no right, no capability record, no claim record and no
experiment state. It selects no transport and no storage technology. ADR-0001
stays Proposed until owner ratification.
