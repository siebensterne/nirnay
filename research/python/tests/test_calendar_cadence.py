# SPDX-License-Identifier: Apache-2.0
"""Generalized real-market cadence validation fixtures and falsifiers.

Every record in this file is a new test registration. No committed evidence,
receipt, provider observation or live service is read or modified.
"""
from __future__ import annotations

import json
from datetime import UTC, datetime
from pathlib import Path
from zoneinfo import ZoneInfo

import pytest

from test_experiment_registry import FIXTURES, _bars, _registered

from quantlab.cli import _load_reference_data, main
from quantlab.experiments import (
    ExperimentError,
    ExperimentRegistry,
    assert_dataset_origin,
    build_search_space,
    calendar_binding,
    dataset_origin,
    explicit_status_observation,
    holdout_key,
    validate_bar_cadence,
    validate_registered_experiment,
    validate_status_observations,
)
from quantlab.experiments.verify import verify_dataset_binding
from quantlab.market import market_event_from_dict
from quantlab.point_in_time import PITDatasetBuilder, write_dataset

MINUTE_NS = 60_000_000_000
FIVE_MINUTE_NS = 300_000_000_000
FIFTEEN_MINUTE_NS = 900_000_000_000
DAILY_NS = 86_400_000_000_000
BASE_NS = 1_735_000_000_000_000_000


def _record(calendar_id: str, *, bar_kind: str = "minute", interval: int = MINUTE_NS,
            basis: str = "synthetic_declared", periods: float = 525_600.0,
            origin: str = "SYNTHETIC") -> dict:
    record = _registered("EXP-CALENDAR-0001")
    record["dataset_origin"] = origin
    record["bar_kind"] = bar_kind
    record["bar_interval_ns"] = interval
    record["annualization_basis"] = basis
    record["periods_per_year"] = periods
    record.update(calendar_binding(calendar_id))
    record["bar_timezone"] = record["calendar_timezone"]
    return record


def _local_ns(year: int, month: int, day: int, hour: int, minute: int,
              zone: str = "Asia/Kolkata") -> int:
    moment = datetime(year, month, day, hour, minute, tzinfo=ZoneInfo(zone))
    delta = moment.astimezone(UTC) - datetime(1970, 1, 1, tzinfo=UTC)
    return (delta.days * 86400 + delta.seconds) * 1_000_000_000 + delta.microseconds * 1000


def _grid(start_ns: int, count: int, interval_ns: int) -> list[int]:
    return [start_ns + offset * interval_ns for offset in range(count)]


@pytest.mark.market_shaped_fixture
def test_synthetic_one_minute_regular_bars_pass() -> None:
    """Verify synthetic one minute regular bars pass."""
    record = _record("synthetic:fixed-1m-v1")
    validate_registered_experiment(record)
    validate_bar_cadence(record, _grid(BASE_NS, 24, MINUTE_NS))


@pytest.mark.market_shaped_fixture
def test_synthetic_daily_fixed_interval_pass() -> None:
    """Verify synthetic daily fixed interval pass."""
    record = _record("synthetic:fixed-1d-v1", bar_kind="daily", interval=DAILY_NS,
                     basis="daily_252", periods=252.0)
    validate_registered_experiment(record)
    validate_bar_cadence(record, _grid(BASE_NS, 12, DAILY_NS))


@pytest.mark.market_shaped_fixture
def test_daily_friday_to_monday_on_valid_calendar_pass() -> None:
    """Verify daily friday to monday on valid calendar pass."""
    record = _record("market:fixture-daily-v1", bar_kind="daily", interval=DAILY_NS,
                     basis="daily_252", periods=252.0, origin="MARKET")
    timestamps = [_local_ns(2025, 1, 3, 15, 30), _local_ns(2025, 1, 6, 15, 30)]
    validate_registered_experiment(record)
    validate_bar_cadence(record, timestamps)


@pytest.mark.market_shaped_fixture
def test_daily_missing_expected_trading_day_fails() -> None:
    """Verify daily missing expected trading day fails."""
    record = _record("market:fixture-daily-v1", bar_kind="daily", interval=DAILY_NS,
                     basis="daily_252", periods=252.0, origin="MARKET")
    timestamps = [_local_ns(2025, 1, 3, 15, 30), _local_ns(2025, 1, 7, 15, 30)]
    with pytest.raises(ExperimentError, match="missing expected bar"):
        validate_bar_cadence(record, timestamps)


