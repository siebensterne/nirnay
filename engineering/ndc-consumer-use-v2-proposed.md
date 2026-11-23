# Nirnay consumer-use and permission-evaluation model — Proposed

<!-- SPDX-License-Identifier: Apache-2.0 -->

- **Status:** Proposed semantic model and declaration-level implementation. Not
  ratified as an NDC contract, not a data licence and not authorization for
  provider contact, collection, dataset acceptance or an experiment.
- **Version:** `consumer-use/v1`. A profile rename, action change, purpose
  change or rejection-code change requires a new version and a migration note.
- **Governing source:** `NDC-SEMANTIC-MODEL-V2-PROPOSED.md`, sections 1,
  2.2–2.5, 4, 4.1, 7 and 9. That document remains Proposed; nothing here
  finalizes an NDC JSON schema.
- **Implementation:** `research/python/src/quantlab/consumer_use.py` and its
  abstract scalar tests in `research/python/tests/test_consumer_use.py`.
- **Boundary:** the evaluator reads declarations and grants. It verifies no
  signature, recomputes no canonical bytes, compares no digest with file bytes
  and inspects no event envelope. Full detached-signature verification is a
  later stage. `dataset_admission.py` validates declarations only and is not a
  complete release verifier.

## 1. Separated concepts

Capability, permission, operation, data object, purpose and evidence remain
independent. A capability claim does not create a grant; a grant does not
create a capability claim; a signature receipt does not create a right. No
generic default may supply a purpose, licensee, recipient, territory or scope.
The declared purpose is exactly `proprietary_own_account_trading_research`;
`academic_research`, `non_commercial_research`, `internal_analysis` and
`historical_archive` are not substitutes.

## 2. Versioned use profiles

`CONSUMER_USE_MODEL_VERSION` is 1. The five declared profiles and their
required consumer actions are:

| Profile | Required actions | Required capability claim |
|---|---|---|
| `reference_lookup` | `read_release` | none |
| `historical_research` | `read_release`, `store_permitted_research_copy`, `replay`, `internal_analytics` | none |
| `intraday_replay_research` | `read_release`, `store_permitted_research_copy`, `replay`, `internal_analytics` | `intraday_capability` |
| `derived_research_dataset` | `read_release`, `store_permitted_research_copy`, `internal_analytics`, `retain_permitted_derived_data`, `create_derived` | none |
| `portfolio_simulation` | `read_release`, `replay`, `internal_analytics` | none |

The five names of `CONSUMER_OPERATIONS` stay authoritative and unchanged.
`read_release` covers manifest and event-payload reading; payload access is a
later stage, not an added right. Retention of derived data maps to
`retain_permitted_derived_data`; creation of derived data is the separate
requested action `create_derived`, which is not a value of
`CONSUMER_OPERATIONS` and is never inferred from a retention grant. No new
consumer enum value is introduced for it.

A request must declare exactly the actions of its profile. Omitting a required
action or adding an action outside the profile is a use-scope refusal. An
extra grant for an unrequested action has no effect on the decision.

## 3. AdmissionRequest

`AdmissionRequest` binds the following declarations:

| Field | Binding |
|---|---|
| `release_identity` | `release_id`, `manifest_sha256`, `data_sha256`, `publisher_identity` |
| `actor` | named consumer actor, equal on each matching grant |
| `use_profile` | one of the five versioned profile identifiers |
| `requested_actions` | explicit actions, each covered by the profile |
| `requested_objects` | object class for each requested action |
| `purpose` | exact reviewed purpose, never a nearby label |
| `licensee`, `recipient` | named legal entity and exact recipient class |
| `scope` | four source dimensions, named instrument/field/modality sets, data-time window |
| `requested_retention` | bounded maximum duration and explicit purge rule |
| `as_of_ns` | positive epoch-nanosecond instant used for PIT consistency |
| `declared_capabilities` | capability claims only; never permissions |

`NOT_APPLICABLE`, `NAMED` and `ALL` are distinct scope states. A runtime
request must use named, non-empty subject sets; an empty `NAMED` set is not
`ALL`. A bounded grant cannot cover an unbounded request. Retention is bounded
with an explicit purge rule; a narrower request may be permitted, an
indefinite request is refused.

## 4. Permission evaluation

`evaluate_consumer_use` is ordered and stops at the first typed refusal. It
checks declaration shape and PIT consistency, then profile coverage and the
request dimensions, then grants. For each requested action it requires exactly
one grant with a `PERMIT` decision whose actor, purpose, licensee, recipient,
action, object, scope, retention and effective interval cover the request. A
missing or denied grant blocks admission. Two grants for one requested action
are ambiguous and block admission. Grants for unrequested actions are ignored
and cannot authorize those actions. The result is an immutable
`AdmissionDecision` with a typed rejection code on every refusal; it is a
scalar verdict, not a receipt for data.

## 5. Typed rejection codes

| Code | Failure |
|---|---|
| `REJECT_PUBLISHER_IDENTITY` | publisher identity absent or different |
| `REJECT_SIGNATURE_INTEGRITY` | no named signature receipt or manifest digest binding |
| `REJECT_SOURCE_LINEAGE` | source lineage reference absent |
| `REJECT_RIGHTS_EVIDENCE` | rights evidence absent, no grant, denied grant or identity mismatch |
| `REJECT_USE_SCOPE` | profile coverage, purpose, recipient, scope or object failure |
| `REJECT_COVERAGE` | no passing named coverage certificate |
| `REJECT_PIT_CONSISTENCY` | as-of basis or instant differs |
| `REJECT_RETENTION` | unbounded or uncovered retention |
| `REJECT_ARTIFACT_DIGESTS` | artifact digests absent or malformed |
| `REJECT_SCHEMA_ENVELOPE` | schema or envelope version is not a positive integer |
| `REJECT_AMBIGUOUS_GRANTS` | duplicate, conflicting or non-unique grants |

These codes classify declaration-level failures. A code other than
`REJECT_USE_SCOPE` is not a claim that cryptographic verification was run or
that a source observation exists.

## 6. Compatibility and non-goals

`dataset_admission.py` validation is unchanged; only its module documentation
states that it is not a full release verifier. No CLI path, research job,
registry, capability record, claim record, experiment path or rights value is
changed. All source rights remain `UNKNOWN`; no release, dataset, data object
or synthetic observation is created by this model. No NDC schema, transport
or storage choice is finalized here.
