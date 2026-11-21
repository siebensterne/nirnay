"""Append-only experiment registry contract and holdout-discipline tests."""
from __future__ import annotations

import base64
import csv
import json
import sqlite3
import threading
from dataclasses import asdict
from pathlib import Path

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

from quantlab.alpha_search import AlphaCosts, Bar, default_candidate_family
from quantlab.cli import main
from quantlab.experiments import (DuplicateExperimentError, ExperimentError, ExperimentRegistry,
                                  HoldoutReuseError, build_search_space, content_hash, holdout_key,
                                  make_attempt)
from quantlab.experiments.cadence import validate_bar_cadence
from quantlab.experiments.schema import (NOT_APPLICABLE_INTERVAL,
                                         SYNTHETIC_ANNUALIZATION_BASIS,
                                         validate_registered_experiment)
from quantlab.experiments.search_adapter import run_registered_search
from quantlab.point_in_time import PITDatasetBuilder

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = ROOT / "research" / "python" / "fixtures"
DATASET_ID = "synthetic:minute-bars"
DATASET_VERSION = DATASET_ID + ":" + "a" * 16
MINUTE_INTERVAL_NS = 60_000_000_000
DAILY_INTERVAL_NS = 86_400_000_000_000


def _lineage() -> dict:
    return {"data_sha256": "a" * 64, "input_sha256": "b" * 64,
            "code_digest": "c" * 64, "config_digest": "d" * 64,
            "source_event_hashes": ["e" * 64], "source_frame_hashes": ["f" * 64],
            "listing_source_hashes": ["1" * 64], "corporate_action_source_hashes": []}


def _configs(count: int = 4) -> list[dict]:
    return [{"family": "momentum", "lookback": value, "threshold_bps": 0.0}
            for value in range(3, 3 + count)]


def _registered(experiment_id: str = "EXP-0001", *, counts: int = 4) -> dict:
    validation = {"start_index": 361, "end_index": 480}
    holdout = {"start_index": 481, "end_index": 600}
    train = {"start_index": 1, "end_index": 360}
    configs = _configs(counts)
    space = build_search_space("momentum", configs, train_period=train,
                               candidate_configs=configs)
    costs = asdict(AlphaCosts())
    return {
        "experiment_id": experiment_id,
        "hypothesis": "Pre-registered momentum screen; not an alpha assertion.",
        "economic_rationale": "Trend persistence is hypothesized but not established.",
        "dataset_ref": "synthetic:minute-bars:seed-20261121",
        "dataset_manifest_sha256": "9" * 64,
        "dataset_point_in_time": True,
        "code_revision": "test-revision-0001",
        "config": {"lookback": 5, "synthetic": True},
        "status": "registered",
        "registered_at_utc": "2026-11-12T00:00:00Z",
        "falsification_rule": "Reject if net holdout expectancy is not positive after costs.",
        "dataset_id": DATASET_ID,
        "dataset_version": DATASET_VERSION,
        "dataset_lineage": _lineage(),
        "universe_id": "fixture-universe",
        "feature_version": "features.v1",
        "strategy_code_hash": "7" * 64,
        "search_space": space,
        "attempted_configs": [],
        "validation_period": validation,
        "holdout_period": holdout,
        "holdout_key": holdout_key(DATASET_VERSION, "fixture-universe", "features.v1",
                                   validation, holdout),
        "cost_model": {"assumptions": costs, "cost_model_sha256": content_hash(costs)},
        "execution_assumptions": {"execution_latency_model": "not_modelled",
                                  "queue_model": "not_modelled",
                                  "liquidity_model": "not_calibrated"},
        "bar_kind": "minute",
        "bar_interval_ns": MINUTE_INTERVAL_NS,
        "timestamp_semantics": "bar_close",
        "calendar_id": "synthetic:continuous-minute-v1",
        "session_definition": "Synthetic continuous minute bars; no exchange session",
        "annualization_basis": SYNTHETIC_ANNUALIZATION_BASIS,
        "periods_per_year": 525_600.0,
        "point_in_time_basis": "chronological_only",
    }