@pytest.mark.market_shaped_fixture
def test_scheduled_holiday_pass() -> None:
    """Verify scheduled holiday pass."""
    record = _record("market:fixture-daily-v1", bar_kind="daily", interval=DAILY_NS,
                     basis="daily_252", periods=252.0, origin="MARKET")
    timestamps = [_local_ns(2025, 1, 17, 15, 30), _local_ns(2025, 1, 21, 15, 30)]
    validate_bar_cadence(record, timestamps)
    with_holiday = sorted([*timestamps, _local_ns(2025, 1, 20, 15, 30)])
    with pytest.raises(ExperimentError, match="unexpected bar timestamp"):
        validate_bar_cadence(record, with_holiday)


@pytest.mark.market_shaped_fixture
def test_five_minute_bars_with_one_minute_basis_fail() -> None:
    """Verify five minute bars with one minute basis fail."""
    wrong_basis = _record("synthetic:fixed-5m-v1", interval=FIVE_MINUTE_NS,
                          basis="minute_nse_cash_94500", periods=94_500.0)
    with pytest.raises(ExperimentError, match="bar_interval_ns differs"):
        validate_registered_experiment(wrong_basis)

    one_minute = _record("synthetic:fixed-1m-v1", basis="minute_nse_cash_94500",
                         periods=94_500.0)
    bars = _grid(BASE_NS, 12, FIVE_MINUTE_NS)
    with pytest.raises(ExperimentError, match="missing expected bar"):
        validate_bar_cadence(one_minute, bars)


@pytest.mark.market_shaped_fixture
def test_five_minute_bars_with_five_minute_basis_pass() -> None:
    """Verify five minute bars with five minute basis pass."""
    record = _record("synthetic:fixed-5m-v1", interval=FIVE_MINUTE_NS,
                     basis="minute_nse_cash_5m_18900", periods=18_900.0)
    validate_registered_experiment(record)
    validate_bar_cadence(record, _grid(BASE_NS, 12, FIVE_MINUTE_NS))


@pytest.mark.market_shaped_fixture
def test_fifteen_minute_bars_with_fifteen_minute_basis_pass() -> None:
    """Verify fifteen minute bars with fifteen minute basis pass."""
    record = _record("synthetic:fixed-15m-v1", interval=FIFTEEN_MINUTE_NS,
                     basis="minute_nse_cash_15m_6300", periods=6_300.0)
    validate_registered_experiment(record)
    validate_bar_cadence(record, _grid(BASE_NS, 12, FIFTEEN_MINUTE_NS))


@pytest.mark.market_shaped_fixture
def test_intraday_overnight_boundary_pass() -> None:
    """Verify intraday overnight boundary pass."""
    record = _record("market:fixture-overnight-v1", interval=FIFTEEN_MINUTE_NS,
                     basis="minute_nse_cash_15m_6300", periods=6_300.0, origin="MARKET")
    timestamps = [
        _local_ns(2025, 1, 9, 23, 0),
        _local_ns(2025, 1, 9, 23, 15),
        _local_ns(2025, 1, 9, 23, 30),
        _local_ns(2025, 1, 9, 23, 45),
        _local_ns(2025, 1, 10, 0, 0),
        _local_ns(2025, 1, 10, 0, 15),
        _local_ns(2025, 1, 10, 0, 30),
        _local_ns(2025, 1, 10, 0, 45),
        _local_ns(2025, 1, 10, 1, 0),
    ]
    validate_registered_experiment(record)
    validate_bar_cadence(record, timestamps)


@pytest.mark.market_shaped_fixture
def test_missing_minute_inside_active_session_fails() -> None:
    """Verify missing minute inside active session fails."""
    record = _record("market:fixture-minute-v1", basis="minute_nse_cash_94500",
                     periods=94_500.0, origin="MARKET")
    timestamps = [
        _local_ns(2025, 1, 9, 9, 15),
        _local_ns(2025, 1, 9, 9, 16),
        _local_ns(2025, 1, 9, 9, 18),
        _local_ns(2025, 1, 9, 9, 19),
    ]
    with pytest.raises(ExperimentError, match="missing expected bar"):
        validate_bar_cadence(record, timestamps)


