# SPDX-License-Identifier: Apache-2.0
"""Versioned consumer-use profiles and permission evaluation.

This module implements the proposed NDC v2 consumer model: a job declares a
use profile and an explicit set of requested actions; each requested action is
then matched against its own permission grant. Purpose, licensee, recipient,
scope, retention and as-of are separate dimensions. Capability claims are kept
separate from permissions and never create an entitlement.

The legacy ``evaluate_consumer_use`` entry point checks scalar declarations
only. ``preflight_consumer_admission`` adds the W2.1 C1-C3 conformance rules:
six separate time axes with grant validity at decision time, exact action-
object pair coverage by a unique grant, and four separate capability facts.
Neither entry point is a release verifier: no detached signature is checked,
no canonical manifest bytes are recomputed, no digest is matched against a
file, and no event envelope is inspected. Full detached-signature verification
is a later implementation stage. No right, release, dataset, data object or
experiment is created here.
"""
from __future__ import annotations

import hashlib
import re
from dataclasses import dataclass
from enum import StrEnum

from .dataset_admission import CONSUMER_OPERATIONS

CONSUMER_USE_MODEL_VERSION = 1
PURPOSE = "proprietary_own_account_trading_research"
RECIPIENT_CLASS = "internal"
PIT_BASIS = "knowledge_time"
COVERAGE_VERDICT = "PASS"

# Names from the authoritative consumer enumeration; the values are reused,
# not renamed or extended by this model.
READ_RELEASE = "read_release"
STORE_RESEARCH_COPY = "store_permitted_research_copy"
REPLAY = "replay"
INTERNAL_ANALYTICS = "internal_analytics"
RETAIN_DERIVED_DATA = "retain_permitted_derived_data"
# Creation is a distinct requested action. It is not a value of
# CONSUMER_OPERATIONS and is never inferred from a retention grant.
CREATE_DERIVED = "create_derived"
INTRADAY_CAPABILITY = "intraday_capability"

ACTION_OBJECTS: dict[str, tuple[str, ...]] = {
    READ_RELEASE: ("release_manifest", "event_envelope"),
    STORE_RESEARCH_COPY: ("research_copy",),
    REPLAY: ("research_copy",),
    INTERNAL_ANALYTICS: ("research_dataset",),
    RETAIN_DERIVED_DATA: ("derived_artifact",),
    CREATE_DERIVED: ("derived_artifact",),
}
PLACEHOLDER_RE = re.compile(
    r"\b(?:UNKNOWN|PENDING|FAILED|FAIL|BLOCKED|PROHIBITED|UNVERIFIED|DENIED|"
    r"EXPIRED|REVOKED|NO|NONE|TBD|PLACEHOLDER|UNSPECIFIED)\b",
    re.IGNORECASE,
)
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")


class RejectionCode(StrEnum):
    """Typed, distinct reason a consumer-use request is not admitted."""

    PUBLISHER_IDENTITY = "REJECT_PUBLISHER_IDENTITY"
    SIGNATURE_INTEGRITY = "REJECT_SIGNATURE_INTEGRITY"
    SOURCE_LINEAGE = "REJECT_SOURCE_LINEAGE"
    RIGHTS_EVIDENCE = "REJECT_RIGHTS_EVIDENCE"
    USE_SCOPE = "REJECT_USE_SCOPE"
    COVERAGE = "REJECT_COVERAGE"
    PIT_CONSISTENCY = "REJECT_PIT_CONSISTENCY"
    RETENTION = "REJECT_RETENTION"
    ARTIFACT_DIGESTS = "REJECT_ARTIFACT_DIGESTS"
    SCHEMA_ENVELOPE = "REJECT_SCHEMA_ENVELOPE"
    AMBIGUOUS_GRANTS = "REJECT_AMBIGUOUS_GRANTS"


class ScopeState(StrEnum):
    """Distinct scope states; an empty NAMED set is not ALL."""

    NAMED = "NAMED"
    ALL = "ALL"
    NOT_APPLICABLE = "NOT_APPLICABLE"


class GrantDecision(StrEnum):
    """The decision recorded on one written permission grant."""

    PERMIT = "PERMIT"
    DENY = "DENY"


@dataclass(frozen=True, slots=True)
class AdmissionDecision:
    """Immutable scalar decision with a typed code on every refusal."""

    admitted: bool
    code: RejectionCode | None
    reason: str


@dataclass(frozen=True, slots=True)
class UseProfile:
    """One versioned consumer use with its required actions and capabilities."""

    profile_id: str
    version: int
    required_actions: tuple[str, ...]
    required_capabilities: frozenset[str]

    @property
    def required_pairs(self) -> frozenset[tuple[str, str]]:
        """Return R_p, the exact (action, object-kind) pairs this profile needs."""
        return frozenset(
            (action, object_kind)
            for action in self.required_actions
            for object_kind in ACTION_OBJECTS.get(action, ())
        )


USE_PROFILES: dict[str, UseProfile] = {
    "reference_lookup": UseProfile(
        "reference_lookup",
        CONSUMER_USE_MODEL_VERSION,
        (READ_RELEASE,),
        frozenset(),
    ),
    "historical_research": UseProfile(
        "historical_research",
        CONSUMER_USE_MODEL_VERSION,
        (READ_RELEASE, STORE_RESEARCH_COPY, REPLAY, INTERNAL_ANALYTICS),
        frozenset(),
    ),
    "intraday_replay_research": UseProfile(
        "intraday_replay_research",
        CONSUMER_USE_MODEL_VERSION,
        (READ_RELEASE, STORE_RESEARCH_COPY, REPLAY, INTERNAL_ANALYTICS),
        frozenset({INTRADAY_CAPABILITY}),
    ),
    "derived_research_dataset": UseProfile(
        "derived_research_dataset",
        CONSUMER_USE_MODEL_VERSION,
        (
            READ_RELEASE,
            STORE_RESEARCH_COPY,
            INTERNAL_ANALYTICS,
            RETAIN_DERIVED_DATA,
            CREATE_DERIVED,
        ),
        frozenset(),
    ),
    "portfolio_simulation": UseProfile(
        "portfolio_simulation",
        CONSUMER_USE_MODEL_VERSION,
        (READ_RELEASE, REPLAY, INTERNAL_ANALYTICS),
        frozenset(),
    ),
}


