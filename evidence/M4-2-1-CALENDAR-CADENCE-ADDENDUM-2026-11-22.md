# M4.2.1 calendar-cadence addendum — 2026-11-22

New dated addendum to `evidence/M4-CADENCE-CORRECTION-2026-11-21.md`. The two
existing M4 records and the four artifacts under `evidence/m4/` are immutable
and were neither opened for editing nor regenerated. This addendum describes an
implementation change, not new research evidence.

## What changed

The former validator compared every pair of ordered bar timestamps with one
declared fixed elapsed interval. That rule cannot represent a real trading day:
weekends, holidays, auctions, scheduled breaks, early closes and an overnight
session each interrupt the grid without implying a missing bar. The validator
now compares observations with an expected schedule resolved from a checked-in
calendar artifact:

- `quantlab.experiments.calendar` resolves `calendar_id` from a fixed registry
  of files under `research/python/fixtures/calendars/`, recomputes the canonical
  SHA-256 of the artifact and fails closed on an unknown identifier, a wrong
  digest or a conflicting `calendar_source`, `calendar_timezone`, `bar_kind`,
  `bar_interval_ns` or `bar_anchor`. No database, provider service, network
  call or wall clock is consulted.
- Session artifacts model regular weekday sessions, auction treatment,
  scheduled breaks, early-close dates and holidays. One artifact models an
  overnight session that crosses local midnight. The expected schedule is the
  set of scheduled observation timestamps in the observed UTC-nanosecond range.
  Gaps are absent from that set rather than filled.
- A bar outside the expected set is rejected. An expected bar with no price
  observation is rejected unless an explicit coverage status covers it. No bar
  is invented from a timestamp difference, and no price is forward-filled.
- `dataset_origin` is explicit (`SYNTHETIC` or `MARKET`) and validated against
  the dataset manifest. Identifier substrings and config booleans are never
  used to infer origin; `config.synthetic=true` cannot override an explicit
  `MARKET` declaration. `bar_timezone` (IANA name or `UTC`), `bar_anchor`, the
  calendar digest fields and a bounds-checked annualisation basis are now part
  of the immutable registration. A named basis fixes bar family, interval and
  period count: daily 252, NSE-cash one-minute 94_500, five-minute 18_900 and
  fifteen-minute 6_300. The CLI remains unable to replace the registered factor.
- Explicit coverage statuses are `no_trade`, `not_listed`, `suspended`,
  `halted`, `market_closed` and `missing`. A status observation carries
  `price=None` and `fabricated=false`; a numeric price on a status record is
  refused.

Legacy records without the new fields cannot register a new search or start a
new registered run. Already committed records, receipts and dated evidence are
not edited, re-signed or re-run.

## Test data inspected

- `research/python/fixtures/alpha-bars-minute.seed-20261121.csv`: 600 close
  rows at 60-second spacing; its first timestamp is 1736000000000000000 ns.
- `research/python/fixtures/alpha-bars.synthetic.csv`: the original 420-bar
  fixture; left unchanged and not used as a cadence reference.
- `research/python/fixtures/canonical-events.example.jsonl` and
  `reference-data.example.json`: used only to exercise the manifest-origin
  binding on a locally built synthetic publication.
- Eight checked-in calendar artifacts; their digests are recomputed at runtime
  from the artifact bytes by `calendar_binding`.

## Mandatory falsifiers and pass cases

`research/python/tests/test_calendar_cadence.py` adds 19 tests:

| Test | Expectation |
|---|---|
| `test_synthetic_one_minute_regular_bars_pass` | pass |
| `test_synthetic_daily_fixed_interval_pass` | pass |
| `test_daily_friday_to_monday_on_valid_calendar_pass` | pass |
| `test_daily_missing_expected_trading_day_fails` | fail closed |
| `test_scheduled_holiday_pass` | pass; a bar on the holiday fails |
| `test_five_minute_bars_with_one_minute_basis_fail` | fail closed |
| `test_five_minute_bars_with_five_minute_basis_pass` | pass |
| `test_fifteen_minute_bars_with_fifteen_minute_basis_pass` | pass |
| `test_intraday_overnight_boundary_pass` | pass |
| `test_missing_minute_inside_active_session_fails` | fail closed |
| `test_auction_and_scheduled_break_bars_rejected` | fail closed |
| `test_declared_early_close_pass` | pass; a later bar fails |
| `test_unknown_calendar_id_fails` | fail closed |
| `test_incorrect_calendar_digest_fails` | fail closed |
| `test_config_cannot_mark_market_dataset_synthetic` | fail closed |
| `test_no_trade_and_suspended_instruments_yield_explicit_status` | explicit status, no bar |
| `test_legacy_record_without_dataset_origin_fails_new_search` | fail closed |
| `test_dataset_origin_matches_published_manifest` | matching origin passes, mismatch fails |
| `test_cli_cannot_override_registered_annualisation` | no basis override; periods assertion only |

Three existing registry tests were updated to the explicit-origin and
checked-in-calendar declarations: the daily/minute falsifier, the calendar
metadata falsifier and the shared record helpers in three test modules. No
committed evidence or receipt was changed.

## Local checks

Worktree `/Users/tourist/Documents/nirnay-calcad`, branch `fix/calendar-cadence`,
base commit `a934325`, working tree not committed. Interpreter: Python 3.15.2
(code target remains Python 3.12; no separate Python 3.12 run is claimed here).
No commit, push or merge was performed.

| Command | Result |
|---|---|
| `python3 -m pytest -q` | exit 0; 390 passed, 2 subtests passed (371 existing, 19 new) |
| `make check` | exit 0; all gates passed, including 390 Python tests and 2 subtests, the workspace Rust test suite, Clippy `-D warnings`, contract/schema gates and added-line style gates |

Counts are from the uncommitted worktree at the base commit named above.

## Scope limits

No provider observation, credential, account identifier, licensed dataset or
live service was accessed. All calendars are checked-in fixtures and are not
licensed exchange reference data. All rights in
`configs/providers/source-products.v1.json` remain `UNKNOWN`; no right,
entitlement or completeness claim is granted by this change. No experiment was
executed, rerun, registered or reselected, no holdout was inspected, and no
research result is asserted. This is an implemented, not independently verified
contract correction. CAP-055 and CLM-050 record the implementation only.
