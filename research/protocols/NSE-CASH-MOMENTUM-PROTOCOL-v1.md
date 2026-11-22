# NSE Cash-Equity Long-Only Monthly Cross-Sectional Momentum — Research Protocol v1

## 0. Document control and status

| Field | Value |
|---|---|
| Protocol ID | `NSE-CASH-MOMENTUM-PROTOCOL-v1` |
| Version | v1 |
| Date authored | 2026-11-22 |
| Worktree / branch | `docs/m45-protocol` at base `a934325` |
| Claim state | **Proposed** (design exists; no experiment registered, run, or reported) |
| Scope | One first experiment: NSE cash-equity, long-only, monthly cross-sectional momentum, real data only |
| Governing documents | `AGENTS.md`, `engineering/verification-policy.md`, `engineering/research/dataset-acceptance-contract.md`, `engineering/research/compute-and-validation.md`, `engineering/research/point-in-time-data.md` |
| Data rights | **UNKNOWN** for all source products; no written permission recorded |
| Dataset status | **BLOCKED** under the dataset acceptance contract |

This document fixes the hypothesis, universe, signal, rebalance, portfolio,
benchmark, cost model, split rule, search grid, and falsifiers **before any
return series is computed, plotted, ranked, or viewed**. Sections 8 and 9 hold
placeholders where a value depends on qualified dataset coverage; the rule for
freezing those placeholders is stated here and does not depend on returns.
Nothing in this document authorises an experiment, a registration, a holdout
claim, a live-routing change, or a statement about profitability.

This protocol is a design artefact only. It contains no parameter chosen from a
return observation and no synthetic-data pilot result. Synthetic fixtures may
exercise code paths but must not inform any parameter below.

## 1. Economic hypothesis and failure modes

### 1.1 Hypothesis (falsifiable)

**H1.** On a point-in-time universe of NSE cash equities, a long-only portfolio
that holds the highest-momentum names, formed on a 12-month total-return
lookback with a one-month skip, rebalanced monthly, earns a positive
net-of-cost return and a positive net-of-cost return relative to the named
total-return benchmark, with the sign stable across the train, validation, and
untouched holdout periods defined in section 8.

**H0 (null).** The net-of-cost mean return and the net-of-cost benchmark-relative
return are zero or negative after the multiple-testing adjustment in section
10.4, or the sign is unstable across the three periods.

### 1.2 Why the effect might exist

- Information diffuses gradually. Price underreaction to earnings and corporate
  news, followed by continued drift, produces cross-sectional persistence over
  multi-month horizons.
- Institutional frictions delay capital allocation. Index-rebalancing flows,
  mandate constraints, and gradual analyst revisions create delayed demand for
  recent winners.
- The venue has structural features that can widen the effect. NSE cash equities
  have a broad retail share, dispersed information, and limits to arbitrage in
  smaller names, each associated with slower price adjustment.

### 1.3 Why the effect may fail

- Momentum crashes. After market declines or in high-volatility states, past
  winners can sharply underperform; a long-only book cannot offset this with a
  short leg.
- Short-horizon reversal. The skipped month exists to avoid bid-ask bounce and
  reversal; a residual reversal component can still erase the formation return.
- Costs and impact. Brokerage, STT/CTT, stamp duty, exchange and SEBI charges,
  GST, slippage, and market impact can consume the gross spread, especially in
  the less liquid names that momentum tilts toward.
- Data integrity. Survivorship, missing delisting returns, corporate-action
  errors, symbol remaps, and suspension gaps can manufacture an apparent spread.
- Regime dependence and crowding. The spread can concentrate in a limited set of
  episodes or decay after publication; a single-regime result is not evidence of
  a persistent effect.
- Multiplicity. Searching a grid of plausible specifications inflates the
  chance of a positive validation outcome under H0.

Each failure mode maps to a falsification condition in section 10.

## 2. Point-in-time universe

### 2.1 Eligible security types

The eligible set at any formation date is fully paid ordinary equity shares in
the deliverable cash segment of the National Stock Exchange of India mainboard.
Included: equity shares admitted to dealings in the daily deliverable segment,
with a valid effective-dated listing record and a valid security identifier
mapping. Excluded:

- derivatives, futures, options, and index products;
- mutual fund units, ETFs, REITs, InvITs, bonds, debentures, preference shares,
  GDRs, warrants, partly paid shares, rights issues, and when-issued lines;
