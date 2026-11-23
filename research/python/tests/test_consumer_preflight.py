# SPDX-License-Identifier: Apache-2.0
"""Abstract scalar tests for C1-C3 consumer admission conformance.

All values are integers, names and digests. No market observation, fixture,
provider endpoint, dataset, release, experiment or live path is involved.
"""
from __future__ import annotations

from dataclasses import fields, replace

import pytest

from quantlab.consumer_use import (
    INTRADAY_CAPABILITY,
    READ_RELEASE,
    PURPOSE,
    RECIPIENT_CLASS,
    TIME_AXIS_NAMES,
    USE_PROFILES,
    CapabilityFacts,
    ConsumerUseRequest,
    GrantDecision,
    IdentityScope,
    PreflightDecision,
    PreflightGrant,
    PreflightRejectionCode,
    ReleaseCapabilityEvidence,
    ReleaseDeclarations,
    ReleaseEvidence,
    ReleaseIdentity,
    RequestedUse,
    RetentionPolicy,
    ScopeSet,
    ScopeState,
    TimeAxes,
    preflight_consumer_admission,
)
from quantlab.dataset_admission import AdmissionReceipt

NS_2023 = 1_672_531_200_000_000_000
NS_2024 = 1_704_067_200_000_000_000
NS_2025 = 1_735_689_600_000_000_000
NS_2026 = 1_767_225_600_000_000_000
NS_2027 = 1_798_627_200_000_000_000
DIGEST_A = "a" * 64
DIGEST_B = "b" * 64
DIGEST_C = "c" * 64
OBJECT_ID = "object:abstract-1"
OBJECT_VERSION = "v1"
RIGHTS_RECORD = "written:rights-record-2026"


def _axes(**overrides: object) -> TimeAxes:
    """Build six independent time axes with declared owners."""
    base = TimeAxes(
        data_as_of_ns=NS_2024,
        source_data_time_from_ns=NS_2024 - 100,
        source_data_time_to_ns=NS_2024,
        producer_operation_time_ns=NS_2024 + 10,
        decision_at_ns=NS_2026,
        release_available_at_ns=NS_2024 + 20,
        knowledge_time_ns=NS_2024 - 200,
    )
    return replace(base, **overrides)


def _scope(**overrides: object) -> IdentityScope:
    """Build a named runtime identity scope."""
    base = IdentityScope(
        provider="provider-a",
        source_product="product-x",
        venue="venue-a",
        segment="segment-cash",
        dataset="dataset-d1",
        release_id="release:abstract-2026",
        territory="IN",
        usage_mode="non_display",
        delivery_method="file",
        instruments=ScopeSet(ScopeState.NAMED, frozenset({"inst-a"})),
        fields=ScopeSet(ScopeState.NAMED, frozenset({"field-a"})),
        modalities=ScopeSet(ScopeState.NAMED, frozenset({"trade"})),
        data_time_from_ns=NS_2024 - 200,
        data_time_to_ns=NS_2024 + 200,
    )
    return replace(base, **overrides)


def _uses(profile_id: str) -> tuple[RequestedUse, ...]:
    """Build the exact required pairs of one declared profile."""
    profile = USE_PROFILES[profile_id]
    return tuple(
        RequestedUse(
            action=action,
            object_kind=object_kind,
            object_id=OBJECT_ID,
            object_version=OBJECT_VERSION,
            scope=_scope(),
        )
        for action, object_kind in sorted(profile.required_pairs)
    )


def _request(profile_id: str = "historical_research",
             **overrides: object) -> ConsumerUseRequest:
    """Build a valid abstract consumer preflight request."""
    profile = USE_PROFILES[profile_id]
    base = ConsumerUseRequest(
        release_identity=ReleaseIdentity(
            release_id="release:abstract-2026",
            manifest_sha256=DIGEST_A,
            data_sha256=DIGEST_B,
            publisher_identity="publisher-key-1",
        ),
        actor="nirnay-research-job",
        use_profile=profile_id,
        requested_uses=_uses(profile_id),
        purpose=PURPOSE,
        licensee="nirnay-legal-entity",
        recipient_class=RECIPIENT_CLASS,
        recipient_id="nirnay-research-job-001",
        time_axes=_axes(),
        requested_retention=RetentionPolicy(90, "purge:explicit"),
        declared_capabilities=frozenset(profile.required_capabilities),
    )
    return replace(base, **overrides)


