∆, [21.11.2026 06:33]
### Nirnay: rigorous technical review

My verdict: Nirnay is a serious early-stage quantitative research and paper-trading engineering project. It has several design decisions that I would preserve in an institutional trading platform. However, it is not yet comparable to the quantitative research, execution or HFT systems operated by established proprietary trading firms.

There is an important distinction, though: Nirnay is considerably closer to a prop-trading software architecture than Fieldbook.

Fieldbook is intended to collect information. Nirnay is intended to consume market information, construct historical datasets, research strategies, simulate orders, maintain account state and enforce risk controls.

That makes them potentially complementary systems:

* Fieldbook: global financial information acquisition and preservation.

* Nirnay: quantitative research, portfolio simulation, paper trading and eventually controlled trading infrastructure.

I reviewed the actual repository structure and implementation, including its Rust workspace, Python research engine, market-data ingestion, exchange reference parsers, point-in-time data builder, backtesters, risk authority, OMS, KDB-X integration, verification code and dated engineering evidence.

I did not just review its README.

### 1\. What Nirnay actually is

The current repository describes itself as an India-first systematic research, market-data capture, point-in-time dataset, event-driven paper simulation and controlled execution-contract platform.

Its key implementation areas are:

| Subsystem               | Main implementation                              |
| ----------------------- | ------------------------------------------------ |
| Financial domain        | india-domain, trading-core                       |
| Market-data archival    | market-archive                                   |
| Market-data events      | market-event, market-kernel                      |
| Market books            | market-book                                      |
| Broker data acquisition | Upstox, FYERS                                    |
| Reference data          | india-reference                                  |
| Research                | Python quantlab                                  |
| Backtesting             | Single-instrument and portfolio engines          |
| Statistical validation  | Holm, HAC, CSCV/PBO, Deflated Sharpe             |
| Paper execution         | paper-engine                                     |
| Risk                    | risk-engine, risk-authority                      |
| OMS                     | authoritative-oms, Python durable OMS            |
| Broker abstraction      | broker-adapter                                   |
| Strategy runtime        | strategy-runtime                                 |
| Analytics               | KDB-X/q                                          |
| Verification            | Independent C++ oracle, tests and evidence gates |

The Rust workspace has 2024-edition code, an explicitly layered package architecture, restricted unsafe code and strict Clippy policies. Python is used for research and some risk/OMS services, while the Rust side contains domain contracts, market-data components, paper-engine and risk components. raw.githubusercontent.com +3

This is a sensible separation of concerns.

### The actual architecture

flowchart TB
    A[Upstox / FYERS / Reference Sources] --> B[Raw Data Capture]
    B --> C[Raw Archive and Verification]
    C --> D[Canonical Market Events]
    D --> E[Point-in-Time Dataset]
    E --> F[Quant Research]
    F --> G[Strategy Validation]
    G --> H[Event-Driven Backtesting]
    H --> I[Paper Strategy Runtime]
    I --> J[Pre-trade Risk Authority]
    J --> K[Paper OMS]
    K --> L[Simulated Broker]
    L --> M[Account and Fill Records]
    M --> N[Reconciliation and Analytics]
The conceptual architecture is good.

∆, [21.11.2026 06:33]
The important caveat is that several of these connections are implemented as separate contracts or testable components rather than a completely integrated, independently certified production pipeline.

For example, the Rust broker adapter is a contract and simulated broker, not a functioning real broker gateway. The risk service has an independent-signature design but has not been deployed as an independent production service. raw.githubusercontent.com +3

### 2\. What is genuinely impressive

I would not dismiss this project as another retail trading bot.

There are some sophisticated ideas here.

### A. Financial correctness and exact arithmetic

The use of domain-specific types such as:

* Paise

* PricePaise

* Quantity

* Lots

* InstrumentId

* Venue

* LogicalNanos

is a good decision.

The Indian financial domain is represented using explicit financial units rather than allowing arbitrary numeric values to flow across the system.

The transaction-cost model also has effective-dated charge schedules and integer-based monetary calculations. Historical calculations can therefore select the cost schedule applicable to a particular trade date instead of using today's charges for historical trades. raw.githubusercontent.com +1

My assessment: Good domain engineering. This is an important foundation for avoiding subtle financial calculation errors.

It is not, by itself, an advantage unique to institutional firms, but it is the correct standard.

### B. Market-data semantics

I particularly like the work on providers.py, market.py and reconcile.py.

The project makes a distinction between:

* A quote observation.

* A depth observation.

* A last-trade observation.

* A confirmed trade print.

* An exchange sequence.

