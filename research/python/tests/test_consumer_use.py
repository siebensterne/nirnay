# SPDX-License-Identifier: Apache-2.0
"""Abstract scalar tests for consumer-use profiles and permission evaluation."""
from __future__ import annotations

from dataclasses import replace

import pytest

from quantlab.consumer_use import (
    ACTION_OBJECTS,
    CONSUMER_OPERATIONS,
    CREATE_DERIVED,
    INTERNAL_ANALYTICS,
    INTRADAY_CAPABILITY,
    READ_RELEASE,
    REPLAY,
    RETAIN_DERIVED_DATA,
    STORE_RESEARCH_COPY,
    USE_PROFILES,
    AdmissionDecision,
    AdmissionRequest,
    GrantDecision,
    PermissionGrant,
    RejectionCode,
    ReleaseDeclarations,
    ReleaseIdentity,
    RetentionPolicy,
    ScopeSet,
    ScopeState,
    UseScope,
    evaluate_consumer_use,
)
from quantlab.dataset_admission import AdmissionReceipt

AS_OF_NS = 1_700_000_000_000_000_000
DIGEST_A = "a" * 64
DIGEST_B = "b" * 64
DIGEST_C = "c" * 64
ALL_OBJECTS = (
    "release_manifest",
    "event_envelope",
    "research_copy",
    "research_dataset",
    "derived_artifact",
)


def _request_scope() -> UseScope:
    """Build an abstract named runtime scope."""
    return UseScope(
        provider="provider-a",
        product="product-x",
        venue="venue-a",
        segment="segment-cash",
        instruments=ScopeSet(ScopeState.NAMED, frozenset({"inst-a"})),
        fields=ScopeSet(ScopeState.NAMED, frozenset({"field-a"})),
        modalities=ScopeSet(ScopeState.NAMED, frozenset({"trade"})),
        data_time_from_ns=AS_OF_NS - 1_000_000_000,
        data_time_to_ns=AS_OF_NS,
    )


def _grant_scope() -> UseScope:
    """Build an abstract grant scope containing the runtime scope."""
    scope = _request_scope()
    return replace(
        scope,
        instruments=ScopeSet(ScopeState.NAMED, frozenset({"inst-a", "inst-b"})),
        fields=ScopeSet(ScopeState.NAMED, frozenset({"field-a", "field-b"})),
        data_time_from_ns=scope.data_time_from_ns - 1,
        data_time_to_ns=AS_OF_NS + 1,
    )


def _request(profile: str, actions: tuple[str, ...]) -> AdmissionRequest:
    """Build a valid abstract admission request for one declared profile."""
    return AdmissionRequest(
        release_identity=ReleaseIdentity(
            release_id="release:abstract-2026",
            manifest_sha256=DIGEST_A,
            data_sha256=DIGEST_B,
            publisher_identity="publisher-key-1",
        ),
        actor="nirnay-research-job",
        use_profile=profile,
        requested_actions=actions,
        requested_objects=ALL_OBJECTS,
        purpose="proprietary_own_account_trading_research",
        licensee="nirnay-legal-entity",
        recipient="internal",
        scope=_request_scope(),
        requested_retention=RetentionPolicy(90, "purge:explicit"),
        as_of_ns=AS_OF_NS,
        declared_capabilities=frozenset(
            {INTRADAY_CAPABILITY} if profile == "intraday_replay_research" else set()
        ),
    )


def _grant(action: str, **overrides: object) -> PermissionGrant:
    """Build one written grant for a single requested action."""
    grant = PermissionGrant(
        grant_id=f"grant:{action}",
        actor="nirnay-research-job",
        action=action,
        data_object=ACTION_OBJECTS[action][0],
        decision=GrantDecision.PERMIT,
        purpose="proprietary_own_account_trading_research",
        licensee="nirnay-legal-entity",
        recipient="internal",
        scope=_grant_scope(),
        retention=RetentionPolicy(90, "purge:explicit"),
        effective_from_ns=AS_OF_NS - 100,
        effective_until_ns=AS_OF_NS + 100,
        evidence_reference="written:rights-record-2026",
    )
    return replace(grant, **overrides)


def _grants(actions: tuple[str, ...], **overrides: object) -> tuple[PermissionGrant, ...]:
    """Build the matching grants for a declared action set."""
    return tuple(_grant(action, **overrides) for action in actions)