@dataclass(frozen=True, slots=True)
class ScopeSet:
    """A named, all or not-applicable set of instruments, fields or modalities."""

    state: ScopeState
    values: frozenset[str] = frozenset()


@dataclass(frozen=True, slots=True)
class UseScope:
    """Source identity, subject sets and the data-time window of a use."""

    provider: str
    product: str
    venue: str
    segment: str
    instruments: ScopeSet
    fields: ScopeSet
    modalities: ScopeSet
    data_time_from_ns: int | None
    data_time_to_ns: int | None


@dataclass(frozen=True, slots=True)
class RetentionPolicy:
    """Bounded retention duration with an explicit purge rule."""

    max_days: int | None
    purge_rule: str


@dataclass(frozen=True, slots=True)
class ReleaseIdentity:
    """The immutable release identity a consumer request binds to."""

    release_id: str
    manifest_sha256: str
    data_sha256: str
    publisher_identity: str


@dataclass(frozen=True, slots=True)
class AdmissionRequest:
    """A declared consumer request; no default scope, purpose or recipient."""

    release_identity: ReleaseIdentity
    actor: str
    use_profile: str
    requested_actions: tuple[str, ...]
    requested_objects: tuple[str, ...]
    purpose: str
    licensee: str
    recipient: str
    scope: UseScope
    requested_retention: RetentionPolicy
    as_of_ns: int
    declared_capabilities: frozenset[str] = frozenset()


@dataclass(frozen=True, slots=True)
class PermissionGrant:
    """One written grant for exactly one action, object and identity."""

    grant_id: str
    actor: str
    action: str
    data_object: str
    decision: GrantDecision
    purpose: str
    licensee: str
    recipient: str
    scope: UseScope
    retention: RetentionPolicy
    effective_from_ns: int | None
    effective_until_ns: int | None
    evidence_reference: str


@dataclass(frozen=True, slots=True)
class ReleaseDeclarations:
    """Named receipts and versions from earlier verification stages.

    Their presence is checked as a declaration. No receipt is produced by this
    module and no cryptographic verification is performed here.
    """

    schema_version: int
    envelope_version: int
    publisher_identity: str
    signature_receipt: str
    manifest_sha256: str
    artifact_digests: tuple[str, ...]
    source_lineage_reference: str
    rights_evidence_reference: str
    coverage_certificate: str
    coverage_verdict: str
    pit_basis: str
    pit_as_of_ns: int


def use_profile(profile_id: str) -> UseProfile | None:
    """Return the declared profile for ``profile_id`` or None when unknown."""
    return USE_PROFILES.get(profile_id)


def _named(value: object) -> bool:
    """Require a non-empty name that carries no denial or placeholder token."""
    return (
        isinstance(value, str)
        and bool(value.strip())
        and PLACEHOLDER_RE.search(value) is None
    )


def _digest(value: object) -> bool:
    """Require a lowercase hex SHA-256 digest string."""
    return isinstance(value, str) and SHA256_RE.fullmatch(value) is not None


def _reject(code: RejectionCode, reason: str) -> AdmissionDecision:
    """Build one fail-closed decision carrying a typed code."""
    return AdmissionDecision(admitted=False, code=code, reason=reason)


def _check_declarations(request: AdmissionRequest,
                        declarations: ReleaseDeclarations) -> AdmissionDecision | None:
    """Check named release declarations without claiming verification."""
    identity = request.release_identity
    for field in ("schema_version", "envelope_version"):
        value = getattr(declarations, field)
        if type(value) is not int or value <= 0:
            return _reject(RejectionCode.SCHEMA_ENVELOPE, f"{field} is not a positive integer")
    if not _named(declarations.publisher_identity) or (
        declarations.publisher_identity != identity.publisher_identity
    ):
        return _reject(RejectionCode.PUBLISHER_IDENTITY, "publisher identity differs")
    if not _named(declarations.signature_receipt) or not _digest(identity.manifest_sha256) or (
        declarations.manifest_sha256 != identity.manifest_sha256
    ):
        return _reject(RejectionCode.SIGNATURE_INTEGRITY, "no named signature or manifest receipt")
    if not _digest(identity.data_sha256) or not declarations.artifact_digests or not all(
        _digest(item) for item in declarations.artifact_digests
    ):
        return _reject(RejectionCode.ARTIFACT_DIGESTS, "artifact digests are missing or malformed")
    if not _named(declarations.source_lineage_reference):
        return _reject(RejectionCode.SOURCE_LINEAGE, "source lineage is not named")
    if not _named(declarations.rights_evidence_reference):
        return _reject(RejectionCode.RIGHTS_EVIDENCE, "rights evidence is not named")
    if not _named(declarations.coverage_certificate) or (
        declarations.coverage_verdict != COVERAGE_VERDICT
    ):
        return _reject(RejectionCode.COVERAGE, "no passing named coverage certificate")
    if declarations.pit_basis != PIT_BASIS or declarations.pit_as_of_ns != request.as_of_ns:
        return _reject(RejectionCode.PIT_CONSISTENCY, "PIT basis or as-of instant differs")
    return None