* A locally generated sequence.

For example, an LTP change is not automatically treated as a confirmed trade print with an aggressor side.

Similarly, two broker feeds are not combined by summing their quantities.

This avoids inventing liquidity or incorrectly inferring trade events.

The cross-provider reconciliation checks instrument identity, venue, timestamp semantics, freshness and price differences. raw.githubusercontent.com +2

These details indicate someone is thinking about market-data correctness rather than merely displaying quotes.

### C. Raw-data preservation and deterministic replay

The Rust market-archive crate is another good component.

It separates raw capture from normalized market events.

It supports:

* Original provider payload preservation.

* Hash-based integrity checks.

* Capture-session manifests.

* Explicit sequence scopes.

* Gap tracking.

* Bounded readers.

* Incomplete journal detection.

* Deterministic replay.

One particularly good decision is that a capture-session sequence cannot be mistaken for a genuine provider sequence.

The source code documents this distinction directly. raw.githubusercontent.com +2

This is important for reproducible quantitative research.

A backtest is only as trustworthy as its underlying historical information.

### D. Point-in-time dataset construction

This is one of the most important capabilities in the entire application.

The point_in_time.py implementation maintains historical listing identity with:

* Valid-time information.

* Knowledge-time information.

* Corporate-action revision lineage.

* Historical universe membership.

* Source/frame/event lineage.

* Immutable dataset publication.

* Dataset checksums.

* Optional detached Ed25519 signatures.

It also keeps raw prices separate from derived split/bonus-adjusted views.

This is the right general philosophy for serious quantitative research. raw.githubusercontent.com +1

Why does this matter?

Imagine testing an Indian equity strategy over 2016–2026.

A naïve data provider might give you today's surviving stocks and their adjusted histories.

That can introduce survivorship bias and look-ahead bias.

A good research platform needs to know which stocks existed at each historical date, what information was available at that time, and when a corporate action became known.

∆, [21.11.2026 06:33]
Nirnay has thought about this problem.

However, its own documentation acknowledges that the current point-in-time implementation has unresolved treatment for rights, mergers, demergers, delistings and incomplete historical universe membership. Those are important limitations, not minor details. raw.githubusercontent.com +1

### 3\. Backtesting: promising engineering, but not yet institutional-grade simulation

This is one of the areas where I see the biggest gap between Nirnay and a mature quantitative trading system.

There are two separate backtesting implementations.

### A. Single-instrument event-driven backtester

The implementation in research/python/src/quantlab/backtest.py supports:

* Historical quotes.

* Displayed L2 depth.

* Confirmed trade observations.

* Order submission latency.

* Cancellation latency.

* Partial fills.

* Transaction costs.

* Spread and market-impact assumptions.

* Participation limits.

* Queue-ahead approximation.

This is a good foundation.

I particularly like that the backtester does not simply assume that a limit order fills whenever the market touches its price.

It tracks displayed liquidity ahead of an order and uses confirmed trade events to consume the estimated queue. It also avoids repeatedly counting the same displayed liquidity in successive snapshots. raw.githubusercontent.com +2

However, there is a serious limitation.

This is not a complete exchange matching-engine simulator.

The book is market-by-price (MBP), not market-by-order (MBO).

The simulator therefore cannot accurately establish:

* Your exact position in an exchange FIFO queue.

* Which individual orders were ahead of you.

* Which individual orders were cancelled.

* Hidden liquidity.

* Exact priority changes.

* The effect of order amendments at individual-order level.

The queue model is an approximation based on visible depth.

That may be useful for early research, but strategies whose returns depend on queue position, passive fills or very short holding periods need much more accurate information.

### B. Multi-instrument portfolio backtester

The second implementation, portfolio_backtest.py, is a positive development.

It supports a portfolio rather than just an isolated instrument, including:

* Multiple NSE cash-equity instruments.

* Separate orders by symbol.

* Cash reservations.

* Exposure reservations.

* Long-only positions.

* Partial fills.

* Costs and market-impact assumptions.

* Participation constraints.

But its own specification is explicit: this is a single-provider, long-only, cash-funded portfolio model.

It does not support short inventory, borrowing or margin-based financing. It also does not provide exact exchange FIFO execution simulation. GitHub +2

Here is what I would consider missing for a serious institutional backtesting framework:

| Capability                     | Nirnay today                       | Required for advanced research                    |
| ------------------------------ | ---------------------------------- | ------------------------------------------------- |
| Single-instrument simulation   | Implemented                        | Yes                                               |
| Multiple instruments           | Implemented, constrained           | Yes                                               |
| Cash and exposure reservations | Implemented                        | Yes                                               |
| Partial fills                  | Implemented                        | Yes                                               |
| Latency model                  | Basic configurable model           | Calibrated latency distributions                  |
| L2 queue estimation            | Approximation                      | Venue-specific execution model                    |