- securities on the SME or Emerge platform, which have separate listing and
  liquidity regimes;
- any instrument without an effective-dated equity identity mapping in the
  accepted dataset.

F&O eligibility of an underlying does not change cash-segment eligibility; the
signal and the trades use cash-segment instruments only.

### 2.2 Point-in-time membership and delisting

Universe membership is taken from a point-in-time constituent history with
effective dates and a source digest, not from a current symbol list. At a
formation date `F`, a security is in the eligible set when:

1. its listing effective date is on or before `F`;
2. its delisting effective date, when present, is after `F` — delisted and
   subsequently delisted securities remain in the history up to their delisting
   date, with the delisting return recorded;
3. its trading status at the information cut-off (section 4.2) is active;
4. it satisfies the liquidity and price screens in section 5.4 on data known at
   that cut-off.

No survivorship is permitted. A dataset whose constituent history omits
securities delisted before its extraction date fails this protocol. A security
delisted inside the holding month is sold at its delisting price where such a
price exists; where no delisting price exists, the last valid total-return price
is used and the treatment is reported under section 12, item 10. Excluding
delisted names from the history is a falsification trigger under section 10,
item 6.

### 2.3 Listing and suspension rules

- A newly listed security enters the eligible set only after a minimum price
  history equal to the lookback plus skip window (section 3.1) exists.
- A security suspended at the formation cut-off is excluded from new positions
  at that rebalance.
- A held security that becomes suspended is marked at its last valid total-return
  price and exited at the first subsequent rebalance where it trades; the number
  of affected position-months is reported.
- A security that is delisted, has its listing cancelled, or loses its identity
  mapping is removed at the effective date and its realised return booked.

Status flags, listings, and mappings must carry knowledge times. A status that
became known after the cut-off must not be applied to that formation.

## 3. Signal

### 3.1 Lookback, skip, and definition

Rebalance is monthly. Let the formation date `F_m` be the last trading session of
calendar month `m`. For the holding month `m+1`:

- **Skip interval:** calendar month `m`, the most recent completed month. Its
  return is excluded from the signal.
- **Lookback interval:** the 12 calendar months `m-12` through `m-1`, measured
  from the close of the last trading session of month `m-13` to the close of the
  last trading session of month `m-1`.
- **Signal score:** the cumulative total return over the lookback interval. A
  security with no valid price at either endpoint is not scored.

This is a 12-month formation, one-month skip, one-month hold specification.
Search variants in section 9 change only the dimensions listed there.

### 3.2 Price versus total return

The signal and all portfolio returns use total-return prices that incorporate
dividends, splits, bonuses, and rights, using only corporate actions whose
knowledge time is at or before the formation cut-off. A revision first received
after the cut-off is not used for that formation. Raw exchange close prices are
computed as a diagnostic only; they are not used for scoring, returns, or
benchmark comparison. If total-return prices are unavailable for the qualified
coverage, the experiment does not proceed — raw price is not accepted as a
proxy.

### 3.3 Missing-data handling

- Require a valid price on at least 80% of expected sessions in the lookback,
  based on the declared session calendar.
- If an endpoint price is missing, substitute the nearest valid price within two
  sessions of that endpoint; outside that window the security is unscored for
  that formation. No forward-fill is applied across a delisting or a gap longer
  than the calendar allows.
- Require positive price and positive traded volume on the endpoint session or
  its approved substitute.
- A security with a missing or unresolved corporate-action record in the
  lookback is excluded for that formation and the exclusion is counted.
- Every exclusion and substitution is counted and reported; a rule applied to
  more than 5% of the cross-section in any formation is flagged as a
  data-quality failure.

## 4. Rebalance

### 4.1 Calendar rule

Trades are placed for the first trading session of month `m+1`, following the
formation at the last session of month `m`. The portfolio is rebalanced every
month with no discretionary delay. The session calendar is the accepted
exchange calendar with holidays and sessions declared in the dataset
declaration; raw timestamp differences do not define sessions.

### 4.2 Information cut-off

The cut-off is the close of the formation date `F_m`. Every input — prices,
total-return adjustments, listings, suspension status, sector classification,
liquidity measures, surveillance flags, and benchmark data — must have a
knowledge time at or before that cut-off. Data published or revised afterwards
is not used for that formation. The cut-off and the execution session are
distinct, so the two cannot coincide.