def _report(counts: int) -> dict:
    report = {"schema_version": 1, "status": "rejected_or_unproven",
              "warning": "Research screening only.", "candidate_family_size": counts,
              "candidate_scores": [{"config": config, "validation_net_mean": 0.0,
                                    "validation_holm_p": 1.0, "holdout_net_mean": 0.0}
                                   for config in _configs(counts)],
              "selected": _configs(counts)[0], "untouched_holdout_net_mean": 0.0,
              "multiple_testing_diagnostics": {
                  "cscv_pbo": {"uses_final_holdout": False},
                  "deflated_sharpe_validation_only": {"trials": counts}}}
    report["report_sha256"] = content_hash(report)
    return report


def _terminal(registered: dict, *, status: str = "completed", counts: int = 4) -> dict:
    report = _report(counts)
    attempts = [make_attempt(config, {"validation_net_mean": 0.0, "selected": False})
                for config in _configs(counts)]
    return {**registered, "status": status, "attempted_configs": attempts,
            "result": {"status": report["status"], "report_sha256": report["report_sha256"]},
            "validation": {"dataset_point_in_time": True, "holdout_used_for_selection": False},
            "results": report, "diagnostics": report["multiple_testing_diagnostics"]}


def _bars() -> list[Bar]:
    with (FIXTURES / "alpha-bars-minute.seed-20261121.csv").open(
        newline="", encoding="utf-8"
    ) as stream:
        return [Bar(int(row["timestamp_ns"]), float(row["close"]))
                for row in csv.DictReader(stream)]


def test_extended_record_freezes_deterministically():
    from quantlab.experiments import freeze_registered_experiment

    first, digest = freeze_registered_experiment(_registered())
    second, second_digest = freeze_registered_experiment(_registered())
    assert first == second
    assert digest == second_digest
    assert len(digest) == 64


def test_unknown_and_nonfinite_fields_still_rejected():
    from quantlab.experiments import validate_experiment

    record = _registered()
    record["unregistered_field"] = "not in schema"
    with pytest.raises(ExperimentError):
        validate_experiment(record)
    record = _registered()
    record["config"] = {"threshold": float("nan")}
    with pytest.raises(ExperimentError):
        validate_experiment(record)


def test_append_is_hash_chained_and_sequence_contiguous(tmp_path):
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    first = registry.register(_registered())
    registry.append_result(_terminal(_registered()))
    rows = registry.records()
    assert [row.sequence for row in rows] == [1, 2]
    assert rows[0].previous_hash == "0" * 64
    assert rows[1].previous_hash == first.record_hash


def test_verify_detects_tampered_body_and_reordered_records(tmp_path):
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    registry.register(_registered())
    registry.append_result(_terminal(_registered()))
    path = registry.path
    connection = sqlite3.connect(path)
    connection.execute("DROP TRIGGER experiment_records_no_update")
    connection.execute(
        "UPDATE experiment_records SET body_json=REPLACE(body_json, 'fixture-universe', "
        "'tampered-universe') WHERE sequence=1"
    )
    connection.commit()
    connection.close()
    with pytest.raises(ExperimentError, match="hash mismatch"):
        registry.verify_integrity()

    registry = ExperimentRegistry(tmp_path / "chain.sqlite3")
    registry.register(_registered())
    connection = sqlite3.connect(registry.path)
    connection.execute("DROP TRIGGER experiment_records_no_update")
    connection.execute(
        "UPDATE experiment_records SET previous_hash=? WHERE sequence=1", ("1" * 64,)
    )
    connection.commit()
    connection.close()
    with pytest.raises(ExperimentError, match="predecessor mismatch"):
        registry.verify_integrity()


