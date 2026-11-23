# SPDX-License-Identifier: Apache-2.0
"""Versioned consumer-use profiles and permission evaluation.

This module implements the proposed NDC v2 consumer model: a job declares a
use profile and an explicit set of requested actions; each requested action is
then matched against its own permission grant. Purpose, licensee, recipient,
scope, retention and as-of are separate dimensions. Capability claims are kept
separate from permissions and never create an entitlement.

The evaluator checks scalar declarations only. It is not a release verifier:
no detached signature is checked, no canonical manifest bytes are recomputed,
no digest is matched against a file, and no event envelope is inspected. Full
detached-signature verification is a later implementation stage. No right,
release, dataset, data object or experiment is created here.
"""
from __future__ import annotations

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


__all__ = [
    "CONSUMER_OPERATIONS",
    "CONSUMER_USE_MODEL_VERSION",
    "CREATE_DERIVED",
    "INTRADAY_CAPABILITY",
    "PIT_BASIS",
    "PURPOSE",
    "RECIPIENT_CLASS",
    "USE_PROFILES",
    "AdmissionDecision",
    "AdmissionRequest",
    "GrantDecision",
    "PermissionGrant",
    "RejectionCode",
    "ReleaseDeclarations",
    "ReleaseIdentity",
    "ScopeSet",
    "ScopeState",
    "UseProfile",
    "UseScope",
    "evaluate_consumer_use",
    "use_profile",
]
