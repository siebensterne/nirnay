# Policy boundary: withdrawn synthetic regression coverage — 2026-11-22

**Status:** active CI regression coverage reduced by 52 Python tests under the owner
directive `evidence/OWNER-DIRECTIVE-2026-11-22.md` (D1–D3). This record is a
historical and governance record; it is not a performance or validation result.

## Question resolved

Which tests constructed or consumed invented market observations, and what
coverage is no longer asserted after the no-synthetic-data directive took effect?

## Rule applied

A test is withdrawn when it reads a checked-in market-shaped fixture file:
`alpha-bars.synthetic.csv`, `alpha-bars-minute.seed-20261121.csv`,
`canonical-events.example.jsonl`, `reference-data.example.json`, or a calendar
artifact under `research/python/fixtures/calendars/`. Directive D3 states that a
fixture modelling a session, holiday, bar anchor or market-shaped sequence is
not a pure mathematical test. These tests are registered with the pytest marker
`market_shaped_fixture` and deselected centrally in `pytest.ini` via
`-m "not market_shaped_fixture"`. The marker applies to `make check` and hosted
CI alike; no per-job override restores them.

Retained: pure mathematical tests using scalar values and abstract timestamps,
and infrastructure tests that construct no invented market observation (risk,
OMS, ledger, KMS, repository and schema gates). Those remain in the active suite.

## Withdrawn coverage — exact inventory

Whole modules (module-level marker), 42 tests.

`research/python/tests/test_calendar_cadence.py` (19):
  - `test_synthetic_one_minute_regular_bars_pass`
  - `test_synthetic_daily_fixed_interval_pass`
  - `test_daily_friday_to_monday_on_valid_calendar_pass`
  - `test_daily_missing_expected_trading_day_fails`
  - `test_scheduled_holiday_pass`
  - `test_five_minute_bars_with_one_minute_basis_fail`
  - `test_five_minute_bars_with_five_minute_basis_pass`
  - `test_fifteen_minute_bars_with_fifteen_minute_basis_pass`
  - `test_intraday_overnight_boundary_pass`
  - `test_missing_minute_inside_active_session_fails`
  - `test_auction_and_scheduled_break_bars_rejected`
  - `test_declared_early_close_pass`
  - `test_unknown_calendar_id_fails`
  - `test_incorrect_calendar_digest_fails`
  - `test_config_cannot_mark_market_dataset_synthetic`
  - `test_no_trade_and_suspended_instruments_yield_explicit_status`
  - `test_legacy_record_without_dataset_origin_fails_new_search`
  - `test_dataset_origin_matches_published_manifest`
  - `test_cli_cannot_override_registered_annualisation`

`research/python/tests/test_quant_pipeline_e2e.py` (1):
  - `test_synthetic_research_to_paper_pipeline_composes_without_claiming_live_readiness`

`research/python/tests/test_feature_lineage.py` (7):
  - `test_definition_digest_is_stable_and_rejects_empty`
  - `test_knowledge_time_after_as_of_rejected`
  - `test_future_as_of_for_event_rejected`
  - `test_decoded_lineage_rejects_unknown_fields`
  - `test_builder_binds_feature_lineage_to_rows_and_manifest`
  - `test_published_dataset_detects_tampered_manifest_lineage`
  - `test_published_dataset_detects_tampered_row_lineage`

`research/python/tests/test_paper_projection_consumption.py` (6):
  - `test_paper_replay_consumes_projected_events`
  - `test_unresolved_instrument_and_missing_lineage_fail`
  - `test_provider_mismatch_fails`
  - `test_reference_binding_must_match_projection_sidecar`
  - `test_fresh_process_replay_is_byte_identical`
  - `test_replay_denied_by_registry_emits_no_output`

`research/python/tests/test_research_package_shims.py` (9):
  - `test_dataset_lineage_and_identity_helpers`
  - `test_bind_dataset_returns_lineage_and_digest`
  - `test_environment_digest_is_path_order_stable_and_refuses_duplicates`
  - `test_cost_assumptions_digest_matches_canonical_values`
  - `test_report_digest_helpers_reject_tampering`
  - `test_report_bytes_are_byte_stable`
  - `test_signed_head_message_is_domain_separated`
  - `test_validation_shim_keeps_diagnostics_and_adds_leakage_detector`
  - `test_strategy_shim_keeps_deterministic_fixture`

Mixed module `research/python/tests/test_experiment_registry.py`, 10 tests with
function-level markers:
  - `test_all_attempted_configs_persisted_including_losers`
  - `test_dsr_trial_count_equals_registry_attempts`
  - `test_report_binds_dataset_manifest_digest`
  - `test_cscv_diagnostics_never_use_final_holdout`
  - `test_cli_alpha_search_requires_registered_experiment`
  - `test_minute_registration_runs_with_declared_annualisation`
  - `test_daily_registration_rejects_minute_timestamps`
  - `test_minute_registration_rejects_incompatible_interval`
  - `test_cli_periods_per_year_cannot_override_registration`
  - `test_calendar_metadata_missing_or_inconsistent_rejected`

Total withdrawn: 42 + 10 = **52 tests**.

## Reproducible inspection

At the exact head of this change:

```text
python3 -m pytest -q
```

produces `338 passed, 52 deselected, 2 subtests passed`. The marker registration
and deselection are visible in `pytest.ini`; the per-test markers are visible in
the files listed above. To inspect the withdrawn set, run
`python3 -m pytest -q -m market_shaped_fixture --collect-only` — this is an
inspection command only and is not part of any passing gate. No withdrawn test is
auto-enabled by environment variable, profile or CI job.

## Coverage withdrawn — do not claim it still runs

The following regression assertions no longer run in active CI: calendar schedule
validation, bar-anchor and annualisation binding, canonical-event and PIT dataset
composition, feature lineage tamper detection, paper projection replay, dataset
lineage helpers and the affected registry/report/cadence guards. Any capability
or claim that relies on those tests is unverified in the active suite. Historical
test counts (390 Python tests at earlier heads) remain historical records of
those heads and are not current coverage.

The underlying code is unchanged. Withdrawal removes evidence, not behaviour.
No real-market validation, source completeness, rights determination or alpha is
asserted by the remaining suite.

## Assumptions, unknowns and failure cases

- Assumption: the fixture-file boundary matches D3; inline scalar values in
  retained math and infrastructure tests do not constitute market datasets.
- Not established: rights for any source, dataset acceptance, exchange-calendar
  validity, or that any withdrawn behaviour is correct against real observations.
- Failure case: a withdrawn test is re-enabled without owner approval; a retained
  test is described as verifying market-shaped data; a historical count is quoted
  as current coverage. Each is prohibited.
- Owner dependency: permission to reintroduce any fixture-driven test would
  require a new owner decision and a versioned amendment, not an agent edit.