∆, [21.11.2026 06:33]
| L3 queue simulation            | Absent                             | Required where strategy depends on queue priority |
| Market-impact model            | Simplified                         | Empirically calibrated                            |
| Short selling                  | Not implemented in portfolio model | Required for equity long/short                    |
| Borrow availability            | Absent                             | Required for short strategies                     |
| Margin and financing           | Absent                             | Required for leveraged strategies                 |
| Futures                        | Not established                    | Required for derivatives                          |
| Options                        | Not established                    | Required for derivatives                          |
| Corporate-action cashflows     | Incomplete                         | Required                                          |
| Auctions and halts             | Incomplete                         | Required                                          |
| Multi-venue execution          | Absent                             | Important for global strategies                   |
| Funding and settlement         | Limited                            | Required across asset classes                     |

The problem isn't that Nirnay has a simplistic backtester. The problem would be treating its current backtest results as evidence of executable strategy profitability.

My verdict: a credible early simulator, not an institutional execution simulator.

### 4\. Quant research and alpha discovery

This is another interesting part of Nirnay.

The Python research package includes:

* Strategy candidate generation.

* Momentum and mean-reversion families.

* EMA-based strategies.

* Walk-forward evaluation.

* Holdout testing.

* Holm-Bonferroni multiple-testing adjustment.

* Heteroskedasticity and autocorrelation-consistent statistical testing.

* Combinatorial purged-style cross-validation analysis.

* Probability of Backtest Overfitting.

* Deflated Sharpe Ratio diagnostics.

This is significantly more mature than simply running a strategy and looking at its Sharpe ratio. GitHub +2

But there are three critical issues.

### A. The strategy search is narrow

The candidate search is a finite, bounded collection of predefined momentum, mean-reversion and EMA configurations.

It is not a general-purpose factor research platform.

It does not yet provide a comprehensive framework for:

* Cross-sectional equity factors.

* Fundamental factor research.

* Statistical arbitrage.

* Factor portfolios.

* Pairs and basket trading.

* Intraday order-flow alpha.

* Market-making signals.

* Volatility strategies.

* Options pricing and volatility arbitrage.

* Futures term-structure strategies.

* Global macro strategies.

* Machine-learning research at scale.

The underlying research engine has a useful design, but it is a relatively small research surface. GitHub +1

### B. Statistical validation is not a substitute for good data

I like that Nirnay implements PBO and Deflated Sharpe diagnostics.

However, these diagnostics do not magically remove:

* Look-ahead bias.

* Survivorship bias.

* Data-snooping bias outside the tested candidate family.

* Incorrect corporate-action adjustments.

* Unrealistic transaction costs.

* Non-independent observations.

* Regime instability.

* Unmodelled market impact.

The implementation itself cautions that its Deflated Sharpe calculation is approximate and should not be used as the sole strategy acceptance criterion. GitHub +1

A major improvement would be a formal experiment-management system.

Every experiment should retain:

* Dataset ID and immutable version.

* Source-data lineage.

* Universe definition.

* Feature-generation version.

* Strategy code hash.

* Parameter search space.

* All attempted configurations.

* Validation and holdout periods.

* Cost and execution assumptions.

* Results and statistical diagnostics.

∆, [21.11.2026 06:33]
This prevents a researcher from repeatedly trying strategies against the same supposedly untouched holdout set.

### C. There is no demonstrated alpha

The repository has a deterministic EMA crossover strategy that serves as a systems-testing reference.

The strategy runtime currently supports one narrow strategy family rather than arbitrary production strategy implementations.

The project also explicitly records validated strategy net expectancy as unmeasured. GitHub +3

That is an important distinction.

A successful backtest-engine test means the backtester correctly implements the tested behavior.

It does not establish that the strategy is profitable.

A mature quantitative operation needs both excellent software infrastructure and independently demonstrated alpha.

### 5\. Risk management: one of Nirnay's strongest architectural areas

I think the risk architecture is one of the most promising components.

There are two important implementations:

* Python risk_service, including signed independent evidence.

* Rust risk-authority, including account state and atomic risk reservations.

The Rust authority is particularly interesting.

It maintains state involving:

* Positions.

* Pending orders.

* Account exposure.

* Daily P&L.

* Order rates.

* Quote health.

* Kill-switch status.

* Reconciliation state.