def _grant(use: RequestedUse, **overrides: object) -> PreflightGrant:
    """Build one written grant for one exact requested pair."""
    base = PreflightGrant(
        grant_id=f"grant:{use.action}:{use.object_kind}",
        decision=GrantDecision.PERMIT,
        actor="nirnay-research-job",
        action=use.action,
        object_kind=use.object_kind,
        object_id=use.object_id,
        object_version=use.object_version,
        purpose=PURPOSE,
        licensee="nirnay-legal-entity",
        recipient_class=RECIPIENT_CLASS,
        recipient_id="nirnay-research-job-001",
        scope=use.scope,
        retention=RetentionPolicy(90, "purge:explicit"),
        effective_from_ns=NS_2023,
        effective_until_ns=NS_2027,
        evidence_reference=RIGHTS_RECORD,
    )
    return replace(base, **overrides)


def _grants(request: ConsumerUseRequest,
            **overrides: object) -> tuple[PreflightGrant, ...]:
    """Build grants covering every requested pair of a request."""
    return tuple(_grant(use, **overrides) for use in request.requested_uses)


def _release(**capability_overrides: object) -> ReleaseEvidence:
    """Build release-owned declarations and capability evidence."""
    capabilities = ReleaseCapabilityEvidence(
        producer_declared_capabilities=frozenset({INTRADAY_CAPABILITY}),
        release_available_capabilities=frozenset({INTRADAY_CAPABILITY}),
        verified_capabilities=frozenset(),
        evidence_reference=RIGHTS_RECORD,
    )
    return ReleaseEvidence(
        declarations=ReleaseDeclarations(
            schema_version=1,
            envelope_version=1,
            publisher_identity="publisher-key-1",
            signature_receipt="signature:release:abstract-2026",
            manifest_sha256=DIGEST_A,
            artifact_digests=(DIGEST_C,),
            source_lineage_reference="lineage:abstract-2026",
            rights_evidence_reference=RIGHTS_RECORD,
            coverage_certificate="coverage:certificate-2026",
            coverage_verdict="PASS",
            pit_basis="knowledge_time",
            pit_as_of_ns=NS_2024,
        ),
        capabilities=replace(capabilities, **capability_overrides),
    )


def _evaluate(request: ConsumerUseRequest | None = None,
              grants: tuple[PreflightGrant, ...] | None = None,
              release: ReleaseEvidence | None = None) -> PreflightDecision:
    """Evaluate the supplied abstract declarations and return the verdict."""
    return preflight_consumer_admission(
        request if request is not None else _request(),
        grants if grants is not None else _grants(request or _request()),
        release if release is not None else _release(),
    )


def test_control_request_is_admitted_with_four_separate_capability_facts() -> None:
    """A valid request passes and keeps the four capability facts distinct."""
    decision = _evaluate()
    assert decision.admitted
    assert decision.code is None
    assert decision.capability_facts == CapabilityFacts(
        required_capabilities=frozenset(),
        producer_declared_capabilities=frozenset({INTRADAY_CAPABILITY}),
        release_available_capabilities=frozenset({INTRADAY_CAPABILITY}),
        verified_capabilities=frozenset(),
    )
    assert tuple(field.name for field in fields(CapabilityFacts)) == (
        "required_capabilities",
        "producer_declared_capabilities",
        "release_available_capabilities",
        "verified_capabilities",
    )


def test_expired_grant_at_decision_time_blocks_historical_data_request() -> None:
    """A 2024 data selection does not revive a grant that lapsed in 2025."""
    request = _request()
    grants = _grants(request, effective_from_ns=NS_2024, effective_until_ns=NS_2025)
    decision = _evaluate(request, grants)
    assert decision.code is PreflightRejectionCode.GRANT_EXPIRED_AT_DECISION
    assert decision.data_as_of_ns == NS_2024
    assert decision.decision_at_ns == NS_2026


@pytest.mark.parametrize("data_as_of_ns", [NS_2023, NS_2024, NS_2025 - 1])
def test_historical_data_as_of_does_not_change_grant_validity(data_as_of_ns: int) -> None:
    """Moving only the historical selection axis never changes grant validity."""
    request = _request(
        time_axes=_axes(
            data_as_of_ns=data_as_of_ns,
            knowledge_time_ns=data_as_of_ns - 200,
            source_data_time_from_ns=data_as_of_ns - 100,
            source_data_time_to_ns=data_as_of_ns,
            producer_operation_time_ns=data_as_of_ns + 10,
            release_available_at_ns=data_as_of_ns + 20,
        ),
        requested_uses=tuple(
            replace(
                use,
                scope=replace(
                    use.scope,
                    data_time_from_ns=data_as_of_ns - 200,
                    data_time_to_ns=data_as_of_ns + 200,
                ),
            )
            for use in _uses("historical_research")
        ),
    )
    release = _release()
    release = ReleaseEvidence(
        declarations=replace(release.declarations, pit_as_of_ns=data_as_of_ns),
        capabilities=release.capabilities,
    )
    grants = _grants(request, effective_from_ns=NS_2023, effective_until_ns=NS_2025)
    decision = _evaluate(request, grants, release)
    assert decision.code is PreflightRejectionCode.GRANT_EXPIRED_AT_DECISION


