# Event-driven simulation and alpha research

**Status:** deterministic reference simulator and finite-family research screen implemented. They are research tools, not exchange simulators or profitability evidence.

## 1. Event-driven backtester

`quantlab.backtest.EventDrivenBacktester` consumes canonical `MarketEvent` values for a single instrument. A separate `quantlab.portfolio_backtest.MultiInstrumentPortfolioBacktester` consumes a selected provider's canonical stream across multiple NSE cash instruments. The portfolio engine adds per-symbol order state, portfolio cash/equity marks, cross-symbol open-buy reservations and gross-exposure caps. The shared explicit portfolio scope remains cash-funded, long-only NSE cash equities; it is not a margin, derivatives or short-borrow engine.

The single-instrument engine's scope is:

- one resolved canonical cash-equity instrument;
- one selected provider stream;
- long-only inventory, no shorts;
- one active order at a time;
- integer share quantities and `Decimal` prices/cash;
- limit order intent with time-in-force expiry;
- configurable submission/cancel latency, fee rates, participation cap and square-root taker impact coefficient.

If more than one provider appears for the same replay and no `source_provider` is selected, the engine rejects the data. Upstox and FYERS are two observations of the same underlying liquidity, not two books whose sizes may be added.

### Matching assumptions

- A marketable limit activates only after configured order latency and is constrained by its limit price.
- The first activation can consume the current displayed opposing book tranche once. Later snapshots contribute only positive net displayed-size increments; unchanged book quantity is not re-counted as new liquidity.
- Each tranche is subject to maximum participation.
- Taker fees and square-root impact are applied to executable touch price. Impact is itemised as a positive cost but not debited twice; the modeled execution price already embeds it. Spread cost is measured against the last fresh two-sided book midpoint; maker price improvement may be represented as a negative spread cost. `gross_pnl` adds fees, modeled impact and signed spread cost back to net marked P&L; `net_pnl` remains the actual cash-plus-inventory-mark result.
- Resting/passive fills require an explicit `event_kind=trade`, a trade price at/through the limit and a compatible known aggressor side. Unknown aggressor does not prove queue execution.
- L2 queue-ahead is conservative: displayed quantity at the limit is ahead; additions increase the high-water estimate; only qualifying prints consume that estimate. The seeded sensitivity model is deterministic per seed/order ID, not a calibrated probability distribution.
- Time-in-force includes cancel latency. Orders can still fill until modeled cancel effectiveness.

The engine does not claim FIFO. Aggregate depth does not expose all order IDs, hidden size,
  priority or exact queue position. FYERS Data Socket is five levels; its separate 50-level
  Protobuf TBT socket is not implemented by the current recorder. Upstox V3 documents five-level
  Full and 30-level Full D30 (Plus) modes; this active recorder path does not claim D30 support.
  Their update callbacks are not proof of a complete exchange event tape. Current normalizers emit
  last-trade *observations*, not aggressor-tagged unique prints, so those observations do not
  advance a passive queue. Passive fills in tests require explicitly constructed trade fixtures
  from a source that provides prints and aggressor direction.

### Model limitations

The single-instrument engine does not model multi-instrument cash/margin allocation. The portfolio engine adds long-only cash allocation and gross exposure reservation but does not model cross-order queue priority or margin. Neither engine models options Greeks/expiry/assignment, short borrow, financing, corporate-action cash settlement, dividends, auction uncrossing, circuit limits, venue rejects, exchange throttles, broker routing, hidden/iceberg liquidity, order amendments, queue-jump rules, matching-engine ordering or real venue fees by effective date. Both use a simple positive square-root impact coefficient, not a calibrated market-impact model. They do not translate two providers into a consolidated matching engine.

Do not use the backtest for capital allocation, compliance sign-off or live routing until a tick/TBT or exchange-grade dataset, exchange calendar and instrument terms, independent model calibration, broker/venue event semantics and out-of-sample paper execution evidence have been verified.

## 1.1 Multi-instrument cash portfolio simulator

`MultiInstrumentPortfolioBacktester` supports multiple instruments from exactly one provider stream. A replay with mixed Upstox/FYERS observations must explicitly select one provider; it never adds liquidity across two observations of the same venue book. The engine validates canonical instrument mapping and per-instrument local sequence monotonicity; orders are independent per instrument and the cash budget reserves estimated notional plus fee/impact allowance for every still-open buy order before accepting another instrument's buy.