Risk evaluation and reservation happen together under a synchronized state transition, preventing concurrent requests from both being authorized using the same stale account state. GitHub +3

The risk policies also cover useful controls:

| Risk control               | Implementation |
| -------------------------- | -------------- |
| Maximum order quantity     | Yes            |
| Maximum order notional     | Yes            |
| Maximum position           | Yes            |
| Gross exposure             | Yes            |
| Daily loss                 | Yes            |
| Maximum open orders        | Yes            |
| Orders per second          | Yes            |
| Price deviation            | Yes            |
| Quote age                  | Yes            |
| Instrument allowlist       | Yes            |
| Strategy/account allowlist | Yes            |
| Kill switch                | Yes            |
| Market-data health         | Yes            |

This is good basic pre-trade risk engineering.

### But there is an integration gap

A risk authority and an OMS existing in the same repository does not mean the two have a fully certified end-to-end security boundary.

The risk service's independent-source and signing architecture is designed to require separate deployment, access controls and key management.

However, evidence documents that the KMS implementation, production identity controls, deployed service and independent data collectors are not yet established. GitHub +3

The newer Rust OMS stores references to risk permits, but the complete live risk-to-OMS-to-broker authorization path is not established.

A risk permit must be cryptographically and transactionally bound to the precise:

* Account.

* Instrument.

* Side.

* Quantity.

* Price limits.

* Strategy.

* Expiration.

* Order intent.

It must also be impossible to reuse or replay an expired or already consumed authorization.

The entire boundary needs concurrency testing, crash testing and independent security review.

### An important recent defect

The 21 November engineering evidence records a meaningful issue: the Rust risk authority could start in a default-ready state without the required start evidence.

The latest documented correction changed the initial state to fail closed, with the session closed, account snapshot unavailable, kill switch active and reconciliation blocked. GitHub +1

I consider this a positive sign about the development process: the project has an evidence trail that exposes defects rather than hiding them.

But a fixed defect is not the same as a completed institutional risk system.

My verdict: good architecture for a paper-trading risk authority; not yet independently certified production risk infrastructure.

∆, [21.11.2026 06:33]
### 6\. OMS and execution: better thought out than implemented

The OMS deserves particular attention.

There is a Rust authoritative-oms and a Python SQLite-backed durable OMS.

The designs include:

* Explicit order lifecycle states.

* Append-only order-event history.

* Durable intent handling.

* Transactional outbox patterns.

* Idempotency controls.

* Unknown-outcome handling.

* Cancellation state.

* Execution-event handling.

* Recovery and reconciliation.

The Python implementation uses SQLite WAL with full synchronization and atomic transactions for intent, event and outbox persistence. GitHub +2

The Rust implementation introduces a separate append-only commit journal, writer locking and fail-closed handling of uncertain commits.

One good operational rule is that an uncertain write must not automatically be retried as though nothing happened. A fresh recovery process needs to determine the authoritative state first. GitHub +2

That is the right instinct.

### The limitations

The current OMS is not a production institutional OMS.

It has no demonstrated:

* Exchange-certified execution gateway.

* Live broker order submission.

* Production multi-broker routing.

* Distributed high availability.

* Global account projection.

* Complete post-trade accounting.

* Production-grade multi-region recovery.

* Independently certified exactly-once or equivalent broker-delivery semantics.

The newer typed-fill implementation is useful, but its evidence explicitly stops short of claiming complete account projection or broker accounting correctness. GitHub +1

Also, the Rust broker adapter is an abstraction and simulation boundary. It is not proof that a real broker can be safely connected.

The repository currently states that live order routing is hard-disabled. Kite integration is read-only, and the other broker integration work has not produced a certified live execution path. GitHub +1

My verdict: promising durable paper OMS design, not a production execution management system.

### 7\. Market data: one of the biggest limitations

Nirnay's market-data architecture is well considered, but the actual provider coverage is narrow.

The implemented sources primarily concern:

* Upstox.

* FYERS.

* Kite read-only operational integration.

* NSE/BSE reference data.

The provider adapters are not equivalent to direct exchange feeds.

### What they actually provide

| Source | Demonstrated capability                                  | Important limitation                                                            |
| ------ | -------------------------------------------------------- | ------------------------------------------------------------------------------- |
| Upstox | Raw V3 Protobuf capture and decoding                     | Direct exchange packet capture and full exchange sequencing are not established |
| FYERS  | Data Socket callback capture, including five-level depth | Captures decoded SDK callback JSON, not original wire packets                   |
| Kite   | Read-only account and reference operations               | No order placement                                                              |
| NSE    | Reference-file parsing and validation                    | Not a complete direct market-data integration                                   |
| BSE    | Security-master parsing and validation                   | Not a complete direct market-data integration                                   |