@pytest.mark.market_shaped_fixture
def test_auction_and_scheduled_break_bars_rejected() -> None:
    """Verify auction and scheduled break bars rejected."""
    record = _record("market:fixture-minute-v1", basis="minute_nse_cash_94500",
                     periods=94_500.0, origin="MARKET")
    opening = [_local_ns(2025, 1, 9, 9, 15), _local_ns(2025, 1, 9, 9, 16)]
    during_auction = sorted([*opening, _local_ns(2025, 1, 9, 9, 10)])
    with pytest.raises(ExperimentError, match="unexpected bar timestamp"):
        validate_bar_cadence(record, during_auction)
    during_break = sorted([*opening, _local_ns(2025, 1, 9, 11, 5)])
    with pytest.raises(ExperimentError, match="unexpected bar timestamp"):
        validate_bar_cadence(record, during_break)


@pytest.mark.market_shaped_fixture
def test_declared_early_close_pass() -> None:
    """Verify declared early close pass."""
    record = _record("market:fixture-minute-v1", basis="minute_nse_cash_94500",
                     periods=94_500.0, origin="MARKET")
    first = _local_ns(2025, 2, 14, 9, 15)
    whole_session = _grid(first, 196, MINUTE_NS)
    timestamps = [value for offset, value in enumerate(whole_session)
                  if not 105 <= offset < 120]
    assert timestamps[-1] == _local_ns(2025, 2, 14, 12, 30)
    validate_bar_cadence(record, timestamps)
    with_late_bar = sorted([*timestamps, _local_ns(2025, 2, 14, 12, 31)])
    with pytest.raises(ExperimentError, match="unexpected bar timestamp"):
        validate_bar_cadence(record, with_late_bar)


@pytest.mark.market_shaped_fixture
def test_unknown_calendar_id_fails() -> None:
    """Verify unknown calendar id fails."""
    record = _record("synthetic:fixed-1m-v1")
    record["calendar_id"] = "synthetic:not-registered-v1"
    with pytest.raises(ExperimentError, match="unknown calendar_id"):
        validate_bar_cadence(record, _grid(BASE_NS, 12, MINUTE_NS))


@pytest.mark.market_shaped_fixture
def test_incorrect_calendar_digest_fails() -> None:
    """Verify incorrect calendar digest fails."""
    record = _record("synthetic:fixed-1m-v1")
    record["calendar_sha256"] = "0" * 64
    with pytest.raises(ExperimentError, match="calendar_sha256 differs"):
        validate_bar_cadence(record, _grid(BASE_NS, 12, MINUTE_NS))


@pytest.mark.market_shaped_fixture
def test_config_cannot_mark_market_dataset_synthetic() -> None:
    """Verify config cannot mark market dataset synthetic."""
    record = _record("synthetic:fixed-1m-v1", origin="MARKET")
    record["config"] = {"synthetic": True}
    record["dataset_ref"] = "synthetic:identifier-that-must-not-be-inferred"
    assert dataset_origin(record) == "MARKET"
    with pytest.raises(ExperimentError, match="market dataset cannot use a synthetic calendar"):
        validate_bar_cadence(record, _grid(BASE_NS, 12, MINUTE_NS))


@pytest.mark.market_shaped_fixture
def test_no_trade_and_suspended_instruments_yield_explicit_status() -> None:
    """Verify no trade and suspended instruments yield explicit status."""
    record = _record("synthetic:fixed-1m-v1")
    timestamps = [BASE_NS, BASE_NS + 2 * MINUTE_NS]
    no_trade = explicit_status_observation(BASE_NS + MINUTE_NS, "no_trade", instrument_id="INST-1")
    assert no_trade["price"] is None and no_trade["fabricated"] is False
    validate_bar_cadence(record, timestamps, statuses=[no_trade])

    suspended = explicit_status_observation(BASE_NS + MINUTE_NS, "suspended",
                                            instrument_id="INST-2")
    validate_bar_cadence(record, timestamps, statuses={BASE_NS + MINUTE_NS: suspended["status"]})
    with pytest.raises(ExperimentError, match="missing expected bar"):
        validate_bar_cadence(record, timestamps)
    with pytest.raises(ExperimentError, match="must not carry a price"):
        validate_status_observations([{**no_trade, "price": 0.0}])


