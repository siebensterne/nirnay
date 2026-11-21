# M4 cadence and annualisation correction — 2026-11-21

Dated addendum to `evidence/M4-RESEARCH-EVIDENCE-2026-11-21.md`. The original
record is immutable: this addendum does not edit it or any artifact under
`evidence/m4/`. This is a contract correction, not new research evidence.

## Defect identified in the consumed registration

- Experiment `EXP-M4-0001` consumed a synthetic series whose timestamps are
  spaced 60 seconds apart, while its recorded hypothesis described the bars as
  `daily`. The record carried no bar-kind or cadence declaration.
- The run annualised returns with 252 periods per year through the former CLI
  default; the record carried no annualisation declaration. A 252-factor does
  not correspond to 60-second sampling. The validation and holdout Sharpe values
  reported in the original record (8.0463000070 and 5.4890170282) are therefore
  **not valid annualised estimates**. They remain in the historical record and
  are not restated or reused here.
- The registration also carried a bare numeric `order_latency_ns: 1` with no
  execution-latency model, and reported `0.0` holdout fields for unevaluated
  candidate configurations.

## Preservation and rerun status

The `EXP-M4-0001` holdout was consumed on 2026-11-21 and is preserved. The
original dated record and the four artifacts in `evidence/m4/` were not edited,
regenerated or removed. No experiment was rerun, no new holdout was inspected,
and no candidate was reselected. No licensed data, provider observation,
credential or live service was accessed.

## Corrected contract (implemented, not independently verified)

- The immutable record now requires `bar_kind`, `bar_interval_ns` (positive
  integer for fixed sampling, explicit `not_applicable` for tick/event),
  `timestamp_semantics`, a versioned `calendar_id`, `session_definition`,
  `annualization_basis`, `periods_per_year` and `point_in_time_basis`.
- A named annualisation basis fixes both its period count and bar family; an
  arbitrary positive factor requires `synthetic_declared`. The registered
  `periods_per_year` is the only factor used by a registered search. The CLI
  `--periods-per-year` flag now defaults to `None` and is an assertion that must
  match the registration; a conflicting value is rejected.
- `quantlab.experiments.cadence.validate_bar_cadence` fails closed on ordered
  timestamps that disagree with a daily or minute registration, on a daily
  registration with minute-spaced bars, on a minute registration with an
  incompatible interval, on a synthetic dataset without a synthetic calendar and
  fixed interval, and on missing or inconsistent market calendar metadata. It
  does not invent bars: exchange holidays, overnight breaks and auctions need
  calendar and session semantics that cannot be inferred from raw timestamp
  differences.
- `dataset_point_in_time` is bounded by `point_in_time_basis`. Ordered
  timestamps (`chronological_only`) do not certify point-in-time universe or
  corporate-action correctness; that requires declared lineage and a named
  universe version. The semantics are recorded in
  `engineering/research/dataset-acceptance-contract.md`.
- Execution assumptions require an explicit `execution_latency_model`; a bare
  one-nanosecond numeric latency is rejected. Unevaluated candidates carry
  `not_evaluated` holdout fields with null metrics, while the selected
  candidate's measured holdout values are preserved.
- A new synthetic fixture,
  `research/python/fixtures/alpha-bars-minute.seed-20261121.csv`, contains 600
  bars at 60-second spacing generated from seed 20261121. The positive test
  registers a minute-cadence record under a new experiment id and reuses neither
  the old 420-bar fixture nor `EXP-M4-0001`.

No alpha, profitability, execution-realism or live-readiness claim is made.
CAP-054 and CLM-049 describe the implemented contract only.

## Local checks

Worktree `/Users/tourist/Documents/nirnay-cadence`, branch
`feat/m4-cadence-contract`, base commit `221f75f68558b5a29e381b861a7a6fc5f0dff3a8`,
working tree not committed. Interpreter: Python 3.15.2 (code target remains
Python 3.12; no separate Python 3.12 run is claimed here).

| Command | Result |
|---|---|
| `python3 -m pytest -q` | exit 0; 371 passed, 2 subtests passed |
| `make check` | exit 0; all gates passed, including 371 Python tests and 2 subtests, the workspace Rust test suite, Clippy `-D warnings`, contract/schema gates and added-line style gates |

Counts are from the uncommitted worktree at the base commit named above.
