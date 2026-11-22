from __future__ import annotations

import json
import pytest
from pathlib import Path

from quantlab.cli import main


def _dump(path: Path, rows):
    path.write_text("".join(json.dumps(row, sort_keys=True) + "\n" for row in rows), encoding="utf-8")


def test_upstox_normalize_cli_links_canonical_events_to_raw_frames(tmp_path):
    raw = tmp_path / "decoded.jsonl"
    mapping = tmp_path / "instrument-map.json"
    output = tmp_path / "canonical.jsonl"
    mapping.write_text(json.dumps({"NSE_EQ|X": "in.x"}), encoding="utf-8")
    payload = {
        "currentTs": "1794372000000",
        "feeds": {"NSE_EQ|X": {"fullFeed": {"marketFF": {
            "ltpc": {"ltp": "100", "ltt": "1794371999000", "ltq": "2"},
            "marketLevel": {"bidAskQuote": [{"bidP": "99.9", "bidQ": "10", "askP": "100.1", "askQ": "11"}]},
            "vtt": "1000",
        }}}},
    }
    _dump(raw, [{"schema_version": 1, "provider": "upstox",
                 "received_at_utc_ns": 1794372000000000000,
                 "received_monotonic_ns": 12345, "capture_sequence": 1,
                 "payload_sha256": "a" * 64, "message": payload}])
    assert main(["normalize", "--provider", "upstox", "--input", str(raw),
                 "--instrument-map", str(mapping), "--output", str(output)]) == 0
    events = [json.loads(line) for line in output.read_text().splitlines()]
    assert len(events) == 1  # First snapshot seeds the last-trade observation baseline.
    assert events[0]["instrument_id"] == "in.x"
    assert events[0]["source_frame_sha256"] == "a" * 64
    report = json.loads(output.with_suffix(".jsonl.quality.json").read_text())
    assert report["output_sha256"]


def test_fyers_normalize_refuses_journal_gap_by_default(tmp_path):
    raw = tmp_path / "fyers.jsonl"
    mapping = tmp_path / "instrument-map.json"
    output = tmp_path / "canonical.jsonl"
    mapping.write_text(json.dumps({"NSE:SBIN-EQ": "in.sbin"}), encoding="utf-8")
    _dump(raw, [{"event_type": "gap", "received_utc_ns": 10,
                 "payload": {"first_missing_local_sequence": 3, "last_missing_local_sequence": 4,
                             "dropped_callback_count": 2}}])
    assert main(["normalize", "--provider", "fyers", "--input", str(raw),
                 "--instrument-map", str(mapping), "--output", str(output)]) == 2
    assert not output.exists()


def test_reconcile_cli_is_deterministic_and_serializes_decimal_results(tmp_path):
    events = tmp_path / "events.jsonl"
    output = tmp_path / "result.json"
    common = {"schema_version": 1, "instrument_id": "in.sbin", "venue": "NSE",
        "event_kind": "quote", "received_utc_ns": 100, "received_monotonic_ns": None,
        "local_sequence": 1, "source_timestamp_ns": None, "source_timestamp_semantics": None,
        "last_trade_time_ns": 90, "last_trade_time_semantics": "last_trade",
        "last_price": "100", "last_quantity": 1, "cumulative_volume": None,
        "aggressor_side": None, "bids": [{"price": "99.9", "quantity": 10, "order_count": None}],
        "asks": [{"price": "100.1", "quantity": 10, "order_count": None}],
        "quality_flags": [], "source_frame_sha256": None}
    rows = [dict(common, event_id="up", provider="upstox", provider_instrument_id="NSE_EQ|X", raw_sha256="a"*64),
            dict(common, event_id="fy", provider="fyers", provider_instrument_id="NSE:X-EQ", raw_sha256="b"*64)]
    _dump(events, rows)
    assert main(["reconcile", "--events", str(events), "--now-utc-ns", "101",
                 "--output", str(output)]) == 0
    report = json.loads(output.read_text())
    assert report["results"][0]["status"] == "consistent_within_tolerance"
    assert report["results"][0]["price_difference_bps"] == "0"


def test_normalize_cli_group_publication_refuses_existing_sidecar_without_partial_output(tmp_path):
    raw = tmp_path / "decoded.jsonl"
    mapping = tmp_path / "instrument-map.json"
    output = tmp_path / "canonical.jsonl"
    sidecar = output.with_suffix(".jsonl.quality.json")
    mapping.write_text(json.dumps({"NSE_EQ|X": "in.x"}), encoding="utf-8")
    payload = {"currentTs": "1794372000000", "feeds": {"NSE_EQ|X": {"fullFeed": {"marketFF": {
        "ltpc": {"ltp": "100", "ltt": "1794371999000", "ltq": "2"},
        "marketLevel": {"bidAskQuote": [{"bidP": "99.9", "bidQ": "10", "askP": "100.1", "askQ": "11"}]},
        "vtt": "1000"}}}}}
    _dump(raw, [{"schema_version": 1, "provider": "upstox",
                 "received_at_utc_ns": 1794372000000000000, "capture_sequence": 1,
                 "payload_sha256": "a" * 64, "message": payload}])
    sidecar.write_text("reserved", encoding="utf-8")
    assert main(["normalize", "--provider", "upstox", "--input", str(raw),
                 "--instrument-map", str(mapping), "--output", str(output)]) == 2
    assert not output.exists()
    assert sidecar.read_text(encoding="utf-8") == "reserved"


def test_jsonl_reader_rejects_blank_lines_as_ambiguous_source_records(tmp_path):
    from quantlab.cli import _read_jsonl
    source = tmp_path / "source.jsonl"
    source.write_text('{"x":1}\n\n', encoding="utf-8")
    with pytest.raises(ValueError, match="blank lines"):
        _read_jsonl(source)


