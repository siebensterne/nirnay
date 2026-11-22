"""Paper engine consumption of the canonical projection."""
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

from quantlab.cli import main, reference_revision_digest
from quantlab.market import canonical_bytes

import pytest


SRC = Path(__file__).resolve().parents[1] / "src"
FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "canonical-events.example.jsonl"
FIXTURES = Path(__file__).resolve().parent / "fixtures"
PERMIT_REGISTRY = FIXTURES / "registry-replay-permitted.test.json"
DENY_REGISTRY = (
    Path(__file__).resolve().parents[3] / "configs" / "providers" / "source-products.v1.json"
)


def _events(directory: Path, mutate=None) -> Path:
    directory.mkdir(parents=True, exist_ok=True)
    rows = [json.loads(line) for line in FIXTURE.read_text().splitlines()]
    if mutate:
        mutate(rows)
    path = directory / "events.jsonl"
    path.write_text("".join(json.dumps(row, sort_keys=True) + "\n" for row in rows),
                    encoding="utf-8")
    return path


def _run(events: Path, output: Path, reference: Path | None = None) -> int:
    argv = ["paper-replay", "--events", str(events), "--provider", "upstox",
            "--output", str(output), "--registry", str(PERMIT_REGISTRY)]
    if reference:
        argv += ["--reference-revision", str(reference)]
    return main(argv)


@pytest.mark.market_shaped_fixture
def test_paper_replay_consumes_projected_events(tmp_path):
    events = _events(tmp_path / "valid")
    output = tmp_path / "result.json"
    assert _run(events, output) == 0
    result = json.loads(output.read_text())
    assert result["summary"]["event_count"] == 3
    assert result["summary"]["order_count"] == 0, "the null strategy emits no intents"
    assert result["lineage_events"] == 3


@pytest.mark.market_shaped_fixture
def test_unresolved_instrument_and_missing_lineage_fail(tmp_path):
    unresolved = _events(tmp_path / "unresolved",
                         lambda rows: rows[0].update(instrument_id=None))
    assert _run(unresolved, tmp_path / "u.json") == 2

    unlinked = _events(tmp_path / "unlinked",
                       lambda rows: rows[1].update(source_frame_sha256=None))
    assert _run(unlinked, tmp_path / "l.json") == 2


@pytest.mark.market_shaped_fixture
def test_provider_mismatch_fails(tmp_path):
    events = _events(tmp_path / "provider", lambda rows: rows[0].update(provider="fyers"))
    assert main(["paper-replay", "--events", str(events), "--provider", "upstox",
                 "--output", str(tmp_path / "out.json"),
                 "--registry", str(PERMIT_REGISTRY)]) == 2


@pytest.mark.market_shaped_fixture
def test_reference_binding_must_match_projection_sidecar(tmp_path):
    events = _events(tmp_path / "bound")
    body = {"schema_version": 1, "instruments": {}, "venues": {}}
    digest = hashlib.sha256(canonical_bytes(body)).hexdigest()
    body["revision_sha256"] = digest
    reference = tmp_path / "reference.json"
    reference.write_text(json.dumps(body, sort_keys=True), encoding="utf-8")
    assert reference_revision_digest(reference) == digest
    sidecar = events.with_suffix(".jsonl.quality.json")
    sidecar.write_text(json.dumps({
        "reference_revision_sha256": digest,
        "output_sha256": hashlib.sha256(events.read_bytes()).hexdigest(),
    }), encoding="utf-8")
    assert _run(events, tmp_path / "bound.json", reference) == 0
    sidecar.write_text(json.dumps({"reference_revision_sha256": "0" * 64}), encoding="utf-8")
    assert _run(events, tmp_path / "unbound.json", reference) == 2


@pytest.mark.market_shaped_fixture
def test_fresh_process_replay_is_byte_identical(tmp_path):
    events = _events(tmp_path / "replay")
    outputs = []
    for name in ("first", "second"):
        output = tmp_path / f"{name}.json"
        env = dict(os.environ, PYTHONPATH=str(SRC))
        result = subprocess.run(
            [sys.executable, "-m", "quantlab.cli", "paper-replay", "--events", str(events),
             "--provider", "upstox", "--output", str(output),
             "--registry", str(PERMIT_REGISTRY)],
            env=env, capture_output=True, text=True, check=False)
        assert result.returncode == 0, result.stderr
        outputs.append(output.read_bytes())
    assert outputs[0] == outputs[1]


@pytest.mark.market_shaped_fixture
def test_replay_denied_by_registry_emits_no_output(tmp_path):
    events = _events(tmp_path / "denied")
    output = tmp_path / "denied.json"
    assert main(["paper-replay", "--events", str(events), "--provider", "upstox",
                 "--output", str(output), "--registry", str(DENY_REGISTRY)]) == 2
    assert not output.exists(), "denied replay emits no frames or output"
