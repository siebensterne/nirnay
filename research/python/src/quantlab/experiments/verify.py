"""Integrity verification for the append-only experiment registry.

The checks are internal-consistency checks only. They prove that stored records
have not been reordered or edited and that an optional head signature verifies
against an explicitly trusted key. They do not certify source completeness,
data rights, alpha or live readiness.
"""
from __future__ import annotations

import base64
import json
from pathlib import Path
from typing import Any, Mapping, Sequence

from ..data import (dataset_identity_from_manifest, dataset_lineage_from_manifest,
                    dataset_origin_from_manifest)
from .cadence import assert_dataset_origin
from .schema import ExperimentError, content_hash


EXPERIMENT_HEAD_SIGNATURE_DOMAIN = b"NIRNAY-EXPERIMENT-REGISTRY-HEAD-V1\x00"


def verify_chain(records: Sequence[Mapping[str, Any]]) -> str:
    """Verify contiguous sequence, predecessor links and per-record digests."""
    previous = "0" * 64
    for expected_sequence, row in enumerate(records, 1):
        if row["sequence"] != expected_sequence or row["previous_hash"] != previous:
            raise ExperimentError(
                f"experiment chain sequence/predecessor mismatch at {expected_sequence}"
            )
        digest = content_hash({"body": row["body"], "previous_hash": row["previous_hash"]})
        if digest != row["record_hash"]:
            raise ExperimentError(f"experiment record hash mismatch at {expected_sequence}")
        if content_hash(row["body"]) != row["body_sha256"]:
            raise ExperimentError(f"experiment body digest mismatch at {expected_sequence}")
        previous = digest
    return previous


def signed_head_message(head_hash: str) -> bytes:
    """Build the fixed-size domain-separated message signed for a registry head."""
    try:
        digest = bytes.fromhex(head_hash)
    except ValueError as exc:
        raise ExperimentError("registry head hash is not hexadecimal") from exc
    if len(digest) != 32:
        raise ExperimentError("registry head hash must be a SHA-256 digest")
    return EXPERIMENT_HEAD_SIGNATURE_DOMAIN + digest


def verify_head_signature(head_hash: str, signature_b64: str, key_id: str,
                          trusted_public_keys: Mapping[str, str] | None) -> bool:
    """Verify a detached Ed25519 head signature against an explicit trust map."""
    if not isinstance(trusted_public_keys, dict) or not trusted_public_keys:
        raise ExperimentError(
            "signed registry verification requires an explicit trusted_public_keys registry"
        )
    if key_id not in trusted_public_keys:
        raise ExperimentError("registry signing key is not present in trusted_public_keys")
    try:
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

        public = base64.b64decode(trusted_public_keys[key_id], validate=True)
        signature = base64.b64decode(signature_b64, validate=True)
        if len(public) != 32 or len(signature) != 64:
            raise ExperimentError("invalid Ed25519 registry public key or signature length")
        Ed25519PublicKey.from_public_bytes(public).verify(signature, signed_head_message(head_hash))
    except ImportError as exc:
        raise ExperimentError(
            "registry signature verification requires the security extra"
        ) from exc
    except (ValueError, TypeError) as exc:
        raise ExperimentError("registry head signature verification failed") from exc
    except Exception as exc:
        raise ExperimentError("registry head signature verification failed") from exc
    return True


def verify_dataset_binding(body: Mapping[str, Any], dataset_dir: str | Path) -> dict[str, Any]:
    """Bind a frozen record to a published immutable dataset directory."""
    from ..point_in_time import verify_published_dataset

    root = Path(dataset_dir)
    manifest_path = root / "manifest.json"
    if manifest_path.is_symlink() or not manifest_path.is_file():
        raise ExperimentError("dataset publication manifest is missing or redirected")
    try:
        publication = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ExperimentError("dataset publication manifest is unreadable") from exc
    if not isinstance(publication, dict) or "dataset" not in publication:
        raise ExperimentError("dataset publication manifest has an invalid shape")
    dataset = publication["dataset"]
    if not isinstance(dataset, dict):
        raise ExperimentError("dataset manifest must be an object")
    if content_hash(publication) != body.get("dataset_manifest_sha256"):
        raise ExperimentError("dataset_manifest_sha256 differs from the published manifest")
    dataset_origin_from_manifest(dataset)
    assert_dataset_origin(body, dataset)
    identity = dataset_identity_from_manifest(dataset)
    if body.get("dataset_id") != identity["dataset_id"]:
        raise ExperimentError("record dataset_id differs from the published manifest")
    if body.get("dataset_version") != identity["dataset_version"]:
        raise ExperimentError("record dataset_version differs from the published manifest")
    if body.get("dataset_lineage") != dataset_lineage_from_manifest(dataset):
        raise ExperimentError("record dataset_lineage differs from the published manifest")
    verified = verify_published_dataset(root)
    return {"status": verified["status"], "dataset_id": identity["dataset_id"],
            "dataset_version": identity["dataset_version"],
            "manifest_sha256": content_hash(publication)}