def test_storage_rejects_update_and_delete(tmp_path):
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    registry.register(_registered())
    connection = sqlite3.connect(registry.path)
    with pytest.raises(sqlite3.IntegrityError):
        connection.execute("UPDATE experiment_records SET status='candidate'")
    with pytest.raises(sqlite3.IntegrityError):
        connection.execute("DELETE FROM experiment_records")
    with pytest.raises(sqlite3.IntegrityError):
        connection.execute("DELETE FROM holdout_claims")
    connection.close()
    assert registry.verify_integrity()["record_count"] == 1


def test_duplicate_experiment_id_rejected(tmp_path):
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    registry.register(_registered())
    with pytest.raises(DuplicateExperimentError):
        registry.register(_registered())


def test_holdout_reuse_rejected_after_terminal_record(tmp_path):
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    registry.register(_registered("EXP-0001"))
    registry.append_result(_terminal(_registered("EXP-0001")))
    with pytest.raises(HoldoutReuseError):
        registry.register(_registered("EXP-0002"))


def test_universe_version_and_feature_digest_participate_in_holdout_key():
    from quantlab.experiments import validate_registered_experiment

    record = _registered()
    record["universe_version"] = "universe.v2"
    record["feature_digest"] = "5" * 64
    record["holdout_key"] = holdout_key(
        record["dataset_version"], record["universe_id"], record["feature_version"],
        record["validation_period"], record["holdout_period"],
        record["universe_version"], record["feature_digest"],
    )
    validate_registered_experiment(record)
    record["universe_version"] = "universe.v3"
    with pytest.raises(ExperimentError, match="holdout_key"):
        validate_registered_experiment(record)


def test_concurrent_registration_claims_holdout_once(tmp_path):
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    barrier = threading.Barrier(2)
    results: list[str | None] = [None, None]

    def attempt(index: int) -> None:
        local = ExperimentRegistry(tmp_path / "registry.sqlite3")
        barrier.wait(timeout=10)
        try:
            local.register(_registered(f"EXP-{index:04d}"))
            results[index] = "ok"
        except ExperimentError as exc:
            results[index] = type(exc).__name__

    threads = [threading.Thread(target=attempt, args=(index,)) for index in range(2)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=15)
    assert results.count("ok") == 1
    assert "HoldoutReuseError" in results


def test_verification_only_claim_requires_replicates(tmp_path):
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    record = _registered("EXP-0002")
    record["holdout_use"] = "verification_only"
    with pytest.raises(ExperimentError, match="replicates"):
        registry.claim_holdout(record, claim_kind="verification_only")
    registry.register(_registered("EXP-0001"))
    with pytest.raises(ExperimentError, match="replicates"):
        registry.claim_holdout(record, replicates="EXP-9999",
                               claim_kind="verification_only")
    record["replicates"] = "EXP-0001"
    claim = registry.claim_holdout(record, claim_kind="verification_only")
    assert claim.claim_kind == "verification_only"
    assert registry.verify_integrity()["holdout_claims"] == 2


def test_validation_holdout_overlap_rejected():
    record = _registered()
    record["holdout_period"] = {"start_index": 330, "end_index": 420}
    with pytest.raises(ExperimentError, match="overlap"):
        from quantlab.experiments import validate_registered_experiment
        validate_registered_experiment(record)


def test_nonchronological_periods_rejected():
    record = _registered()
    from quantlab.experiments import build_search_space

    record["search_space"] = build_search_space(
        "momentum", _configs(), train_period={"start_index": 400, "end_index": 440}
    )
    with pytest.raises(ExperimentError, match="overlap"):
        from quantlab.experiments import validate_registered_experiment
        validate_registered_experiment(record)


def test_leaked_dataset_cannot_be_candidate(tmp_path):
    record = _terminal(_registered(), status="candidate")
    record["dataset_point_in_time"] = False
    with pytest.raises(ExperimentError, match="leakage"):
        ExperimentRegistry(tmp_path / "registry.sqlite3").append_result(record)