The FYERS implementation does not establish exchange-level sequence continuity. Its local sequence is a collection sequence, not proof of the original exchange packet sequence.

The available FYERS implementation also does not demonstrate the 50-level TBT data path. GitHub +2

### Real data versus synthetic data

There is some actual source evidence, which is better than a project based entirely on mock feeds.

One recorded Upstox capture contains 36 five-level observations over approximately 87 seconds, with zero reported gaps.

∆, [21.11.2026 06:33]
The project also records NSE and BSE security-master file conformance. For the particular BSE file tested, 20,954 physical rows resulted in 4,856 accepted, 14,808 excluded and 1,290 rejected. These are findings for that particular file/version, not general BSE data-quality statistics. GitHub +1

This evidence is useful, but it is not sufficient to establish:

* Continuous live-feed reliability.

* Full trading-session data completeness.

* High-volume packet handling.

* Long-duration source stability.

* Complete historical market coverage.

* Exchange-level sequence recovery.

A 36-observation capture is a smoke test, not a market-data certification.

### A licensing issue that I would resolve before expanding

The November 20 rights correction is important.

The project previously did not adequately distinguish unknown permissions from explicit permissions or prohibitions.

It has now introduced three states:

UNKNOWN, PERMITTED, PROHIBITED.

All capture, storage, replay and redistribution rights for the relevant products remain UNKNOWN pending written evidence.

The system is designed to fail closed when rights are unknown. GitHub +1

That is the correct design.

It also means that data collection, historical retention and research replay should not be considered commercially authorized until the actual agreements support those uses.

### 8\. How does Nirnay compare with established quantitative prop firms?

This comparison needs some care.

A firm such as Jane Street or Hudson River Trading does not operate one application called its trading system.

It operates an ecosystem.

It typically includes specialized systems for market data, historical research, strategy development, simulation, portfolio construction, order management, risk, execution, operations and monitoring.

The internal architecture of these firms is proprietary. I am comparing Nirnay with their publicly documented engineering requirements and industry infrastructure, not claiming to know their private source code.

### Comparison by subsystem

| Subsystem                  | Nirnay                                     | Established prop-firm standard                             | Gap        |
| -------------------------- | ------------------------------------------ | ---------------------------------------------------------- | ---------- |
| Market-data capture        | Broker-derived feeds and raw journals      | Direct exchange feeds, redundant paths, full event capture | Very large |
| Instrument universe        | India-first cash-equity emphasis           | Broad global, multi-asset coverage                         | Very large |
| Historical data            | Archival and PIT foundations               | Deep, validated tick and reference history                 | Very large |
| Research                   | Python research engine and bounded search  | Extensive research and factor infrastructure               | Very large |
| Strategy validation        | Serious statistical diagnostics            | Extensive independent research validation                  | Large      |
| Event-driven backtesting   | Basic fills and queue approximation        | Venue-specific calibrated simulation                       | Very large |
| Portfolio construction     | Early cash-equity model                    | Multi-asset, multi-strategy portfolios                     | Very large |
| Pre-trade risk             | Good foundations                           | Independent, integrated institutional controls             | Large      |
| OMS                        | Durable paper OMS                          | Certified production OMS/EMS                               | Very large |
| Execution                  | Paper only                                 | Multiple production execution pathways                     | Absent     |

∆, [21.11.2026 06:33]
| Distributed infrastructure | Early-stage                                | Distributed, fault-tolerant production systems             | Very large |
| Performance engineering    | No trading-latency certification           | Extensive performance measurement and optimization         | Very large |
| Operational control        | Tests, evidence and monitoring foundations | Continuous production operations and incident controls     | Large      |

The most important takeaway:

Nirnay has more of the right architectural concepts than it has demonstrated production capabilities.

That is a meaningful distinction.

A mature firm doesn't just have a risk engine. It has verified that the engine operates correctly under concurrent order submissions, source failure, delayed market data, broker disconnections, partial fills, restarts and reconciliation problems.

Likewise, having a backtester doesn't mean its fills resemble real-world execution.

### 9\. How far is it from HFT?

This is where I would be particularly direct.

Nirnay is not an HFT application.

In fact, the November 20 M2 programme authorization explicitly limits the current roadmap to one NSE cash-equity vertical and excludes international expansion, colocated HFT and FPGA work. GitHub

That is important because HFT is not just ordinary strategy code written in a faster language.