def _declarations() -> ReleaseDeclarations:
    """Build named scalar declarations from earlier verification stages."""
    return ReleaseDeclarations(
        schema_version=1,
        envelope_version=1,
        publisher_identity="publisher-key-1",
        signature_receipt="signature:release:abstract-2026",
        manifest_sha256=DIGEST_A,
        artifact_digests=(DIGEST_C,),
        source_lineage_reference="lineage:abstract-2026",
        rights_evidence_reference="written:rights-record-2026",
        coverage_certificate="coverage:certificate-2026",
        coverage_verdict="PASS",
        pit_basis="knowledge_time",
        pit_as_of_ns=AS_OF_NS,
    )


def _evaluate(request: AdmissionRequest,
              grants: tuple[PermissionGrant, ...],
              declarations: ReleaseDeclarations | None = None) -> AdmissionDecision:
    """Evaluate the supplied abstract declarations and return the decision."""
    return evaluate_consumer_use(request, grants, declarations or _declarations())


def _profile_request(profile: str) -> AdmissionRequest:
    """Build a valid request for one of the five declared profiles."""
    declared = USE_PROFILES[profile]
    return _request(profile, declared.required_actions)


def _profile_grants(profile: str) -> tuple[PermissionGrant, ...]:
    """Build the matching grants for one of the five declared profiles."""
    return _grants(USE_PROFILES[profile].required_actions)


def test_five_versioned_profiles_declare_the_documented_actions() -> None:
    """The five versioned profiles reuse the existing consumer vocabulary."""
    assert set(USE_PROFILES) == {
        "reference_lookup",
        "historical_research",
        "intraday_replay_research",
        "derived_research_dataset",
        "portfolio_simulation",
    }
    assert all(item.version == 1 for item in USE_PROFILES.values())
    assert USE_PROFILES["reference_lookup"].required_actions == (READ_RELEASE,)
    assert USE_PROFILES["historical_research"].required_actions == (
        READ_RELEASE, STORE_RESEARCH_COPY, REPLAY, INTERNAL_ANALYTICS
    )
    assert USE_PROFILES["intraday_replay_research"].required_capabilities == frozenset(
        {INTRADAY_CAPABILITY}
    )
    assert USE_PROFILES["derived_research_dataset"].required_actions == (
        READ_RELEASE, STORE_RESEARCH_COPY, INTERNAL_ANALYTICS,
        RETAIN_DERIVED_DATA, CREATE_DERIVED,
    )
    assert USE_PROFILES["portfolio_simulation"].required_actions == (
        READ_RELEASE, REPLAY, INTERNAL_ANALYTICS
    )
    assert CREATE_DERIVED not in CONSUMER_OPERATIONS
    assert set(CONSUMER_OPERATIONS) <= set(ACTION_OBJECTS)


@pytest.mark.parametrize("profile", list(USE_PROFILES))
def test_requested_actions_cover_each_profile_without_omission(profile: str) -> None:
    """A profile cannot omit a required action and omitting one blocks admission."""
    request = _profile_request(profile)
    grants = _profile_grants(profile)
    assert _evaluate(request, grants).admitted
    for action in USE_PROFILES[profile].required_actions:
        reduced = tuple(item for item in request.requested_actions if item != action)
        decision = _evaluate(replace(request, requested_actions=reduced), _grants(reduced))
        assert decision.code is RejectionCode.USE_SCOPE


def test_extra_grant_never_authorises_an_unrequested_action() -> None:
    """An extra grant is ignored; a request outside its profile is refused."""
    request = _profile_request("reference_lookup")
    grants = _grants((READ_RELEASE, CREATE_DERIVED))
    assert _evaluate(request, grants).admitted
    extra = replace(
        request,
        requested_actions=(READ_RELEASE, CREATE_DERIVED),
    )
    decision = _evaluate(extra, _grants((READ_RELEASE, CREATE_DERIVED)))
    assert decision.code is RejectionCode.USE_SCOPE


def test_derived_creation_requires_its_own_permitted_grant() -> None:
    """Retention of derived data never implies creation of derived data."""
    actions = USE_PROFILES["derived_research_dataset"].required_actions
    request = _request("derived_research_dataset", actions)
    without_creation = _grants(tuple(item for item in actions if item != CREATE_DERIVED))
    decision = _evaluate(request, without_creation)
    assert decision.code is RejectionCode.RIGHTS_EVIDENCE
    denied = (*_grants(actions), _grant(CREATE_DERIVED, decision=GrantDecision.DENY))
    denied_decision = _evaluate(request, denied)
    assert denied_decision.code is RejectionCode.AMBIGUOUS_GRANTS
    single_denied = tuple(
        _grant(action, decision=GrantDecision.DENY) if action == CREATE_DERIVED else _grant(action)
        for action in actions
    )
    assert _evaluate(request, single_denied).code is RejectionCode.RIGHTS_EVIDENCE