def _check_scope_shape(scope: UseScope) -> bool:
    """Require a runtime request with named dimensions and a valid time window."""
    if not all(_named(value) for value in (
        scope.provider, scope.product, scope.venue, scope.segment
    )):
        return False
    for subject in (scope.instruments, scope.fields, scope.modalities):
        if subject.state is not ScopeState.NAMED or not subject.values:
            return False
        if not all(_named(value) for value in subject.values):
            return False
    if scope.data_time_from_ns is not None and type(scope.data_time_from_ns) is not int:
        return False
    if scope.data_time_to_ns is not None and type(scope.data_time_to_ns) is not int:
        return False
    return None in (scope.data_time_from_ns, scope.data_time_to_ns) or (
        scope.data_time_from_ns <= scope.data_time_to_ns
    )


def _check_use(request: AdmissionRequest) -> AdmissionDecision | None:
    """Check profile coverage, declared identity, retention and objects."""
    profile = use_profile(request.use_profile)
    if profile is None or profile.version != CONSUMER_USE_MODEL_VERSION:
        return _reject(RejectionCode.USE_SCOPE, "unknown or stale use profile")
    actions = request.requested_actions
    if not isinstance(actions, tuple) or len(set(actions)) != len(actions) or (
        set(actions) != set(profile.required_actions)
    ):
        return _reject(RejectionCode.USE_SCOPE, "requested actions differ from the profile")
    if request.declared_capabilities != profile.required_capabilities:
        return _reject(RejectionCode.USE_SCOPE, "declared capabilities differ from the profile")
    if request.purpose != PURPOSE or request.recipient != RECIPIENT_CLASS:
        return _reject(RejectionCode.USE_SCOPE, "purpose or recipient is not the declared use")
    if not _named(request.actor) or not _named(request.licensee) or (
        not _named(request.release_identity.release_id)
    ):
        return _reject(RejectionCode.USE_SCOPE, "actor, licensee or release id is not named")
    if type(request.as_of_ns) is not int or request.as_of_ns <= 0:
        return _reject(RejectionCode.PIT_CONSISTENCY, "as_of_ns is not a positive integer")
    if not _check_scope_shape(request.scope):
        return _reject(RejectionCode.USE_SCOPE, "request scope is not a named runtime scope")
    objects = request.requested_objects
    if not isinstance(objects, tuple) or not objects or not all(_named(item) for item in objects):
        return _reject(RejectionCode.USE_SCOPE, "requested objects are not named")
    for action in actions:
        if not any(item in ACTION_OBJECTS[action] for item in objects):
            return _reject(RejectionCode.USE_SCOPE, f"{action} has no requested object")
    retention = request.requested_retention
    if type(retention.max_days) is not int or retention.max_days <= 0 or (
        not _named(retention.purge_rule)
    ):
        return _reject(RejectionCode.RETENTION, "requested retention is unbounded")
    return None


def _set_contained(requested: ScopeSet, granted: ScopeSet) -> bool:
    """Require one scope set to contain the requested named members."""
    if granted.state is ScopeState.ALL:
        return True
    if granted.state is not ScopeState.NAMED or requested.state is not ScopeState.NAMED:
        return False
    return requested.values <= granted.values


def _scope_contained(requested: UseScope, granted: UseScope) -> bool:
    """Require every requested scope dimension to be contained in the grant."""
    for field in ("provider", "product", "venue", "segment"):
        if getattr(requested, field) != getattr(granted, field):
            return False
    for field in ("instruments", "fields", "modalities"):
        if not _set_contained(getattr(requested, field), getattr(granted, field)):
            return False
    for suffix in ("data_time_from_ns", "data_time_to_ns"):
        requested_bound = getattr(requested, suffix)
        granted_bound = getattr(granted, suffix)
        if requested_bound is None and granted_bound is not None:
            return False
        if requested_bound is not None and granted_bound is not None:
            if suffix.endswith("from_ns") and granted_bound > requested_bound:
                return False
            if suffix.endswith("to_ns") and granted_bound < requested_bound:
                return False
    return True


def _effective(grant: PermissionGrant, as_of_ns: int) -> bool:
    """Require the grant decision to be effective at the request as-of instant."""
    if grant.effective_from_ns is not None and as_of_ns < grant.effective_from_ns:
        return False
    return grant.effective_until_ns is None or as_of_ns <= grant.effective_until_ns