def test_terminal_requires_result_validation(tmp_path):
    record = _registered()
    record["status"] = "completed"
    with pytest.raises(ExperimentError, match="terminal experiment status requires result object"):
        ExperimentRegistry(tmp_path / "registry.sqlite3").append_result(record)


def test_all_attempted_configs_persisted_including_losers(tmp_path):
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    record = _registered(counts=25)
    family = [asdict(config) for config in default_candidate_family()]
    record["search_space"] = build_search_space(
        "momentum", family,
        train_period={"start_index": 1, "end_index": 360},
    )
    record["search_space"]["candidate_configs"] = family
    record["search_space"]["search_space_sha256"] = content_hash(
        {key: value for key, value in record["search_space"].items()
         if key != "search_space_sha256"}
    )
    registry.register(record)
    report, receipt = run_registered_search(registry, "EXP-0001", _bars())
    stored = registry.record("EXP-0001")
    assert len(stored.body["attempted_configs"]) == 25
    assert len(report["candidate_scores"]) == 25
    assert any(attempt["outcome"]["selected"] is False
               for attempt in stored.body["attempted_configs"])
    assert receipt.status in {"completed", "rejected"}


def test_dsr_trial_count_equals_registry_attempts(tmp_path):
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    record = _registered()
    registry.register(record)
    _, _ = run_registered_search(registry, "EXP-0001", _bars())
    stored = registry.record("EXP-0001")
    attempts = len(stored.body["attempted_configs"])
    assert stored.body["results"]["multiple_testing_diagnostics"][
        "deflated_sharpe_validation_only"]["trials"] == attempts


def test_report_binds_dataset_manifest_digest(tmp_path):
    from quantlab.cli import _load_reference_data as load_references
    from quantlab.experiments.reproducibility import bind_published_dataset
    from quantlab.market import market_event_from_dict as decode
    from quantlab.point_in_time import write_dataset

    events = [decode(json.loads(line)) for line in
              (FIXTURES / "canonical-events.example.jsonl").read_text().splitlines() if line]
    rows, manifest = PITDatasetBuilder(
        load_references(FIXTURES / "reference-data.example.json")
    ).build(events, dataset_id="e2e-synthetic-nse-session",
            created_at_utc_ns=1_735_789_503_000_000_000, universe_id="fixture-universe")
    dataset_dir = write_dataset(rows, manifest, tmp_path / "published")
    binding = bind_published_dataset(dataset_dir)
    record = _registered()
    record["dataset_manifest_sha256"] = binding["dataset_manifest_sha256"]
    record["dataset_id"] = binding["dataset_id"]
    record["dataset_version"] = binding["dataset_version"]
    record["dataset_lineage"] = binding["dataset_lineage"]
    record["holdout_key"] = holdout_key(
        record["dataset_version"], record["universe_id"], record["feature_version"],
        record["validation_period"], record["holdout_period"],
    )
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    registry.register(record)
    assert registry.verify_integrity(dataset_dir=dataset_dir)["status"] == (
        "verified_internal_integrity"
    )
    manifest_path = dataset_dir / "manifest.json"
    payload = json.loads(manifest_path.read_text())
    payload["dataset"]["row_count"] = 999
    manifest_path.write_text(json.dumps(payload, sort_keys=True), encoding="utf-8")
    with pytest.raises(ExperimentError, match="manifest"):
        registry.verify_integrity(dataset_dir=dataset_dir)