@pytest.mark.parametrize(("field", "value", "code"), [
    ("purpose", "academic_research", RejectionCode.USE_SCOPE),
    ("purpose", "internal_analysis", RejectionCode.USE_SCOPE),
    ("purpose", "proprietary own-account trading research", RejectionCode.USE_SCOPE),
    ("recipient", "public", RejectionCode.USE_SCOPE),
    ("licensee", "fieldbook-operator", RejectionCode.RIGHTS_EVIDENCE),
])
def test_wrong_purpose_recipient_or_licensee_is_refused(
    field: str, value: str, code: RejectionCode
) -> None:
    """Decision dimensions are exact and cannot be replaced by nearby values."""
    request = replace(_profile_request("historical_research"), **{field: value})
    decision = _evaluate(request, _profile_grants("historical_research"))
    assert decision.code is code


def test_grant_licensee_and_recipient_must_match_the_request() -> None:
    """A grant for another licensee or recipient does not cover this request."""
    request = _profile_request("historical_research")
    actions = request.requested_actions
    for field, value in (("licensee", "other-entity"), ("recipient", "participant")):
        grants = _grants(actions, **{field: value})
        assert _evaluate(request, grants).code is RejectionCode.RIGHTS_EVIDENCE


@pytest.mark.parametrize("action", [READ_RELEASE, STORE_RESEARCH_COPY,
                                    INTERNAL_ANALYTICS, RETAIN_DERIVED_DATA, CREATE_DERIVED])
def test_denied_or_missing_action_grant_blocks_admission(action: str) -> None:
    """No action is admitted on a denial or on another action's grant."""
    profile = "derived_research_dataset"
    request = _profile_request(profile)
    actions = request.requested_actions
    missing = _grants(tuple(item for item in actions if item != action))
    assert _evaluate(request, missing).code is RejectionCode.RIGHTS_EVIDENCE
    denied = tuple(
        _grant(item, decision=GrantDecision.DENY) if item == action else _grant(item)
        for item in actions
    )
    assert _evaluate(request, denied).code is RejectionCode.RIGHTS_EVIDENCE


def test_intraday_capability_is_not_a_permission() -> None:
    """An intraday claim is a capability, not an added action or entitlement."""
    profile = "intraday_replay_research"
    request = replace(_profile_request(profile), declared_capabilities=frozenset())
    grants = _profile_grants(profile)
    assert _evaluate(request, grants).code is RejectionCode.USE_SCOPE
    capable = replace(request, declared_capabilities=frozenset({INTRADAY_CAPABILITY}))
    assert _evaluate(capable, grants).admitted
    widened = replace(capable, requested_actions=(*request.requested_actions, CREATE_DERIVED))
    extra_grants = _grants((*request.requested_actions, CREATE_DERIVED))
    assert _evaluate(widened, extra_grants).code is RejectionCode.USE_SCOPE


@pytest.mark.parametrize("field", ["provider", "product", "venue", "segment"])
def test_scope_dimension_mismatch_is_refused(field: str) -> None:
    """A grant for a different source dimension does not cover the request."""
    request = _profile_request("historical_research")
    scope = replace(_grant_scope(), **{field: "other-" + field})
    grants = _grants(request.requested_actions, scope=scope)
    assert _evaluate(request, grants).code is RejectionCode.USE_SCOPE


def test_subject_sets_and_time_bounds_must_be_contained() -> None:
    """A narrower grant set or a bounded grant cannot cover a wider request."""
    request = _profile_request("historical_research")
    actions = request.requested_actions
    narrow = replace(_grant_scope(), instruments=ScopeSet(ScopeState.NAMED, frozenset()))
    assert _evaluate(request, _grants(actions, scope=narrow)).code is RejectionCode.USE_SCOPE
    unbounded = replace(request, scope=replace(_request_scope(),
                                               data_time_from_ns=None, data_time_to_ns=None))
    assert _evaluate(unbounded, _grants(actions)).code is RejectionCode.USE_SCOPE


def test_retention_request_cannot_exceed_grant() -> None:
    """A bounded grant cannot cover a longer requested retention."""
    request = replace(
        _profile_request("historical_research"),
        requested_retention=RetentionPolicy(400, "purge:explicit"),
    )
    decision = _evaluate(request, _profile_grants("historical_research"))
    assert decision.code is RejectionCode.RETENTION
    indefinite = replace(request, requested_retention=RetentionPolicy(None, "purge:explicit"))
    assert _evaluate(indefinite, _profile_grants("historical_research")).code is (
        RejectionCode.RETENTION
    )