def test_grant_not_yet_effective_at_decision_is_refused() -> None:
    """A future effective date does not cover an earlier decision instant."""
    request = _request(time_axes=_axes(decision_at_ns=NS_2024))
    grants = _grants(request, effective_from_ns=NS_2025, effective_until_ns=NS_2027)
    assert _evaluate(request, grants).code is (
        PreflightRejectionCode.GRANT_NOT_EFFECTIVE_AT_DECISION
    )


def test_post_expiry_conditions_are_kept_separate_and_do_not_revive_a_grant() -> None:
    """Written post-expiry terms never extend operational validity."""
    request = _request()
    grants = _grants(
        request,
        effective_until_ns=NS_2025,
        post_expiry_conditions=("renewal-pending", "restricted-read-only"),
    )
    assert _evaluate(request, grants).code is (
        PreflightRejectionCode.GRANT_EXPIRED_AT_DECISION
    )


def test_release_time_axes_do_not_determine_grant_validity() -> None:
    """Producer-owned time axes may vary while grant validity stays unchanged."""
    request = _request(time_axes=_axes(
        producer_operation_time_ns=NS_2023,
        release_available_at_ns=NS_2023 + 1,
        knowledge_time_ns=NS_2023 - 1,
    ))
    assert _evaluate(request, _grants(request), _release()).admitted


def test_knowledge_time_after_data_as_of_is_pit_inconsistency() -> None:
    """A fact known after the selection instant cannot back that selection."""
    request = _request(time_axes=_axes(
        knowledge_time_ns=NS_2025,
        producer_operation_time_ns=NS_2025 + 10,
        release_available_at_ns=NS_2025 + 20,
    ))
    assert _evaluate(request, _grants(request), _release()).code is (
        PreflightRejectionCode.PIT_CONSISTENCY
    )


def test_malformed_time_axes_are_refused() -> None:
    """Missing or inverted time axes carry their own typed code."""
    request = _request(time_axes=_axes(decision_at_ns=0))
    assert _evaluate(request, _grants(request)).code is PreflightRejectionCode.TIME_AXES
    inverted = _request(time_axes=_axes(
        producer_operation_time_ns=NS_2024 + 50,
        release_available_at_ns=NS_2024 + 20,
    ))
    assert _evaluate(inverted, _grants(inverted)).code is PreflightRejectionCode.TIME_AXES


def test_missing_required_pair_is_refused() -> None:
    """A profile pair without a requested use blocks admission."""
    request = _request()
    reduced = replace(request, requested_uses=request.requested_uses[:-1])
    assert _evaluate(reduced, _grants(reduced)).code is PreflightRejectionCode.MISSING_PAIR


def test_no_grant_for_a_pair_is_refused() -> None:
    """A required pair with no matching grant blocks admission."""
    request = _request()
    grants = _grants(request)[:-1]
    assert _evaluate(request, grants).code is PreflightRejectionCode.MISSING_PAIR


def test_wrong_object_id_or_version_is_refused() -> None:
    """A grant for another object instance is not coverage of this pair."""
    request = _request()
    assert _evaluate(request, _grants(request, object_id="object:other")).code is (
        PreflightRejectionCode.OBJECT_IDENTITY
    )
    assert _evaluate(request, _grants(request, object_version="v2")).code is (
        PreflightRejectionCode.OBJECT_IDENTITY
    )


@pytest.mark.parametrize(("field", "value", "code"), [
    ("actor", "other-actor", PreflightRejectionCode.ACTOR_IDENTITY),
    ("licensee", "other-entity", PreflightRejectionCode.LICENSEE_IDENTITY),
    ("recipient_class", "public", PreflightRejectionCode.RECIPIENT_IDENTITY),
    ("recipient_id", "other-recipient", PreflightRejectionCode.RECIPIENT_IDENTITY),
    ("purpose", "academic_research", PreflightRejectionCode.PURPOSE),
    ("territory", "US", PreflightRejectionCode.TERRITORY),
    ("usage_mode", "display", PreflightRejectionCode.USAGE_MODE),
    ("delivery_method", "api", PreflightRejectionCode.DELIVERY_METHOD),
])
def test_wrong_identity_dimension_is_refused(field: str, value: str,
                                             code: PreflightRejectionCode) -> None:
    """Every bound identity dimension is exact on the matching grant."""
    request = _request()
    if field in {"territory", "usage_mode", "delivery_method"}:
        grant_scope = replace(_scope(), **{field: value})
        grants = _grants(request, scope=grant_scope)
    else:
        grants = _grants(request, **{field: value})
    assert _evaluate(request, grants).code is code