def _check_grants(request: AdmissionRequest,
                  grants: tuple[PermissionGrant, ...]) -> AdmissionDecision | None:
    """Evaluate every requested action against exactly one matching grant."""
    by_action: dict[str, list[PermissionGrant]] = {}
    for grant in grants:
        by_action.setdefault(grant.action, []).append(grant)
    if len({grant.grant_id for grant in grants}) != len(grants):
        return _reject(RejectionCode.AMBIGUOUS_GRANTS, "grant ids are not unique")
    for action in request.requested_actions:
        matches = by_action.get(action, [])
        if not matches:
            return _reject(RejectionCode.RIGHTS_EVIDENCE, f"{action} has no grant")
        if len(matches) != 1:
            return _reject(RejectionCode.AMBIGUOUS_GRANTS, f"{action} has no unique grant")
        grant = matches[0]
        if grant.decision is not GrantDecision.PERMIT:
            return _reject(RejectionCode.RIGHTS_EVIDENCE, f"{action} is denied")
        if not _named(grant.evidence_reference) or not _named(grant.grant_id):
            return _reject(RejectionCode.RIGHTS_EVIDENCE, f"{action} has no written evidence")
        if any(getattr(grant, field) != getattr(request, field) for field in (
            "actor", "purpose", "licensee", "recipient"
        )):
            return _reject(RejectionCode.RIGHTS_EVIDENCE, f"{action} grant identity differs")
        if not _effective(grant, request.as_of_ns):
            return _reject(RejectionCode.RIGHTS_EVIDENCE, f"{action} grant is not effective")
        if grant.data_object not in request.requested_objects or (
            grant.data_object not in ACTION_OBJECTS[action]
        ):
            return _reject(RejectionCode.USE_SCOPE, f"{action} grant object is not requested")
        if not _scope_contained(request.scope, grant.scope):
            return _reject(RejectionCode.USE_SCOPE, f"{action} grant scope does not cover")
        granted_retention = grant.retention
        if type(granted_retention.max_days) is not int or (
            granted_retention.max_days <= 0 or not _named(granted_retention.purge_rule)
        ) or request.requested_retention.max_days > granted_retention.max_days:
            return _reject(RejectionCode.RETENTION, f"{action} retention is not covered")
    return None


def evaluate_consumer_use(request: AdmissionRequest,
                          grants: tuple[PermissionGrant, ...],
                          declarations: ReleaseDeclarations) -> AdmissionDecision:
    """Evaluate declarations then every requested action, failing closed."""
    check = _check_declarations(request, declarations)
    if check is not None:
        return check
    check = _check_use(request)
    if check is not None:
        return check
    check = _check_grants(request, grants)
    if check is not None:
        return check
    return AdmissionDecision(admitted=True, code=None, reason="")


# --- W2.1 C1-C3 declaration-level consumer admission preflight ---------------
# The layer below is additive. It keeps the earlier declaration model intact
# and adds a fail-closed preflight that binds six independent time axes, exact
# (action, object) pair coverage and four separate capability facts. It never
# produces an admission receipt and never claims a verified NDC release.

PREFLIGHT_DECISION_KIND = "PREFLIGHT_DECISION"
TIME_AXIS_NAMES: tuple[str, ...] = (
    "data_as_of",
    "source_data_time",
    "producer_operation_time",
    "decision_at",
    "release_available_at",
    "knowledge_time",
)


class PreflightRejectionCode(StrEnum):
    """Typed reason a consumer admission preflight does not pass."""

    SCHEMA_ENVELOPE = "REJECT_SCHEMA_ENVELOPE"
    PUBLISHER_IDENTITY = "REJECT_PUBLISHER_IDENTITY"
    SIGNATURE_INTEGRITY = "REJECT_SIGNATURE_INTEGRITY"
    ARTIFACT_DIGESTS = "REJECT_ARTIFACT_DIGESTS"
    SOURCE_LINEAGE = "REJECT_SOURCE_LINEAGE"
    RIGHTS_EVIDENCE = "REJECT_RIGHTS_EVIDENCE"
    COVERAGE = "REJECT_COVERAGE"
    TIME_AXES = "REJECT_TIME_AXES"
    PIT_CONSISTENCY = "REJECT_PIT_CONSISTENCY"
    USE_SCOPE = "REJECT_USE_SCOPE"
    OBJECT_IDENTITY = "REJECT_OBJECT_IDENTITY"
    MISSING_PAIR = "REJECT_MISSING_PAIR"
    ACTOR_IDENTITY = "REJECT_ACTOR_IDENTITY"
    LICENSEE_IDENTITY = "REJECT_LICENSEE_IDENTITY"
    RECIPIENT_IDENTITY = "REJECT_RECIPIENT_IDENTITY"
    PURPOSE = "REJECT_PURPOSE"
    SOURCE_BINDING = "REJECT_SOURCE_BINDING"
    DATASET_BINDING = "REJECT_DATASET_BINDING"
    TERRITORY = "REJECT_TERRITORY"
    USAGE_MODE = "REJECT_USAGE_MODE"
    DELIVERY_METHOD = "REJECT_DELIVERY_METHOD"
    DATA_TIME = "REJECT_DATA_TIME"
    RETENTION = "REJECT_RETENTION"
    DENIED_GRANT = "REJECT_DENIED_GRANT"
    AMBIGUOUS_GRANTS = "REJECT_AMBIGUOUS_GRANTS"
    GRANT_NOT_EFFECTIVE_AT_DECISION = "REJECT_GRANT_NOT_EFFECTIVE_AT_DECISION"
    GRANT_EXPIRED_AT_DECISION = "REJECT_GRANT_EXPIRED_AT_DECISION"
    REQUIRED_CAPABILITY = "REJECT_REQUIRED_CAPABILITY"
    PRODUCER_CAPABILITY = "REJECT_PRODUCER_CAPABILITY"
    RELEASE_CAPABILITY = "REJECT_RELEASE_CAPABILITY"
    VERIFIED_CAPABILITY = "REJECT_VERIFIED_CAPABILITY"
    CAPABILITY_ASSERTION = "REJECT_CAPABILITY_ASSERTION"