It involves a different infrastructure discipline.

### What established low-latency firms publicly emphasize

Jane Street's public low-latency engineering description discusses low-level network applications, high throughput, performance counters, CPU-cache architecture and user-space packet processing technologies. janestreet.com

Hudson River Trading publicly describes systems engineering at the latency-throughput frontier, involving C++, operating systems, CPU architecture, network hardware and protocols. Hudson River Trading

Databento's published market-data architecture describes colocated connectivity, redundant capture infrastructure, FPGA-based packet capture and zero-copy processing. Databento Docs

These examples illustrate the differences in engineering focus.

### Direct HFT comparison

| Engineering requirement                  | Nirnay today     |
| ---------------------------------------- | ---------------- |
| Exchange multicast acquisition           | Not demonstrated |
| Native exchange order-by-order feeds     | Not demonstrated |
| Packet-level sequence recovery           | Not demonstrated |
| Multiple redundant exchange feed paths   | Not demonstrated |
| Hardware receive timestamps              | Not demonstrated |
| PTP clock synchronization evidence       | Not demonstrated |
| Colocation                               | Not demonstrated |
| DPDK / kernel-bypass networking          | Not demonstrated |
| FPGA / SmartNIC acquisition              | Not implemented  |
| Zero-copy market-data distribution       | Not demonstrated |
| CPU pinning and cache optimization       | Not demonstrated |
| Lock-free hot-path processing            | Not demonstrated |
| p50/p99/p99.9 trading-latency benchmarks | Not available    |
| Peak event-rate benchmarks               | Not established  |
| Hardware-in-the-loop failure testing     | Not established  |

There is also a telling implementation detail.

The market archive performs sync_data on individual appends, favoring durability over throughput. That is a reasonable choice for an early raw journal. It is not a design that should automatically be placed on the critical microsecond-sensitive processing path. GitHub +1

There is nothing wrong with this for Nirnay's current paper-trading objective.

But it demonstrates that the priorities are currently data integrity and deterministic correctness, not HFT latency.

### My HFT assessment

I would not spend money on FPGA, kernel bypass or colocation work at this stage.

First establish:

* Correct exchange data.

* Genuine order-level book reconstruction.

* Reliable event timestamps.

* Reproducible historical data.

* Accurate execution simulation.

∆, [21.11.2026 06:33]
* Proven strategy performance.

Only then would I consider a separate low-latency runtime.

For a strategy operating at 15-minute, hourly or daily horizons, HFT infrastructure may provide little or no meaningful advantage.

For a market-making or latency-sensitive strategy, it can be fundamental.

Those should be separate engineering programmes.

### 10\. Engineering quality: an area where Nirnay deserves credit

I want to distinguish the quality of the engineering process from the maturity of the product.

Nirnay has several good software-engineering practices:

* Versioned Rust workspace.

* Explicit package boundaries.

* Strict compiler and lint configuration.

* Restricted unsafe code.

* Deterministic fixtures.

* Independent oracle testing.

* Claims and capability ledgers.

* Evidence records tied to particular changes.

* Explicit distinctions between implemented, tested and verified.

* Fail-closed behavior for uncertain states.

The 19 November evidence records 455 Rust test cases, 162 Python tests and 11/11 hosted CI checks for the specific commit being certified. Newer evidence documents additional OMS and risk changes on November 21. These figures must not be mistaken for a rerun of the entire current main branch or a production certification. GitHub +3

I particularly like the claims ledger.

Many repositories say a feature is complete because the developer has implemented a function.

Nirnay attempts to distinguish:

* The code exists.

* The tests pass.

* The integration works.

* Actual provider data has been tested.

* The behavior has been independently verified.

* Production operation has been certified.

That is precisely the kind of discipline I would want to retain.

However, I would not confuse rigorous test engineering with trading-system maturity.

A test using synthetic market events cannot prove the correctness of an actual exchange feed.

A paper broker cannot prove the correctness of a live broker adapter.

A simulated risk event cannot prove the independence of a production risk-authority deployment.

The project itself makes these distinctions, and I agree with that approach.

### 11\. Overall technical scorecard

These are my qualitative assessments of the implementation I inspected, not measured benchmark scores.