### 4.3 Execution and no same-close look-ahead

Execution price is the opening-auction price of the first trading session of
month `m+1`. If that price is unavailable, the next-session VWAP is used; if
both are unavailable, the position is carried and the event is reported. Trading
at the formation-date close, or at any price formed before the cut-off, is
prohibited. Signal computation and order generation are separated by the
cut-off, and the executed price is stamped with the execution session, not the
formation session.

## 5. Portfolio construction

### 5.1 Holdings count

The base portfolio holds `N = 20` names: the 20 highest signal scores among
eligible securities at the formation date. If fewer than 20 securities are
eligible, all eligible names are held and the remainder is cash.

### 5.2 Weighting

Base weighting is equal weight, `1/N` of portfolio value per name at formation.
No risk-based or signal-magnitude weighting is used in the base configuration.

### 5.3 Cash

Uninvested capital, including amounts from unfilled orders and from an eligible
set smaller than `N`, is held as cash. The base configuration accrues 0% on
cash. A declared sensitivity case accrues the 91-day Treasury-bill yield from a
dated source with effective dates; the source is named before the run.

### 5.4 Concentration and liquidity screening

- **Liquidity floor:** median daily traded value of at least INR 10 crore over
  the trailing 60 sessions, and a close price of at least INR 5, each measured
  at the cut-off. The grid variant in section 9 raises this floor to INR 25 crore.
- **Continuity:** positive volume on at least 80% of sessions in the trailing 60
  sessions.
- **Surveillance:** exclusion of any security on an exchange surveillance or
  graded-surveillance list at the cut-off, where that status exists in the
  accepted dataset.
- **Sector cap:** no more than 30% of portfolio weight in one sector, using the
  classification effective at the cut-off. If breached, the lowest-momentum name
  in the over-weight sector is replaced by the highest-momentum name from
  another sector until the cap holds.
- **Issuer uniqueness:** one line per issuer; duplicate share lines are
  collapsed to the most liquid line before ranking.
- **Measured concentration:** top-5 weight share and Herfindahl index are
  reported each formation (section 12, item 6).

## 6. Benchmark

The named primary benchmark is the **NIFTY 500 Total Return Index (NIFTY 500
TRI)**; the secondary benchmark is the **NIFTY 50 Total Return Index (NIFTY 50
TRI)**. A benchmark-relative return is reported only when the accepted dataset
provides the index total-return history and its historical constituents, both
effective-dated and sourced from a dated record with matching rights. Returns
are computed from the same total-return basis and the same calendar as the
portfolio, with no look-ahead in index membership.

If historical constituents or a usable total-return history are unavailable,
the restricted claim applies: the report states absolute gross and net returns
and absolute drawdown only, names the missing benchmark input, and makes no
statement of outperformance, alpha, tracking error, or information ratio. The
restricted claim is applied before returns are inspected and cannot be lifted
afterwards on the basis of a favourable result.

## 7. Costs

All cost parameters are placeholders until an effective-dated schedule from the
accepted dataset is available; they are frozen at registration and must not be
chosen from a return series. Each component carries a source document and an
effective date, and each trade uses the schedule in force on its execution date.
A component that cannot be effective-dated marks the affected period as
unpriced and is reported, not silently held constant.

| Component | Base rule | Dating requirement |
|---|---|---|
| Brokerage | `C_BROKER` percent of traded value per side | effective-dated tariff schedule |
| Securities transaction tax | `C_STT` on delivery trades by side | effective-dated statutory schedule |
| Commodity transaction tax | `C_CTT` where applicable | effective-dated statutory schedule |
| Stamp duty | `C_STAMP` on buy side by state of execution | effective-dated state schedule |
| Exchange transaction charge | `C_EXCH` per unit of traded value | effective-dated exchange circular |
| SEBI fee and IPFT | `C_REG` per unit of traded value | effective-dated regulatory circular |
| GST | applied to taxable components at `C_GST` | effective-dated tax schedule |
| DP and custody | `C_DP` per delivered scrip | effective-dated schedule |
| Slippage | `C_SLIP` basis points of traded value, one way | from accepted spread/impact study |
| Market impact | `C_IMPACT` coefficient in the chosen impact form, calibrated from accepted data | from accepted liquidity study |

Cost stress cases, fixed before the run:

| Case | Slippage multiplier | Impact multiplier |
|---|---|---|
| Base | 1.0 | 1.0 |
| Stressed | 2.0 | 1.5 |
| Severe | 3.0 | 2.0 |

Costs are charged on both entry and exit, applied to the execution-session
price, and reported by component (section 12, item 7). Selection across
candidates uses base-case net return on validation data only; stress cases are
reported for the selected configuration and enter the falsification test in
section 10, item 3.

## 8. Validation split and freezing rule

Coverage is determined by the accepted dataset, not by a return target. Let
`D_start` be the first session and `D_end` the last session in the qualified
coverage, where qualified coverage requires a valid point-in-time universe,
total-return prices, session calendar, and corporate-action history. The
experiment does not proceed when qualified coverage spans fewer than 10 years.

Split rule, applied to qualified coverage in chronological order only:

| Partition | Placeholder | Share of qualified sessions |
|---|---|---|
| Train | `TRAIN_START` .. `TRAIN_END` | earliest 50% |
| Validation | `VAL_START` .. `VAL_END` | next 25% |
| Untouched holdout | `HOLD_START` .. `HOLD_END` | final 25% |

Boundaries are month-end sessions nearest to the 50% and 75% cumulative-session
points, computed from the session calendar. All four boundary formulas, the
coverage start and end, and the resulting dates are written into an immutable
dated freeze record with a content digest before any return is computed. The
holdout is evaluated once, for the single configuration selected on train and
validation, after the search in section 9 is closed.

Void condition: if any return, signal-performance statistic, or ranked outcome
for the holdout period is viewed before the freeze record is written, the
holdout is void. A replacement holdout must then be drawn from a newly qualified
coverage definition and frozen in a new dated record with the reason recorded.

## 9. Search protocol

The search grid is fixed and bounded. Candidate configurations are the Cartesian
product of the dimensions below; no dimension outside this list may be added.

| Dimension | Fixed candidate values |
|---|---|
| Lookback months | 9, 12 |
| Skip months | 1 |
| Holding months | 1 |
| Holdings count `N` | 20, 30 |
| Liquidity floor, median daily traded value | INR 10 crore, INR 25 crore |
| Weighting | equal |

Total trial count = 2 × 2 × 2 = **8 configurations**. Each configuration is
evaluated on train and validation only. The single configuration with the
highest validation net-of-cost return at base cost is selected; ties are broken
by lower validation maximum drawdown, then by lower turnover. The selected
configuration is applied once to the holdout.

Adaptive search is prohibited after holdout inspection. No configuration may be
added, removed, or re-weighted after the holdout result is seen, and no holdout
statistic may guide a re-run. Any further exploration requires a new protocol
version, a new registration, and a fresh holdout; it cannot reuse this holdout
label. The 8-trial count is the denominator for the multiple-testing adjustment
in section 11.

## 10. Falsification criteria

H1 is rejected when any one of the following holds. Thresholds are fixed before
the run.

1. **Direction and significance.** Validation net-of-cost benchmark-relative
   mean return is at or below zero, or is not distinguishable from zero at the
   5% level after the adjustment in section 11.
2. **Holdout.** Holdout net-of-cost return is at or below zero, or holdout
   net-of-cost return relative to the benchmark is at or below zero. A holdout
   that is void under section 8 cannot support H1.
3. **Cost stress.** Base-case costs consume more than 50% of gross return, or
   net return turns negative under the stressed cost case. Performance that
   survives base cost but fails the stressed case rejects H1.
4. **Instability.** The sign of mean excess return differs between train,
   validation, and holdout; or the sign flips between neighbouring grid
   configurations; or the top five months contribute more than 50% of cumulative
   excess return.
5. **Concentration and liquidity.** Removing the lowest-liquidity held names or
   the most concentrated sector removes the positive excess return; or the
   result depends on names below the liquidity floor.
6. **Delisting dependence.** Excluding delisted securities from the history
   raises net return by more than 2 percentage points per year, or a material
   share of the spread comes from names with unresolved delisting returns.
7. **Tail risk.** Maximum drawdown exceeds 40%, or the worst month is below
   -20%, on net return over the combined train-plus-validation period.
8. **Data integrity.** Any unresolved corporate-action error, rights gap,
   universe gap, or post-cut-off revision touching the scored period invalidates
   the run.

A rejection is recorded under the Falsified state of `engineering/verification-policy.md`
with the dated evidence and the digest of the run that produced it. A result
that fails to reject H0 is not a capability or profitability claim.

