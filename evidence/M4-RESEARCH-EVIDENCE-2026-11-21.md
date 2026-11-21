# M4 synthetic research-pilot evidence — 2026-11-21

Dated record of the first pre-registered experiment executed through the new
registry. **Synthetic fixture only.** This is a process demonstration, not
research on licensed market data, not validated alpha and not permission to
trade.

## Environment and identity

| Field | Value |
|---|---|
| Code snapshot | `0a726a6b4b2a2b78220a68fffcf5c0f737173235` |
| Interpreter | Python 3.12.13 |
| Dataset | `research/python/fixtures/alpha-bars.synthetic.csv`, 420 close-only bars |
| Dataset SHA-256 | `0348c1437ad9930ea6d6d143b25b6bf3cd2ede9664ee3c58adbae2cb5b539023` |
| Experiment | `EXP-M4-0001` |

## Pre-registration (before any result was read)

- Hypothesis: a short-horizon momentum screen on synthetic daily bars may earn
  positive net returns after the declared costs.
- Falsification rule: reject unless the untouched holdout net mean is positive
  after declared costs; do not reopen the holdout after inspection.
- Search space: four momentum configurations (lookback 3/5, threshold 0/5/10 bps
  as declared in the record), expected trials 4, one embargo at each split.
- Chronology: train return bars 1–251, validation 253–335, untouched holdout
  337–419; embargoed boundaries 252 and 336.
- Costs: the registry's declared `AlphaCosts` assumptions; no venue calibration.
- Holdout key:
  `5fc7e760aec22f5c7162e827577b03e55eea4fc812a2641836cbb434ef7d94a7`.
- Registration receipt record hash:
  `679050a52e18127313551abe0f4cc77673ea717e312039923d56cd8fab688d9e`.

## Execution

One registered search was run under Python 3.12 with no cost overrides. The
registry appended a terminal record (sequence 2, status `completed`, four
attempted configurations including losers). No terminal record was edited in
place and the holdout was not used for selection.

| Result | Value |
|---|---|
| Report SHA-256 | `2170dcf955b09b5b267634fd63fc590d43f1f3c066fb51bc321fce5bf14fb04f` |
| Terminal record hash | `0c8a295987706225cc1599647f32ef6e86787b6d2da93bb328ebd16e28531288` (committed in `evidence/m4/terminal-receipt.json`) |
| Selected configuration | momentum lookback 3, threshold 0 bps |
| Validation net mean | 0.0004373 (return units of the synthetic bars); Holm p 0.0163735 |
| Untouched holdout net mean | 0.0002292 |
| Holdout approximate p-value | 0.0344668 (approximate; see report) |
| Report status | `positive_holdout_screen_passed_not_alpha_certified` |

## What this does and does not establish

It establishes that the registry enforces pre-registration, complete attempted
configuration accounting, split geometry and holdout discipline, and that a
reproducible report and terminal record were produced on a synthetic fixture.
The legacy close-based screen does not emit drawdown, turnover, exposure,
concentration, tail-loss, period-stability or cost-sensitivity diagnostics;
those are therefore **not** reported here and must not be inferred from the
net mean. Costs are assumptions, not effective-dated venue charges.

The positive screen is a property of synthetic data and is not evidence of
alpha, tradability, data rights, execution realism or profitability. All
licensed-data work remains blocked by the dataset acceptance contract
(`engineering/research/dataset-acceptance-contract.md`) and the UNKNOWN rights.
