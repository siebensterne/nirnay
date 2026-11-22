"""Point-in-time dataset package.

Re-exports the existing flat point-in-time implementation under its target
package name; ``dataset_lineage_from_manifest`` is the new declarative binding
used by the experiment registry. This is not a new dataset service or store.
"""
from __future__ import annotations

from typing import Any

from ..point_in_time import (CorporateAction, DatasetManifest, DatasetRow, PITDatasetBuilder,
                             PITListing, PITReferenceData, PITUniverseMembership,
                             verify_published_dataset, write_dataset)
from .corporate_actions import (SUPPORTED_KINDS, SyntheticCorporateAction,
                                UnsupportedCorporateAction, adjustment_factor,
                                adjustment_factors)
from .universe import UniverseMembershipRow, UniverseSpec

_LINEAGE_FIELDS = (
    "data_sha256",
    "input_sha256",
    "code_digest",
    "config_digest",
    "source_event_hashes",
    "source_frame_hashes",
    "listing_source_hashes",
    "corporate_action_source_hashes",
)


def dataset_lineage_from_manifest(manifest: dict[str, Any] | DatasetManifest) -> dict[str, Any]:
    """Extract the immutable lineage digest block from a dataset manifest."""
    body = manifest.as_dict() if isinstance(manifest, DatasetManifest) else manifest
    if not isinstance(body, dict) or any(field not in body for field in _LINEAGE_FIELDS):
        raise ValueError("dataset manifest is missing lineage digest fields")
    lineage: dict[str, Any] = {}
    for field in _LINEAGE_FIELDS:
        value = body[field]
        lineage[field] = list(value) if isinstance(value, tuple) else value
    return lineage


def dataset_origin_from_manifest(manifest: dict[str, Any] | DatasetManifest) -> str:
    """Return the explicit origin declared by a dataset manifest."""
    body = manifest.as_dict() if isinstance(manifest, DatasetManifest) else manifest
    origin = body.get("dataset_origin") if isinstance(body, dict) else None
    if origin not in {"SYNTHETIC", "MARKET"}:
        raise ValueError("dataset manifest dataset_origin must be SYNTHETIC or MARKET")
    return origin


def dataset_identity_from_manifest(manifest: dict[str, Any] | DatasetManifest) -> dict[str, str]:
    """Return the base dataset id and its versioned identity."""
    body = manifest.as_dict() if isinstance(manifest, DatasetManifest) else manifest
    if not isinstance(body, dict) or not isinstance(body.get("dataset_id"), str):
        raise ValueError("dataset manifest is missing dataset_id")
    versioned = body["dataset_id"]
    if ":" not in versioned:
        raise ValueError("dataset_id must carry a version suffix")
    return {"dataset_id": versioned.rsplit(":", 1)[0], "dataset_version": versioned}


__all__ = [
    "SUPPORTED_KINDS",
    "SyntheticCorporateAction",
    "UnsupportedCorporateAction",
    "UniverseMembershipRow",
    "UniverseSpec",
    "CorporateAction",
    "DatasetManifest",
    "DatasetRow",
    "PITDatasetBuilder",
    "PITListing",
    "PITReferenceData",
    "PITUniverseMembership",
    "adjustment_factor",
    "adjustment_factors",
    "dataset_identity_from_manifest",
    "dataset_lineage_from_manifest",
    "dataset_origin_from_manifest",
    "verify_published_dataset",
    "write_dataset",
]