# Fixed precedence for simultaneous dimension failures. It does not depend on
# grant iteration order and defines no "most specific wins" rule.
PREFLIGHT_FAILURE_ORDER: tuple[PreflightRejectionCode, ...] = (
    PreflightRejectionCode.RIGHTS_EVIDENCE,
    PreflightRejectionCode.OBJECT_IDENTITY,
    PreflightRejectionCode.ACTOR_IDENTITY,
    PreflightRejectionCode.LICENSEE_IDENTITY,
    PreflightRejectionCode.RECIPIENT_IDENTITY,
    PreflightRejectionCode.PURPOSE,
    PreflightRejectionCode.SOURCE_BINDING,
    PreflightRejectionCode.DATASET_BINDING,
    PreflightRejectionCode.TERRITORY,
    PreflightRejectionCode.USAGE_MODE,
    PreflightRejectionCode.DELIVERY_METHOD,
    PreflightRejectionCode.USE_SCOPE,
    PreflightRejectionCode.DATA_TIME,
    PreflightRejectionCode.RETENTION,
    PreflightRejectionCode.GRANT_NOT_EFFECTIVE_AT_DECISION,
    PreflightRejectionCode.GRANT_EXPIRED_AT_DECISION,
)


@dataclass(frozen=True, slots=True)
class TimeAxes:
    """Six independent time axes; each keeps its own owner and clock domain.

    ``data_as_of`` selects historical knowledge; ``decision_at`` is when the
    admission decision is evaluated. Grant validity uses only ``decision_at``.
    """

    data_as_of_ns: int
    source_data_time_from_ns: int | None
    source_data_time_to_ns: int | None
    producer_operation_time_ns: int | None
    decision_at_ns: int
    release_available_at_ns: int | None
    knowledge_time_ns: int


@dataclass(frozen=True, slots=True)
class IdentityScope:
    """Source, dataset, territory, usage, delivery and subject scope of a use."""

    provider: str
    source_product: str
    venue: str
    segment: str
    dataset: str
    release_id: str
    territory: str
    usage_mode: str
    delivery_method: str
    instruments: ScopeSet
    fields: ScopeSet
    modalities: ScopeSet
    data_time_from_ns: int | None
    data_time_to_ns: int | None


@dataclass(frozen=True, slots=True)
class RequestedUse:
    """One exact action-object pair requested by a consumer job."""

    action: str
    object_kind: str
    object_id: str
    object_version: str
    scope: IdentityScope


@dataclass(frozen=True, slots=True)
class PreflightGrant:
    """One written grant for an exact action-object pair and full identity."""

    grant_id: str
    decision: GrantDecision
    actor: str
    action: str
    object_kind: str
    object_id: str
    object_version: str
    purpose: str
    licensee: str
    recipient_class: str
    recipient_id: str
    scope: IdentityScope
    retention: RetentionPolicy
    effective_from_ns: int | None
    effective_until_ns: int | None
    evidence_reference: str
    # Kept distinct from operational validity. A written agreement may carry
    # post-expiry terms; those terms never extend or revive the grant.
    post_expiry_conditions: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class ReleaseCapabilityEvidence:
    """Capability facts sourced from release evidence, not from the consumer."""

    producer_declared_capabilities: frozenset[str]
    release_available_capabilities: frozenset[str]
    verified_capabilities: frozenset[str]
    evidence_reference: str


@dataclass(frozen=True, slots=True)
class ReleaseEvidence:
    """Release-owned declarations and capability facts bound to one release."""

    declarations: ReleaseDeclarations
    capabilities: ReleaseCapabilityEvidence


@dataclass(frozen=True, slots=True)
class ConsumerUseRequest:
    """Consumer preflight request; no producer capability is asserted here."""

    release_identity: ReleaseIdentity
    actor: str
    use_profile: str
    requested_uses: tuple[RequestedUse, ...]
    purpose: str
    licensee: str
    recipient_class: str
    recipient_id: str
    time_axes: TimeAxes
    requested_retention: RetentionPolicy
    declared_capabilities: frozenset[str]


@dataclass(frozen=True, slots=True)
class CapabilityFacts:
    """The four separate capability facts, never merged into one claim."""

    required_capabilities: frozenset[str]
    producer_declared_capabilities: frozenset[str]
    release_available_capabilities: frozenset[str]
    verified_capabilities: frozenset[str]


@dataclass(frozen=True, slots=True)
class PreflightDecision:
    """Durable scalar preflight verdict, not a receipt for a verified release.

    ``is_admission_receipt`` and ``release_verified`` are always false. A
    preflight decision records evaluation of declarations and grants only.
    """

    decision_id: str
    admitted: bool
    code: PreflightRejectionCode | None
    reason: str
    decision_at_ns: int
    data_as_of_ns: int
    profile_id: str
    release_id: str
    requested_pair_ids: tuple[str, ...]
    capability_facts: CapabilityFacts
    kind: str = PREFLIGHT_DECISION_KIND
    is_admission_receipt: bool = False
    release_verified: bool = False


def _capability_facts(request: ConsumerUseRequest,
                      release: ReleaseEvidence) -> CapabilityFacts:
    """Collect four capability facts from their separate declared sources."""
    evidence = release.capabilities
    return CapabilityFacts(
        required_capabilities=frozenset(request.declared_capabilities),
        producer_declared_capabilities=frozenset(evidence.producer_declared_capabilities),
        release_available_capabilities=frozenset(evidence.release_available_capabilities),
        verified_capabilities=frozenset(evidence.verified_capabilities),
    )


def _preflight_decision(request: ConsumerUseRequest, admitted: bool,
                        code: PreflightRejectionCode | None, reason: str,
                        facts: CapabilityFacts) -> PreflightDecision:
    """Build one deterministic, durable preflight decision."""
    pair_ids = tuple(sorted(
        f"{use.action}:{use.object_kind}:{use.object_id}:{use.object_version}"
        for use in request.requested_uses
    ))
    basis = "|".join((
        str(request.time_axes.decision_at_ns),
        str(request.time_axes.data_as_of_ns),
        request.use_profile,
        request.release_identity.release_id,
        ",".join(pair_ids),
        "" if code is None else code.value,
        reason,
    ))
    decision_id = "preflight:" + hashlib.sha256(basis.encode("utf-8")).hexdigest()[:16]
    return PreflightDecision(
        decision_id=decision_id,
        admitted=admitted,
        code=code,
        reason=reason,
        decision_at_ns=request.time_axes.decision_at_ns,
        data_as_of_ns=request.time_axes.data_as_of_ns,
        profile_id=request.use_profile,
        release_id=request.release_identity.release_id,
        requested_pair_ids=pair_ids,
        capability_facts=facts,
    )