def test_signed_head_verifies_and_tampered_head_fails(tmp_path):
    private = Ed25519PrivateKey.generate()
    public = private.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    trusted = {"key-1": base64.b64encode(public).decode("ascii")}

    class Signer:
        public_key_b64 = trusted["key-1"]

        def __init__(self, key):
            self.key = key

        def sign(self, message):
            return self.key.sign(message)

    class BadSigner:
        def sign(self, message):
            return b"\x00" * 64

    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    registry.register(_registered())
    registry.sign_head(Signer(private), "key-1")
    assert registry.verify_integrity(trusted_public_keys=trusted)["signature_verified"] is True
    with pytest.raises(ExperimentError, match="trusted_public_keys"):
        registry.verify_integrity()
    bad = ExperimentRegistry(tmp_path / "bad.sqlite3")
    bad.register(_registered())
    bad.sign_head(BadSigner(), "key-1")
    with pytest.raises(ExperimentError):
        bad.verify_integrity(trusted_public_keys=trusted)


def test_cscv_diagnostics_never_use_final_holdout(tmp_path):
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    registry.register(_registered())
    _, _ = run_registered_search(registry, "EXP-0001", _bars())
    report = registry.record("EXP-0001").body["results"]
    assert report["multiple_testing_diagnostics"]["cscv_pbo"]["uses_final_holdout"] is False
    assert report["warning"].startswith("Research screening only")


def test_rejected_experiment_is_retained(tmp_path):
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    registry.register(_registered())
    registry.append_result(_terminal(_registered(), status="rejected"))
    assert registry.verify_integrity()["record_count"] == 2
    assert registry.record("EXP-0001", terminal_only=True).status == "rejected"


def test_cli_alpha_search_requires_registered_experiment(tmp_path):
    csv_path = tmp_path / "bars.csv"
    bars = _bars()
    csv_path.write_text("timestamp_ns,close\n" + "".join(
        f"{bar.timestamp_ns},{bar.close}\n" for bar in bars
    ), encoding="utf-8")
    registry_path = tmp_path / "registry.sqlite3"
    assert main(["alpha-search", str(csv_path), "--registry", str(registry_path),
                 "--experiment-id", "EXP-9999"]) == 2
    record = _registered(counts=25)
    family = [asdict(config) for config in default_candidate_family()]
    record["search_space"] = build_search_space(
        "momentum", family, train_period={"start_index": 1, "end_index": 360}
    )
    record["search_space"]["candidate_configs"] = family
    record["search_space"]["search_space_sha256"] = content_hash(
        {key: value for key, value in record["search_space"].items()
         if key != "search_space_sha256"}
    )
    record_path = tmp_path / "record.json"
    record_path.write_text(json.dumps(record, sort_keys=True), encoding="utf-8")
    registry = ExperimentRegistry(registry_path)
    registry.register(record)
    assert main(["alpha-search", str(csv_path), "--registry", str(registry_path),
                 "--experiment-id", "EXP-0001", "--output", str(tmp_path / "report.json")]) == 0
    assert (tmp_path / "report.json").exists()


def test_minute_registration_runs_with_declared_annualisation(tmp_path: Path) -> None:
    """A new minute fixture runs under its own registration and declared factor."""
    registered = _registered("EXP-CADENCE-0001")
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    registry.register(registered)
    bars = _bars()
    assert len(bars) == 600
    report, receipt = run_registered_search(registry, "EXP-CADENCE-0001", bars)
    declared = registered["periods_per_year"]
    assert report["periods_per_year_for_sharpe_annualisation"] == declared
    assert registry.record("EXP-CADENCE-0001").body[
        "results"]["periods_per_year_for_sharpe_annualisation"] == declared
    assert receipt.status in {"completed", "rejected"}
    attempts = registry.record("EXP-CADENCE-0001").body["attempted_configs"]
    selected = [item for item in attempts if item["outcome"]["selected"]]
    assert len(selected) == 1
    assert selected[0]["outcome"]["holdout_evaluation"] == "evaluated"
    assert isinstance(selected[0]["outcome"]["holdout_net_mean"], float)
    for attempt in attempts:
        outcome = attempt["outcome"]
        if outcome["selected"]:
            continue
        assert outcome["holdout_evaluation"] == "not_evaluated"
        assert outcome["holdout_net_mean"] is None
        assert outcome["holdout_sharpe"] is None
        assert outcome["holdout_observations"] is None
    for score in report["candidate_scores"]:
        if score["holdout_evaluation"] == "not_evaluated":
            assert score["holdout_net_mean"] is None