def test_source_dataset_and_release_bindings_are_refused() -> None:
    """Source, dataset and release identity cannot be replaced by a grant."""
    request = _request()
    other_source = replace(_scope(), provider="provider-b")
    other_dataset = replace(_scope(), dataset="dataset-d2")
    other_release = replace(_scope(), release_id="release:other")
    for scope, code in (
        (other_source, PreflightRejectionCode.SOURCE_BINDING),
        (other_dataset, PreflightRejectionCode.DATASET_BINDING),
        (other_release, PreflightRejectionCode.DATASET_BINDING),
    ):
        assert _evaluate(request, _grants(request, scope=scope)).code is code


def test_grant_for_other_object_or_unrequested_action_is_irrelevant() -> None:
    """A denial on another object or action cannot affect requested pairs."""
    request = _request()
    extra = _grant(
        RequestedUse(READ_RELEASE, "other_kind", "other", "v9", _scope()),
        decision=GrantDecision.DENY,
        grant_id="grant:other-object",
    )
    unrequested = _grant(
        RequestedUse("create_derived", "derived_artifact", OBJECT_ID, OBJECT_VERSION, _scope()),
        decision=GrantDecision.DENY,
        grant_id="grant:unrequested-action",
    )
    assert _evaluate(request, (*_grants(request), extra, unrequested)).admitted


def test_explicit_deny_for_a_required_pair_is_refused() -> None:
    """An explicit DENY on a required pair blocks admission."""
    request = _request()
    grants = tuple(
        _grant(use, decision=GrantDecision.DENY) if use == request.requested_uses[0]
        else _grant(use)
        for use in request.requested_uses
    )
    assert _evaluate(request, grants).code is PreflightRejectionCode.DENIED_GRANT


def test_ambiguous_overlapping_grants_are_refused() -> None:
    """Two applicable grants for one pair are conflicting authority."""
    request = _request()
    duplicate = replace(_grants(request)[0], grant_id="grant:second")
    decision = _evaluate(request, (*_grants(request), duplicate))
    assert decision.code is PreflightRejectionCode.AMBIGUOUS_GRANTS


def test_grant_iteration_order_does_not_change_the_verdict() -> None:
    """Conflicting grants resolve by fixed precedence, not by list position."""
    request = _request()
    wrong_actor = _grant(
        request.requested_uses[0], grant_id="grant:wrong-actor", actor="other-actor"
    )
    wrong_purpose = _grant(
        request.requested_uses[0], grant_id="grant:wrong-purpose", purpose="academic_research"
    )
    others = tuple(_grant(use) for use in request.requested_uses[1:])
    first = _evaluate(request, (wrong_actor, wrong_purpose, *others))
    second = _evaluate(request, (*others, wrong_purpose, wrong_actor))
    assert first.code is PreflightRejectionCode.ACTOR_IDENTITY
    assert second.code is first.code
    assert first.decision_id == second.decision_id
    permit = _grant(request.requested_uses[0], grant_id="grant:permit-only")
    assert _evaluate(request, (permit, wrong_purpose, *others)).admitted
    assert _evaluate(request, (*others, wrong_purpose, permit)).admitted


def test_duplicate_grant_ids_are_refused() -> None:
    """Duplicate grant identity is ambiguous regardless of pair coverage."""
    request = _request()
    grants = _grants(request)
    duplicate = replace(grants[0])
    assert _evaluate(request, (*grants, duplicate)).code is (
        PreflightRejectionCode.AMBIGUOUS_GRANTS
    )


def test_scope_and_retention_must_be_covered() -> None:
    """Subject sets, data time and retention are separate grant dimensions."""
    request = _request()
    narrow = replace(_scope(), instruments=ScopeSet(ScopeState.NAMED, frozenset()))
    assert _evaluate(request, _grants(request, scope=narrow)).code is (
        PreflightRejectionCode.USE_SCOPE
    )
    late_grant_scope = replace(
        _scope(), data_time_from_ns=NS_2024 - 400, data_time_to_ns=NS_2024 - 300
    )
    assert _evaluate(request, _grants(request, scope=late_grant_scope)).code is (
        PreflightRejectionCode.DATA_TIME
    )
    short = RetentionPolicy(30, "purge:explicit")
    assert _evaluate(request, _grants(request, retention=short)).code is (
        PreflightRejectionCode.RETENTION
    )


