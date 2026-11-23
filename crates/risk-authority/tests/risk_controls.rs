#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Typed fixture falsifiers for the institutional risk controls. All sectors,
//! depth, marks, greeks and borrow values are synthetic; no real master or live
//! instrument is referenced.

use std::collections::{
    BTreeMap, //
    BTreeSet,
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
    GreekLimits, //
    RiskPolicy,
    ValidatedRiskPolicy,
};
use trading_core::{
    AccountId, //
    AlgoId,
    BorrowAvailability,
    Continuity,
    Greeks,
    Instrument,
    InstrumentId,
    InstrumentType,
    IntentId,
    JournalRef,
    LiquidityDepth,
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
    Segment,
    SettlementRegime,
    Side,
    StrategyId,
    TickAssignment,
    TradeDate,
    Venue,
    permit_digest,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const ACCOUNT: u64 = 1;
const STRATEGY: u64 = 2;
const OTHER_STRATEGY: u64 = 20;
const ALGO: u64 = 3;
const PRICE: i64 = 1_000;

fn date() -> Result<TradeDate, Box<dyn std::error::Error>> {
    Ok(TradeDate::new(2026, 10, 28)?)
}

fn instrument(id: &str, derivative: bool) -> Result<Instrument, Box<dyn std::error::Error>> {
    let from = TradeDate::new(2026, 1, 1)?;
    let to = TradeDate::new(2026, 12, 31)?;
    Ok(Instrument {
        instrument_id: InstrumentId::new(id)?,
        instrument_type: InstrumentType::Index,
        isin: None,
        name: "Synthetic control fixture".to_owned(),
        underlying_instrument_id: None,
        expiry_date: derivative.then_some(date()?),
        strike_price_paise: None,
        option_kind: None,
        listings: vec![Listing {
            venue: Venue::Nse,
            segment: Segment::Cash,
            venue_symbol: "SYN-CTRL".to_owned(),
            token: None,
            effective_from: from,
            effective_to: Some(to),
            status: ListingStatus::Active,
            settlement_regime: SettlementRegime::Standard,
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
                    schedule_revision: "control-fixture".to_owned(),
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

fn policy() -> Result<RiskPolicy, Box<dyn std::error::Error>> {
    Ok(RiskPolicy {
        version: "risk-controls-v1".to_owned(),
        allowed_accounts: BTreeSet::from([AccountId::new(ACCOUNT)?]),
        allowed_strategies: BTreeSet::from([
            StrategyId::new(STRATEGY)?,
            StrategyId::new(OTHER_STRATEGY)?,
        ]),
        allowed_algos: BTreeSet::from([AlgoId::new(ALGO)?]),
        allowed_instruments: BTreeSet::from([
            InstrumentId::new("nse.cash.synctl-a")?,
            InstrumentId::new("nse.cash.synctl-b")?,
            InstrumentId::new("nse.cash.synderiv")?,
            InstrumentId::new("nse.cash.unlabelled")?,
        ]),
        max_order_quantity: Quantity::new(10)?,
        max_order_notional_paise: Paise(100_000),
        max_position_quantity: Quantity::new(100)?,
        max_gross_exposure_paise: Paise(100_000),
        max_daily_loss_paise: Paise(5_000),
        max_open_orders: 10,
        max_orders_per_second: 10,
        max_price_deviation_ppm: 100_000,
        max_quote_age_ns: 1_000,
        allow_short: true,
        max_strategy_exposure_paise: Paise(9_000),
        max_sector_exposure_paise: Some(Paise(9_000)),
        stress_scenario: None,
        max_stress_loss_paise: None,
        max_participation_ppm: None,
        greek_limits: Some(GreekLimits {
            max_abs_delta_notional_paise: 10_000,
            max_abs_gamma: 50,
            max_abs_vega: 100,
            max_abs_theta: 50,
        }),
        max_borrow_quantity: Some(Quantity::new(10)?),
        max_financing_cost_paise: Some(Paise(100)),
    })
}

fn sectors() -> Result<BTreeMap<InstrumentId, String>, Box<dyn std::error::Error>> {
    Ok(BTreeMap::from([
        (
            InstrumentId::new("nse.cash.synctl-a")?,
            "synthetic-bank".to_owned(),
        ),
        (
            InstrumentId::new("nse.cash.synctl-b")?,
            "synthetic-bank".to_owned(),
        ),
        (
            InstrumentId::new("nse.cash.synderiv")?,
            "synthetic-derivative".to_owned(),
        ),
    ]))
}

fn authority(
    raw: RiskPolicy,
    with_sectors: bool,
) -> Result<RiskAuthority, Box<dyn std::error::Error>> {
    let instruments = vec![
        instrument("nse.cash.synctl-a", false)?,
        instrument("nse.cash.synctl-b", false)?,
        instrument("nse.cash.synderiv", true)?,
        instrument("nse.cash.unlabelled", false)?,
    ];
    let mut value = RiskAuthority::new(
        AccountId::new(ACCOUNT)?,
        ValidatedRiskPolicy::from_policy(&raw)?,
        instruments,
        date()?,
    );
    if with_sectors {
        value = value.with_sectors(sectors()?);
    }
    value.set_session_open(true)?;
    value.set_snapshot_available(true)?;
    let tokens = value.register_all_owners()?;
    value.set_kill_switch(
        tokens.operator,
        false,
        "fixture-operator".to_owned(),
        "fixture".to_owned(),
        Some("release-fixture".to_owned()),
    )?;
    value.set_reconciliation_blocked(
        tokens.reconciliation,
        false,
        "fixture".to_owned(),
        Some("resolution-fixture".to_owned()),
    )?;
    Ok(value)
}

fn intent(
    id: u64,
    instrument_id: &str,
    strategy: u64,
    quantity: i64,
    side: Side,
) -> Result<OrderIntent, Box<dyn std::error::Error>> {
    Ok(OrderIntent {
        intent_id: IntentId::new(id)?,
        account_id: AccountId::new(ACCOUNT)?,
        strategy_id: StrategyId::new(strategy)?,
        strategy_version: "strategy-v1".to_owned(),
        algo_id: AlgoId::new(ALGO)?,
        instrument_id: InstrumentId::new(instrument_id)?,
        venue: Venue::Nse,
        side,
        product: Product::Intraday,
        quantity: Quantity::new(quantity)?,
        limit_price: PricePaise::new(Paise(PRICE))?,
        reason: "control fixture intent".to_owned(),
        evidence: vec![JournalRef {
            sequence: 1,
            hash_hex: "ab".repeat(32),
        }],
        expires_at: LogicalNanos::new(20_000),
    })
}

fn observations(
    value: &RiskAuthority,
    greeks: Option<Greeks>,
    borrow: Option<BorrowAvailability>,
) -> Result<Observations, Box<dyn std::error::Error>> {
    let tokens = value.register_all_owners()?;
    let at = LogicalNanos::ZERO;
    let quote = QuoteSnapshot {
        instrument_id: InstrumentId::new("nse.cash.synctl-a")?,
        venue: Venue::Nse,
        bid_price: Some(PricePaise::new(Paise(995))?),
        bid_quantity: Quantity::new(10)?,
        ask_price: Some(PricePaise::new(Paise(1_005))?),
        ask_quantity: Quantity::new(10)?,
        continuity: Continuity::Snapshot,
    };
    Ok(Observations {
        market: Some(Observation::new(
            tokens.market,
            ObservationSource::MarketDataKernel,
            at,
            at,
            1,
            MarketObservation {
                feed: FeedObservation {
                    quote: Some(quote),
                    received_at: at,
                    gap: None,
                    book_complete: true,
                },
                session_open: true,
                kernel_schema_version: 3,
                source_sequence: Some(1),
                connection_epoch: Some(1),
                depth: Some(LiquidityDepth {
                    displayed_quantity: Quantity::new(10)?,
                    adv_quantity: Quantity::new(100)?,
                }),
                marks_paise: BTreeMap::from([(
                    InstrumentId::new("nse.cash.synctl-a")?,
                    Paise(PRICE),
                )]),
                greeks,
                borrow,
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
                opening_balance_evidence_ref: Some("statement-fixture".to_owned()),
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
                    reconciliation_id: "recon-fixture".to_owned(),
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
                actor_id: "fixture-operator".to_owned(),
                reason: "fixture".to_owned(),
                release_evidence_ref: Some("release-fixture".to_owned()),
            },
        )),
    })
}

#[test]
fn permit_digest_changes_when_an_authorized_intent_is_tampered() -> TestResult {
    let mut raw = policy()?;
    raw.max_sector_exposure_paise = None;
    raw.max_participation_ppm = None;
    raw.greek_limits = None;
    raw.max_borrow_quantity = None;
    raw.max_financing_cost_paise = None;
    let value = authority(raw, false)?;
    let order = intent(1, "nse.cash.synctl-a", STRATEGY, 2, Side::Buy)?;
    let reservation =
        value.evaluate_and_reserve(order.clone(), observations(&value, None, None)?, None)?;
    let digest = value.permit_digest(&reservation, &order)?;
    assert_eq!(
        digest,
        permit_digest(
            reservation.permit_id,
            &reservation.assessment.policy_version,
            reservation.snapshot_revision,
            &order
        ),
        "the authority exposes the same canonical digest as the OMS"
    );
    let mut tampered = order;
    tampered.quantity = Quantity::new(3)?;
    assert_ne!(
        digest,
        value.permit_digest(&reservation, &tampered)?,
        "tampering with an authorized intent changes its digest"
    );
    Ok(())
}