def _preflight_reject(request: ConsumerUseRequest, code: PreflightRejectionCode,
                      reason: str, facts: CapabilityFacts) -> PreflightDecision:
    """Build one fail-closed preflight decision with a typed code."""
    return _preflight_decision(request, False, code, reason, facts)


def _check_time_axes(axes: TimeAxes) -> PreflightRejectionCode | None:
    """Check axis shape and producer-owned ordering, independent of rights."""
    required = (axes.data_as_of_ns, axes.decision_at_ns, axes.knowledge_time_ns)
    if any(type(value) is not int or value <= 0 for value in required):
        return PreflightRejectionCode.TIME_AXES
    optional = (
        axes.source_data_time_from_ns,
        axes.source_data_time_to_ns,
        axes.producer_operation_time_ns,
        axes.release_available_at_ns,
    )
    if any(value is not None and (type(value) is not int or value <= 0) for value in optional):
        return PreflightRejectionCode.TIME_AXES
    if (
        axes.source_data_time_from_ns is not None
        and axes.source_data_time_to_ns is not None
        and axes.source_data_time_from_ns > axes.source_data_time_to_ns
    ):
        return PreflightRejectionCode.TIME_AXES
    if axes.release_available_at_ns is not None and (
        axes.release_available_at_ns < axes.knowledge_time_ns
    ):
        return PreflightRejectionCode.TIME_AXES
    if axes.producer_operation_time_ns is not None and (
        axes.release_available_at_ns is not None
        and axes.producer_operation_time_ns > axes.release_available_at_ns
    ):
        return PreflightRejectionCode.TIME_AXES
    if axes.decision_at_ns < axes.knowledge_time_ns:
        return PreflightRejectionCode.TIME_AXES
    return None


def _check_preflight_declarations(request: ConsumerUseRequest,
                                  release: ReleaseEvidence) -> PreflightRejectionCode | None:
    """Check named release declarations without claiming cryptographic checks."""
    declarations = release.declarations
    identity = request.release_identity
    for field in ("schema_version", "envelope_version"):
        value = getattr(declarations, field)
        if type(value) is not int or value <= 0:
            return PreflightRejectionCode.SCHEMA_ENVELOPE
    if not _named(declarations.publisher_identity) or (
        declarations.publisher_identity != identity.publisher_identity
    ):
        return PreflightRejectionCode.PUBLISHER_IDENTITY
    if not _named(declarations.signature_receipt) or not _digest(identity.manifest_sha256) or (
        declarations.manifest_sha256 != identity.manifest_sha256
    ):
        return PreflightRejectionCode.SIGNATURE_INTEGRITY
    if not _digest(identity.data_sha256) or not declarations.artifact_digests or not all(
        _digest(item) for item in declarations.artifact_digests
    ):
        return PreflightRejectionCode.ARTIFACT_DIGESTS
    if not _named(declarations.source_lineage_reference):
        return PreflightRejectionCode.SOURCE_LINEAGE
    if not _named(declarations.rights_evidence_reference):
        return PreflightRejectionCode.RIGHTS_EVIDENCE
    if not _named(declarations.coverage_certificate) or (
        declarations.coverage_verdict != COVERAGE_VERDICT
    ):
        return PreflightRejectionCode.COVERAGE
    if not _named(release.capabilities.evidence_reference) or (
        release.capabilities.evidence_reference != declarations.rights_evidence_reference
    ):
        return PreflightRejectionCode.RIGHTS_EVIDENCE
    if declarations.pit_basis != PIT_BASIS or (
        declarations.pit_as_of_ns != request.time_axes.data_as_of_ns
    ):
        return PreflightRejectionCode.PIT_CONSISTENCY
    return None


def _check_identity_scope(scope: IdentityScope) -> bool:
    """Require a named runtime scope with a valid data-time window."""
    named_fields = (
        scope.provider, scope.source_product, scope.venue, scope.segment,
        scope.dataset, scope.release_id, scope.territory,
        scope.usage_mode, scope.delivery_method,
    )
    if not all(_named(value) for value in named_fields):
        return False
    for subject in (scope.instruments, scope.fields, scope.modalities):
        if subject.state is not ScopeState.NAMED or not subject.values:
            return False
        if not all(_named(value) for value in subject.values):
            return False
    for bound in (scope.data_time_from_ns, scope.data_time_to_ns):
        if bound is not None and (type(bound) is not int or bound <= 0):
            return False
    return None in (scope.data_time_from_ns, scope.data_time_to_ns) or (
        scope.data_time_from_ns <= scope.data_time_to_ns
    )