def test_missing_producer_capability_is_refused() -> None:
    """A profile requirement needs a matching producer declaration in release."""
    request = _request("intraday_replay_research")
    release = _release(producer_declared_capabilities=frozenset())
    assert _evaluate(request, _grants(request), release).code is (
        PreflightRejectionCode.PRODUCER_CAPABILITY
    )


def test_missing_release_available_capability_is_refused() -> None:
    """A producer declaration alone does not make a release capability available."""
    request = _request("intraday_replay_research")
    release = _release(release_available_capabilities=frozenset())
    assert _evaluate(request, _grants(request), release).code is (
        PreflightRejectionCode.RELEASE_CAPABILITY
    )


def test_nirnay_cannot_assert_source_or_verified_capability() -> None:
    """A consumer declares its requirement; source facts come from release."""
    request = _request(declared_capabilities=frozenset({INTRADAY_CAPABILITY}))
    assert _evaluate(request, _grants(request)).code is (
        PreflightRejectionCode.CAPABILITY_ASSERTION
    )
    verified_release = _release(verified_capabilities=frozenset({INTRADAY_CAPABILITY}))
    valid_request = _request()
    assert _evaluate(valid_request, _grants(valid_request), verified_release).code is (
        PreflightRejectionCode.VERIFIED_CAPABILITY
    )


def test_missing_profile_requirement_declaration_is_refused() -> None:
    """A job cannot omit the capability requirement its profile declares."""
    request = _request("intraday_replay_research", declared_capabilities=frozenset())
    assert _evaluate(request, _grants(request)).code is (
        PreflightRejectionCode.REQUIRED_CAPABILITY
    )


def test_producer_declaration_is_sourced_from_release_evidence() -> None:
    """Changing release evidence changes producer facts, not consumer facts."""
    request = _request("intraday_replay_research")
    release = _release(
        producer_declared_capabilities=frozenset({INTRADAY_CAPABILITY}),
        release_available_capabilities=frozenset({INTRADAY_CAPABILITY}),
    )
    decision = _evaluate(request, _grants(request), release)
    assert decision.admitted
    assert decision.capability_facts.required_capabilities == frozenset(
        {INTRADAY_CAPABILITY}
    )
    assert decision.capability_facts.producer_declared_capabilities == frozenset(
        {INTRADAY_CAPABILITY}
    )


def test_preflight_decision_is_durable_and_distinct_from_a_receipt() -> None:
    """The verdict is an immutable scalar and never an admission receipt."""
    request = _request()
    first = _evaluate(request, _grants(request))
    second = _evaluate(request, _grants(request))
    assert first == second
    assert first.decision_id == second.decision_id
    assert first.kind == "PREFLIGHT_DECISION"
    assert first.is_admission_receipt is False
    assert first.release_verified is False
    assert not isinstance(first, AdmissionReceipt)
    assert first.requested_pair_ids == tuple(sorted(
        f"{use.action}:{use.object_kind}:{use.object_id}:{use.object_version}"
        for use in request.requested_uses
    ))


def test_rejection_is_durable_and_carries_the_typed_code() -> None:
    """A refusal has a stable identity and a typed code."""
    request = _request()
    grants = _grants(request, effective_until_ns=NS_2025)
    first = _evaluate(request, grants)
    second = _evaluate(request, grants)
    assert first == second
    assert first.code is PreflightRejectionCode.GRANT_EXPIRED_AT_DECISION
    assert first.reason
    assert first.release_verified is False


def test_time_axis_names_and_rejection_codes_are_stable() -> None:
    """The six axes and typed refusal vocabulary are explicit."""
    assert TIME_AXIS_NAMES == (
        "data_as_of",
        "source_data_time",
        "producer_operation_time",
        "decision_at",
        "release_available_at",
        "knowledge_time",
    )
    values = [item.value for item in PreflightRejectionCode]
    assert len(values) == len(set(values))
    assert "REJECT_GRANT_EXPIRED_AT_DECISION" in values
    assert "REJECT_MISSING_PAIR" in values
    assert "REJECT_PRODUCER_CAPABILITY" in values
    assert "REJECT_AMBIGUOUS_GRANTS" in values