| Area                      | Assessment                     | Reason                                                           |
| ------------------------- | ------------------------------ | ---------------------------------------------------------------- |
| Software architecture     | Good foundation                | Clear domain boundaries and layered Rust/Python design           |
| Financial data modelling  | Good foundation                | Exact arithmetic and explicit financial types                    |
| Raw-data integrity        | Good foundation                | Journaling, hashes, replay and sequence semantics                |
| Point-in-time correctness | Promising                      | Proper temporal lineage, but incomplete coverage                 |
| Market connectivity       | Early stage                    | Limited provider and instrument coverage                         |
| Order-book reconstruction | Early stage                    | Mainly aggregated depth, no demonstrated L3                      |
| Quant research            | Early stage                    | Useful statistical tools, limited research universe              |
| Backtesting               | Early stage                    | Useful simulation mechanics, simplified execution                |
| Strategy development      | Early stage                    | One reference strategy and finite candidate families             |
| Portfolio management      | Early stage                    | Limited cash-equity portfolio model                              |

∆, [21.11.2026 06:33]
| Risk management           | Promising                      | Good fail-closed architecture, integration incomplete            |
| OMS                       | Promising paper implementation | Durable state handling, no certified live execution              |
| HFT capabilities          | Absent                         | No demonstrated low-latency infrastructure or benchmarks         |
| Production readiness      | Not established                | Live integrations, operational certification and resilience gaps |

My classification:

Nirnay is an early institutional-style research and paper-trading infrastructure project, not an institutional trading platform.

It is more sophisticated than:

* A retail trading bot.

* A collection of Jupyter notebooks.

* A basic Python backtesting application.

* A simple broker API automation project.

But it has a substantial distance to cover before reaching the standards of established quantitative prop firms.

### 12\. What I would change to make Nirnay genuinely competitive

I would not try to turn it into a full HFT platform immediately.

I would first make it an exceptional quantitative research and simulation system.

There are six development priorities.

### Priority 1: Fix the data foundation

This has the highest priority.

Nirnay's research results cannot be better than its historical data.

I would require:

* Licensed institutional-quality historical data.

* Complete historical instrument universes.

* Delisted securities.

* Historical corporate actions.

* Full timestamp and source lineage.

* Historical corrections.

* Reliable historical market-depth records.

* Consistent instrument identification.

* Reproducible dataset publication.

This is where your Fieldbook project becomes strategically important.

### Priority 2: Build a genuinely comprehensive research framework

Expand beyond the current predefined strategy families.

I would introduce separate research modules:

quantlab/
│
├── data/
│   ├── datasets
│   ├── point_in_time
│   ├── universe
│   └── corporate_actions
│
├── features/
│   ├── technical
│   ├── fundamental
│   ├── microstructure
│   ├── volatility
│   ├── statistical
│   └── macro
│
├── strategies/
│   ├── momentum
│   ├── mean_reversion
│   ├── statistical_arbitrage
│   ├── factor
│   ├── market_making
│   ├── volatility
│   ├── derivatives
│   └── macro
│
├── experiments/
│   ├── registry
│   ├── parameter_search
│   ├── walk_forward
│   └── reproducibility
│
├── validation/
│   ├── statistical
│   ├── robustness
│   ├── leakage_detection
│   └── multiple_testing
│
├── simulation/
│   ├── event_replay
│   ├── execution
│   ├── portfolio
│   └── costs
│
└── reporting/
    ├── performance
    ├── risk
    └── attribution
The major addition should be an experiment registry that tracks every experiment, not just successful strategies.

### Priority 3: Rebuild the backtesting standard

Introduce a proper event-driven simulation system.

It needs to consume:

* Historical trades.

* Order additions.

* Order modifications.

* Cancellations.

* Order-book states.

* Market status.

* Auction events.

* Instrument changes.

It also needs configurable execution assumptions.

For example:

| Execution variable   | Requirement                   |
| -------------------- | ----------------------------- |
| Queue priority       | Venue-specific                |
| Latency              | Distribution-based            |
| Order acknowledgment | Explicit                      |
| Partial executions   | Event-driven                  |
| Market impact        | Calibrated                    |
| Fees                 | Effective-dated               |
| Slippage             | Venue and instrument-specific |
| Borrow               | Availability and cost         |
| Margin               | Initial and maintenance       |
| Liquidity            | Historical                    |
| Funding              | Time-dependent                |

An execution simulator should be calibrated using actual paper and, where appropriately authorized, real execution observations.

∆, [21.11.2026 06:33]
### Priority 4: Institutional risk and OMS integration

Complete the relationship between:

Strategy → Risk Authority → OMS → Simulated Execution → Account Projection

Each component should have a separately defined contract.

Add:

* Portfolio-wide limits.

* Multi-strategy exposure limits.

* Sector and concentration limits.

* Stress scenarios.

* Liquidity risk.

* Derivatives Greeks.

* Borrow and financing risk.

* Account-wide kill switches.