## 11. Uncertainty and multiple testing

Report standard errors and 95% intervals for mean returns using a method that
respects serial dependence and overlapping-horizon structure. Apply a
Holm-Bonferroni adjustment across the 8 registered trials to every reported
p-value and interval; report both raw and adjusted values. Report the number of
trials, the selection rule, and the holdout-evaluation count. A point estimate
without its adjusted interval is not a basis for accepting H1.

## 12. Required report outputs

The eventual run record must contain each of the following, with units and
computation notes:

1. **Gross and net returns.** Monthly and annualised, for each candidate and
   for the selected configuration on each partition.
2. **Benchmark-relative return.** Excess return, tracking error, and
   information ratio against the named TRI, or the restricted claim of section
   6 when benchmark inputs are missing.
3. **Drawdown and tail loss.** Maximum drawdown, worst month, 5th-percentile
   monthly return, and a reported tail-loss measure.
4. **Turnover.** One-way and two-way turnover per rebalance, mean annual
   turnover, and replaced names per rebalance.
5. **Market and sector exposure.** Portfolio beta to the named benchmark and
   sector weight series with the effective-dated classification.
6. **Concentration.** Top-5 weight share, Herfindahl index, and holding count
   per formation.
7. **Costs by component.** Currency amount and basis points for every row in
   section 7, per partition and per rebalance.
8. **Cost sensitivity.** Returns across the three cost stress cases, plus the
   slippage and impact break-even levels.
9. **Year and regime performance.** Calendar-year returns and returns grouped by
   a pre-declared market-state definition fixed before the run.
10. **Delisted-security treatment.** Count of delisted holdings, delisting
    returns used, price-substitution cases, and the sensitivity in section 10, item 6.
11. **Uncertainty and multiple-testing adjustments.** The outputs of section
    11, including raw and adjusted statistics.
12. **Digests.** SHA-256 digests of the accepted dataset declaration, the code
    revision, the feature definitions, the frozen configuration, and this report,
    each labelled with its artifact path, plus the frozen split dates and the
    cost-schedule effective dates.

## 13. Owner decision

- The dataset must pass the dataset acceptance contract before any work under
  this protocol is registered or described as research on real market data.
  A blocked or partially accepted dataset keeps this protocol in Proposed state.
- Data rights remain **UNKNOWN** for all source products. No written permission,
  no registration, and no replay of captured provider data is implied here.
- No experiment may be registered and no holdout result may be claimed until an
  accepted dataset exists and the section 8 split dates are frozen in a dated
  record.
- Live trading, broker connectivity, platform-readiness statements, and
  milestone promotion are outside this protocol and require separate owner
  authority under `AGENTS.md`.
- Owner dependencies: dataset acceptance evidence; rights determination with
  document references and review date; dated cost schedules with effective
  dates; total-return and benchmark-constituent sources; independent review
  sign-off; and the freeze record for the split dates.

## 14. Review standard

| Dimension | Standard for this protocol |
|---|---|
| Question | Does a long-only 12-1 monthly momentum rule on NSE cash equities earn positive net-of-cost absolute and benchmark-relative return, with stable sign across three time partitions? |
| Evidence | Point-in-time universe with delistings, total-return prices, effective-dated actions and cost schedules, named TRI with historical constituents, and a frozen split; each item dated and digested. |
| Assumptions | Total-return reconstruction is correct at cut-off; execution at next-session open is attainable; cost schedules are complete and effective-dated; the accepted calendar is correct. |
| Not established | Profitability, data rights, benchmark-constituent availability, cost-schedule completeness, and dataset acceptance. Each is explicitly unresolved here. |
| Failure cases | The eight falsification conditions in section 10, plus the void condition in section 8 and the restricted benchmark claim in section 6. |
| Inspection method | Re-derive split dates from qualified coverage; verify cut-off stamps against execution stamps; recompute scores from total-return inputs; audit exclusions and substitutions; recompute costs by component; repeat the 8-trial adjustment. |
| Independent review | Required before registration and before any holdout claim, covering financial method, point-in-time correctness, data rights, and cost dating. A passing local gate is not independent review. |
| Owner dependency | Acceptance, rights, cost and benchmark sources, independent review, and the freeze record are owner-gated; none is satisfiable by an agent edit. Nothing here signs a milestone. |
