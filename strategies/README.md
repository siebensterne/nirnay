# Strategies

## Deterministic reference strategy

`quantlab.strategies.DeterministicEmaCross` is the shipped deterministic streaming
EMA-cross example. It accepts canonical market events, maintains causal recursive EMA
state, emits long-only `OrderIntent` values and has no broker, OMS, risk-service, cash
or position-authority access.

Under owner directive D11 the synthetic `quantlab demo` driver and its fixed price
session were removed, together with the CLI route. The strategy module remains as an
interface contract only; no synthetic demonstration is runnable and no strategy result
is presented as research evidence. It is not an endorsed strategy, investment
recommendation or evidence of profitability.

## Strategy admission

A strategy is added here only after its economic hypothesis, falsifier, universe, point-in-time data requirements, cost assumptions, benchmarks, validation protocol and evidence bundle have been registered. Strategy modules emit intents; they never access broker SDKs, OMS mutation methods, authoritative cash/positions, risk permits or live-routing switches.

Candidate promotion requires, at minimum: immutable dataset and strategy hashes; chronological train/validation/untouched holdout; explicit embargo/purge policy; multiple-testing accounting; effective dated brokerage/exchange/tax charges; spread, slippage, market-impact and capacity assumptions; event-level simulation; regime and sensitivity analysis; paper evidence; and independent review. A positive bar-close screen is not alpha certification.
