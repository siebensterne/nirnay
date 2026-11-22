"""Bitemporal Indian-market reference data and point-in-time dataset building.

Raw provider values are immutable. Listing validity uses valid time plus
knowledge time. Corporate actions are immutable, source-hashed announcements;
adjustment is a derived, versioned view never written over raw prices.
"""
from __future__ import annotations

import hashlib
import json
import os
import tempfile
import fcntl
from dataclasses import asdict, dataclass
from datetime import date, datetime, timezone
from fractions import Fraction
from pathlib import Path
from typing import Any, Iterable, Protocol
from zoneinfo import ZoneInfo

from .features.lineage import (FeatureLineage, assert_feature_lineage, feature_lineage_digest,
                               validate_feature_lineage)
from .market import MarketEvent, canonical_bytes

IST = ZoneInfo("Asia/Kolkata")
DATASET_SIGNATURE_DOMAIN = b"NIRNAY-PIT-DATASET-MANIFEST-SHA256-V1\x00"


class DatasetSigner(Protocol):
    public_key_b64: str
    def sign(self, message: bytes) -> bytes: ...


def _valid_sha256(value: str) -> bool:
    return isinstance(value, str) and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def _utc_date(ns: int) -> date:
    # Integer-second conversion avoids float rounding across an IST midnight.
    return datetime.fromtimestamp(ns // 1_000_000_000, tz=timezone.utc).astimezone(IST).date()


def _feature_observation_time_ns(event: dict[str, Any]) -> int:
    """Select the same observation clock used by the point-in-time builder."""
    for field in ("source_timestamp_ns", "last_trade_time_ns", "received_utc_ns"):
        value = event.get(field)
        if type(value) is int and value > 0:
            return value
    raise ValueError("event has no usable observation timestamp for feature validation")


def _sha_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


@dataclass(frozen=True, slots=True)
class PITListing:
    listing_id: str
    provider: str
    provider_instrument_id: str
    instrument_id: str
    venue: str
    valid_from: date
    valid_to: date | None
    recorded_at_ns: int
    source_id: str
    source_sha256: str
    supersedes_listing_id: str | None = None
    status: str = "active"

    def validate(self) -> None:
        if any(not isinstance(value, str) or not value.strip() for value in
               (self.listing_id, self.provider, self.provider_instrument_id,
                self.instrument_id, self.venue, self.source_id)):
            raise ValueError("listing identities and provenance must be non-empty strings")
        if self.provider not in {"upstox", "fyers"}:
            raise ValueError("unsupported listing provider")
        if self.status not in {"active", "cancelled"}:
            raise ValueError("listing status must be active or cancelled")
        if self.status == "cancelled" and not self.supersedes_listing_id:
            raise ValueError("a cancelled listing must supersede a prior listing")
        if type(self.valid_from) is not date or (self.valid_to is not None and type(self.valid_to) is not date):
            raise ValueError("listing valid dates must be date-only values, not timestamps")
        if self.valid_to is not None and self.valid_to < self.valid_from:
            raise ValueError("listing valid_to precedes valid_from")
        if type(self.recorded_at_ns) is not int or self.recorded_at_ns <= 0:
            raise ValueError("listing recorded_at_ns must be a positive integer epoch nanosecond value")
        if not _valid_sha256(self.source_sha256):
            raise ValueError("listing source_sha256 must be lowercase SHA-256 hex")


@dataclass(frozen=True, slots=True)
class PITUniverseMembership:
    """Bitemporal membership of a canonical instrument in a named universe."""
    membership_id: str
    universe_id: str
    instrument_id: str
    valid_from: date
    valid_to: date | None
    recorded_at_ns: int
    source_id: str
    source_sha256: str
    supersedes_membership_id: str | None = None
    status: str = "active"

    def validate(self) -> None:
        if any(not isinstance(value, str) or not value.strip() for value in
               (self.membership_id, self.universe_id, self.instrument_id, self.source_id)):
            raise ValueError("universe membership identity and provenance must be non-empty strings")
        if self.status not in {"active", "cancelled"}:
            raise ValueError("universe membership status must be active or cancelled")
        if self.status == "cancelled" and not self.supersedes_membership_id:
            raise ValueError("cancelled universe membership must supersede a prior record")
        if type(self.valid_from) is not date or (self.valid_to is not None and type(self.valid_to) is not date):
            raise ValueError("universe membership valid dates must be date-only values, not timestamps")
        if self.valid_to is not None and self.valid_to < self.valid_from:
            raise ValueError("universe membership valid_to precedes valid_from")
        if type(self.recorded_at_ns) is not int or self.recorded_at_ns <= 0:
            raise ValueError("universe membership recorded_at_ns must be positive")
        if not _valid_sha256(self.source_sha256):
            raise ValueError("universe membership source_sha256 must be lowercase SHA-256 hex")


@dataclass(frozen=True, slots=True)
class CorporateAction:
    action_id: str
    instrument_id: str
    kind: str
    announced_at_ns: int
    effective_date: date
    recorded_at_ns: int
    source_retrieved_at_ns: int
    source_id: str
    source_sha256: str
    # New-share / old-share ratio. For a 2-for-1 split: 2/1.
    share_ratio_numerator: int = 1
    share_ratio_denominator: int = 1
    cash_amount_paise: int | None = None
    currency: str = "INR"
    # NSE-style dates are preserved separately: effective_date is the ex-date
    # used for price/share transformations; record_date controls entitlement.
    record_date: date | None = None
    payable_date: date | None = None
    status: str = "announced"
    supersedes_action_id: str | None = None

    def validate(self) -> None:
        if any(not isinstance(value, str) or not value.strip() for value in
               (self.action_id, self.instrument_id, self.kind, self.source_id, self.currency)):
            raise ValueError("corporate-action identity, currency and provenance must be non-empty strings")
        if (type(self.effective_date) is not date or
                (self.record_date is not None and type(self.record_date) is not date) or
                (self.payable_date is not None and type(self.payable_date) is not date)):
            raise ValueError("corporate-action dates must be date-only values, not timestamps")
        if self.kind not in {"split", "bonus", "dividend", "rights", "merger", "demerger", "delisting", "symbol_change", "other"}:
            raise ValueError(f"unsupported corporate-action kind: {self.kind}")
        if self.status not in {"announced", "revised", "cancelled", "effective"}:
            raise ValueError("corporate-action status must be announced, revised, cancelled or effective")
        if self.status in {"revised", "cancelled"} and not self.supersedes_action_id:
            raise ValueError("a revised/cancelled corporate action must supersede a prior action")
        if self.record_date is not None and self.record_date < self.effective_date:
            raise ValueError("record_date must not precede ex/effective date")
        if self.payable_date is not None and self.record_date is not None and self.payable_date < self.record_date:
            raise ValueError("payable_date must not precede record_date")
        if type(self.share_ratio_numerator) is not int or type(self.share_ratio_denominator) is not int or self.share_ratio_numerator <= 0 or self.share_ratio_denominator <= 0:
            raise ValueError("corporate-action ratio must use positive exact integers")
        if type(self.announced_at_ns) is not int or type(self.recorded_at_ns) is not int or type(self.source_retrieved_at_ns) is not int:
            raise ValueError("corporate-action timestamps must be integer epoch nanoseconds")
        if self.cash_amount_paise is not None and (type(self.cash_amount_paise) is not int or self.cash_amount_paise < 0):
            raise ValueError("cash amount must be a non-negative integer number of paise")
        if self.currency != "INR":
            raise ValueError("this India-first pipeline currently accepts INR actions only")
        if self.recorded_at_ns < self.announced_at_ns:
            raise ValueError("recorded_at cannot precede public announcement; backdating is forbidden")
        if self.source_retrieved_at_ns < self.announced_at_ns:
            raise ValueError("source retrieval cannot precede public announcement")
        if self.recorded_at_ns < self.source_retrieved_at_ns:
            raise ValueError("recorded_at cannot precede source retrieval; backdating is forbidden")
        if self.announced_at_ns <= 0 or self.recorded_at_ns <= 0 or self.source_retrieved_at_ns <= 0:
            raise ValueError("corporate-action timestamps must be positive epoch nanoseconds")
        if not _valid_sha256(self.source_sha256):
            raise ValueError("corporate-action source_sha256 must be lowercase SHA-256 hex")


class PITReferenceData:
    def __init__(self, listings: Iterable[PITListing], actions: Iterable[CorporateAction] = (),
                 universe_memberships: Iterable[PITUniverseMembership] = ()):
        self.listings = tuple(listings)
        self.actions = tuple(actions)
        self.universe_memberships = tuple(universe_memberships)
        for row in self.listings: row.validate()
        for action in self.actions: action.validate()
        for membership in self.universe_memberships: membership.validate()
        if len({x.listing_id for x in self.listings}) != len(self.listings):
            raise ValueError("duplicate listing_id")
        if len({x.action_id for x in self.actions}) != len(self.actions):
            raise ValueError("duplicate action_id")
        if len({x.membership_id for x in self.universe_memberships}) != len(self.universe_memberships):
            raise ValueError("duplicate universe membership id")
        self._validate_listing_revisions()
        self._validate_action_revisions()
        self._validate_membership_revisions()

    def _validate_listing_revisions(self) -> None:
        by_id = {x.listing_id: x for x in self.listings}
        superseded: set[str] = set()
        for listing in self.listings:
            parent_id = listing.supersedes_listing_id
            if parent_id is None:
                continue
            parent = by_id.get(parent_id)
            if parent is None:
                raise ValueError(f"listing {listing.listing_id} supersedes an unknown listing")
            if parent_id in superseded:
                raise ValueError("listing revision history must be linear; multiple revisions supersede one listing")
            superseded.add(parent_id)
            if (parent.provider, parent.provider_instrument_id) != (listing.provider, listing.provider_instrument_id):
                raise ValueError("listing revision must retain provider and provider instrument identity")
            if parent.recorded_at_ns >= listing.recorded_at_ns:
                raise ValueError("listing revision must be recorded after the record it supersedes")

    def _validate_membership_revisions(self) -> None:
        by_id = {x.membership_id: x for x in self.universe_memberships}
        superseded: set[str] = set()
        for item in self.universe_memberships:
            parent_id = item.supersedes_membership_id
            if parent_id is None:
                continue
            parent = by_id.get(parent_id)
            if parent is None:
                raise ValueError(f"universe membership {item.membership_id} supersedes an unknown record")
            if parent_id in superseded:
                raise ValueError("universe membership revision history must be linear")
            superseded.add(parent_id)
            if (parent.universe_id, parent.instrument_id) != (item.universe_id, item.instrument_id):
                raise ValueError("universe membership revisions must retain universe and instrument identity")
            if parent.recorded_at_ns >= item.recorded_at_ns:
                raise ValueError("universe membership revision must be recorded after its parent")

    def resolve_membership(self, universe_id: str, instrument_id: str, *, valid_on: date,
                           known_at_ns: int) -> PITUniverseMembership | None:
        visible = [x for x in self.universe_memberships
                   if x.universe_id == universe_id and x.instrument_id == instrument_id
                   and x.recorded_at_ns <= known_at_ns]
        superseded = {x.supersedes_membership_id for x in visible if x.supersedes_membership_id}
        candidates = [x for x in visible if x.membership_id not in superseded
                      and x.status == "active" and x.valid_from <= valid_on
                      and (x.valid_to is None or valid_on <= x.valid_to)]
        if len(candidates) > 1:
            raise ValueError(f"ambiguous point-in-time universe membership for {universe_id}:{instrument_id}")
        return candidates[0] if candidates else None

    def _validate_action_revisions(self) -> None:
        by_id = {x.action_id: x for x in self.actions}
        superseded: set[str] = set()
        for action in self.actions:
            parent_id = action.supersedes_action_id
            if parent_id is None:
                continue
            parent = by_id.get(parent_id)
            if parent is None:
                raise ValueError(f"corporate action {action.action_id} supersedes an unknown action")
            if parent_id in superseded:
                raise ValueError("corporate-action revision history must be linear; multiple revisions supersede one action")
            superseded.add(parent_id)
            if parent.instrument_id != action.instrument_id:
                raise ValueError("corporate-action revision must retain canonical instrument identity")
            if parent.recorded_at_ns >= action.recorded_at_ns:
                raise ValueError("corporate-action revision must be recorded after the record it supersedes")

    def resolve(self, provider: str, provider_instrument_id: str, *, valid_on: date,
                known_at_ns: int) -> PITListing:
        visible = [x for x in self.listings if x.recorded_at_ns <= known_at_ns]
        superseded = {x.supersedes_listing_id for x in visible if x.supersedes_listing_id}
        candidates = [x for x in visible
                      if x.listing_id not in superseded
                      and x.status == "active"
                      and x.provider == provider
                      and x.provider_instrument_id == provider_instrument_id
                      and x.valid_from <= valid_on
                      and (x.valid_to is None or valid_on <= x.valid_to)]
        if len(candidates) != 1:
            reason = "missing" if not candidates else "ambiguous"
            raise ValueError(f"{reason} point-in-time instrument mapping for {provider}:{provider_instrument_id} on {valid_on}")
        return candidates[0]

    def known_actions(self, instrument_id: str, *, known_at_ns: int) -> tuple[CorporateAction, ...]:
        """Return as-known chain heads, including cancellations, at knowledge time."""
        visible = [x for x in self.actions if x.instrument_id == instrument_id and x.recorded_at_ns <= known_at_ns]
        superseded = {x.supersedes_action_id for x in visible if x.supersedes_action_id}
        heads = [x for x in visible if x.action_id not in superseded]
        return tuple(sorted(heads, key=lambda x: (x.effective_date, x.action_id)))

    def action_revision_lineage(self, action: CorporateAction, *, known_at_ns: int) -> tuple[str, ...]:
        visible = {x.action_id: x for x in self.actions if x.recorded_at_ns <= known_at_ns}
        chain = [action.action_id]
        parent = action.supersedes_action_id
        while parent is not None:
            if parent not in visible:
                raise ValueError("corporate-action revision parent is not known at requested knowledge time")
            chain.append(parent)
            parent = visible[parent].supersedes_action_id
        return tuple(reversed(chain))

    def split_adjustment(self, instrument_id: str, *, observation_date: date,
                         decision_knowledge_ns: int) -> tuple[Fraction, tuple[str, ...]]:
        """Return exact factor for prices observed before an announced split/bonus.

        The transformation is a view at the decision's knowledge time. It does
        not edit the captured price. Rights, mergers and demergers are not
        approximated: a caller must supply an independently approved transform.
        """
        factor = Fraction(1, 1)
        lineage: list[str] = []
        for action in self.known_actions(instrument_id, known_at_ns=decision_knowledge_ns):
            if action.status == "cancelled" or action.effective_date <= observation_date:
                continue
            if action.kind in {"split", "bonus"}:
                factor *= Fraction(action.share_ratio_denominator, action.share_ratio_numerator)
                lineage.append(action.action_id)
            elif action.kind in {"rights", "merger", "demerger", "delisting", "symbol_change", "other"}:
                raise ValueError(f"action {action.action_id} ({action.kind}) requires an approved point-in-time transform")
            # Dividends remain cash-flow events; raw price is not dividend-adjusted.
        return factor, tuple(lineage)


@dataclass(frozen=True, slots=True)
class DatasetRow:
    event: dict[str, Any]
    instrument_id: str
    venue: str
    session_date_ist: str
    session_date_source: str
    listing_id: str
    listing_source_sha256: str
    universe_membership_id: str | None
    universe_membership_source_sha256: str | None
    action_lineage: tuple[str, ...]
    action_revision_lineage: tuple[str, ...]
    known_action_ids: tuple[str, ...]
    action_source_hashes: tuple[str, ...]
    split_price_factor_num: int
    split_price_factor_den: int
    point_in_time: bool
    feature_lineage: FeatureLineage | None = None
    feature_lineage_sha256: str | None = None

    def as_dict(self) -> dict[str, Any]:
        return {"event": self.event, "instrument_id": self.instrument_id, "venue": self.venue,
                "session_date_ist": self.session_date_ist,
                "session_date_source": self.session_date_source,
                "listing_id": self.listing_id,
                "listing_source_sha256": self.listing_source_sha256,
                "universe_membership_id": self.universe_membership_id,
                "universe_membership_source_sha256": self.universe_membership_source_sha256,
                "action_lineage": list(self.action_lineage),
                "action_revision_lineage": list(self.action_revision_lineage),
                "known_action_ids": list(self.known_action_ids),
                "action_source_hashes": list(self.action_source_hashes),
                "split_price_factor_num": self.split_price_factor_num,
                "split_price_factor_den": self.split_price_factor_den,
                "point_in_time": self.point_in_time,
                "feature_lineage": None if self.feature_lineage is None
                else self.feature_lineage.as_dict(),
                "feature_lineage_sha256": self.feature_lineage_sha256}


def _source_code_digest() -> str:
    """Digest of the active quantlab source package that built a dataset."""
    from pathlib import Path

    package = Path(__file__).resolve().parent
    digest = hashlib.sha256()
    for path in sorted(package.glob("*.py")):
        digest.update(path.name.encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def _config_digest(**config: Any) -> str:
    return hashlib.sha256(canonical_bytes(config)).hexdigest()


@dataclass(frozen=True, slots=True)
class DatasetManifest:
    schema_version: int
    dataset_id: str
    dataset_origin: str
    created_at_utc_ns: int
    knowledge_policy: str
    timezone: str
    session_date_policy: str
    row_count: int
    providers: tuple[str, ...]
    universe_id: str | None
    universe_membership_filter_applied: bool
    survivorship_control_source_hashes: tuple[str, ...]
    survivorship_completeness_independently_verified: bool
    source_event_hashes: tuple[str, ...]
    source_frame_hashes: tuple[str, ...]
    listing_source_hashes: tuple[str, ...]
    corporate_action_source_hashes: tuple[str, ...]
    point_in_time: bool
    raw_prices_mutated: bool
    adjustment_policy: str
    warnings: tuple[str, ...]
    data_sha256: str
    input_sha256: str
    code_digest: str
    config_digest: str
    feature_lineage: FeatureLineage | None = None
    feature_lineage_sha256: str | None = None

    def as_dict(self) -> dict[str, Any]:
        return asdict(self)


class PITDatasetBuilder:
    def __init__(self, reference_data: PITReferenceData):
        self.reference_data = reference_data

    def build(self, events: Iterable[MarketEvent], *, dataset_id: str,
              created_at_utc_ns: int, max_event_age_ns: int | None = None,
              universe_id: str | None = None,
              input_sha256: str | None = None,
              feature_lineage: FeatureLineage | None = None,
              dataset_origin: str = "SYNTHETIC"
              ) -> tuple[list[DatasetRow], DatasetManifest]:
        if not isinstance(dataset_id, str) or not dataset_id.strip() or type(created_at_utc_ns) is not int or created_at_utc_ns <= 0:
            raise ValueError("dataset_id and positive integer created_at_utc_ns are required")
        if dataset_origin not in {"SYNTHETIC", "MARKET"}:
            raise ValueError("dataset_origin must be SYNTHETIC or MARKET")
        if feature_lineage is not None:
            validate_feature_lineage(feature_lineage, created_at_utc_ns=created_at_utc_ns)
        feature_digest = None
        if feature_lineage is not None:
            feature_digest = feature_lineage_digest(feature_lineage)
        if max_event_age_ns is not None and (type(max_event_age_ns) is not int or max_event_age_ns < 0):
            raise ValueError("max_event_age_ns must be a non-negative integer")
        rows: list[DatasetRow] = []
        providers: set[str] = set()
        seen_event_ids: set[str] = set()
        event_hashes: set[str] = set()
        source_frame_hashes: set[str] = set()
        missing_source_frame_count = 0
        used_listing_hashes: set[str] = set()
        used_action_hashes: set[str] = set()
        used_membership_hashes: set[str] = set()
        warnings: set[str] = set()
        input_hasher = hashlib.sha256()
        if universe_id is None:
            warnings.add("no_point_in_time_universe_membership_filter_applied_survivorship_not_certified")
        elif not universe_id.strip():
            raise ValueError("universe_id must be a non-empty string")
        for event in events:
            input_hasher.update(canonical_bytes(event.as_dict()) + b"\n")
            if event.event_id in seen_event_ids:
                raise ValueError(f"duplicate canonical event_id in dataset input: {event.event_id}")
            seen_event_ids.add(event.event_id)
            if event.received_utc_ns > created_at_utc_ns:
                continue
            if max_event_age_ns is not None and created_at_utc_ns - event.received_utc_ns > max_event_age_ns:
                warnings.add("events_older_than_requested_max_age_were_excluded")
                continue
            # Select the event/session date from the strongest provider-supplied
            # observation clock available. Knowledge time remains local receive
            # time: a late record may not see reference data learned afterwards.
            if event.source_timestamp_ns is not None:
                observation_time_ns = event.source_timestamp_ns
                session_date_source = event.source_timestamp_semantics or "provider_source_timestamp"
            elif event.last_trade_time_ns is not None:
                observation_time_ns = event.last_trade_time_ns
                session_date_source = event.last_trade_time_semantics or "provider_last_trade_time"
            else:
                observation_time_ns = event.received_utc_ns
                session_date_source = "local_receive_time_fallback"
                warnings.add("session_date_uses_local_receive_time_for_events_without_provider_time")
            session_date = _utc_date(observation_time_ns)
            listing = self.reference_data.resolve(event.provider, event.provider_instrument_id,
                                                  valid_on=session_date,
                                                  known_at_ns=event.received_utc_ns)
            if event.venue != listing.venue:
                raise ValueError(
                    f"provider event venue {event.venue!r} conflicts with point-in-time listing venue "
                    f"{listing.venue!r} for {event.provider}:{event.provider_instrument_id}"
                )
            membership = None
            if universe_id is not None:
                membership = self.reference_data.resolve_membership(
                    universe_id, listing.instrument_id, valid_on=session_date,
                    known_at_ns=event.received_utc_ns)
                if membership is None:
                    continue
                used_membership_hashes.add(membership.source_sha256)
            known_actions = self.reference_data.known_actions(
                listing.instrument_id, known_at_ns=event.received_utc_ns)
            factor, lineage = self.reference_data.split_adjustment(
                listing.instrument_id, observation_date=session_date,
                decision_knowledge_ns=event.received_utc_ns)
            known_action_ids = tuple(action.action_id for action in known_actions)
            revision_lineage = tuple(
                action_id
                for action in known_actions
                for action_id in self.reference_data.action_revision_lineage(action, known_at_ns=event.received_utc_ns)
            )
            action_source_hashes = tuple(sorted({
                source_hash
                for action_id in revision_lineage
                for source_hash in [next(a.source_sha256 for a in self.reference_data.actions if a.action_id == action_id)]
            }))
            if feature_lineage is not None:
                validate_feature_lineage(feature_lineage, event_utc_ns=observation_time_ns)
            used_listing_hashes.add(listing.source_sha256)
            used_action_hashes.update(action_source_hashes)
            row = DatasetRow(event=event.as_dict(), instrument_id=listing.instrument_id,
                             venue=listing.venue, session_date_ist=session_date.isoformat(),
                             session_date_source=session_date_source,
                             listing_id=listing.listing_id, listing_source_sha256=listing.source_sha256,
                             universe_membership_id=None if membership is None else membership.membership_id,
                             universe_membership_source_sha256=None if membership is None else membership.source_sha256,
                             action_lineage=lineage,
                             action_revision_lineage=revision_lineage,
                             known_action_ids=known_action_ids, action_source_hashes=action_source_hashes,
                             split_price_factor_num=factor.numerator,
                             split_price_factor_den=factor.denominator, point_in_time=True,
                             feature_lineage=feature_lineage,
                             feature_lineage_sha256=feature_digest)
            rows.append(row)
            providers.add(event.provider)
            event_hashes.add(event.raw_sha256)
            if event.source_frame_sha256 is not None:
                source_frame_hashes.add(event.source_frame_sha256)
            else:
                missing_source_frame_count += 1
            if event.provider == "synthetic" or "synthetic_fixture" in event.quality_flags:
                warnings.add("synthetic_fixture_data_present_not_market_data")
        if missing_source_frame_count:
            warnings.add("some_rows_lack_original_capture_frame_hash")
        rows.sort(key=lambda x: (x.event["received_utc_ns"], x.instrument_id,
                                 x.event["provider"], x.event["event_id"]))
        provenance_input = input_sha256 or input_hasher.hexdigest()
        code_digest = _source_code_digest()
        config_digest = _config_digest(
            dataset_id=dataset_id, dataset_origin=dataset_origin,
            created_at_utc_ns=created_at_utc_ns,
            max_event_age_ns=max_event_age_ns, universe_id=universe_id,
        )
        data_digest = hashlib.sha256(b"\n".join(canonical_bytes(r.as_dict()) for r in rows)).hexdigest()
        listing_hashes = tuple(sorted(used_listing_hashes))
        action_hashes = tuple(sorted(used_action_hashes))
        manifest_body = {
            "schema_version": 1, "dataset_id": dataset_id,
            "dataset_origin": dataset_origin,
            "universe_id": universe_id,
            "universe_membership_filter_applied": universe_id is not None,
            "survivorship_control_source_hashes": sorted(used_membership_hashes),
            "survivorship_completeness_independently_verified": False,
            "created_at_utc_ns": created_at_utc_ns,
            "knowledge_policy": "each observation sees only listing/action records recorded by its receive time",
            "timezone": "Asia/Kolkata", "row_count": len(rows),
            "session_date_policy": "provider_source_timestamp_then_last_trade_time_then_local_receive_fallback; knowledge_time_is_local_receive_time",
            "providers": sorted(providers), "source_event_hashes": sorted(event_hashes),
            "source_frame_hashes": sorted(source_frame_hashes),
            "listing_source_hashes": listing_hashes, "corporate_action_source_hashes": action_hashes,
            "point_in_time": True, "raw_prices_mutated": False,
            "adjustment_policy": "exact split/bonus share-ratio view; dividends remain cash events; complex actions require approved transform",
            "warnings": sorted(warnings), "data_sha256": data_digest,
            "input_sha256": provenance_input,
            "code_digest": code_digest, "config_digest": config_digest,
        }
        dataset_hash = hashlib.sha256(canonical_bytes(manifest_body)).hexdigest()
        manifest = DatasetManifest(
            schema_version=1, dataset_id=f"{dataset_id}:{dataset_hash[:16]}",
            dataset_origin=dataset_origin,
            created_at_utc_ns=created_at_utc_ns,
            knowledge_policy=manifest_body["knowledge_policy"], timezone="Asia/Kolkata",
            session_date_policy=manifest_body["session_date_policy"],
            row_count=len(rows), providers=tuple(sorted(providers)),
            universe_id=universe_id,
            universe_membership_filter_applied=universe_id is not None,
            survivorship_control_source_hashes=tuple(sorted(used_membership_hashes)),
            survivorship_completeness_independently_verified=False,
            source_event_hashes=tuple(sorted(event_hashes)),
            source_frame_hashes=tuple(sorted(source_frame_hashes)),
            listing_source_hashes=listing_hashes, corporate_action_source_hashes=action_hashes,
            point_in_time=True, raw_prices_mutated=False,
            adjustment_policy=manifest_body["adjustment_policy"],
            warnings=tuple(sorted(warnings)), data_sha256=data_digest,
            input_sha256=provenance_input, code_digest=code_digest,
            config_digest=config_digest, feature_lineage=feature_lineage,
            feature_lineage_sha256=feature_digest)
        return rows, manifest


def write_dataset(rows: Iterable[DatasetRow], manifest: DatasetManifest, output_dir: str | Path,
                  *, parquet: bool = False, signer: DatasetSigner | None = None,
                  signing_key_id: str | None = None) -> Path:
    """Atomically publish one immutable dataset directory.

    The destination must not exist (even as an empty directory). Concurrent
    publishers serialize on a sibling lock file; all artifacts are fsynced in
    a staging directory before one atomic rename makes the dataset visible.
    """
    if (signer is None) != (signing_key_id is None):
        raise ValueError("dataset signer and signing_key_id must be supplied together")
    if signing_key_id is not None and (not isinstance(signing_key_id, str) or not signing_key_id.strip()):
        raise ValueError("signing_key_id must be a non-empty string")
    values = list(rows)
    if not values:
        raise ValueError("zero-row datasets are not admitted")
    if len(values) != manifest.row_count:
        raise ValueError("row count differs from dataset manifest")
    content = b"\n".join(canonical_bytes(row.as_dict()) for row in values) + b"\n"
    digest = hashlib.sha256(content.rstrip(b"\n")).hexdigest()
    if digest != manifest.data_sha256:
        raise ValueError("dataset rows differ from the manifest's canonical data digest")

    root = Path(output_dir)
    if not root.name or root == Path(root.anchor):
        raise ValueError("dataset destination must name a child directory")
    parent = root.parent.resolve()
    parent.mkdir(parents=True, exist_ok=True)
    root = parent / root.name
    lock_path = parent / f".{root.name}.publish.lock"
    lock_fd = os.open(lock_path, os.O_RDWR | os.O_CREAT, 0o600)
    staging: Path | None = None
    try:
        fcntl.flock(lock_fd, fcntl.LOCK_EX)
        if root.is_symlink() or root.exists():
            raise FileExistsError("dataset destination already exists; immutable datasets are never overwritten")

        staging = Path(tempfile.mkdtemp(prefix=f".{root.name}.staging-", dir=parent))
        data_path = staging / "events.jsonl"
        with data_path.open("xb") as stream:
            stream.write(content)
            stream.flush()
            os.fsync(stream.fileno())
        artifacts: dict[str, str] = {"events.jsonl": _sha_file(data_path)}
        if parquet:
            try:
                import pyarrow as pa
                import pyarrow.parquet as pq
            except ImportError as exc:
                raise RuntimeError("Parquet export requires the research extra: pip install -e 'research/python[research]' ") from exc
            table = pa.Table.from_pylist([row.as_dict() for row in values])
            metadata = dict(table.schema.metadata or {})
            metadata.update({
                b"nirnay.schema": b"point-in-time-events.v1",
                b"nirnay.dataset_id": manifest.dataset_id.encode("utf-8"),
                b"nirnay.data_sha256": manifest.data_sha256.encode("ascii"),
                b"nirnay.row_count": str(manifest.row_count).encode("ascii"),
            })
            table = table.replace_schema_metadata(metadata)
            parquet_path = staging / "events.parquet"
            pq.write_table(table, parquet_path, compression="zstd", use_dictionary=True,
                           write_statistics=True, row_group_size=65_536,
                           version="2.6", data_page_version="2.0")
            with parquet_path.open("r+b") as stream:
                stream.flush()
                os.fsync(stream.fileno())
            artifacts["events.parquet"] = _sha_file(parquet_path)
        manifest_payload = {
            "dataset": manifest.as_dict(),
            "artifacts": artifacts,
            "publication": "atomic_directory_rename",
        }
        manifest_path = staging / "manifest.json"
        with manifest_path.open("xb") as stream:
            stream.write(canonical_bytes(manifest_payload) + b"\n")
            stream.flush()
            os.fsync(stream.fileno())
        if signer is not None and signing_key_id is not None:
            manifest_bytes = canonical_bytes(manifest_payload)
            manifest_digest = hashlib.sha256(manifest_bytes).digest()
            # Sign a fixed-size digest rather than a potentially large
            # manifest. This fits KMS Ed25519 raw-message limits even when a
            # production dataset contains millions of source hashes.
            signed_bytes = DATASET_SIGNATURE_DOMAIN + manifest_digest
            signature = signer.sign(signed_bytes)
            if not isinstance(signature, bytes) or len(signature) != 64:
                raise ValueError("dataset signer must return a 64-byte Ed25519 signature")
            # Refuse a signer which returned a signature that does not match its
            # advertised public key before publishing an immutable artifact.
            try:
                import base64
                from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
                public_key = base64.b64decode(signer.public_key_b64, validate=True)
                if len(public_key) != 32:
                    raise ValueError("dataset signer public key must be 32-byte Ed25519")
                Ed25519PublicKey.from_public_bytes(public_key).verify(signature, signed_bytes)
            except ImportError as exc:
                raise RuntimeError("dataset signing verification requires the security extra") from exc
            except Exception as exc:
                raise ValueError("dataset signer returned an invalid signature/public-key pair") from exc
            signature_payload = {
                "schema_version": 1, "algorithm": "Ed25519",
                "key_id": signing_key_id,
                "manifest_sha256": manifest_digest.hex(),
                "signature_b64": base64.b64encode(signature).decode("ascii"),
            }
            signature_path = staging / "signature.json"
            with signature_path.open("xb") as stream:
                stream.write(canonical_bytes(signature_payload) + b"\n")
                stream.flush()
                os.fsync(stream.fileno())
        staging_fd = os.open(staging, os.O_RDONLY)
        try:
            os.fsync(staging_fd)
        finally:
            os.close(staging_fd)
        # The lock prevents competing Nirnay publishers from racing this
        # no-overwrite check. `os.rename` is atomic because staging is a sibling.
        if root.exists() or root.is_symlink():
            raise FileExistsError("dataset destination appeared during publication")
        os.rename(staging, root)
        staging = None
        parent_fd = os.open(parent, os.O_RDONLY)
        try:
            os.fsync(parent_fd)
        finally:
            os.close(parent_fd)
    finally:
        if staging is not None and staging.exists():
            import shutil
            shutil.rmtree(staging, ignore_errors=True)
        fcntl.flock(lock_fd, fcntl.LOCK_UN)
        os.close(lock_fd)
    return root


def verify_published_dataset(dataset_dir: str | Path, *,
                             trusted_public_keys: dict[str, str] | None = None) -> dict[str, Any]:
    """Verify a published dataset's file hashes, row count and canonical data hash.

    This checks corruption and internal consistency. It is not a digital
    signature, entitlement check, source-truth attestation or data-quality proof.
    """
    root = Path(dataset_dir)
    if root.is_symlink() or not root.is_dir():
        raise ValueError("dataset path must be a real directory")
    manifest_path = root / "manifest.json"
    data_path = root / "events.jsonl"
    if manifest_path.is_symlink() or data_path.is_symlink() or not manifest_path.is_file() or not data_path.is_file():
        raise ValueError("dataset must contain regular manifest.json and events.jsonl files")
    try:
        publication = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError("dataset publication manifest is unreadable or malformed") from exc
    if not isinstance(publication, dict) or set(publication) != {"dataset", "artifacts", "publication"}:
        raise ValueError("dataset publication manifest has missing or unknown fields")
    if publication["publication"] != "atomic_directory_rename":
        raise ValueError("unsupported dataset publication method")
    dataset = publication["dataset"]
    artifacts = publication["artifacts"]
    if not isinstance(dataset, dict) or not isinstance(artifacts, dict) or "events.jsonl" not in artifacts:
        raise ValueError("dataset manifest or artifact map is invalid")
    expected_manifest_fields = set(DatasetManifest.__dataclass_fields__)
    optional_manifest_fields = {"feature_lineage", "feature_lineage_sha256"}
    if not expected_manifest_fields - optional_manifest_fields <= set(dataset) or not set(
        dataset
    ) <= expected_manifest_fields:
        raise ValueError("dataset manifest fields do not match the active schema contract")
    if dataset.get("schema_version") != 1 or dataset.get("timezone") != "Asia/Kolkata":
        raise ValueError("unsupported dataset manifest schema or timezone")
    if type(dataset.get("created_at_utc_ns")) is not int or dataset["created_at_utc_ns"] <= 0:
        raise ValueError("dataset creation timestamp is invalid")
    if not isinstance(dataset.get("dataset_id"), str) or not dataset["dataset_id"]:
        raise ValueError("dataset identity is invalid")
    if dataset.get("feature_lineage") is not None:
        try:
            assert_feature_lineage(dataset["feature_lineage"],
                                   dataset.get("feature_lineage_sha256"),
                                   created_at_utc_ns=dataset["created_at_utc_ns"])
        except ValueError as exc:
            raise ValueError(f"dataset manifest feature lineage is invalid: {exc}") from exc
    actual_names = {entry.name for entry in root.iterdir()}
    signature_path = root / "signature.json"
    if signature_path.is_symlink():
        raise ValueError("dataset signature sidecar must not be a symlink")
    has_signature = signature_path.exists()
    expected_names = {"manifest.json", *artifacts.keys()} | ({"signature.json"} if has_signature else set())
    if actual_names != expected_names:
        raise ValueError("dataset directory contains missing or unmanifested artifacts")
    allowed_names = {"events.jsonl", "events.parquet"}
    if any(name not in allowed_names for name in artifacts):
        raise ValueError("dataset manifest contains an unknown artifact path")
    for name, expected in artifacts.items():
        if not _valid_sha256(expected):
            raise ValueError(f"invalid SHA-256 for artifact {name}")
        path = root / name
        if path.is_symlink() or not path.is_file() or _sha_file(path) != expected:
            raise ValueError(f"dataset artifact missing, redirected or checksum mismatch: {name}")
    lines = data_path.read_bytes().splitlines()
    if not lines:
        raise ValueError("dataset is empty; zero-row datasets are not admitted")
    try:
        rows = [json.loads(line) for line in lines]
    except json.JSONDecodeError as exc:
        raise ValueError("dataset JSONL contains malformed JSON") from exc
    if any(not isinstance(row, dict) for row in rows):
        raise ValueError("dataset JSONL rows must be objects")
    if dataset.get("row_count") != len(rows):
        raise ValueError("dataset row_count differs from events.jsonl")
    if any(not isinstance(row.get("event"), dict) or row.get("point_in_time") is not True for row in rows):
        raise ValueError("dataset includes rows without canonical event and point-in-time flag")
    for row in rows:
        if row.get("feature_lineage") is None and row.get("feature_lineage_sha256") is None:
            continue
        try:
            assert_feature_lineage(row.get("feature_lineage"),
                                   row.get("feature_lineage_sha256"),
                                   event_utc_ns=_feature_observation_time_ns(row["event"]))
        except ValueError as exc:
            raise ValueError(f"dataset row feature lineage is invalid: {exc}") from exc
    canonical_digest = hashlib.sha256(b"\n".join(canonical_bytes(row) for row in rows)).hexdigest()
    if dataset.get("data_sha256") != canonical_digest:
        raise ValueError("canonical data_sha256 differs from dataset rows")
    for field in ("input_sha256", "code_digest", "config_digest"):
        if not _valid_sha256(dataset.get(field)):
            raise ValueError(f"dataset {field} is not a SHA-256 digest")
    if dataset.get("raw_prices_mutated") is not False or dataset.get("point_in_time") is not True:
        raise ValueError("dataset manifest has an invalid adjustment or knowledge policy")
    if dataset.get("universe_membership_filter_applied") is not (dataset.get("universe_id") is not None):
        raise ValueError("dataset universe filter metadata is inconsistent")
    if dataset.get("survivorship_completeness_independently_verified") is not False:
        raise ValueError("dataset cannot self-certify universe source completeness")
    def row_hashes(path: str) -> list[str]:
        found = set()
        for index, row in enumerate(rows, 1):
            value = row.get(path)
            if value is not None:
                if not _valid_sha256(value):
                    raise ValueError(f"invalid {path} hash in dataset row {index}")
                found.add(value)
        return sorted(found)
    event_hash_values = {row["event"].get("raw_sha256") for row in rows}
    if any(not _valid_sha256(value) for value in event_hash_values):
        raise ValueError("dataset contains an invalid canonical source-event hash")
    source_frame_hash_values = {
        row["event"].get("source_frame_sha256") for row in rows
        if row["event"].get("source_frame_sha256") is not None
    }
    if any(not _valid_sha256(value) for value in source_frame_hash_values):
        raise ValueError("dataset contains an invalid source capture-frame hash")
    expected_hash_sets = {
        "source_event_hashes": sorted(event_hash_values),
        "source_frame_hashes": sorted(source_frame_hash_values),
        "listing_source_hashes": row_hashes("listing_source_sha256"),
        "survivorship_control_source_hashes": row_hashes("universe_membership_source_sha256"),
        "corporate_action_source_hashes": sorted({
            digest for row in rows for digest in row.get("action_source_hashes", [])
        }),
    }
    for field, expected in expected_hash_sets.items():
        if dataset.get(field) != expected:
            raise ValueError(f"dataset manifest {field} does not match published row lineage")
    if dataset["universe_membership_filter_applied"]:
        if any(row.get("universe_membership_id") is None or
               row.get("universe_membership_source_sha256") is None for row in rows):
            raise ValueError("universe-filtered dataset contains rows without PIT membership lineage")
    elif any(row.get("universe_membership_id") is not None for row in rows):
        raise ValueError("unfiltered dataset contains undeclared universe membership rows")

    parquet_verified = False
    if "events.parquet" in artifacts:
        parquet_path = root / "events.parquet"
        try:
            import pyarrow.parquet as pq
        except ImportError as exc:
            raise RuntimeError("Parquet artifact exists but verification requires pyarrow; install research extra") from exc
        try:
            parquet_file = pq.ParquetFile(parquet_path)
            table = parquet_file.read()
            metadata = table.schema.metadata or {}
            if metadata.get(b"nirnay.schema") != b"point-in-time-events.v1":
                raise ValueError("Parquet schema metadata is missing or unsupported")
            if metadata.get(b"nirnay.dataset_id", b"").decode("utf-8") != dataset["dataset_id"]:
                raise ValueError("Parquet dataset_id metadata differs from manifest")
            if metadata.get(b"nirnay.data_sha256", b"").decode("ascii") != canonical_digest:
                raise ValueError("Parquet canonical data digest metadata differs from JSONL")
            if metadata.get(b"nirnay.row_count", b"").decode("ascii") != str(len(rows)):
                raise ValueError("Parquet row_count metadata differs from JSONL")
            parquet_rows = table.to_pylist()
            if len(parquet_rows) != len(rows):
                raise ValueError("Parquet row count differs from canonical JSONL")
            # Canonical semantic equality is essential: artifact SHA alone only
            # proves that the Parquet bytes stayed unchanged, not that they
            # encode the same records as the JSONL source-of-truth artifact.
            for index, (json_row, parquet_row) in enumerate(zip(rows, parquet_rows), 1):
                if canonical_bytes(json_row) != canonical_bytes(parquet_row):
                    raise ValueError(f"Parquet semantic row differs from JSONL at row {index}")
            parquet_verified = True
        except (OSError, ValueError, TypeError) as exc:
            raise ValueError(f"Parquet semantic verification failed: {exc}") from exc

    signature_verified = False
    signature_key_id = None
    if has_signature:
        if not isinstance(trusted_public_keys, dict) or not trusted_public_keys:
            raise RuntimeError("signed dataset verification requires an explicit trusted_public_keys registry")
        try:
            signature = json.loads(signature_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            raise ValueError("dataset signature sidecar is malformed") from exc
        required_signature_fields = {"schema_version", "algorithm", "key_id", "manifest_sha256", "signature_b64"}
        if not isinstance(signature, dict) or set(signature) != required_signature_fields:
            raise ValueError("dataset signature envelope has missing or unknown fields")
        if signature["schema_version"] != 1 or signature["algorithm"] != "Ed25519":
            raise ValueError("unsupported dataset signature algorithm or schema")
        signature_key_id = signature["key_id"]
        if not isinstance(signature_key_id, str) or signature_key_id not in trusted_public_keys:
            raise ValueError("dataset signing key is not present in trusted_public_keys")
        manifest_bytes = canonical_bytes(publication)
        manifest_hash = hashlib.sha256(manifest_bytes).hexdigest()
        if signature["manifest_sha256"] != manifest_hash:
            raise ValueError("dataset signature manifest digest mismatch")
        try:
            import base64
            from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
            public = base64.b64decode(trusted_public_keys[signature_key_id], validate=True)
            signature_bytes = base64.b64decode(signature["signature_b64"], validate=True)
            if len(public) != 32 or len(signature_bytes) != 64:
                raise ValueError("invalid Ed25519 dataset public key or signature length")
            Ed25519PublicKey.from_public_bytes(public).verify(
                signature_bytes, DATASET_SIGNATURE_DOMAIN + bytes.fromhex(manifest_hash))
            signature_verified = True
        except ImportError as exc:
            raise RuntimeError("signed dataset verification requires the security extra") from exc
        except Exception as exc:
            raise ValueError("dataset cryptographic signature verification failed") from exc

    return {"status": "verified_internal_integrity", "row_count": len(rows),
            "dataset_id": dataset.get("dataset_id"), "data_sha256": canonical_digest,
            "artifact_sha256": artifacts, "parquet_semantically_verified": parquet_verified,
            # Keep the original API field while exposing the more explicit name.
            "cryptographic_signature": signature_verified,
            "cryptographic_signature_verified": signature_verified,
            "signature_key_id": signature_key_id,
            "quality_or_entitlement_certified": False}