def test_daily_registration_rejects_minute_timestamps(tmp_path: Path) -> None:
    """A daily registration never accepts minute-spaced observations."""
    registered = _registered("EXP-CADENCE-DAILY")
    registered["bar_kind"] = "daily"
    registered["bar_interval_ns"] = DAILY_INTERVAL_NS
    timestamps = [bar.timestamp_ns for bar in _bars()]
    with pytest.raises(ExperimentError, match="bar interval"):
        validate_bar_cadence(registered, timestamps)
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    registry.register(registered)
    with pytest.raises(ExperimentError, match="bar interval"):
        run_registered_search(registry, "EXP-CADENCE-DAILY", _bars())
    assert registry.record("EXP-CADENCE-DAILY").status == "registered"
    registered["bar_interval_ns"] = MINUTE_INTERVAL_NS
    with pytest.raises(ExperimentError, match="shorter than an exchange trading day"):
        validate_bar_cadence(registered, timestamps)


def test_minute_registration_rejects_incompatible_interval() -> None:
    """A minute registration rejects intervals that disagree with the bars."""
    registered = _registered()
    timestamps = [bar.timestamp_ns for bar in _bars()]
    registered["bar_interval_ns"] = 300_000_000_000
    with pytest.raises(ExperimentError, match="disagrees"):
        validate_bar_cadence(registered, timestamps)
    registered["bar_interval_ns"] = DAILY_INTERVAL_NS
    with pytest.raises(ExperimentError, match="minute sampling range"):
        validate_bar_cadence(registered, timestamps)


def test_annualisation_factor_conflicts_with_registered_basis() -> None:
    """A named basis fixes its period count and bar family."""
    registered = _registered()
    registered["bar_kind"] = "daily"
    registered["bar_interval_ns"] = DAILY_INTERVAL_NS
    registered["annualization_basis"] = "daily_252"
    registered["periods_per_year"] = 252.0
    validate_registered_experiment(registered)
    registered["periods_per_year"] = 365.0
    with pytest.raises(ExperimentError, match="periods_per_year differs"):
        validate_registered_experiment(registered)
    registered["periods_per_year"] = 252.0
    registered["annualization_basis"] = "unlisted_basis"
    with pytest.raises(ExperimentError, match="unknown annualization_basis"):
        validate_registered_experiment(registered)
    registered["annualization_basis"] = "daily_252"
    registered["bar_kind"] = "minute"
    with pytest.raises(ExperimentError, match="requires daily bars"):
        validate_registered_experiment(registered)


def test_cli_periods_per_year_cannot_override_registration(tmp_path: Path) -> None:
    """The CLI value is an assertion and can never replace the registered factor."""
    csv_path = tmp_path / "bars.csv"
    bars = _bars()
    csv_path.write_text("timestamp_ns,close\n" + "".join(
        f"{bar.timestamp_ns},{bar.close}\n" for bar in bars
    ), encoding="utf-8")
    record = _registered("EXP-CADENCE-CLI", counts=25)
    family = [asdict(config) for config in default_candidate_family()]
    record["search_space"] = build_search_space(
        "momentum", family, train_period={"start_index": 1, "end_index": 360}
    )
    record["search_space"]["candidate_configs"] = family
    record["search_space"]["search_space_sha256"] = content_hash(
        {key: value for key, value in record["search_space"].items()
         if key != "search_space_sha256"}
    )
    registry_path = tmp_path / "registry.sqlite3"
    registry = ExperimentRegistry(registry_path)
    registry.register(record)
    conflicting = ["alpha-search", str(csv_path), "--registry", str(registry_path),
                   "--experiment-id", "EXP-CADENCE-CLI", "--periods-per-year", "252"]
    assert main(conflicting) == 2
    assert registry.record("EXP-CADENCE-CLI").status == "registered"
    matching = ["alpha-search", str(csv_path), "--registry", str(registry_path),
                "--experiment-id", "EXP-CADENCE-CLI", "--periods-per-year", "525600",
                "--output", str(tmp_path / "report.json")]
    assert main(matching) == 0
    report = json.loads((tmp_path / "report.json").read_text(encoding="utf-8"))
    assert report["periods_per_year_for_sharpe_annualisation"] == 525600.0