def _decoded_row(sequence, **overrides):
    row = {
        "schema_version": 1, "provider": "upstox",
        "received_at_utc_ns": 1794372000000000000 + sequence,
        "capture_sequence": sequence,
        "payload_sha256": "a" * 64,
        "message": {"currentTs": "1794372000000", "feeds": {}},
    }
    row.update(overrides)
    return row


def test_upstox_normalize_rejects_non_increasing_capture_sequence(tmp_path):
    raw = tmp_path / "decoded.jsonl"
    mapping = tmp_path / "instrument-map.json"
    output = tmp_path / "canonical.jsonl"
    mapping.write_text(json.dumps({}), encoding="utf-8")
    _dump(raw, [_decoded_row(2), _decoded_row(1)])
    assert main(["normalize", "--provider", "upstox", "--input", str(raw),
                 "--instrument-map", str(mapping), "--output", str(output)]) == 2
    assert not output.exists()


def test_upstox_normalize_rejects_missing_capture_sequence(tmp_path):
    raw = tmp_path / "decoded.jsonl"
    mapping = tmp_path / "instrument-map.json"
    output = tmp_path / "canonical.jsonl"
    mapping.write_text(json.dumps({}), encoding="utf-8")
    row = _decoded_row(1)
    row.pop("capture_sequence")
    _dump(raw, [row])
    assert main(["normalize", "--provider", "upstox", "--input", str(raw),
                 "--instrument-map", str(mapping), "--output", str(output)]) == 2
    assert not output.exists()


def test_upstox_normalize_marks_sequence_gaps_not_research_grade(tmp_path):
    raw = tmp_path / "decoded.jsonl"
    mapping = tmp_path / "instrument-map.json"
    output = tmp_path / "canonical.jsonl"
    mapping.write_text(json.dumps({}), encoding="utf-8")
    payload = {"currentTs": "1794372000000", "feeds": {
        "NSE_EQ|X": {"ltpc": {"ltp": "100", "ltt": "1794371999000", "ltq": "1"}}
    }}
    _dump(raw, [_decoded_row(1, message=payload), _decoded_row(3, message=payload)])
    assert main(["normalize", "--provider", "upstox", "--input", str(raw),
                 "--instrument-map", str(mapping), "--output", str(output)]) == 0
    report = json.loads(output.with_suffix(".jsonl.quality.json").read_text())
    assert report["research_grade"] is False
    assert any("capture_sequence_gap" in warning for warning in report["warnings"])
    assert report["ordering_contract"].startswith("capture_sequence")


def test_iter_jsonl_streams_rows_without_materializing(tmp_path):
    from quantlab.cli import _iter_jsonl
    source = tmp_path / "source.jsonl"
    _dump(source, [{"n": 1}, {"n": 2}])
    iterator = _iter_jsonl(source)
    assert next(iterator) == {"n": 1}
    assert next(iterator) == {"n": 2}


def test_upstox_normalize_rolls_back_output_if_report_link_fails(tmp_path, monkeypatch):
    import quantlab.cli as cli
    raw = tmp_path / "decoded.jsonl"
    mapping = tmp_path / "instrument-map.json"
    output = tmp_path / "canonical.jsonl"
    sidecar = output.with_suffix(".jsonl.quality.json")
    mapping.write_text(json.dumps({}), encoding="utf-8")
    payload = {"currentTs": "1794372000000", "feeds": {
        "NSE_EQ|X": {"ltpc": {"ltp": "100", "ltt": "1794371999000", "ltq": "1"}}
    }}
    _dump(raw, [_decoded_row(1, message=payload)])
    real_link = cli.os.link
    calls = []

    def failing_link(source, destination):
        calls.append(destination)
        if len(calls) == 2:
            raise OSError("injected report link failure")
        return real_link(source, destination)

    monkeypatch.setattr(cli.os, "link", failing_link)
    assert main(["normalize", "--provider", "upstox", "--input", str(raw),
                 "--instrument-map", str(mapping), "--output", str(output)]) == 2
    assert len(calls) == 2, "the injected report-link failure actually fired"
    assert not output.exists()
    assert not sidecar.exists()


@pytest.mark.market_shaped_fixture
def test_list_experiments_cli_shows_rejected_runs_and_attempt_counts(tmp_path, capsys):
    from quantlab.experiments import ExperimentRegistry, content_hash, make_attempt
    from test_parameter_search import _configs, _registered

    path = tmp_path / "registry.sqlite3"
    registry = ExperimentRegistry(path)
    registered = _registered()
    registry.register(registered)
    attempts = [make_attempt(config, {"validation_net_mean": 0.0})
                for config in _configs()]
    report = {"candidate_family_size": len(attempts), "status": "rejected_or_unproven"}
    report["report_sha256"] = content_hash(report)
    terminal = {
        **registered,
        "status": "rejected",
        "attempted_configs": attempts,
        "result": {"status": report["status"], "report_sha256": report["report_sha256"]},
        "validation": {"dataset_point_in_time": True, "holdout_used_for_selection": False},
        "results": report,
        "diagnostics": {},
    }
    registry.append_result(terminal)
    assert main(["list-experiments", "--registry", str(path)]) == 0
    payload = json.loads(capsys.readouterr().out)
    assert payload["status"] == "listed"
    assert payload["count"] == 2
    rejected = [item for item in payload["experiments"] if item["status"] == "rejected"]
    assert len(rejected) == 1
    assert rejected[0]["experiment_id"] == "GRID-0001"
    assert rejected[0]["attempted_configs"] == 4