def test_effective_interval_must_contain_the_as_of_instant() -> None:
    """A lapsed grant is not a permission at the requested as-of instant."""
    request = _profile_request("historical_research")
    grants = _grants(request.requested_actions, effective_until_ns=AS_OF_NS - 1)
    assert _evaluate(request, grants).code is RejectionCode.RIGHTS_EVIDENCE


def test_ambiguous_grants_are_refused() -> None:
    """Two grants for one requested action are conflicting authority."""
    request = _profile_request("historical_research")
    grants = _grants(request.requested_actions)
    duplicate = (*grants, replace(grants[0], grant_id="grant:read-release-2"))
    assert _evaluate(request, duplicate).code is RejectionCode.AMBIGUOUS_GRANTS


def test_rejection_codes_are_distinct_and_typed() -> None:
    """Every named failure mode carries its own stable rejection code."""
    values = [item.value for item in RejectionCode]
    assert len(values) == len(set(values)) == 11
    assert set(values) == {
        "REJECT_PUBLISHER_IDENTITY",
        "REJECT_SIGNATURE_INTEGRITY",
        "REJECT_SOURCE_LINEAGE",
        "REJECT_RIGHTS_EVIDENCE",
        "REJECT_USE_SCOPE",
        "REJECT_COVERAGE",
        "REJECT_PIT_CONSISTENCY",
        "REJECT_RETENTION",
        "REJECT_ARTIFACT_DIGESTS",
        "REJECT_SCHEMA_ENVELOPE",
        "REJECT_AMBIGUOUS_GRANTS",
    }


def _mutate(case: str, request: AdmissionRequest, grants: tuple[PermissionGrant, ...],
            declarations: ReleaseDeclarations) -> tuple[
                AdmissionRequest, tuple[PermissionGrant, ...], ReleaseDeclarations
            ]:
    """Apply one abstract mutation used by the typed-code test."""
    if case == "schema":
        return request, grants, replace(declarations, envelope_version=0)
    if case == "publisher":
        return request, grants, replace(declarations, publisher_identity="other-publisher")
    if case == "signature":
        return request, grants, replace(declarations, signature_receipt="UNKNOWN")
    if case == "lineage":
        return request, grants, replace(declarations, source_lineage_reference="PENDING")
    if case == "rights":
        return request, grants, replace(declarations, rights_evidence_reference="UNKNOWN")
    if case == "coverage":
        return request, grants, replace(declarations, coverage_verdict="PARTIAL")
    if case == "pit":
        return request, grants, replace(declarations, pit_as_of_ns=AS_OF_NS + 1)
    if case == "artifacts":
        return request, grants, replace(declarations, artifact_digests=())
    if case == "use":
        return replace(request, requested_actions=(READ_RELEASE,)), grants, declarations
    if case == "retention":
        changed = replace(request, requested_retention=RetentionPolicy(400, "purge:explicit"))
        return changed, grants, declarations
    if case == "grants":
        return request, (*grants, replace(grants[0], grant_id="grant:second")), declarations
    raise AssertionError(f"unsupported mutation case: {case}")


@pytest.mark.parametrize(("case", "code"), [
    ("schema", RejectionCode.SCHEMA_ENVELOPE),
    ("publisher", RejectionCode.PUBLISHER_IDENTITY),
    ("signature", RejectionCode.SIGNATURE_INTEGRITY),
    ("lineage", RejectionCode.SOURCE_LINEAGE),
    ("rights", RejectionCode.RIGHTS_EVIDENCE),
    ("coverage", RejectionCode.COVERAGE),
    ("pit", RejectionCode.PIT_CONSISTENCY),
    ("artifacts", RejectionCode.ARTIFACT_DIGESTS),
    ("use", RejectionCode.USE_SCOPE),
    ("retention", RejectionCode.RETENTION),
    ("grants", RejectionCode.AMBIGUOUS_GRANTS),
])
def test_each_failure_mode_returns_its_typed_code(case: str, code: RejectionCode) -> None:
    """One mutated declaration produces exactly one typed rejection code."""
    request = _profile_request("historical_research")
    grants = _profile_grants("historical_research")
    changed_request, changed_grants, changed_declarations = _mutate(
        case, request, grants, _declarations()
    )
    decision = evaluate_consumer_use(changed_request, changed_grants, changed_declarations)
    assert decision.admitted is False
    assert decision.code is code


def test_admission_decision_is_distinct_from_the_existing_receipt() -> None:
    """The new decision is a scalar verdict, not an admission receipt for data."""
    decision = _evaluate(_profile_request("reference_lookup"),
                         _profile_grants("reference_lookup"))
    assert not isinstance(decision, AdmissionReceipt)
    assert decision.code is None
    assert decision.reason == ""