def _check_preflight_use(request: ConsumerUseRequest,
                         profile: UseProfile) -> PreflightRejectionCode | None:
    """Check profile pair coverage, request identity, scope and retention."""
    if request.purpose != PURPOSE:
        return PreflightRejectionCode.PURPOSE
    if request.recipient_class != RECIPIENT_CLASS or not _named(request.recipient_id):
        return PreflightRejectionCode.RECIPIENT_IDENTITY
    if not _named(request.actor):
        return PreflightRejectionCode.ACTOR_IDENTITY
    if not _named(request.licensee):
        return PreflightRejectionCode.LICENSEE_IDENTITY
    if not _named(request.release_identity.release_id):
        return PreflightRejectionCode.DATASET_BINDING
    uses = request.requested_uses
    if not isinstance(uses, tuple) or not uses:
        return PreflightRejectionCode.MISSING_PAIR
    requested_pairs = frozenset((use.action, use.object_kind) for use in uses)
    if len(requested_pairs) != len(uses):
        return PreflightRejectionCode.USE_SCOPE
    missing = profile.required_pairs - requested_pairs
    if missing:
        return PreflightRejectionCode.MISSING_PAIR
    if requested_pairs - profile.required_pairs:
        return PreflightRejectionCode.USE_SCOPE
    for use in uses:
        if not all(_named(value) for value in (
            use.action, use.object_kind, use.object_id, use.object_version
        )):
            return PreflightRejectionCode.OBJECT_IDENTITY
        if not _check_identity_scope(use.scope):
            return PreflightRejectionCode.USE_SCOPE
        if use.scope.release_id != request.release_identity.release_id:
            return PreflightRejectionCode.DATASET_BINDING
        axes = request.time_axes
        if not _interval_contained(
            axes.source_data_time_from_ns, axes.source_data_time_to_ns,
            use.scope.data_time_from_ns, use.scope.data_time_to_ns,
        ):
            return PreflightRejectionCode.DATA_TIME
    requested_retention = request.requested_retention
    if type(requested_retention.max_days) is not int or requested_retention.max_days <= 0 or (
        not _named(requested_retention.purge_rule)
    ):
        return PreflightRejectionCode.RETENTION
    return None


def _interval_contained(requested_from: int | None, requested_to: int | None,
                        granted_from: int | None, granted_to: int | None) -> bool:
    """Require a bounded request to fit inside a possibly bounded window."""
    return (
        not (requested_from is None and granted_from is not None)
        and not (requested_to is None and granted_to is not None)
        and not (
            requested_from is not None
            and granted_from is not None
            and granted_from > requested_from
        )
        and not (
            requested_to is not None
            and granted_to is not None
            and granted_to < requested_to
        )
    )


def _retention_covered(requested: RetentionPolicy, granted: RetentionPolicy) -> bool:
    """Require a bounded grant with an explicit purge rule to cover a request."""
    return (
        type(granted.max_days) is int and granted.max_days > 0
        and _named(granted.purge_rule)
        and type(requested.max_days) is int and requested.max_days > 0
        and requested.max_days <= granted.max_days
    )


def _preflight_capability_check(request: ConsumerUseRequest, profile: UseProfile,
                                facts: CapabilityFacts) -> PreflightRejectionCode | None:
    """Enforce the four separate capability facts at declaration level."""
    if request.declared_capabilities != profile.required_capabilities:
        if profile.required_capabilities - request.declared_capabilities:
            return PreflightRejectionCode.REQUIRED_CAPABILITY
        return PreflightRejectionCode.CAPABILITY_ASSERTION
    if facts.verified_capabilities:
        return PreflightRejectionCode.VERIFIED_CAPABILITY
    for capability in sorted(facts.required_capabilities):
        if capability not in facts.producer_declared_capabilities:
            return PreflightRejectionCode.PRODUCER_CAPABILITY
        if capability not in facts.release_available_capabilities:
            return PreflightRejectionCode.RELEASE_CAPABILITY
    return None


def _grant_dimension_failures(request: ConsumerUseRequest, use: RequestedUse,
                              grant: PreflightGrant,
                              decision_at_ns: int) -> tuple[PreflightRejectionCode, ...]:
    """Return every dimension a pair-matched grant fails, in fixed order."""
    failures: set[PreflightRejectionCode] = set()
    if not _named(grant.grant_id) or not _named(grant.evidence_reference):
        failures.add(PreflightRejectionCode.RIGHTS_EVIDENCE)
    if grant.object_id != use.object_id or grant.object_version != use.object_version:
        failures.add(PreflightRejectionCode.OBJECT_IDENTITY)
    if grant.actor != request.actor:
        failures.add(PreflightRejectionCode.ACTOR_IDENTITY)
    if grant.licensee != request.licensee:
        failures.add(PreflightRejectionCode.LICENSEE_IDENTITY)
    if grant.recipient_class != request.recipient_class or (
        grant.recipient_id != request.recipient_id
    ):
        failures.add(PreflightRejectionCode.RECIPIENT_IDENTITY)
    if grant.purpose != request.purpose:
        failures.add(PreflightRejectionCode.PURPOSE)
    if any(getattr(grant.scope, field) != getattr(use.scope, field) for field in (
        "provider", "source_product", "venue", "segment"
    )):
        failures.add(PreflightRejectionCode.SOURCE_BINDING)
    if any(getattr(grant.scope, field) != getattr(use.scope, field) for field in (
        "dataset", "release_id"
    )):
        failures.add(PreflightRejectionCode.DATASET_BINDING)
    if grant.scope.territory != use.scope.territory:
        failures.add(PreflightRejectionCode.TERRITORY)
    if grant.scope.usage_mode != use.scope.usage_mode:
        failures.add(PreflightRejectionCode.USAGE_MODE)
    if grant.scope.delivery_method != use.scope.delivery_method:
        failures.add(PreflightRejectionCode.DELIVERY_METHOD)
    if any(not _set_contained(getattr(use.scope, field), getattr(grant.scope, field))
           for field in ("instruments", "fields", "modalities")):
        failures.add(PreflightRejectionCode.USE_SCOPE)
    if not _interval_contained(
        use.scope.data_time_from_ns, use.scope.data_time_to_ns,
        grant.scope.data_time_from_ns, grant.scope.data_time_to_ns,
    ):
        failures.add(PreflightRejectionCode.DATA_TIME)
    if not _retention_covered(request.requested_retention, grant.retention):
        failures.add(PreflightRejectionCode.RETENTION)
    if type(grant.effective_from_ns) is not int or type(grant.effective_until_ns) is not int:
        failures.add(PreflightRejectionCode.RIGHTS_EVIDENCE)
    else:
        if decision_at_ns < grant.effective_from_ns:
            failures.add(PreflightRejectionCode.GRANT_NOT_EFFECTIVE_AT_DECISION)
        if decision_at_ns > grant.effective_until_ns:
            failures.add(PreflightRejectionCode.GRANT_EXPIRED_AT_DECISION)
    return tuple(code for code in PREFLIGHT_FAILURE_ORDER if code in failures)