It retains the conservative market-by-price queue logic, explicit trade/aggressor requirement, order/cancel latency, time-in-force, participation floor (never rounds a positive but sub-share cap up to one), partial fills and no automatic retry semantics. It marks cash and positions to the latest midpoint observed for each instrument, reports gross exposure, ending equity, fees, impact, spread attribution and unfilled quantity. It rejects sells exceeding current long inventory. It does not implement cross-instrument event-time synchronization beyond deterministic local receive ordering, portfolio-level margin, shorts, borrowed inventory, settlement cycles, dividend cash flows, corporate-action position mutation, auctions, halts, exchange order priority, derivatives, FX or cross-currency cash. Missing final marks are not synthesized; the report must be reviewed for unmarked residual positions.

Both simulators remain deterministic research models. L2 queue modeling is not FIFO and cannot be called exchange-certified. A real market-by-order feed and venue-specific matching rules are needed for stronger queue claims.

## 2. Deterministic strategy fixture

`DeterministicEmaCross` is a streaming recursive EMA crossover with one long entry and
exit intent. It accepts canonical quote/depth observations, maintains state, has no
broker call path, and emits `OrderIntent` only. Under owner directive D11 the synthetic demo
fixture (fixed prices, explicit seed and generated event IDs) and its CLI route were
retired; the strategy remains as an interface contract and none of its results are
research evidence.

## 3. Finite-family alpha search

`quantlab.alpha_search` searches a declared family of 25 configurations:

- 12 momentum lookback/threshold configurations;
- 9 mean-reversion lookback/z-threshold configurations;
- 4 fast/slow EMA-cross configurations.

It does not synthesize arbitrary Python or Rust code. The finite family makes the number of trials explicit and bounds researcher degrees of freedom. All candidates are evaluated on the same close series and declared cost assumptions.

The process:

1. Validate strictly increasing bar timestamps and positive finite closes.
2. Use chronological train, validation and untouched final holdout windows.
3. Use a one-return embargo at each split boundary.
4. Compute indicators causally (signal through close `t-1`, return into close `t`).
5. Start each segment flat and include entry turnover cost; liquidate at its final close and include exit turnover cost. No position or P&L carries between train/validation/holdout.
6. Score each candidate on validation net mean return; apply Holm family-wise adjustment to approximate HAC p-values as a tie-break.
7. Freeze one selected candidate and evaluate the untouched holdout once.
8. Report all candidates, full family size, cost assumptions, dataset hash, time range, split indices and approximate statistics.
9. Compute development-only combinatorially symmetric cross-validation (CSCV) probability-of-backtest-overfitting (PBO) diagnostics and validation-only Deflated Sharpe Ratio diagnostics; the untouched final holdout is excluded from both diagnostics and candidate selection.

The Sharpe annualisation factor is caller-supplied because the CLI cannot infer the sampling interval from irregular timestamps. Costs are flat one-way bps assumptions (brokerage, exchange/regulatory, taxes, spread, slippage and impact) rather than a dated Indian fee schedule. The close-only returns model is not an execution simulator; it omits queue, gap, borrow, financing, market capacity, order rejects, tick/lot constraints and several market microstructure effects. The HAC/normal p-value, CSCV/PBO implementation and Deflated Sharpe diagnostic are approximate research statistics, not proof of predictive significance. The candidate family is predeclared and bounded, not automatic arbitrary strategy-code generation. The search is close-based, and its net costs are caller-supplied flat bps rather than a dated, instrument/product-specific Indian fee schedule. The report can reject a candidate; it cannot certify alpha.

Example CSV format:

```csv
timestamp_ns,close
1800000000000000000,100.00
1800000060000000000,100.12
1800000120000000000,100.08
```

```bash
quantlab alpha-search bars.csv \
  --periods-per-year 252 \
  --brokerage-bps 2 \
  --exchange-regulatory-bps 0.5 \
  --taxes-bps 0.5 \
  --spread-bps 2 \
  --slippage-bps 1 \
  --market-impact-bps 1 \
  --output data/research/alpha-screen.json
```

The output file is immutable/no-overwrite. A positive holdout screen is not a trade recommendation. Any strategy promoted beyond research needs an independently governed hypothesis, point-in-time dataset, versioned artifact, effective cost schedule, multiple-testing register, walk-forward stability, regime sensitivity, event-level simulation, paper execution and independent approval.
