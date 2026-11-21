#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Opt-in concurrent authorization benchmark. All fixtures are synthetic and
//! the workload is seeded. The BENCH-* line reports measured latencies and
//! throughput only; no latency or throughput threshold is asserted and this is
//! not a certificate of deployed risk-service performance.

use std::{
    collections::{
        BTreeMap, //
        BTreeSet,
    }, //
    sync::{
        Arc, //
        Barrier,
    },
    thread,
    time::Instant,
};

use risk_authority::{
    AccountJournalObservation, //
    FeedObservation,
    MarketObservation,
    Observation,
    ObservationSource,
    Observations,
    OmsObservation,
    OperatorObservation,
    OpsObservation,
    ReconciliationObservation,
    RiskAuthority,
};
use risk_engine::{
    RiskPolicy, //
    ValidatedRiskPolicy,
};
use trading_core::{
    AccountId, //
    AlgoId,
    Continuity,
    Instrument,
    InstrumentId,
    InstrumentType,
    IntentId,
    JournalRef,
    Listing,
    ListingStatus,
    ListingTerms,
    LogicalNanos,
    OrderIntent,
    Paise,
    PricePaise,
    Product,
    Quantity,
    QuoteSnapshot,
    ReconciliationReport,
    ReconciliationStatus,
    Side,
    StrategyId,
    TickAssignment,
    TradeDate,
    Venue,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const ACCOUNT: u64 = 1;
const STRATEGY: u64 = 2;
const ALGO: u64 = 3;
const TOTAL: u64 = 4_000;
const WORKERS: u64 = 8;
const SEED: u64 = 20_261_121;

fn instrument_id() -> Result<InstrumentId, Box<dyn std::error::Error>> {
    Ok(InstrumentId::new("nse.cash.synidx")?)
}

fn instrument() -> Result<Instrument, Box<dyn std::error::Error>> {
    let from = TradeDate::new(2026, 1, 1)?;
    let to = TradeDate::new(2026, 12, 31)?;
    Ok(Instrument {
        instrument_id: instrument_id()?,
        instrument_type: InstrumentType::Index,
        isin: None,
        name: "Synthetic index".to_owned(),
        underlying_instrument_id: None,
        expiry_date: None,
        strike_price_paise: None,
        option_kind: None,
        listings: vec![Listing {
            venue: Venue::Nse,
            segment: trading_core::Segment::Cash,
            venue_symbol: "SYNIDX".to_owned(),
            token: None,
            effective_from: from,
            effective_to: Some(to),
            status: ListingStatus::Active,
            settlement_regime: trading_core::SettlementRegime::Standard,
            z_category: false,
            settlement_group: None,
            terms: vec![ListingTerms {
                effective_from: from,
                effective_to: Some(to),
                tick_assignment: TickAssignment {
                    tick_paise: 5,
                    reference_close_paise: None,
                    reference_close_date: None,
                    reference_source: None,
                    reference_sha256_hex: None,
                    effective_month: None,
                    schedule_revision: "bench-fixture".to_owned(),
                    schedule_effective_from: None,
                    schedule_sha256_hex: None,
                    calendar_revision: None,
                    calendar_sha256_hex: None,
                },
                market_lot: 1,
                quantity_increment: 1,
            }],
        }],
        corporate_actions: Vec::new(),
    })
}

fn policy() -> Result<ValidatedRiskPolicy, Box<dyn std::error::Error>> {
    // Amplified limits keep every seeded intent acceptable so the measured
    // latency is the authorization path, not rejection bookkeeping.
    let raw = RiskPolicy {
        version: "risk-bench-v1".to_owned(),
        allowed_accounts: BTreeSet::from([AccountId::new(ACCOUNT)?]),
        allowed_strategies: BTreeSet::from([StrategyId::new(STRATEGY)?]),
        allowed_algos: BTreeSet::from([AlgoId::new(ALGO)?]),
        allowed_instruments: BTreeSet::from([instrument_id()?]),
        max_order_quantity: Quantity::new(20)?,
        max_order_notional_paise: Paise(1_000_000),
        max_position_quantity: Quantity::new(100_000)?,
        max_gross_exposure_paise: Paise(100_000_000),
        max_daily_loss_paise: Paise(100_000_000),
        max_open_orders: 100_000,
        max_orders_per_second: 100_000,
        max_price_deviation_ppm: 1_000_000,
        max_quote_age_ns: 1_000_000_000,
        allow_short: false,
        // Amplified for the 4,000-intent workload: the production default is
        // tighter, but this opt-in harness measures throughput, not policy.
        max_strategy_exposure_paise: Paise(1_000_000_000),
        max_sector_exposure_paise: None,
        stress_scenario: None,
        max_stress_loss_paise: None,
        max_participation_ppm: None,
        greek_limits: None,
        max_borrow_quantity: None,
        max_financing_cost_paise: None,
    };
    Ok(ValidatedRiskPolicy::from_policy(&raw)?)
}

fn authority() -> Result<RiskAuthority, Box<dyn std::error::Error>> {
    let authority = RiskAuthority::new(
        AccountId::new(ACCOUNT)?,
        policy()?,
        vec![instrument()?],
        TradeDate::new(2026, 10, 28)?,
    );
    authority.set_session_open(true)?;
    authority.set_snapshot_available(true)?;
    let tokens = authority.register_all_owners()?;
    authority.set_kill_switch(
        tokens.operator,
        false,
        "bench-operator".to_owned(),
        "bench fixture".to_owned(),
        Some("bench-release-evidence".to_owned()),
    )?;
    authority.set_reconciliation_blocked(
        tokens.reconciliation,
        false,
        "bench resolution".to_owned(),
        Some("bench-resolution-evidence".to_owned()),
    )?;
    Ok(authority)
}

fn intent(id: u64, quantity: i64) -> Result<OrderIntent, Box<dyn std::error::Error>> {
    Ok(OrderIntent {
        intent_id: IntentId::new(id)?,
        account_id: AccountId::new(ACCOUNT)?,
        strategy_id: StrategyId::new(STRATEGY)?,
        strategy_version: "bench-strategy-v1".to_owned(),
        algo_id: AlgoId::new(ALGO)?,
        instrument_id: instrument_id()?,
        venue: Venue::Nse,
        side: Side::Buy,
        product: Product::Intraday,
        quantity: Quantity::new(quantity)?,
        limit_price: PricePaise::new(Paise(1_000))?,
        reason: "benchmark intent".to_owned(),
        evidence: vec![JournalRef {
            sequence: 1,
            hash_hex: "ab".repeat(32),
        }],
        expires_at: LogicalNanos::new(100_000_000),
    })
}

fn observation() -> Result<FeedObservation, Box<dyn std::error::Error>> {
    Ok(FeedObservation {
        quote: Some(QuoteSnapshot {
            instrument_id: instrument_id()?,
            venue: Venue::Nse,
            bid_price: Some(PricePaise::new(Paise(995))?),
            bid_quantity: Quantity::new(10)?,
            ask_price: Some(PricePaise::new(Paise(1_005))?),
            ask_quantity: Quantity::new(10)?,
            continuity: Continuity::Snapshot,
        }),
        received_at: LogicalNanos::ZERO,
        gap: None,
        book_complete: true,
    })
}

fn observations(authority: &RiskAuthority, feed: FeedObservation) -> Observations {
    let tokens = authority.register_all_owners().expect("registered owners");
    let at = LogicalNanos::ZERO;
    Observations {
        market: Some(Observation::new(
            tokens.market,
            ObservationSource::MarketDataKernel,
            at,
            at,
            1,
            MarketObservation {
                feed,
                session_open: true,
                kernel_schema_version: 3,
                source_sequence: Some(1),
                connection_epoch: Some(1),
                depth: None,
                marks_paise: std::collections::BTreeMap::new(),
                greeks: None,
                borrow: None,
                synthetic: true,
            },
        )),
        oms: Some(Observation::new(
            tokens.oms,
            ObservationSource::Oms,
            at,
            at,
            1,
            OmsObservation {
                fills_digest: "a".repeat(64),
                unresolved_count: 0,
                last_journal_digest: "b".repeat(64),
                is_poisoned: false,
            },
        )),
        account_journal: Some(Observation::new(
            tokens.account_journal,
            ObservationSource::AccountJournal,
            at,
            at,
            1,
            AccountJournalObservation {
                head_hash_hex: "c".repeat(64),
                record_count: 1,
                ledger_state_digest: "d".repeat(64),
                opening_balance_evidence_ref: Some("bench-statement".to_owned()),
            },
        )),
        reconciliation: Some(Observation::new(
            tokens.reconciliation,
            ObservationSource::Reconciliation,
            at,
            at,
            1,
            ReconciliationObservation {
                report: ReconciliationReport {
                    reconciliation_id: "bench-recon".to_owned(),
                    status: ReconciliationStatus::Matched,
                    expected_positions: BTreeMap::new(),
                    observed_positions: Some(BTreeMap::new()),
                    expected_active_orders: BTreeSet::new(),
                    observed_active_orders: Some(BTreeSet::new()),
                    differences: Vec::new(),
                },
                replay_digest_matched: true,
            },
        )),
        ops: Some(Observation::new(
            tokens.ops,
            ObservationSource::Ops,
            at,
            at,
            1,
            OpsObservation {
                feed_age_ns: 0,
                sequence_gaps: 0,
                journal_append_latency_ns: 1_000,
                journal_queue_depth: 0,
                disk_available_bytes: 10_000_000,
                unknown_orders: 0,
                reconciliation_differences: 0,
            },
        )),
        operator: Some(Observation::new(
            tokens.operator,
            ObservationSource::OperatorControl,
            at,
            at,
            1,
            OperatorObservation {
                kill_switch_engaged: false,
                actor_id: "bench-operator".to_owned(),
                reason: "bench fixture".to_owned(),
                release_evidence_ref: Some("bench-release-evidence".to_owned()),
            },
        )),
    }
}

fn next_random(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state
}

fn percentile(sorted: &[u128], fraction: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

#[cfg(test)]
mod percentile_tests {
    use super::percentile;

    #[test]
    fn percentile_selects_known_quantiles() {
        let samples = [10_u128, 20, 30, 40, 50];
        assert_eq!(percentile(&samples, 0.5), 30, "median sample");
        assert_eq!(percentile(&samples, 0.999), 50, "upper sample");
        assert_eq!(percentile(&[], 0.5), 0, "empty input has no rank");
    }
}

#[test]
#[ignore = "opt-in performance measurement"]
fn seeded_concurrent_authorization_throughput() -> TestResult {
    let authority = Arc::new(authority()?);
    let barrier = Arc::new(Barrier::new(WORKERS as usize));
    let chunk = TOTAL / WORKERS;
    let mut handles = Vec::with_capacity(WORKERS as usize);
    for worker in 0..WORKERS {
        let authority = Arc::clone(&authority);
        let barrier = Arc::clone(&barrier);
        let ids: Vec<u64> = (worker * chunk + 1..=(worker + 1) * chunk).collect();
        handles.push(thread::spawn(move || {
            let observations = observations(&authority, observation().expect("observation"));
            let mut state = SEED.wrapping_add(worker);
            let mut latencies = Vec::with_capacity(ids.len());
            let mut accepted = 0_u64;
            let mut rejected = 0_u64;
            barrier.wait();
            for id in ids {
                let quantity = 1 + (next_random(&mut state) % 5) as i64;
                let target = intent(id, quantity).expect("intent");
                let started = Instant::now();
                let result = authority.evaluate_and_reserve(target, observations.clone(), None);
                latencies.push(started.elapsed().as_nanos());
                if result.is_ok() {
                    accepted += 1;
                } else {
                    rejected += 1;
                }
            }
            (latencies, accepted, rejected)
        }));
    }
    let started = Instant::now();
    let mut latencies = Vec::with_capacity(TOTAL as usize);
    let mut accepted = 0_u64;
    let mut rejected = 0_u64;
    for handle in handles {
        let (worker_latencies, worker_accepted, worker_rejected) =
            handle.join().expect("worker thread");
        latencies.extend(worker_latencies);
        accepted += worker_accepted;
        rejected += worker_rejected;
    }
    let elapsed = started.elapsed();
    latencies.sort_unstable();
    eprintln!(
        "BENCH-RISK-AUTH intents={TOTAL} workers={WORKERS} accepted={accepted} rejected={rejected} seconds={:.3} authorizations_per_sec={:.1} p50_us={:.1} p99_us={:.1} p999_us={:.1} seed={SEED} source=synthetic-local",
        elapsed.as_secs_f64(),
        accepted as f64 / elapsed.as_secs_f64(),
        percentile(&latencies, 0.50) as f64 / 1000.0,
        percentile(&latencies, 0.99) as f64 / 1000.0,
        percentile(&latencies, 0.999) as f64 / 1000.0,
    );
    assert_eq!(accepted + rejected, TOTAL, "every intent has a decision");
    assert_eq!(
        accepted, TOTAL,
        "amplified limits accept the seeded workload"
    );
    assert_eq!(latencies.len(), TOTAL as usize, "every latency is recorded");
    let permits: BTreeSet<u64> = (1..=TOTAL).collect();
    assert_eq!(
        permits.len(),
        TOTAL as usize,
        "seeded intent ids are unique"
    );
    Ok(())
}