def test_calendar_metadata_missing_or_inconsistent_rejected() -> None:
    """Calendar identity must be present and agree with the dataset kind."""
    timestamps = [bar.timestamp_ns for bar in _bars()]
    missing = _registered()
    missing.pop("calendar_id")
    with pytest.raises(ExperimentError, match="missing registry fields"):
        validate_registered_experiment(missing)
    with pytest.raises(ExperimentError, match="calendar_id is missing"):
        validate_bar_cadence(missing, timestamps)

    market = _registered()
    market["dataset_id"] = "fixture:nse-cash"
    market["dataset_ref"] = "fixture:nse-cash:2026-11-11"
    market["config"] = {"lookback": 5}
    market["calendar_id"] = "synthetic:continuous-minute-v1"
    with pytest.raises(ExperimentError, match="market dataset cannot use a synthetic calendar"):
        validate_bar_cadence(market, timestamps)

    synthetic = _registered()
    synthetic["calendar_id"] = "fixture:continuous-minute-v1"
    with pytest.raises(ExperimentError, match="synthetic dataset requires a synthetic calendar"):
        validate_bar_cadence(synthetic, timestamps)
    synthetic["calendar_id"] = "synthetic:continuous-minute-v1"
    synthetic["bar_interval_ns"] = NOT_APPLICABLE_INTERVAL
    with pytest.raises(ExperimentError, match="synthetic dataset requires a fixed"):
        validate_bar_cadence(synthetic, timestamps)


def test_numeric_latency_without_model_rejected() -> None:
    """A one-nanosecond latency needs an explicit event-level model."""
    registered = _registered()
    registered["execution_assumptions"] = {"order_latency_ns": 1}
    with pytest.raises(ExperimentError, match="execution_latency_model"):
        validate_registered_experiment(registered)
    registered["execution_assumptions"] = {"order_latency_ns": 1,
                                           "execution_latency_model": "not_modelled"}
    with pytest.raises(ExperimentError, match="requires event_level_modelled"):
        validate_registered_experiment(registered)
    registered["execution_assumptions"] = {"execution_latency_model": "not_modelled"}
    validate_registered_experiment(registered)


def test_point_in_time_basis_limits_boolean_certification() -> None:
    """Ordering alone does not certify universe or corporate-action lineage."""
    registered = _registered()
    registered["dataset_point_in_time"] = False
    registered["point_in_time_basis"] = "none"
    validate_registered_experiment(registered)
    registered["point_in_time_basis"] = "universe_and_corporate_actions"
    with pytest.raises(ExperimentError, match="requires dataset_point_in_time"):
        validate_registered_experiment(registered)
    registered["dataset_point_in_time"] = True
    registered["point_in_time_basis"] = "none"
    with pytest.raises(ExperimentError, match="cannot certify"):
        validate_registered_experiment(registered)
    registered["point_in_time_basis"] = "universe_and_corporate_actions"
    with pytest.raises(ExperimentError, match="universe_version"):
        validate_registered_experiment(registered)
    registered["universe_version"] = "universe.v2"
    registered["holdout_key"] = holdout_key(
        DATASET_VERSION, "fixture-universe", "features.v1",
        registered["validation_period"], registered["holdout_period"],
        registered["universe_version"],
    )
    with pytest.raises(ExperimentError, match="corporate_action_source_hashes"):
        validate_registered_experiment(registered)