* Independent reconciliation.

* Failover and recovery certification.

I would keep live execution disabled until there is a separate, explicitly authorized programme to certify it.

### Priority 5: Improve research reproducibility and compute infrastructure

A mature quant platform needs to support many researchers and many experiments.

Build:

* Distributed experiment execution.

* Versioned research datasets.

* Feature-store lineage.

* Reproducible research environments.

* Scheduled research jobs.

* Experiment comparison.

* Resource allocation.

* Independent validation reports.

KDB-X is a useful existing component. I would complement rather than automatically replace it.

### Priority 6: Establish actual performance and reliability evidence

Create formal benchmarks.

For the research platform:

* Dataset loading throughput.

* Backtest throughput.

* Memory utilization.

* Historical query latency.

* Concurrent experiment capacity.

For market data:

* Event-processing throughput.

* Dropped-event count.

* Recovery time.

* Queue depth.

* End-to-end data latency.

For risk and OMS:

* Concurrent authorization throughput.

* P50/P99/P99.9 response times.

* Restart recovery.

* Persistence durability.

* Duplicate-event behavior.

* Reconciliation completeness.

For anything eventually entering production, repeat these measurements on the actual target infrastructure.

### 13\. How I would connect Nirnay with Fieldbook

This is probably the most important strategic recommendation given your larger objective.

I would not make Nirnay duplicate Fieldbook's global source-acquisition infrastructure.

I would define two independently deployable platforms with shared standards.

flowchart TB
    subgraph N["NIRNAY — QUANTITATIVE RESEARCH"]
        K --> L[Point-in-Time Dataset Builder]
        L --> M[Feature and Factor Research]
        M --> O[Strategy Research]
        O --> P[Backtesting and Simulation]
        P --> Q[Paper Trading]
        Q --> R[Risk and Performance Analysis]
    end
I would share four things between them:

* Universal instrument identifiers.

* Financial event schemas.

* Timestamp and provenance standards.

* Data-quality and historical-version metadata.

I would keep the following separate:

| Fieldbook owns            | Nirnay owns                    |
| ------------------------- | ------------------------------ |
| Source acquisition        | Research experiments           |
| Raw capture               | Features and factors           |
| Protocol decoding         | Strategy definitions           |
| Global reference data     | Historical strategy evaluation |
| Market-data normalization | Backtesting                    |
| Data-quality verification | Portfolio simulation           |
| Data entitlements         | Paper-trading accounts         |
| Historical preservation   | Strategy and risk analytics    |

This avoids two independent systems developing conflicting definitions of the same instrument, trade event or corporate action.

∆, [21.11.2026 06:33]
It also gives Fieldbook a potentially valuable customer base beyond its own user-facing data products.

Nirnay becomes one of the first sophisticated internal consumers of Fieldbook, without making either product dependent on the other's application logic.

### 14\. My final recommendation

I would preserve Nirnay's existing architecture, but change its ambition from:

India-first paper-trading application

to:

A global quantitative research, historical simulation and strategy validation infrastructure.

I would not prioritize HFT yet.

There is too much foundational work ahead in data quality, historical fidelity, research methodology and execution simulation.

The order matters:

| Development stage | Objective                                                           |
| ----------------- | ------------------------------------------------------------------- |
| First             | Correct and complete financial data                                 |
| Second            | Reproducible quantitative research                                  |
| Third             | Realistic execution simulation                                      |
| Fourth            | Portfolio construction and risk                                     |
| Fifth             | Robust institutional paper-trading infrastructure                   |
| Sixth             | Production execution, only if separately required                   |
| Seventh           | Dedicated HFT infrastructure, if justified by strategy requirements |

The biggest opportunity is the combination of Fieldbook and Nirnay.

Fieldbook could become the global financial information infrastructure. Nirnay could become the research and analytical environment that exploits that information.

One supplies trustworthy information; the other turns it into testable quantitative knowledge.

That is a considerably more interesting long-term architecture than trying to make Nirnay itself collect all the world's data, research strategies, execute orders and operate as an HFT platform.

### Review limitations

This was a static source-code and repository-evidence review. I inspected the implementation and supporting engineering records, but did not independently build and run the entire workspace, connect to real broker accounts, or reproduce live market-data captures. The test counts and operational results cited above are repository-recorded evidence, not independent certification.

Bottom line: Nirnay has a good engineering philosophy and several unusually thoughtful foundations for an early project. Its most important deficiencies are not Rust versus C++, or Python versus OCaml. They are data coverage, market microstructure fidelity, strategy validation, execution realism and production integration. Fix those in that order.
