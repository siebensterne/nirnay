"""Feature lineage as-of/knowledge-time and tamper-detection tests."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

import pytest

pytestmark = pytest.mark.market_shaped_fixture

from quantlab.cli import _load_reference_data
from quantlab.features import (FeatureLineage, feature_definition_digest,
                               feature_lineage_digest)
from quantlab.features.lineage import assert_feature_lineage, feature_lineage_from_dict
from quantlab.market import market_event_from_dict
from quantlab.point_in_time import PITDatasetBuilder, verify_published_dataset, write_dataset

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = ROOT / "research" / "python" / "fixtures"
CREATED_AT = 1_735_789_503_000_000_000


def _events():
    return [market_event_from_dict(json.loads(line)) for line in
            (FIXTURES / "canonical-events.example.jsonl").read_text().splitlines() if line]


def _references():
    return _load_reference_data(FIXTURES / "reference-data.example.json")


def _lineage(as_of: int = 1_735_789_500_000_000_000,
             knowledge: int = 1_735_789_499_000_000_000) -> FeatureLineage:
    return FeatureLineage(feature_version="features.v1",
                          definition_sha256=feature_definition_digest(b"feature-definition-v1"),
                          as_of_utc_ns=as_of, knowledge_utc_ns=knowledge,
                          declared_config={"window": 20})


def _published(tmp_path: Path, lineage: FeatureLineage):
    rows, manifest = PITDatasetBuilder(_references()).build(
        _events(), dataset_id="feature-lineage-fixture", created_at_utc_ns=CREATED_AT,
        universe_id="fixture-universe", feature_lineage=lineage,
    )
    return write_dataset(rows, manifest, tmp_path / "published"), rows, manifest


def test_definition_digest_is_stable_and_rejects_empty():
    digest = feature_definition_digest(b"definition")
    assert digest == hashlib.sha256(b"definition").hexdigest()
    with pytest.raises(ValueError):
        feature_definition_digest(b"")


def test_knowledge_time_after_as_of_rejected():
    with pytest.raises(ValueError, match="knowledge time"):
        _lineage(as_of=100, knowledge=200)


def test_future_as_of_for_event_rejected():
    with pytest.raises(ValueError, match="as-of time"):
        assert_feature_lineage(_lineage().as_dict(), feature_lineage_digest(_lineage()),
                               event_utc_ns=100)


def test_decoded_lineage_rejects_unknown_fields():
    body = _lineage().as_dict()
    body["unexpected"] = True
    with pytest.raises(ValueError, match="unknown fields"):
        feature_lineage_from_dict(body)


def test_builder_binds_feature_lineage_to_rows_and_manifest(tmp_path):
    lineage = _lineage()
    _, rows, manifest = _published(tmp_path, lineage)
    assert rows
    assert all(row.feature_lineage == lineage for row in rows)
    assert all(row.feature_lineage_sha256 == lineage.digest() for row in rows)
    assert manifest.feature_lineage == lineage
    assert manifest.feature_lineage_sha256 == lineage.digest()


def test_published_dataset_detects_tampered_manifest_lineage(tmp_path):
    dataset_dir, _, _ = _published(tmp_path, _lineage())
    assert verify_published_dataset(dataset_dir)["status"] == "verified_internal_integrity"
    path = dataset_dir / "manifest.json"
    payload = json.loads(path.read_text())
    payload["dataset"]["feature_lineage"]["declared_config"] = {"window": 999}
    path.write_text(json.dumps(payload, sort_keys=True), encoding="utf-8")
    with pytest.raises(ValueError, match="feature lineage"):
        verify_published_dataset(dataset_dir)


def test_published_dataset_detects_tampered_row_lineage(tmp_path):
    dataset_dir, _, _ = _published(tmp_path, _lineage())
    events_path = dataset_dir / "events.jsonl"
    rows = [json.loads(line) for line in events_path.read_text().splitlines()]
    rows[0]["feature_lineage"]["definition_sha256"] = "0" * 64
    events_path.write_text("".join(json.dumps(row, sort_keys=True) + "\n" for row in rows),
                           encoding="utf-8")
    manifest_path = dataset_dir / "manifest.json"
    payload = json.loads(manifest_path.read_text())
    payload["artifacts"]["events.jsonl"] = hashlib.sha256(events_path.read_bytes()).hexdigest()
    manifest_path.write_text(json.dumps(payload, sort_keys=True), encoding="utf-8")
    with pytest.raises(ValueError, match="feature lineage"):
        verify_published_dataset(dataset_dir)