def _check_preflight_grants(request: ConsumerUseRequest,
                            grants: tuple[PreflightGrant, ...],
                            decision_at_ns: int) -> PreflightRejectionCode | None:
    """Require one unique applicable PERMIT grant for every requested pair."""
    if len({grant.grant_id for grant in grants}) != len(grants):
        return PreflightRejectionCode.AMBIGUOUS_GRANTS
    for use in request.requested_uses:
        same_pair_kind = tuple(
            grant for grant in grants
            if grant.action == use.action and grant.object_kind == use.object_kind
        )
        if not same_pair_kind:
            return PreflightRejectionCode.MISSING_PAIR
        if not any(
            (grant.object_id, grant.object_version) == (use.object_id, use.object_version)
            for grant in same_pair_kind
        ):
            return PreflightRejectionCode.OBJECT_IDENTITY
        candidates = tuple(
            grant for grant in same_pair_kind
            if (grant.object_id, grant.object_version) == (use.object_id, use.object_version)
        )
        if any(grant.decision is GrantDecision.DENY for grant in candidates):
            return PreflightRejectionCode.DENIED_GRANT
        failures_by_grant = tuple(
            _grant_dimension_failures(request, use, grant, decision_at_ns)
            for grant in candidates
        )
        applicable = tuple(
            grant for grant, failures in zip(candidates, failures_by_grant, strict=True)
            if not failures
        )
        if not applicable:
            for code in PREFLIGHT_FAILURE_ORDER:
                if any(code in failures for failures in failures_by_grant):
                    return code
            return PreflightRejectionCode.RIGHTS_EVIDENCE
        if len(applicable) != 1:
            return PreflightRejectionCode.AMBIGUOUS_GRANTS
    return None


def preflight_consumer_admission(
    request: ConsumerUseRequest,
    grants: tuple[PreflightGrant, ...],
    release: ReleaseEvidence,
) -> PreflightDecision:
    """Evaluate declaration-level consumer admission, failing closed.

    The six time axes stay separate. Grant validity is evaluated at
    ``decision_at``; a historical ``data_as_of`` does not revive or extend a
    grant. The verdict is a preflight decision, not an admission receipt.
    """
    facts = _capability_facts(request, release)
    check = _check_preflight_declarations(request, release)
    if check is not None:
        return _preflight_reject(request, check, "release declarations are refused", facts)
    check = _check_time_axes(request.time_axes)
    if check is not None:
        return _preflight_reject(request, check, "time axes are inconsistent", facts)
    if request.time_axes.knowledge_time_ns > request.time_axes.data_as_of_ns:
        return _preflight_reject(
            request, PreflightRejectionCode.PIT_CONSISTENCY,
            "knowledge time is after the data-as-of instant", facts,
        )
    profile = use_profile(request.use_profile)
    if profile is None:
        return _preflight_reject(
            request, PreflightRejectionCode.USE_SCOPE, "unknown use profile", facts,
        )
    check = _check_preflight_use(request, profile)
    if check is not None:
        return _preflight_reject(request, check, "requested use is refused", facts)
    check = _preflight_capability_check(request, profile, facts)
    if check is not None:
        return _preflight_reject(request, check, "capability facts are refused", facts)
    check = _check_preflight_grants(
        request, grants, request.time_axes.decision_at_ns
    )
    if check is not None:
        return _preflight_reject(request, check, "grant coverage is refused", facts)
    return _preflight_decision(request, True, None, "", facts)


__all__ = [
    "CONSUMER_OPERATIONS",
    "CONSUMER_USE_MODEL_VERSION",
    "CREATE_DERIVED",
    "INTRADAY_CAPABILITY",
    "PIT_BASIS",
    "PREFLIGHT_DECISION_KIND",
    "PREFLIGHT_FAILURE_ORDER",
    "PURPOSE",
    "RECIPIENT_CLASS",
    "TIME_AXIS_NAMES",
    "USE_PROFILES",
    "AdmissionDecision",
    "AdmissionRequest",
    "CapabilityFacts",
    "ConsumerUseRequest",
    "GrantDecision",
    "IdentityScope",
    "PermissionGrant",
    "PreflightDecision",
    "PreflightGrant",
    "PreflightRejectionCode",
    "ReleaseCapabilityEvidence",
    "ReleaseDeclarations",
    "ReleaseEvidence",
    "ReleaseIdentity",
    "RequestedUse",
    "RejectionCode",
    "ScopeSet",
    "ScopeState",
    "TimeAxes",
    "UseProfile",
    "UseScope",
    "evaluate_consumer_use",
    "preflight_consumer_admission",
    "use_profile",
]
