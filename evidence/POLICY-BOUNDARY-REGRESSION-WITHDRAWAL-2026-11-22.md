# Policy boundary: withdrawn synthetic regression coverage — 2026-11-22

**Status:** active CI regression coverage reduced by 90 Python tests under the owner
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
not a pure mathematical test. This includes reads through shared helpers and
fixtures, not only literal file paths in the test body.

The marker set was derived from a runtime file-access audit: `builtins.open`,
`pathlib.Path.open`, `read_text` and `read_bytes` were instrumented during a full
pytest run, and each read was attributed to the running test. The audit was then
repeated against the active (deselected) suite until no active test read an
archived fixture. All 90 markers are function-level; no whole module is
blanket-marked. Markers are registered and deselected centrally in `pytest.ini`
via `-m "not market_shaped_fixture"`, so the rule applies to `make check` and
hosted CI alike; no per-job override restores them.

Retained: pure mathematical tests using scalar values and abstract timestamps,
and infrastructure tests that construct no invented market observation (risk,
OMS, ledger, KMS, repository and schema gates). Those remain in the active suite.

## Withdrawn coverage — exact inventory

`research/python/tests/test_bench.py` (4):
  - `test_synthetic_fixture_scales_and_is_byte_stable`
  - `test_core_suite_emits_measured_distributions_and_stable_hashes`
  - `test_experiments_suite_reads_one_immutable_dataset_concurrently`
  - `test_latency_suite_emits_measured_distributions_and_stable_hashes`

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

`research/python/tests/test_cli_pipeline.py` (1):
  - `test_list_experiments_cli_shows_rejected_runs_and_attempt_counts`

`research/python/tests/test_experiment_comparison.py` (8):
  - `test_byte_stable_comparison_output`
  - `test_differing_dataset_hash_rejected`
  - `test_tampered_record_rejected`
  - `test_non_terminal_record_rejected`
  - `test_leaky_record_rejected`
  - `test_holdout_reuse_rejected`
  - `test_cli_compare_experiments_is_byte_stable_and_refuses_mismatch`
  - `test_registry_backed_comparison`

`research/python/tests/test_experiment_registry.py` (29):
  - `test_extended_record_freezes_deterministically`
  - `test_unknown_and_nonfinite_fields_still_rejected`
  - `test_append_is_hash_chained_and_sequence_contiguous`
  - `test_verify_detects_tampered_body_and_reordered_records`
  - `test_storage_rejects_update_and_delete`
  - `test_duplicate_experiment_id_rejected`
  - `test_holdout_reuse_rejected_after_terminal_record`
  - `test_universe_version_and_feature_digest_participate_in_holdout_key`
  - `test_concurrent_registration_claims_holdout_once`
  - `test_verification_only_claim_requires_replicates`
  - `test_validation_holdout_overlap_rejected`
  - `test_nonchronological_periods_rejected`
  - `test_leaked_dataset_cannot_be_candidate`
  - `test_terminal_requires_result_validation`
  - `test_all_attempted_configs_persisted_including_losers`
  - `test_dsr_trial_count_equals_registry_attempts`
  - `test_report_binds_dataset_manifest_digest`
  - `test_signed_head_verifies_and_tampered_head_fails`
  - `test_cscv_diagnostics_never_use_final_holdout`
  - `test_rejected_experiment_is_retained`
  - `test_cli_alpha_search_requires_registered_experiment`
  - `test_minute_registration_runs_with_declared_annualisation`
  - `test_daily_registration_rejects_minute_timestamps`
  - `test_minute_registration_rejects_incompatible_interval`
  - `test_annualisation_factor_conflicts_with_registered_basis`
  - `test_cli_periods_per_year_cannot_override_registration`
  - `test_calendar_metadata_missing_or_inconsistent_rejected`
  - `test_numeric_latency_without_model_rejected`
  - `test_point_in_time_basis_limits_boolean_certification`

`research/python/tests/test_feature_lineage.py` (3):
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

`research/python/tests/test_parameter_search.py` (4):
  - `test_grid_search_persists_all_attempts_including_losers`
  - `test_out_of_space_config_is_refused_before_evaluation`
  - `test_count_mismatch_is_refused_before_evaluation`
  - `test_nonfinite_evaluator_outcome_is_refused`

`research/python/tests/test_quant_pipeline_e2e.py` (1):
  - `test_synthetic_research_to_paper_pipeline_composes_without_claiming_live_readiness`

`research/python/tests/test_research_jobs.py` (7):
  - `test_manifest_accepts_valid_declaration`
  - `test_manifest_schema_rejects_invalid_cases`
  - `test_manifest_cross_field_invariants`
  - `test_unpinned_environment_is_refused`
  - `test_stale_dataset_hash_is_refused`
  - `test_dry_run_is_deterministic_and_appends_a_run_record`
  - `test_cli_run_research_job`

`research/python/tests/test_research_package_shims.py` (2):
  - `test_dataset_lineage_and_identity_helpers`
  - `test_bind_dataset_returns_lineage_and_digest`

`research/python/tests/test_robustness.py` (1):
  - `test_diagnostic_reading_holdout_fails_terminal_append`

`research/python/tests/test_validation_report.py` (5):
  - `test_validation_report_is_deterministic`
  - `test_tampered_record_is_refused`
  - `test_registry_report_resolves_stored_record`
  - `test_reordered_registry_chain_is_refused`
  - `test_cli_validation_report_writes_stable_bytes`

Total withdrawn: **90 tests** across 13 modules.

## Reproducible inspection

At the exact head of this change:

```text
python3 -m pytest -q
```

produces `300 passed, 90 deselected, 2 subtests passed`. The marker registration
and deselection are visible in `pytest.ini`; the per-test markers are visible in
the files listed above. To inspect the withdrawn set, run
`python3 -m pytest -q -m market_shaped_fixture --collect-only` — this is an
inspection command only and is not part of any passing gate. No withdrawn test is
auto-enabled by environment variable, profile or CI job.

## Coverage withdrawn — do not claim it still runs

The following regression assertions no longer run in active CI: calendar schedule
validation, bar-anchor and annualisation binding, canonical-event and PIT dataset
composition, benchmark and experiment-harness fixtures, feature lineage tamper
detection, paper projection replay, research-job manifests, validation reports,
dataset lineage helpers and the affected registry/report/cadence guards. Any
capability or claim that relies on those tests is unverified in the active suite.
Historical test counts (390 Python tests at earlier heads) remain historical
records of those heads and are not current coverage.

The underlying code is unchanged. Withdrawal removes evidence, not behaviour.
No real-market validation, source completeness, rights determination or alpha is
asserted by the remaining suite.

## Assumptions, unknowns and failure cases

- Assumption: the fixture-file boundary matches D3; inline scalar values in
  retained math and infrastructure tests do not constitute market datasets.
- Not established: rights for any source, dataset acceptance, exchange-calendar
  validity, or that any withdrawn behaviour is correct against real observations.
- Failure case: a withdrawn test is re-enabled without owner approval; a retained
  test reads an archived fixture and is not marked; a historical count is quoted
  as current coverage. Each is prohibited.
- Owner dependency: permission to reintroduce any fixture-driven test would
  require a new owner decision and a versioned amendment, not an agent edit.