@pytest.mark.market_shaped_fixture
def test_legacy_record_without_dataset_origin_fails_new_search(tmp_path: Path) -> None:
    """Verify legacy record without dataset origin fails new search."""
    record = _registered("EXP-CALENDAR-LEGACY")
    record.pop("dataset_origin")
    record["config"] = {"synthetic": True}
    with pytest.raises(ExperimentError, match="missing registry fields"):
        validate_registered_experiment(record)

    registered = _registered("EXP-CALENDAR-STORE")
    registry = ExperimentRegistry(tmp_path / "registry.sqlite3")
    registry.register(registered)
    stored = registry.record("EXP-CALENDAR-STORE")
    assert stored is not None
    stored.body.pop("dataset_origin")
    with pytest.raises(ExperimentError, match="missing registry fields"):
        validate_registered_experiment(stored.body)
    with pytest.raises(ExperimentError, match="dataset_origin must be an explicit"):
        validate_bar_cadence(stored.body, _grid(BASE_NS, 12, MINUTE_NS))


@pytest.mark.market_shaped_fixture
def test_dataset_origin_matches_published_manifest(tmp_path: Path) -> None:
    """Verify dataset origin matches published manifest."""
    from quantlab.experiments.reproducibility import bind_published_dataset

    events = [market_event_from_dict(json.loads(line)) for line in
              (FIXTURES / "canonical-events.example.jsonl").read_text().splitlines() if line]
    rows, manifest = PITDatasetBuilder(
        _load_reference_data(FIXTURES / "reference-data.example.json")
    ).build(events, dataset_id="cadence-origin-fixture",
            created_at_utc_ns=1_735_789_503_000_000_000,
            universe_id="fixture-universe", dataset_origin="SYNTHETIC")
    dataset_dir = write_dataset(rows, manifest, tmp_path / "published")
    binding = bind_published_dataset(dataset_dir)
    record = _record("synthetic:fixed-1m-v1")
    record["dataset_manifest_sha256"] = binding["dataset_manifest_sha256"]
    record["dataset_id"] = binding["dataset_id"]
    record["dataset_version"] = binding["dataset_version"]
    record["dataset_lineage"] = binding["dataset_lineage"]
    record["holdout_key"] = holdout_key(
        binding["dataset_version"], record["universe_id"], record["feature_version"],
        record["validation_period"], record["holdout_period"],
    )
    assert verify_dataset_binding(record, dataset_dir)["status"] == (
        "verified_internal_integrity"
    )
    assert_dataset_origin(record, manifest)
    record["dataset_origin"] = "MARKET"
    with pytest.raises(ExperimentError, match="dataset_origin"):
        verify_dataset_binding(record, dataset_dir)


@pytest.mark.market_shaped_fixture
def test_cli_cannot_override_registered_annualisation(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    """Verify cli cannot override registered annualisation."""
    csv_path = tmp_path / "bars.csv"
    bars = _bars()
    csv_path.write_text("timestamp_ns,close\n" + "".join(
        f"{bar.timestamp_ns},{bar.close}\n" for bar in bars
    ), encoding="utf-8")
    record = _registered("EXP-CALENDAR-CLI", counts=25)
    family = [{"family": "momentum", "lookback": value, "threshold_bps": 0.0}
              for value in range(3, 28)]
    record["search_space"] = build_search_space(
        "momentum", family, train_period={"start_index": 1, "end_index": 360},
        candidate_configs=family,
    )
    registry_path = tmp_path / "registry.sqlite3"
    ExperimentRegistry(registry_path).register(record)
    with pytest.raises(SystemExit):
        main(["alpha-search", str(csv_path), "--registry", str(registry_path),
              "--experiment-id", "EXP-CALENDAR-CLI", "--annualization-basis", "daily_252"])
    capsys.readouterr()
    conflicting = ["alpha-search", str(csv_path), "--registry", str(registry_path),
                   "--experiment-id", "EXP-CALENDAR-CLI", "--periods-per-year", "252"]
    assert main(conflicting) == 2
    assert ExperimentRegistry(registry_path).record("EXP-CALENDAR-CLI").status == "registered"
    output = tmp_path / "report.json"
    matching = ["alpha-search", str(csv_path), "--registry", str(registry_path),
                "--experiment-id", "EXP-CALENDAR-CLI", "--periods-per-year", "525600",
                "--output", str(output)]
    assert main(matching) == 0
    report = json.loads(output.read_text(encoding="utf-8"))
    assert report["periods_per_year_for_sharpe_annualisation"] == record["periods_per_year"]
    assert record["annualization_basis"] == "synthetic_declared"
