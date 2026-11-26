#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.4 risk authority tests. Fixtures are synthetic; no order submission or
//! broker interaction exists in this crate.

use std::{
    collections::{
        BTreeMap, //
        BTreeSet,
    },
    sync::{
        Arc, //
        Barrier,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

use risk_authority::{
    AccountJournalObservation, //
    AccountRecoveryRecord,
    AuthorityError,
    FeedObservation,
    FileControlStore,
    MarketObservation,
    Observation,
    ObservationPolicy,
    ObservationSource,
    Observations,
    OmsObservation,
    OperatorObservation,
    OpsObservation,
    OwnerTokens,
    ReconciliationObservation,
    RiskAuthority,
    SequenceGap,
    authority::{
        DailyPnlBasis, //
        DailyPnlCalendar,
        DailyPnlError,
        DailyPnlRecord,
        DailyPnlRecovery,
        ValuationRecord,
        daily_pnl_positions_digest,
    },
    genesis_digest,
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
    RiskCheck,
    Segment,
    SettlementRegime,
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
const MAX_GROSS: i64 = 8_000;
const PRICE: i64 = 1_000;

fn date() -> Result<TradeDate, Box<dyn std::error::Error>> {
    Ok(TradeDate::new(2026, 10, 28)?)
}

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
            segment: Segment::Cash,
            venue_symbol: "SYNIDX".to_owned(),
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
                    schedule_revision: "test-fixture".to_owned(),
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
    let raw = RiskPolicy {
        version: "risk-v1".to_owned(),
        allowed_accounts: BTreeSet::from([AccountId::new(ACCOUNT)?]),
        allowed_strategies: BTreeSet::from([StrategyId::new(STRATEGY)?]),
        allowed_algos: BTreeSet::from([AlgoId::new(ALGO)?]),
        allowed_instruments: BTreeSet::from([instrument_id()?]),
        max_order_quantity: Quantity::new(10)?,
        max_order_notional_paise: Paise(10_000),
        max_position_quantity: Quantity::new(100)?,
        max_gross_exposure_paise: Paise(MAX_GROSS),
        max_daily_loss_paise: Paise(5_000),
        max_open_orders: 5,
        max_orders_per_second: 5,
        max_price_deviation_ppm: 100_000,
        max_quote_age_ns: 1_000,
        allow_short: false,
        max_strategy_exposure_paise: Paise(1_000_000),
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

fn bare_authority() -> Result<RiskAuthority, Box<dyn std::error::Error>> {
    let authority = RiskAuthority::new(
        AccountId::new(ACCOUNT)?,
        policy()?,
        vec![instrument()?],
        date()?,
    );
    // Readiness must be supplied explicitly; the default is unready.
    authority.set_session_open(true)?;
    authority.set_snapshot_available(true)?;
    release_switch(&authority)?;
    clear_reconciliation(&authority)?;
    Ok(authority)
}

/// Explicit business-day boundary for fixtures. No venue is hardcoded; the
/// time zone and half-open instant window are owner-supplied facts.
fn daily_calendar() -> Result<DailyPnlCalendar, Box<dyn std::error::Error>> {
    Ok(DailyPnlCalendar::new(
        "Asia/Kolkata".to_owned(),
        date()?,
        -86_400_000_000_000,
        86_400_000_000_000,
    ))
}

/// A sealed account-journal event carrying only P&L effects and no position
/// change; zero-net events are not materialized as positions.
fn daily_accounts(
    realized: i64,
    fees: i64,
    funding: i64,
    fx: i64,
) -> Result<Vec<AccountRecoveryRecord>, Box<dyn std::error::Error>> {
    let mut record = AccountRecoveryRecord::new(
        "pnl-fixture".to_owned(),
        AccountId::new(ACCOUNT)?,
        instrument_id()?,
        0,
    )
    .with_pnl_effects(realized, fees, funding, fx);
    record.seal(1, genesis_digest());
    Ok(vec![record])
}

fn daily_valuations() -> Vec<ValuationRecord> {
    Vec::new()
}

#[allow(clippy::too_many_arguments)] // fixture builder takes the exact sealed record fields
fn sealed_daily_record(
    accounts: &[AccountRecoveryRecord],
    valuations: &[ValuationRecord],
    calendar: &DailyPnlCalendar,
    basis: DailyPnlBasis,
    realized: i64,
    unrealized: i64,
    fees: i64,
    funding: i64,
    fx: i64,
) -> Result<DailyPnlRecord, Box<dyn std::error::Error>> {
    sealed_daily_record_for_positions(
        &BTreeMap::new(),
        accounts,
        valuations,
        calendar,
        basis,
        realized,
        unrealized,
        fees,
        funding,
        fx,
    )
}

#[allow(clippy::too_many_arguments)] // fixture builder takes positions plus the exact sealed fields
fn sealed_daily_record_for_positions(
    positions: &BTreeMap<InstrumentId, i64>,
    accounts: &[AccountRecoveryRecord],
    valuations: &[ValuationRecord],
    calendar: &DailyPnlCalendar,
    basis: DailyPnlBasis,
    realized: i64,
    unrealized: i64,
    fees: i64,
    funding: i64,
    fx: i64,
) -> Result<DailyPnlRecord, Box<dyn std::error::Error>> {
    let account_head = accounts.last().map_or_else(
        || genesis_digest().to_owned(),
        |record| record.digest.clone(),
    );
    let valuation_head = valuations.last().map_or_else(
        || genesis_digest().to_owned(),
        |record| record.digest.clone(),
    );
    let mut record = DailyPnlRecord::new(
        AccountId::new(ACCOUNT)?,
        date()?,
        "INR".to_owned(),
        basis,
        calendar,
        0,
    )
    .with_journal(u64::try_from(accounts.len())?, account_head)
    .with_digests(daily_pnl_positions_digest(positions), valuation_head)
    .with_components(realized, unrealized, fees, funding, fx);
    record.seal(1, genesis_digest());
    Ok(record)
}

fn publish_daily(
    authority: &RiskAuthority,
    record: DailyPnlRecord,
    accounts: &[AccountRecoveryRecord],
    valuations: &[ValuationRecord],
    calendar: &DailyPnlCalendar,
) -> Result<(), Box<dyn std::error::Error>> {
    authority.publish_daily_pnl(
        authority.register_all_owners()?.account_journal,
        record,
        accounts,
        valuations,
        calendar,
    )?;
    Ok(())
}

fn publish_zero_daily(authority: &RiskAuthority) -> Result<(), Box<dyn std::error::Error>> {
    let calendar = daily_calendar()?;
    let accounts = daily_accounts(0, 0, 0, 0)?;
    let valuations = daily_valuations();
    let record = sealed_daily_record(
        &accounts,
        &valuations,
        &calendar,
        DailyPnlBasis::RealizedPlusUnrealized,
        0,
        0,
        0,
        0,
        0,
    )?;
    publish_daily(authority, record, &accounts, &valuations, &calendar)
}

fn authority() -> Result<RiskAuthority, Box<dyn std::error::Error>> {
    let authority = bare_authority()?;
    publish_zero_daily(&authority)?;
    Ok(authority)
}

fn daily_recovery_for_accounts(
    accounts: &[AccountRecoveryRecord],
) -> Result<(DailyPnlRecovery, Vec<AccountRecoveryRecord>), Box<dyn std::error::Error>> {
    let calendar = daily_calendar()?;
    // Always include one zero-delta journal event so the observation's
    // account-snapshot evidence remains present even with no fills.
    let mut journal = accounts.to_vec();
    let previous = journal.last().map_or_else(
        || genesis_digest().to_owned(),
        |record| record.digest.clone(),
    );
    let sequence = u64::try_from(journal.len())? + 1;
    let mut pnl_event = AccountRecoveryRecord::new(
        "pnl-fixture".to_owned(),
        AccountId::new(ACCOUNT)?,
        instrument_id()?,
        0,
    );
    pnl_event.seal(sequence, &previous);
    journal.push(pnl_event);

    let mut positions: BTreeMap<InstrumentId, i64> = BTreeMap::new();
    for record in &journal {
        let position = positions.entry(record.instrument_id.clone()).or_insert(0);
        *position = (*position)
            .checked_add(record.quantity_delta)
            .ok_or("account fixture quantity overflow")?;
    }
    positions.retain(|_, quantity| *quantity != 0);
    // Every non-zero position needs a sealed mark with zero unrealized P&L;
    // an unmarked position must never establish a daily snapshot (H2).
    let mut valuations = Vec::new();
    let mut valuation_previous = genesis_digest().to_owned();
    for (index, (instrument, quantity)) in positions.iter().enumerate() {
        let mut valuation = ValuationRecord::new(
            AccountId::new(ACCOUNT)?,
            instrument.clone(),
            *quantity,
            1_000,
            1_000,
            0,
        );
        valuation.seal(u64::try_from(index)? + 1, &valuation_previous);
        valuation_previous = valuation.digest.clone();
        valuations.push(valuation);
    }
    let account_head = journal.last().map_or_else(
        || genesis_digest().to_owned(),
        |record| record.digest.clone(),
    );
    let valuation_head = valuations.last().map_or_else(
        || genesis_digest().to_owned(),
        |record| record.digest.clone(),
    );
    let mut record = DailyPnlRecord::new(
        AccountId::new(ACCOUNT)?,
        date()?,
        "INR".to_owned(),
        DailyPnlBasis::RealizedPlusUnrealized,
        &calendar,
        0,
    )
    .with_journal(u64::try_from(journal.len())?, account_head)
    .with_digests(daily_pnl_positions_digest(&positions), valuation_head);
    record.seal(1, genesis_digest());
    Ok((
        DailyPnlRecovery {
            record: Some(record),
            valuations,
            calendar: Some(calendar),
        },
        journal,
    ))
}

fn super_authority() -> Result<RiskAuthority, Box<dyn std::error::Error>> {
    authority()
}

fn release_switch(authority: &RiskAuthority) -> Result<(), Box<dyn std::error::Error>> {
    let tokens = authority.register_all_owners()?;
    authority.set_kill_switch(
        tokens.operator,
        false,
        "test-operator".to_owned(),
        "fixture bootstrap".to_owned(),
        Some("bootstrap-evidence".to_owned()),
    )?;
    Ok(())
}

fn clear_reconciliation(authority: &RiskAuthority) -> Result<(), Box<dyn std::error::Error>> {
    let tokens = authority.register_all_owners()?;
    authority.set_reconciliation_blocked(
        tokens.reconciliation,
        false,
        "fixture resolution".to_owned(),
        Some("resolution-evidence".to_owned()),
    )?;
    Ok(())
}

/// A newly constructed authority has no evidence of session, account or
/// operational readiness. It must deny new risk without any observation.
#[test]
fn fresh_authority_denies_new_risk() -> TestResult {
    let authority = RiskAuthority::new(
        AccountId::new(ACCOUNT)?,
        policy()?,
        vec![instrument()?],
        date()?,
    );
    release_switch(&authority)?;
    clear_reconciliation(&authority)?;
    publish_zero_daily(&authority)?;
    let missing = authority
        .evaluate_and_reserve(
            intent(1, 2, PRICE, Side::Buy)?,
            Observations::default(),
            None,
        )
        .expect_err("an authority without owner observations must reject");
    assert!(
        matches!(missing, AuthorityError::MissingObservation { .. }),
        "no owner observation is a typed denial"
    );

    let mut closed = observation(&authority)?;
    closed.market.as_mut().expect("market").payload.session_open = false;
    let rejected = authority
        .evaluate_and_reserve(intent(2, 2, PRICE, Side::Buy)?, closed, None)
        .expect_err("a closed session must reject");
    assert!(
        rejected_check(&rejected, RiskCheck::SessionOpen),
        "fresh authority fails the session check"
    );

    let mut unevidenced = observation(&authority)?;
    unevidenced
        .account_journal
        .as_mut()
        .expect("journal")
        .payload
        .opening_balance_evidence_ref = None;
    let rejected = authority
        .evaluate_and_reserve(intent(3, 2, PRICE, Side::Buy)?, unevidenced, None)
        .expect_err("no opening evidence must reject");
    assert!(
        rejected_check(&rejected, RiskCheck::AccountSnapshotAvailable),
        "fresh authority fails the account snapshot check"
    );

    let mut killed = observation(&authority)?;
    killed
        .operator
        .as_mut()
        .expect("operator")
        .payload
        .kill_switch_engaged = true;
    let result = authority.evaluate_and_reserve(intent(4, 2, PRICE, Side::Buy)?, killed, None);
    assert!(
        matches!(result, Err(AuthorityError::KillSwitchEngaged { .. })),
        "an engaged kill switch is a typed denial"
    );

    let mut blocked = observation(&authority)?;
    blocked
        .reconciliation
        .as_mut()
        .expect("reconciliation")
        .payload
        .report
        .status = ReconciliationStatus::Unavailable;
    let result = authority.evaluate_and_reserve(intent(5, 2, PRICE, Side::Buy)?, blocked, None);
    assert!(
        matches!(result, Err(AuthorityError::ReconciliationMismatch { .. })),
        "an unavailable reconciliation is a typed denial"
    );
    Ok(())
}

fn intent(
    id: u64,
    quantity: i64,
    price: i64,
    side: Side,
) -> Result<OrderIntent, Box<dyn std::error::Error>> {
    Ok(OrderIntent {
        intent_id: IntentId::new(id)?,
        account_id: AccountId::new(ACCOUNT)?,
        strategy_id: StrategyId::new(STRATEGY)?,
        strategy_version: "strategy-v1".to_owned(),
        algo_id: AlgoId::new(ALGO)?,
        instrument_id: instrument_id()?,
        venue: Venue::Nse,
        side,
        product: Product::Intraday,
        quantity: Quantity::new(quantity)?,
        limit_price: PricePaise::new(Paise(price))?,
        reason: "test intent".to_owned(),
        evidence: vec![JournalRef {
            sequence: 1,
            hash_hex: "ab".repeat(32),
        }],
        expires_at: LogicalNanos::new(20_000),
    })
}

fn quote() -> Result<QuoteSnapshot, Box<dyn std::error::Error>> {
    Ok(QuoteSnapshot {
        instrument_id: instrument_id()?,
        venue: Venue::Nse,
        bid_price: Some(PricePaise::new(Paise(995))?),
        bid_quantity: Quantity::new(1)?,
        ask_price: Some(PricePaise::new(Paise(1_005))?),
        ask_quantity: Quantity::new(1)?,
        continuity: Continuity::Snapshot,
    })
}

fn feed() -> Result<FeedObservation, Box<dyn std::error::Error>> {
    Ok(FeedObservation {
        quote: Some(quote()?),
        received_at: LogicalNanos::ZERO,
        gap: None,
        book_complete: true,
    })
}

/// Synthetic ready observations for all six owners. The market payload is
/// explicitly labelled synthetic; this is a fixture, not operational evidence.
fn observation(authority: &RiskAuthority) -> Result<Observations, Box<dyn std::error::Error>> {
    observations_for_feed(authority, feed()?)
}

/// Bind the account-journal observation to the same sealed head that produced
/// the authority's P&L snapshot; a different head is tested explicitly below.
fn observations_for_feed(
    authority: &RiskAuthority,
    feed: FeedObservation,
) -> Result<Observations, Box<dyn std::error::Error>> {
    let mut observations = observations_with(feed, authority.register_all_owners()?);
    let snapshot = authority.snapshot()?;
    if let Some(ref digest) = snapshot.daily_pnl_journal_digest {
        observations
            .account_journal
            .as_mut()
            .expect("journal")
            .payload
            .head_hash_hex = digest.clone();
    }
    if let Some(sequence) = snapshot.daily_pnl_journal_sequence {
        observations
            .account_journal
            .as_mut()
            .expect("journal")
            .payload
            .record_count = sequence;
    }
    Ok(observations)
}

fn observations_with(feed: FeedObservation, tokens: OwnerTokens) -> Observations {
    let at = feed.received_at;
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
                feed_age_ns: at.get(),
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
                actor_id: "test-operator".to_owned(),
                reason: "fixture".to_owned(),
                release_evidence_ref: Some("release-fixture".to_owned()),
            },
        )),
    }
}

fn rejected_check(error: &AuthorityError, check: RiskCheck) -> bool {
    match error {
        AuthorityError::Rejected(assessment) => assessment
            .checks
            .iter()
            .any(|result| result.check == check && !result.passed),
        _ => false,
    }
}

#[test]
fn baseline_reservation_is_accepted() -> TestResult {
    let authority = authority()?;
    let reservation = authority.evaluate_and_reserve(
        intent(1, 6, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    )?;
    assert!(
        reservation.assessment.accepted,
        "baseline intent is accepted"
    );
    assert_eq!(
        reservation.quantity.get(),
        6,
        "the reserved quantity is bound"
    );
    assert_eq!(
        reservation.reserved_notional_paise,
        Paise(6_000),
        "exact notional is reserved"
    );
    assert_eq!(
        reservation.snapshot_revision,
        authority.revision()?,
        "the permit records the complete committed state revision"
    );
    Ok(())
}

/// Two simultaneous reservations against one aggregate limit: only one can
/// pass, because evaluation and reservation share one lock and the exposure
/// used by the second call is the authority's freshly updated state.
#[test]
fn concurrent_reservations_cannot_exceed_aggregate_limit() -> TestResult {
    let authority = Arc::new(authority()?);
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for id in 1..=2_u64 {
        let authority = Arc::clone(&authority);
        let barrier = Arc::clone(&barrier);
        let target = intent(id, 6, PRICE, Side::Buy)?;
        handles.push(thread::spawn(move || {
            let observation = observation(&authority).expect("observation");
            barrier.wait();
            authority.evaluate_and_reserve(target, observation, None)
        }));
    }
    let results: Vec<Result<_, _>> = handles
        .into_iter()
        .map(|handle| handle.join().expect("thread"))
        .collect();
    let accepted = results.iter().filter(|result| result.is_ok()).count();
    let rejected = results.iter().filter(|result| result.is_err()).count();
    assert_eq!(
        accepted, 1,
        "exactly one concurrent reservation fits the aggregate limit"
    );
    assert_eq!(rejected, 1, "the competing reservation is rejected");
    let failure = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .expect("rejection");
    assert!(
        rejected_check(failure, RiskCheck::GrossExposure),
        "the aggregate exposure check is the binding constraint"
    );
    let snapshot = authority.snapshot()?;
    assert_eq!(
        snapshot.pending_gross_exposure_paise,
        Paise(6_000),
        "only one notional is held"
    );
    Ok(())
}

/// A caller holding an older account snapshot cannot spend against it.
#[test]
fn stale_snapshot_revision_is_rejected() -> TestResult {
    let authority = authority()?;
    let observed_revision = authority.revision()?;
    authority.evaluate_and_reserve(
        intent(1, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    )?;
    let stale = authority.evaluate_and_reserve(
        intent(2, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        Some(observed_revision),
    );
    assert!(
        matches!(stale, Err(AuthorityError::StaleSnapshot { expected, current }) if expected == observed_revision && current == observed_revision + 1),
        "the stale revision fence is enforced against authority-owned state"
    );
    Ok(())
}

/// Feed health has no caller-supplied boolean: it is derived from the
/// observation. A gapped or stale observation fails the health check even
/// though the caller supplies no health verdict at all.
#[test]
fn caller_cannot_supply_feed_health() -> TestResult {
    let base = feed()?;
    let wire = serde_json::to_value(&base)?;
    let fields = wire.as_object().expect("object");
    let mut names: Vec<&str> = fields.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec!["book_complete", "gap", "quote", "received_at"],
        "the observation carries raw facts, never a health verdict"
    );

    let authority = authority()?;
    let mut gapped = base.clone();
    gapped.gap = Some(SequenceGap {
        expected: Some(4),
        observed: 9,
    });
    let result = authority.evaluate_and_reserve(
        intent(1, 2, PRICE, Side::Buy)?,
        observations_for_feed(&authority, gapped)?,
        None,
    );
    assert!(
        rejected_check(
            &result.expect_err("gapped is refused"),
            RiskCheck::MarketFeedHealth
        ),
        "a gap fails the health check"
    );

    authority.set_now(LogicalNanos::new(5_000))?;
    let result = authority.evaluate_and_reserve(
        intent(2, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    );
    assert!(
        rejected_check(
            &result.expect_err("stale is refused"),
            RiskCheck::MarketFeedHealth
        ),
        "an aged quote fails the health check"
    );
    Ok(())
}

/// Replaying an accepted intent returns the same permit and does not reserve
/// twice; reusing the id with different terms is refused.
#[test]
fn duplicate_intent_is_idempotent_and_conflicts_are_denied() -> TestResult {
    let authority = authority()?;
    let original = authority.evaluate_and_reserve(
        intent(1, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    )?;
    let replay = authority.evaluate_and_reserve(
        intent(1, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    )?;
    assert_eq!(
        replay, original,
        "the same permit is returned for the same intent"
    );
    let snapshot = authority.snapshot()?;
    assert_eq!(
        snapshot.reservation_count, 1,
        "no second reservation was created"
    );
    assert_eq!(
        snapshot.pending_gross_exposure_paise,
        Paise(2_000),
        "exposure was not doubled"
    );

    let conflicting = authority.evaluate_and_reserve(
        intent(1, 3, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    );
    assert!(
        matches!(conflicting, Err(AuthorityError::ConflictingIntent(_))),
        "the same id with different terms is denied"
    );
    Ok(())
}

/// Authority-owned control state overrides any intent: kill switch,
/// reconciliation mismatch and an unavailable account snapshot each block new
/// exposure.
#[test]
fn control_state_blocks_new_exposure() -> TestResult {
    let authority = authority()?;
    let mut killed = observation(&authority)?;
    killed
        .operator
        .as_mut()
        .expect("operator")
        .payload
        .kill_switch_engaged = true;
    let result = authority.evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, killed, None);
    assert!(
        matches!(result, Err(AuthorityError::KillSwitchEngaged { .. })),
        "kill switch yields a typed denial"
    );
    release_switch(&authority)?;

    let mut blocked = observation(&authority)?;
    let report = &mut blocked
        .reconciliation
        .as_mut()
        .expect("reconciliation")
        .payload
        .report;
    report.status = ReconciliationStatus::Mismatch;
    report.differences.push("position differs".to_owned());
    let result = authority.evaluate_and_reserve(intent(2, 2, PRICE, Side::Buy)?, blocked, None);
    assert!(
        matches!(result, Err(AuthorityError::ReconciliationMismatch { .. })),
        "a mismatch yields a typed denial"
    );
    assert!(
        authority.snapshot()?.reconciliation_blocked,
        "the mismatch sets reconciliation_blocked via authority state"
    ); // L3: the block is authority state, not a caller-owned bool
    let still_blocked = authority.evaluate_and_reserve(
        intent(9, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    );
    let latched = still_blocked
        .as_ref()
        .err()
        .is_some_and(|error| rejected_check(error, RiskCheck::Reconciliation));
    assert!(
        latched,
        "a later matched observation does not auto-clear the latch"
    ); // L3: sticky until an explicit operator resolution
    clear_reconciliation(&authority)?;

    let mut missing = observation(&authority)?;
    missing
        .account_journal
        .as_mut()
        .expect("journal")
        .payload
        .opening_balance_evidence_ref = None;
    let result = authority.evaluate_and_reserve(intent(3, 2, PRICE, Side::Buy)?, missing, None);
    let error = result.expect_err("snapshot");
    assert!(
        rejected_check(&error, RiskCheck::AccountSnapshotAvailable),
        "missing snapshot blocks, got {error:?}"
    );
    Ok(())
}

/// A caller cannot assert a different account; the authority's own account
/// identity is used in the context.
#[test]
fn caller_cannot_claim_another_account() -> TestResult {
    let authority = authority()?;
    let mut forged = intent(1, 2, PRICE, Side::Buy)?;
    forged.account_id = AccountId::new(999)?;
    let result = authority.evaluate_and_reserve(forged, observation(&authority)?, None);
    assert!(
        rejected_check(
            &result.expect_err("wrong account"),
            RiskCheck::AccountAllowlist
        ),
        "account identity comes from the authority"
    );
    Ok(())
}

/// Exposure accumulates in authority state, so a fill cannot reset it and let
/// a later intent pass against an understated total.
#[test]
fn filled_exposure_is_owned_and_accumulates() -> TestResult {
    let authority = authority()?;
    let first = authority.evaluate_and_reserve(
        intent(1, 6, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    )?;
    authority.apply_fill(first.intent_id)?;
    let snapshot = authority.snapshot()?;
    assert_eq!(
        snapshot.gross_exposure_paise,
        Paise(6_000),
        "filled notional moves to gross exposure"
    );
    assert_eq!(
        snapshot.pending_gross_exposure_paise,
        Paise(0),
        "pending exposure is cleared"
    );

    let second = authority.evaluate_and_reserve(
        intent(2, 6, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    );
    assert!(
        rejected_check(&second.expect_err("aggregate"), RiskCheck::GrossExposure),
        "a later intent sees the accumulated exposure"
    );
    Ok(())
}

/// Releasing an unused reservation returns its exposure and open-order slot.
#[test]
fn releasing_a_reservation_restores_capacity() -> TestResult {
    let authority = authority()?;
    let reservation = authority.evaluate_and_reserve(
        intent(1, 6, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    )?;
    assert_eq!(
        authority.snapshot()?.open_orders,
        1,
        "one order slot is held"
    );
    authority.release(reservation.intent_id)?;
    let snapshot = authority.snapshot()?;
    assert_eq!(snapshot.open_orders, 0, "the order slot is returned");
    assert_eq!(
        snapshot.pending_gross_exposure_paise,
        Paise(0),
        "the notional is returned"
    );
    assert!(
        authority
            .evaluate_and_reserve(
                intent(2, 6, PRICE, Side::Buy)?,
                observation(&authority)?,
                None
            )
            .is_ok(),
        "capacity is available again"
    );
    Ok(())
}

/// The crate has no OMS, broker, venue SDK or order-submission dependency.
#[test]
fn no_oms_broker_or_live_coupling() -> TestResult {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "oms",
        "broker",
        "upstox",
        "fyers",
        "kite",
        "paper-engine",
        "execution",
    ] {
        assert!(
            !manifest.to_ascii_lowercase().contains(forbidden),
            "manifest must not depend on {forbidden}"
        );
    }
    Ok(())
}

/// A caller cannot claim a receipt time ahead of the authority's clock to
/// defeat the staleness derivation.
#[test]
fn future_observation_receipt_is_rejected() -> TestResult {
    let authority = authority()?;
    let mut observations = observation(&authority)?;
    observations.market.as_mut().expect("market").received_at = LogicalNanos::new(1);
    let result =
        authority.evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, observations, None);
    assert!(
        matches!(result, Err(AuthorityError::FutureObservation)),
        "a receipt time ahead of the authority clock is refused"
    );
    Ok(())
}

/// A permit returned on replay stays bound to the authority's account, not a
/// field the caller can set.
#[test]
fn permit_account_is_authority_owned() -> TestResult {
    let authority = authority()?;
    let reservation = authority.evaluate_and_reserve(
        intent(1, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    )?;
    assert_eq!(
        reservation.account_id,
        AccountId::new(ACCOUNT)?,
        "the permit records the authority account"
    );
    Ok(())
}

/// A consumed intent is distinctly reported on replay and cannot be replayed
/// into a second reservation.
#[test]
fn consumed_intent_replay_is_distinguished() -> TestResult {
    let authority = authority()?;
    let reservation = authority.evaluate_and_reserve(
        intent(1, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    )?;
    authority.apply_fill(reservation.intent_id)?;
    let replay = authority.evaluate_and_reserve(
        intent(1, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    );
    assert!(
        matches!(replay, Err(AuthorityError::AlreadyConsumed(_))),
        "a filled intent is reported as consumed, not merely rejected"
    );
    assert_eq!(
        authority.snapshot()?.reservation_count,
        0,
        "no second reservation exists"
    );
    Ok(())
}

/// A fabricated or deserialized permit must fail self-validation before a
/// downstream consumer could act on it.
#[test]
fn forged_permit_fails_validation() -> TestResult {
    let authority = authority()?;
    let valid = authority.evaluate_and_reserve(
        intent(1, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    )?;
    assert!(
        valid.validate_invariants().is_ok(),
        "an authority permit validates"
    );

    let mut forged = valid.clone();
    forged.assessment.accepted = false;
    assert!(
        matches!(
            forged.validate_invariants(),
            Err(AuthorityError::InvalidReservation(_))
        ),
        "a permit with a rejected assessment is invalid"
    );

    let mut forged = valid;
    forged.assessment.intent_id = IntentId::new(999)?;
    assert!(
        matches!(
            forged.validate_invariants(),
            Err(AuthorityError::InvalidReservation(_))
        ),
        "a permit whose intent id does not match is invalid"
    );
    Ok(())
}

fn clear_source(observations: &mut Observations, source: ObservationSource) {
    match source {
        ObservationSource::MarketDataKernel => observations.market = None,
        ObservationSource::Oms => observations.oms = None,
        ObservationSource::AccountJournal => observations.account_journal = None,
        ObservationSource::Reconciliation => observations.reconciliation = None,
        ObservationSource::Ops => observations.ops = None,
        ObservationSource::OperatorControl => observations.operator = None,
    }
}

fn stale_source(observations: &mut Observations, source: ObservationSource) {
    let stale = LogicalNanos::ZERO;
    match source {
        ObservationSource::MarketDataKernel => {
            observations.market.as_mut().expect("market").observed_at = stale;
        }
        ObservationSource::Oms => {
            observations.oms.as_mut().expect("oms").observed_at = stale;
        }
        ObservationSource::AccountJournal => {
            observations
                .account_journal
                .as_mut()
                .expect("journal")
                .observed_at = stale;
        }
        ObservationSource::Reconciliation => {
            observations
                .reconciliation
                .as_mut()
                .expect("reconciliation")
                .observed_at = stale;
        }
        ObservationSource::Ops => {
            observations.ops.as_mut().expect("ops").observed_at = stale;
        }
        ObservationSource::OperatorControl => {
            observations
                .operator
                .as_mut()
                .expect("operator")
                .observed_at = stale;
        }
    }
}

/// A direct caller that omits any one owner cannot obtain a reservation.
#[test]
fn missing_observation_from_each_source_denies() -> TestResult {
    let authority = authority()?;
    for (index, source) in ObservationSource::ALL.iter().enumerate() {
        let mut observations = observation(&authority)?;
        clear_source(&mut observations, *source);
        let result = authority.evaluate_and_reserve(
            intent((index as u64) + 100, 2, PRICE, Side::Buy)?,
            observations,
            None,
        );
        assert!(
            matches!(
                result,
                Err(AuthorityError::MissingObservation { observation_source }) if observation_source == *source
            ),
            "missing {source} observation denies new risk"
        );
    }
    Ok(())
}

/// Every owner has an independent freshness limit enforced by the authority.
#[test]
fn stale_observation_from_each_source_denies() -> TestResult {
    let authority = authority()?;
    authority.set_now(LogicalNanos::new(2_000_000_000))?;
    let mut fresh = feed()?;
    fresh.received_at = LogicalNanos::new(2_000_000_000);
    for (index, source) in ObservationSource::ALL.iter().enumerate() {
        let mut observations = observations_for_feed(&authority, fresh.clone())?;
        stale_source(&mut observations, *source);
        let result = authority.evaluate_and_reserve(
            intent((index as u64) + 200, 2, PRICE, Side::Buy)?,
            observations,
            None,
        );
        assert!(
            matches!(
                result,
                Err(AuthorityError::StaleObservation { observation_source, .. }) if observation_source == *source
            ),
            "stale {source} observation denies new risk"
        );
    }
    Ok(())
}

#[test]
fn wrong_schema_version_denies() -> TestResult {
    let authority = authority()?;
    let mut observations = observation(&authority)?;
    observations.market.as_mut().expect("market").schema_version = 999;
    let result =
        authority.evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, observations, None);
    assert!(
        matches!(
            result,
            Err(AuthorityError::UnsupportedObservationVersion {
                observation_source: ObservationSource::MarketDataKernel,
                version: 999
            })
        ),
        "an unknown observation schema is a typed denial"
    );
    Ok(())
}

#[test]
fn journal_failure_and_unavailable_reconciliation_block() -> TestResult {
    let authority = authority()?;
    let mut poisoned = observation(&authority)?;
    poisoned.oms.as_mut().expect("oms").payload.is_poisoned = true;
    let result = authority.evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, poisoned, None);
    assert!(
        matches!(
            result,
            Err(AuthorityError::ObservationDigestMismatch {
                observation_source: ObservationSource::Oms
            })
        ),
        "a poisoned OMS journal blocks new risk"
    );

    let mut bad_digest = observation(&authority)?;
    bad_digest
        .account_journal
        .as_mut()
        .expect("journal")
        .payload
        .head_hash_hex = "short".to_owned();
    let result = authority.evaluate_and_reserve(intent(2, 2, PRICE, Side::Buy)?, bad_digest, None);
    assert!(
        matches!(
            result,
            Err(AuthorityError::ObservationDigestMismatch {
                observation_source: ObservationSource::AccountJournal
            })
        ),
        "a failed ledger replay digest blocks new risk"
    );

    let mut unavailable = observation(&authority)?;
    unavailable
        .reconciliation
        .as_mut()
        .expect("reconciliation")
        .payload
        .report
        .status = ReconciliationStatus::Unavailable;
    let result = authority.evaluate_and_reserve(intent(3, 2, PRICE, Side::Buy)?, unavailable, None);
    assert!(
        matches!(result, Err(AuthorityError::ReconciliationMismatch { .. })),
        "unavailable reconciliation blocks exactly like a mismatch"
    );
    assert!(
        authority.snapshot()?.reconciliation_blocked,
        "the authority records the reconciliation block"
    );
    Ok(())
}

#[test]
fn kill_switch_persists_across_restart() -> TestResult {
    let path = std::env::temp_dir().join(format!(
        "nirnay-kill-switch-{}-{}.jsonl",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&path);
    let build = || {
        RiskAuthority::new_with_control_store(
            AccountId::new(ACCOUNT).expect("account"),
            policy().expect("policy"),
            vec![instrument().expect("instrument")],
            date().expect("date"),
            Box::new(FileControlStore::new(&path)),
        )
    };

    let first = build();
    assert!(
        first.kill_switch_engaged()?,
        "no control record fails closed engaged"
    );
    let first_tokens = first.register_all_owners()?;
    first.set_kill_switch(
        first_tokens.operator,
        false,
        "operator-1".to_owned(),
        "documented paper release".to_owned(),
        Some("release-evidence-1".to_owned()),
    )?;
    assert!(
        !first.kill_switch_engaged()?,
        "an evidenced release is applied"
    );
    drop(first);

    let second = build();
    assert!(
        !second.kill_switch_engaged()?,
        "the release survives a process restart"
    );
    let second_tokens = second.register_all_owners()?;
    second.set_kill_switch(
        second_tokens.operator,
        true,
        "operator-2".to_owned(),
        "safety stop".to_owned(),
        None,
    )?;
    drop(second);

    let third = build();
    assert!(
        third.kill_switch_engaged()?,
        "an engaged switch survives a process restart"
    );
    let result =
        third.evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, observation(&third)?, None);
    assert!(
        matches!(result, Err(AuthorityError::KillSwitchEngaged { .. })),
        "a restarted authority with a persisted engaged switch denies new risk"
    );
    let _ = std::fs::remove_file(&path);
    Ok(())
}

/// Risk-reducing calls stay available when health is degraded; only new risk
/// is gated (invariant L5).
#[test]
fn risk_reducing_paths_survive_degraded_health() -> TestResult {
    let authority = authority()?;
    let to_release = authority.evaluate_and_reserve(
        intent(1, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    )?;
    let to_fill = authority.evaluate_and_reserve(
        intent(2, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    )?;
    authority.set_now(LogicalNanos::new(2_000_000_000))?;

    let rejected = authority.evaluate_and_reserve(
        intent(3, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    );
    assert!(
        matches!(rejected, Err(AuthorityError::StaleObservation { .. })),
        "new risk is denied with a stale feed"
    );
    assert!(
        authority.release(to_release.intent_id).is_ok(),
        "release remains available with degraded health"
    );
    assert!(
        authority.apply_fill(to_fill.intent_id).is_ok(),
        "apply_fill remains available with degraded health"
    );
    assert!(
        authority
            .set_kill_switch(
                authority.register_all_owners()?.operator,
                true,
                "operator".to_owned(),
                "flatten".to_owned(),
                None,
            )
            .is_ok(),
        "engaging the kill switch remains available with degraded health"
    );
    Ok(())
}

/// Two concurrent callers cannot observe different health states: freshness
/// and aggregation happen under the same mutex as the reservation.
#[test]
fn health_decision_is_inside_one_lock() -> TestResult {
    let authority = Arc::new(
        RiskAuthority::new(
            AccountId::new(ACCOUNT)?,
            policy()?,
            vec![instrument()?],
            date()?,
        )
        .with_observation_policy(ObservationPolicy {
            max_market_age_ns: 100,
            ..ObservationPolicy::default()
        }),
    );
    release_switch(&authority)?;
    clear_reconciliation(&authority)?;
    publish_zero_daily(&authority)?;
    authority.set_now(LogicalNanos::new(2_000))?;
    let barrier = Arc::new(Barrier::new(2));
    let mut fresh = feed()?;
    fresh.received_at = LogicalNanos::new(2_000);
    let mut handles = Vec::new();
    for (id, stale) in [(1_u64, false), (2_u64, true)] {
        let authority = Arc::clone(&authority);
        let barrier = Arc::clone(&barrier);
        let mut observations = observations_for_feed(&authority, fresh.clone())?;
        if stale {
            stale_source(&mut observations, ObservationSource::MarketDataKernel);
        }
        handles.push(thread::spawn(move || {
            let target = intent(id, 2, PRICE, Side::Buy).expect("intent");
            barrier.wait();
            authority.evaluate_and_reserve(target, observations, None)
        }));
    }
    let results: Vec<Result<_, _>> = handles
        .into_iter()
        .map(|handle| handle.join().expect("thread"))
        .collect();
    let accepted = results.iter().filter(|result| result.is_ok()).count();
    let denied = results
        .iter()
        .filter(|result| matches!(result, Err(AuthorityError::StaleObservation { .. })))
        .count();
    assert_eq!(accepted, 1, "the fresh caller reserves once");
    assert_eq!(denied, 1, "the stale caller is denied by the locked clock");
    Ok(())
}

/// No `safe_for_new_risk` decision or string verdict crosses the authority
/// boundary; failures are typed `AuthorityError` values.
#[test]
fn no_caller_bool_or_string_reason() -> TestResult {
    let source = include_str!("../src/authority.rs");
    assert!(
        !source.contains("safe_for_new_risk"),
        "the authority API has no caller-owned health boolean"
    );
    let authority = authority()?;
    let result = authority.evaluate_and_reserve(
        intent(1, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    );
    assert!(result.is_ok(), "the typed observation bundle is sufficient");
    Ok(())
}

/// An explicit policy can shorten each freshness window.
#[test]
fn observation_policy_is_authority_owned() -> TestResult {
    let authority = RiskAuthority::new(
        AccountId::new(ACCOUNT)?,
        policy()?,
        vec![instrument()?],
        date()?,
    )
    .with_observation_policy(ObservationPolicy {
        max_market_age_ns: 10,
        ..ObservationPolicy::default()
    });
    release_switch(&authority)?;
    clear_reconciliation(&authority)?;
    authority.set_now(LogicalNanos::new(100))?;
    let result = authority.evaluate_and_reserve(
        intent(1, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    );
    assert!(
        matches!(result, Err(AuthorityError::StaleObservation { .. })),
        "an authority-owned policy shortens the market freshness window"
    );
    Ok(())
}

/// A direct caller cannot fabricate all six observations: an observation from
/// a different process-local authority carries a token this authority never
/// issued and is denied before any policy evaluation.
#[test]
fn forged_observations_without_registered_token_are_denied() -> TestResult {
    let authority = authority()?;
    let other = super_authority()?;
    let foreign = observation(&other)?;
    let result = authority.evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, foreign, None);
    assert!(
        matches!(
            result,
            Err(AuthorityError::ProducerTokenMismatch {
                observation_source: ObservationSource::MarketDataKernel
            })
        ),
        "an unregistered producer token is a typed denial"
    );

    let unregistered = RiskAuthority::new(
        AccountId::new(ACCOUNT)?,
        policy()?,
        vec![instrument()?],
        date()?,
    );
    let result = unregistered.evaluate_and_reserve(
        intent(2, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    );
    assert!(
        matches!(
            result,
            Err(AuthorityError::UnregisteredProducer {
                observation_source: ObservationSource::MarketDataKernel
            })
        ),
        "an authority with no registered owners denies new risk"
    );
    Ok(())
}

/// Borrowing another owner's registered token does not authenticate an
/// observation: the token is bound to its own source.
#[test]
fn forging_another_owners_token_is_denied() -> TestResult {
    let authority = authority()?;
    let tokens = authority.register_all_owners()?;
    let mut observations = observation(&authority)?;
    observations.market.as_mut().expect("market").producer_token = tokens.ops;
    let result =
        authority.evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, observations, None);
    assert!(
        matches!(
            result,
            Err(AuthorityError::ProducerTokenMismatch {
                observation_source: ObservationSource::MarketDataKernel
            })
        ),
        "an ops token cannot authenticate the market owner"
    );
    Ok(())
}

/// Kill-switch release and reconciliation-latch clearing are authenticated
/// owner actions with non-empty evidence, not caller-supplied strings.
#[test]
fn protected_control_actions_require_token_and_evidence() -> TestResult {
    let authority = authority()?;
    let tokens = authority.register_all_owners()?;
    let other = authority.register_owner(ObservationSource::Ops)?;

    assert!(
        matches!(
            authority.set_kill_switch(
                other,
                false,
                "forged".to_owned(),
                "forged".to_owned(),
                Some("fake".to_owned()),
            ),
            Err(AuthorityError::ProducerTokenMismatch { .. })
        ),
        "an ops token cannot release the kill switch"
    );
    assert!(
        matches!(
            authority.set_kill_switch(
                tokens.operator,
                false,
                "operator".to_owned(),
                "no evidence".to_owned(),
                None,
            ),
            Err(AuthorityError::MissingEvidence { .. })
        ),
        "a release without evidence is refused"
    );
    assert!(
        matches!(
            authority.set_reconciliation_blocked(
                other,
                false,
                "forged".to_owned(),
                Some("fake".to_owned()),
            ),
            Err(AuthorityError::ProducerTokenMismatch { .. })
        ),
        "an ops token cannot clear the reconciliation latch"
    );
    assert!(
        matches!(
            authority.set_reconciliation_blocked(
                tokens.reconciliation,
                false,
                "no evidence".to_owned(),
                None,
            ),
            Err(AuthorityError::MissingEvidence { .. })
        ),
        "a latch clear without evidence is refused"
    );
    assert!(
        authority
            .set_kill_switch(
                tokens.operator,
                false,
                "operator".to_owned(),
                "documented".to_owned(),
                Some("evidence-ref".to_owned()),
            )
            .is_ok(),
        "an evidenced operator release is accepted"
    );
    Ok(())
}

/// Revision is a complete version of every fact used by risk evaluation, not
/// just of clock and exposure. A real change advances it exactly once; a
/// repeated value is a defined no-op and advances it zero times.
#[test]
fn public_mutations_are_revision_aware() -> TestResult {
    let authority = authority()?;
    let start = authority.revision()?;

    authority.set_session_open(true)?;
    assert_eq!(
        authority.revision()?,
        start,
        "repeating session state is a no-op"
    );
    authority.set_session_open(false)?;
    assert_eq!(
        authority.revision()?,
        start + 1,
        "closing the session is versioned"
    );
    authority.set_session_open(true)?;
    assert_eq!(
        authority.revision()?,
        start + 2,
        "reopening the session is versioned"
    );

    authority.set_snapshot_available(true)?;
    assert_eq!(
        authority.revision()?,
        start + 2,
        "repeating snapshot state is a no-op"
    );
    authority.set_snapshot_available(false)?;
    assert_eq!(
        authority.revision()?,
        start + 3,
        "withdrawing the snapshot is versioned"
    );

    publish_zero_daily(&authority)?;
    assert_eq!(
        authority.revision()?,
        start + 4,
        "publishing sealed daily P&L evidence is versioned"
    );

    authority.set_now(LogicalNanos::new(1_000))?;
    assert_eq!(
        authority.revision()?,
        start + 5,
        "the logical clock is versioned"
    );
    authority.set_now(LogicalNanos::new(1_000))?;
    assert_eq!(
        authority.revision()?,
        start + 5,
        "repeating the clock is a no-op"
    );

    let tokens = authority.register_all_owners()?;
    authority.set_kill_switch(
        tokens.operator,
        true,
        "op".to_owned(),
        "stop".to_owned(),
        None,
    )?;
    assert_eq!(
        authority.revision()?,
        start + 6,
        "engaging the kill switch is versioned"
    );
    authority.set_reconciliation_blocked(
        tokens.reconciliation,
        true,
        "blocked".to_owned(),
        None,
    )?;
    assert_eq!(
        authority.revision()?,
        start + 7,
        "latching reconciliation is versioned"
    );
    authority.set_reconciliation_blocked(
        tokens.reconciliation,
        true,
        "blocked".to_owned(),
        None,
    )?;
    assert_eq!(
        authority.revision()?,
        start + 7,
        "repeating the latch is a no-op"
    );
    Ok(())
}

/// `mutate` returns the revision observed immediately before the single
/// transition under test, so setups with their own earlier transitions do not
/// hide an extra revision bump.
fn assert_stale_fence_after<F>(label: &str, mutate: F) -> TestResult
where
    F: FnOnce(&RiskAuthority) -> Result<u64, Box<dyn std::error::Error>>,
{
    let authority = authority()?;
    let before = mutate(&authority)?;
    let after = authority.revision()?;
    assert_eq!(
        after,
        before + 1,
        "{label}: revision advances exactly once on a real change"
    );
    let stale = authority.evaluate_and_reserve(
        intent(999, 2, PRICE, Side::Buy)?,
        Observations::default(),
        Some(before),
    );
    assert!(
        matches!(stale, Err(AuthorityError::StaleSnapshot { expected, current }) if expected == before && current == after),
        "{label}: a revision observed before the mutation cannot authorize new risk"
    );
    Ok(())
}

/// Each mutation that affects risk evaluation invalidates fences taken before
/// it, including observation-derived latches and all accounting transitions.
#[test]
fn stale_revision_is_rejected_after_every_relevant_mutation() -> TestResult {
    assert_stale_fence_after("session", |authority| {
        let before = authority.revision()?;
        authority.set_session_open(false)?;
        Ok(before)
    })?;
    assert_stale_fence_after("snapshot", |authority| {
        let before = authority.revision()?;
        authority.set_snapshot_available(false)?;
        Ok(before)
    })?;
    assert_stale_fence_after("daily pnl", |authority| {
        let before = authority.revision()?;
        publish_zero_daily(authority)?;
        Ok(before)
    })?;
    assert_stale_fence_after("clock", |authority| {
        let before = authority.revision()?;
        authority.set_now(LogicalNanos::new(1_000))?;
        Ok(before)
    })?;
    assert_stale_fence_after("kill switch", |authority| {
        let tokens = authority.register_all_owners()?;
        let before = authority.revision()?;
        authority.set_kill_switch(
            tokens.operator,
            true,
            "op".to_owned(),
            "stop".to_owned(),
            None,
        )?;
        Ok(before)
    })?;
    assert_stale_fence_after("reconciliation", |authority| {
        let tokens = authority.register_all_owners()?;
        let before = authority.revision()?;
        authority.set_reconciliation_blocked(
            tokens.reconciliation,
            true,
            "blocked".to_owned(),
            None,
        )?;
        Ok(before)
    })?;
    assert_stale_fence_after("reservation", |authority| {
        let before = authority.revision()?;
        authority.evaluate_and_reserve(
            intent(1, 2, PRICE, Side::Buy)?,
            observation(authority)?,
            None,
        )?;
        Ok(before)
    })?;
    assert_stale_fence_after("release", |authority| {
        let reservation = authority.evaluate_and_reserve(
            intent(1, 2, PRICE, Side::Buy)?,
            observation(authority)?,
            None,
        )?;
        let before = authority.revision()?;
        authority.release(reservation.intent_id)?;
        Ok(before)
    })?;
    assert_stale_fence_after("fill", |authority| {
        let reservation = authority.evaluate_and_reserve(
            intent(1, 2, PRICE, Side::Buy)?,
            observation(authority)?,
            None,
        )?;
        let before = authority.revision()?;
        authority.apply_fill(reservation.intent_id)?;
        Ok(before)
    })?;
    assert_stale_fence_after("observation latch", |authority| {
        let mut killed = observation(authority)?;
        killed
            .operator
            .as_mut()
            .expect("operator")
            .payload
            .kill_switch_engaged = true;
        let before = authority.revision()?;
        let error = authority
            .check_observations(&killed)
            .expect_err("an observed engagement is a typed denial");
        assert!(
            matches!(error, AuthorityError::KillSwitchEngaged { .. }),
            "the kill switch observation denies new risk"
        );
        Ok(before)
    })?;
    Ok(())
}

/// Observed fail-closed latches are authority state changes even when the
/// observation bundle ultimately denies new risk; they carry one revision each.
#[test]
fn observation_derived_changes_advance_revision() -> TestResult {
    let authority = authority()?;
    let before = authority.revision()?;
    let mut killed = observation(&authority)?;
    killed
        .operator
        .as_mut()
        .expect("operator")
        .payload
        .kill_switch_engaged = true;
    let engaged = authority
        .check_observations(&killed)
        .expect_err("an observed engagement denies new risk");
    assert!(
        matches!(engaged, AuthorityError::KillSwitchEngaged { .. }),
        "the observed engagement is a typed denial"
    );
    assert_eq!(
        authority.revision()?,
        before + 1,
        "an observed kill-switch engagement is versioned"
    );
    assert!(
        authority.kill_switch_engaged()?,
        "the observation latches engaged"
    );

    let before = authority.revision()?;
    let mut blocked = observation(&authority)?;
    blocked
        .reconciliation
        .as_mut()
        .expect("reconciliation")
        .payload
        .report
        .status = ReconciliationStatus::Unavailable;
    let error = authority
        .check_observations(&blocked)
        .expect_err("an unavailable reconciliation denies");
    assert!(
        matches!(error, AuthorityError::ReconciliationMismatch { .. }),
        "the reconciliation latch is a typed denial"
    );
    assert_eq!(
        authority.revision()?,
        before + 1,
        "a fail-closed latch reached on a failing check is committed once"
    );
    Ok(())
}

mod recovery_tests {
    use super::*;
    use risk_authority::{
        ACCOUNT_ANCHOR_FILE, //
        ACCOUNT_JOURNAL_FILE,
        ACCOUNT_JOURNAL_ID,
        AccountRecoveryRecord,
        BINDING_SUFFIX,
        ControlError,
        ControlStore,
        DAILY_PNL_FILE,
        DecisionOutcome,
        DecisionStore,
        FileControlStore,
        FileDecisionStore,
        InMemoryDecisionStore,
        JournalIdentity,
        KillSwitchState,
        NoopControlStore,
        OMS_ANCHOR_FILE,
        OMS_EXECUTIONS_FILE,
        OMS_JOURNAL_ID,
        OMS_PERMITS_FILE,
        OmsPermitRecord,
        OmsPermitState,
        OwnerSigningKey,
        RecoveryAnchor,
        RecoveryAuthenticity,
        RecoveryBindingRecord,
        RecoveryConfig,
        RecoveryError,
        RecoveryJournalRoot,
        RiskDecisionRecord,
        VerifiedAccountSnapshot,
        VerifiedOmsSnapshot,
        authority::{
            ExecutionEventKind, //
            ExecutionRecoveryRecord,
            Reservation,
        },
        genesis_digest,
        recovery_binding_record,
    };
    use trading_core::permit_digest;

    fn temp_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "nirnay-recovery-{label}-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);

    /// Explicit, uniquely named owner-controlled directory: every fixture that
    /// exercises startup loading names its root and its authenticity model.
    fn temp_root(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "nirnay-recovery-root-{label}-{}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("owner-controlled fixture root");
        root
    }

    /// Raw fixture shape. It is sealed into opaque snapshots before recovery;
    /// an external caller of the library cannot supply records in this form.
    #[derive(Default)]
    struct FixtureRecords {
        oms: Vec<OmsPermitRecord>,
        executions: Vec<ExecutionRecoveryRecord>,
        account: Vec<AccountRecoveryRecord>,
        daily_pnl: DailyPnlRecovery,
    }

    struct VerifiedFixtures {
        oms: VerifiedOmsSnapshot,
        account: VerifiedAccountSnapshot,
    }

    fn oms_identity() -> JournalIdentity {
        JournalIdentity::new(
            ObservationSource::Oms,
            AccountId::new(ACCOUNT).expect("account"),
            OMS_JOURNAL_ID,
        )
    }

    fn account_identity() -> JournalIdentity {
        JournalIdentity::new(
            ObservationSource::AccountJournal,
            AccountId::new(ACCOUNT).expect("account"),
            ACCOUNT_JOURNAL_ID,
        )
    }

    fn owner_key() -> OwnerSigningKey {
        OwnerSigningKey::new(b"nirnay-test-owner-signing-key-20261127".to_vec())
            .expect("test owner key")
    }

    fn write_json_lines<T: serde::Serialize>(
        path: &std::path::Path,
        records: &[T],
    ) -> Result<(), RecoveryError> {
        let mut text = String::new();
        for record in records {
            text.push_str(
                &serde_json::to_string(record)
                    .map_err(|error| RecoveryError::Snapshot(error.to_string()))?,
            );
            text.push('\n');
        }
        std::fs::write(path, text).map_err(|error| RecoveryError::Snapshot(error.to_string()))
    }

    fn write_json<T: serde::Serialize>(
        path: &std::path::Path,
        value: &T,
    ) -> Result<(), RecoveryError> {
        let text = serde_json::to_string(value)
            .map_err(|error| RecoveryError::Snapshot(error.to_string()))?;
        std::fs::write(path, text).map_err(|error| RecoveryError::Snapshot(error.to_string()))
    }

    fn oms_head(records: &[OmsPermitRecord]) -> (u64, String) {
        records
            .last()
            .map_or((0, genesis_digest().to_owned()), |record| {
                (record.sequence, record.digest.clone())
            })
    }

    fn account_head(records: &[AccountRecoveryRecord]) -> (u64, String) {
        records
            .last()
            .map_or((0, genesis_digest().to_owned()), |record| {
                (record.sequence, record.digest.clone())
            })
    }

    /// Fixture owner action: lay down the four required files and both
    /// independently protected terminal anchors under an owner-controlled root.
    fn write_owner_root(
        root: &std::path::Path,
        records: &FixtureRecords,
    ) -> Result<(), RecoveryError> {
        write_json_lines(&root.join(OMS_PERMITS_FILE), &records.oms)?;
        write_json_lines(&root.join(OMS_EXECUTIONS_FILE), &records.executions)?;
        write_json_lines(&root.join(ACCOUNT_JOURNAL_FILE), &records.account)?;
        write_json(&root.join(DAILY_PNL_FILE), &records.daily_pnl)?;
        let (oms_sequence, oms_digest) = oms_head(&records.oms);
        let (account_sequence, account_digest) = account_head(&records.account);
        write_json(
            &root.join(OMS_ANCHOR_FILE),
            &RecoveryAnchor::for_owner_directory(
                &oms_identity(),
                oms_sequence.max(1),
                oms_sequence,
                &oms_digest,
            ),
        )?;
        write_json(
            &root.join(ACCOUNT_ANCHOR_FILE),
            &RecoveryAnchor::for_owner_directory(
                &account_identity(),
                account_sequence.max(1),
                account_sequence,
                &account_digest,
            ),
        )
    }

    /// Fixture owner action: add keyed binding sidecars and signed anchors so
    /// the strict authenticity model can be exercised explicitly.
    fn write_signed_root(
        root: &std::path::Path,
        records: &FixtureRecords,
        key: &OwnerSigningKey,
    ) -> Result<(), RecoveryError> {
        write_owner_root(root, records)?;
        let oms_binding: RecoveryBindingRecord =
            recovery_binding_record(key, &oms_identity(), &records.oms, &records.executions)?;
        let account_binding: RecoveryBindingRecord = recovery_binding_record(
            key,
            &account_identity(),
            &records.account,
            &records.daily_pnl,
        )?;
        write_json(
            &root.join(format!("{OMS_PERMITS_FILE}{BINDING_SUFFIX}")),
            &oms_binding,
        )?;
        write_json(
            &root.join(format!("{ACCOUNT_JOURNAL_FILE}{BINDING_SUFFIX}")),
            &account_binding,
        )?;
        let (oms_sequence, oms_digest) = oms_head(&records.oms);
        let (account_sequence, account_digest) = account_head(&records.account);
        write_json(
            &root.join(OMS_ANCHOR_FILE),
            &RecoveryAnchor::seal(
                key,
                &oms_identity(),
                oms_sequence.max(1),
                oms_sequence,
                &oms_digest,
            )?,
        )?;
        write_json(
            &root.join(ACCOUNT_ANCHOR_FILE),
            &RecoveryAnchor::seal(
                key,
                &account_identity(),
                account_sequence.max(1),
                account_sequence,
                &account_digest,
            )?,
        )
    }

    /// Act as the two journal owners: write the durable journals, run the
    /// authority's startup loader, and hand startup only the sealed snapshots.
    fn verified_records_inner(
        mut records: FixtureRecords,
        fill_pnl: bool,
    ) -> Result<VerifiedFixtures, RecoveryError> {
        if fill_pnl && records.daily_pnl.record.is_none() {
            let (daily, journal) = daily_recovery_for_accounts(&records.account)
                .map_err(|error| RecoveryError::Snapshot(error.to_string()))?;
            records.daily_pnl = daily;
            records.account = journal;
        }
        let root = temp_root("fixture");
        write_owner_root(&root, &records)?;
        let (oms, account) = RecoveryJournalRoot::new(&root, RecoveryAuthenticity::OwnerDirectory)
            .load(AccountId::new(ACCOUNT).expect("account"))?;
        let _ = std::fs::remove_dir_all(&root);
        Ok(VerifiedFixtures { oms, account })
    }

    fn verified_records(records: FixtureRecords) -> Result<VerifiedFixtures, RecoveryError> {
        verified_records_inner(records, true)
    }

    // --- H7/H8: required journals and owner authenticity -------------------

    #[test]
    fn startup_load_rejects_a_missing_required_journal() -> TestResult {
        let root = temp_root("missing-required-journal");
        write_owner_root(&root, &FixtureRecords::default())?;
        let missing = root.join(OMS_PERMITS_FILE);
        std::fs::remove_file(&missing)?;
        let result = RecoveryJournalRoot::new(&root, RecoveryAuthenticity::OwnerDirectory)
            .load(AccountId::new(ACCOUNT)?);
        assert!(
            matches!(result, Err(RecoveryError::JournalMissing(path)) if path == missing),
            "a missing required journal is a typed startup failure, not an empty journal"
        );
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn an_existing_valid_empty_journal_loads_under_both_authenticity_models() -> TestResult {
        let key = owner_key();
        for (label, authenticity) in [
            ("owner-directory", RecoveryAuthenticity::OwnerDirectory),
            ("owner-signature", RecoveryAuthenticity::Strict(key.clone())),
        ] {
            let root = temp_root(&format!("empty-{label}"));
            write_signed_root(&root, &FixtureRecords::default(), &key)?;
            let (oms, account) =
                RecoveryJournalRoot::new(&root, authenticity).load(AccountId::new(ACCOUNT)?)?;
            assert_eq!(oms.terminal_head().sequence, 0, "empty OMS head is genesis");
            assert_eq!(
                oms.terminal_head().digest,
                genesis_digest(),
                "empty OMS head is genesis"
            );
            assert_eq!(
                account.terminal_head().sequence,
                0,
                "empty account head is genesis"
            );
            let _ = std::fs::remove_dir_all(&root);
        }
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_and_non_regular_journal_paths_are_rejected() -> TestResult {
        let root = temp_root("symlink-and-file-type");
        let outside = temp_path("symlink-target");
        let records = FixtureRecords::default();
        write_owner_root(&root, &records)?;
        write_json_lines(&outside, &records.oms)?;
        let permits = root.join(OMS_PERMITS_FILE);

        std::fs::remove_file(&permits)?;
        std::os::unix::fs::symlink(&outside, &permits)?;
        let result = RecoveryJournalRoot::new(&root, RecoveryAuthenticity::OwnerDirectory)
            .load(AccountId::new(ACCOUNT)?);
        assert!(
            matches!(result, Err(RecoveryError::Authenticity(_))),
            "a symlink is not an owner-controlled journal file"
        );

        std::fs::remove_file(&permits)?;
        std::fs::create_dir(&permits)?;
        let result = RecoveryJournalRoot::new(&root, RecoveryAuthenticity::OwnerDirectory)
            .load(AccountId::new(ACCOUNT)?);
        assert!(
            matches!(result, Err(RecoveryError::Authenticity(_))),
            "a directory is not a journal file"
        );

        std::fs::remove_dir(&permits)?;
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&outside);
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn a_group_or_world_writable_journal_directory_is_rejected() -> TestResult {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("untrusted-directory");
        write_owner_root(&root, &FixtureRecords::default())?;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o777))?;
        let result = RecoveryJournalRoot::new(&root, RecoveryAuthenticity::Strict(owner_key()))
            .load(AccountId::new(ACCOUNT)?);
        assert!(
            matches!(result, Err(RecoveryError::Authenticity(_))),
            "an owner-controlled root may not be group- or world-writable"
        );
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn forged_identity_and_foreign_key_bindings_are_rejected() -> TestResult {
        let key = owner_key();
        let root = temp_root("forged-identity");
        write_signed_root(&root, &FixtureRecords::default(), &key)?;
        let strict = RecoveryAuthenticity::Strict(key.clone());

        // A caller-supplied identity cannot override the owner's signature.
        let forged =
            JournalIdentity::new(ObservationSource::Oms, AccountId::new(99)?, OMS_JOURNAL_ID);
        let result = VerifiedOmsSnapshot::verify(
            root.join(OMS_PERMITS_FILE),
            root.join(OMS_EXECUTIONS_FILE),
            &forged,
            &strict,
        );
        assert!(
            matches!(result, Err(RecoveryError::Authenticity(_))),
            "a caller-supplied identity cannot replace the signed journal identity"
        );

        // A key the owner never held cannot authenticate a valid chain.
        let attacker = RecoveryAuthenticity::Strict(
            OwnerSigningKey::new(b"attacker-key".to_vec()).expect("attacker key"),
        );
        let result = RecoveryJournalRoot::new(&root, attacker).load(AccountId::new(ACCOUNT)?);
        assert!(
            matches!(result, Err(RecoveryError::Authenticity(_))),
            "a valid unkeyed chain from an untrusted signer is rejected"
        );

        // Editing a signed binding after the fact fails the keyed check.
        let sidecar = root.join(format!("{OMS_PERMITS_FILE}{BINDING_SUFFIX}"));
        let mut binding: RecoveryBindingRecord =
            serde_json::from_str(&std::fs::read_to_string(&sidecar)?)?;
        binding.journal_id = "some-other-journal".to_owned();
        write_json(&sidecar, &binding)?;
        let result = RecoveryJournalRoot::new(&root, strict).load(AccountId::new(ACCOUNT)?);
        assert!(
            matches!(result, Err(RecoveryError::Authenticity(_))),
            "a forged identity in a binding sidecar is rejected"
        );

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn a_valid_chain_from_an_untrusted_directory_is_rejected() -> TestResult {
        use std::os::unix::fs::PermissionsExt;

        let trusted = temp_root("valid-chain-source");
        let untrusted = temp_root("valid-chain-copy");
        let records = FixtureRecords::default();
        write_owner_root(&trusted, &records)?;
        for name in [OMS_PERMITS_FILE, OMS_EXECUTIONS_FILE] {
            std::fs::copy(trusted.join(name), untrusted.join(name))?;
        }
        std::fs::set_permissions(&untrusted, std::fs::Permissions::from_mode(0o777))?;
        let result = VerifiedOmsSnapshot::verify(
            untrusted.join(OMS_PERMITS_FILE),
            untrusted.join(OMS_EXECUTIONS_FILE),
            &oms_identity(),
            &RecoveryAuthenticity::OwnerDirectory,
        );
        assert!(
            matches!(result, Err(RecoveryError::Authenticity(_))),
            "an identical valid chain in an untrusted directory is not authoritative"
        );
        std::fs::set_permissions(&untrusted, std::fs::Permissions::from_mode(0o700))?;
        let _ = std::fs::remove_dir_all(&trusted);
        let _ = std::fs::remove_dir_all(&untrusted);
        Ok(())
    }

    #[test]
    fn rollback_to_an_older_but_valid_head_is_rejected() -> TestResult {
        let key = owner_key();
        let root = temp_root("rollback-older-head");
        let mut first = OmsPermitRecord::new(
            1,
            IntentId::new(1).expect("intent"),
            "a".repeat(64),
            OmsPermitState::Open,
            0,
            0,
        );
        first.seal(1, genesis_digest());
        let mut second = OmsPermitRecord::new(
            2,
            IntentId::new(2).expect("intent"),
            "b".repeat(64),
            OmsPermitState::Open,
            0,
            0,
        );
        second.seal(2, &first.digest);
        let full = FixtureRecords {
            oms: vec![first.clone(), second.clone()],
            ..FixtureRecords::default()
        };
        write_signed_root(&root, &full, &key)?;
        let (oms, _) = RecoveryJournalRoot::new(&root, RecoveryAuthenticity::Strict(key.clone()))
            .load(AccountId::new(ACCOUNT)?)?;
        assert_eq!(
            oms.terminal_head().sequence,
            2,
            "anchor records the full head"
        );

        // Replace the journal with the older valid prefix and re-sign it with
        // the owner key. Only the independent anchor still knows the rollback.
        let prefix = FixtureRecords {
            oms: vec![first],
            ..FixtureRecords::default()
        };
        write_signed_root(&root, &prefix, &key)?;
        // Restore the original, higher anchor after re-sealing the prefix.
        let (sequence, digest) = oms_head(&full.oms);
        write_json(
            &root.join(OMS_ANCHOR_FILE),
            &RecoveryAnchor::seal(&key, &oms_identity(), 2, sequence, &digest)?,
        )?;
        let result = RecoveryJournalRoot::new(&root, RecoveryAuthenticity::Strict(key))
            .load(AccountId::new(ACCOUNT)?);
        assert!(
            matches!(result, Err(RecoveryError::Authenticity(_))),
            "an older valid prefix cannot pass the independently protected head anchor"
        );

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn terminal_anchor_counter_regression_is_rejected() -> TestResult {
        let root = temp_root("anchor-counter-regression");
        write_owner_root(&root, &FixtureRecords::default())?;
        let anchor = root.join(ACCOUNT_ANCHOR_FILE);
        write_json(
            &anchor,
            &RecoveryAnchor::for_owner_directory(&account_identity(), 0, 0, genesis_digest()),
        )?;
        let result = RecoveryJournalRoot::new(&root, RecoveryAuthenticity::OwnerDirectory)
            .load(AccountId::new(ACCOUNT)?);
        assert!(
            matches!(result, Err(RecoveryError::Authenticity(_))),
            "a zero anchor counter is not a monotonic terminal anchor"
        );
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn correct_owner_controlled_load_succeeds_end_to_end() -> TestResult {
        let key = owner_key();
        let root = temp_root("correct-owner-load");
        let (daily, journal) = daily_recovery_for_accounts(&[])?;
        let records = FixtureRecords {
            account: journal,
            daily_pnl: daily,
            ..FixtureRecords::default()
        };
        write_signed_root(&root, &records, &key)?;
        let (oms, account) = RecoveryJournalRoot::new(&root, RecoveryAuthenticity::Strict(key))
            .load(AccountId::new(ACCOUNT)?)?;

        let control = temp_path("control-correct-owner");
        let decisions = temp_path("decisions-correct-owner");
        let lock = temp_path("lock-correct-owner");
        for path in [&control, &decisions, &lock] {
            let _ = std::fs::remove_file(path);
        }
        let authority = RiskAuthority::recover(RecoveryConfig {
            account_id: AccountId::new(ACCOUNT)?,
            policy: policy()?,
            instruments: vec![instrument()?],
            trade_date: date()?,
            control: Box::new(FileControlStore::new(&control)),
            decisions: Box::new(FileDecisionStore::new(&decisions)),
            oms,
            account,
            sectors: BTreeMap::new(),
            ownership_path: lock.clone(),
        })?;
        assert!(
            authority.startup_error().is_none(),
            "an owner-authenticated load reaches a usable recovered authority"
        );

        for path in [&control, &decisions, &lock] {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    fn recover_with_inner(
        lock_path: &std::path::Path,
        records: FixtureRecords,
        control: Box<dyn risk_authority::ControlStore + Send + Sync>,
        decisions: Box<dyn DecisionStore + Send + Sync>,
        fill_pnl: bool,
    ) -> Result<RiskAuthority, RecoveryError> {
        let fixtures = verified_records_inner(records, fill_pnl)?;
        RiskAuthority::recover(RecoveryConfig {
            account_id: AccountId::new(ACCOUNT).expect("account"),
            policy: policy().expect("policy"),
            instruments: vec![instrument().expect("instrument")],
            trade_date: date().expect("date"),
            control,
            decisions,
            oms: fixtures.oms,
            account: fixtures.account,
            sectors: BTreeMap::new(),
            ownership_path: lock_path.to_path_buf(),
        })
    }

    fn recover_with(
        lock_path: &std::path::Path,
        records: FixtureRecords,
        control: Box<dyn risk_authority::ControlStore + Send + Sync>,
        decisions: Box<dyn DecisionStore + Send + Sync>,
    ) -> Result<RiskAuthority, RecoveryError> {
        recover_with_inner(lock_path, records, control, decisions, true)
    }

    /// Seal exactly the supplied fixture, with no synthetic P&L fill. Used by
    /// the test that proves startup fails when the owner supplied no P&L.
    fn recover_with_no_pnl_fill(
        lock_path: &std::path::Path,
        records: FixtureRecords,
        control: Box<dyn risk_authority::ControlStore + Send + Sync>,
        decisions: Box<dyn DecisionStore + Send + Sync>,
    ) -> Result<RiskAuthority, RecoveryError> {
        recover_with_inner(lock_path, records, control, decisions, false)
    }

    fn build_recovered(
        lock_path: &std::path::Path,
        records: FixtureRecords,
        control: Box<dyn risk_authority::ControlStore + Send + Sync>,
        decisions: Box<dyn DecisionStore + Send + Sync>,
    ) -> Result<RiskAuthority, Box<dyn std::error::Error>> {
        Ok(recover_with(lock_path, records, control, decisions)?)
    }

    fn file_stores(
        control_path: &std::path::Path,
        decision_path: &std::path::Path,
    ) -> (
        Box<dyn risk_authority::ControlStore + Send + Sync>,
        Box<dyn DecisionStore + Send + Sync>,
    ) {
        (
            Box::new(FileControlStore::new(control_path)),
            Box::new(FileDecisionStore::new(decision_path)),
        )
    }

    #[test]
    fn file_control_store_rejects_duplicate_and_regressing_revisions() -> TestResult {
        let path = temp_path("control-revision");
        let _ = std::fs::remove_file(&path);
        let store = FileControlStore::new(&path);
        let first = KillSwitchState {
            enabled: true,
            actor_id: "owner".to_owned(),
            reason: "start".to_owned(),
            revision: 1,
            at_ns: 0,
        };
        store.append_kill_switch(&first)?;

        let mut duplicate = first.clone();
        duplicate.enabled = false;
        assert!(
            matches!(
                store.append_kill_switch(&duplicate),
                Err(ControlError::Corrupt(_))
            ),
            "a duplicate revision is rejected rather than silently replacing the record"
        );
        let mut regression = first.clone();
        regression.revision = 0;
        assert!(
            matches!(
                store.append_kill_switch(&regression),
                Err(ControlError::Corrupt(_))
            ),
            "a regressing revision is rejected"
        );
        assert_eq!(
            store.load_kill_switch()?,
            Some(first),
            "only the exact first record remains authoritative"
        );
        let _ = std::fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn control_store_corruption_is_an_explicit_startup_error() -> TestResult {
        let path = temp_path("control-corrupt");
        std::fs::write(&path, "{not-json}\n")?;
        let authority = RiskAuthority::new_with_control_store(
            AccountId::new(ACCOUNT)?,
            policy()?,
            vec![instrument()?],
            date()?,
            Box::new(FileControlStore::new(&path)),
        );
        assert!(
            authority.startup_error().is_some(),
            "an unreadable control store is a typed startup error"
        );
        assert!(
            authority.kill_switch_engaged()?,
            "the authority still fails closed engaged"
        );
        let denied = authority.evaluate_and_reserve(
            intent(1, 2, PRICE, Side::Buy)?,
            observation(&authority)?,
            None,
        );
        assert!(
            matches!(denied, Err(AuthorityError::ControlFailure(_))),
            "new risk is denied while the control store is unavailable"
        );
        let _ = std::fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn recovery_reconstructs_reservations_exposures_and_positions() -> TestResult {
        let control = temp_path("control-recovery");
        let decisions = temp_path("decisions-recovery");
        let lock = temp_path("lock-recovery");
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);

        let (c1, d1) = file_stores(&control, &decisions);
        let first = build_recovered(&lock, FixtureRecords::default(), c1, d1)?;
        assert!(
            first.snapshot()?.kill_switch_engaged,
            "recovery starts engaged without a persisted release"
        );
        let tokens = first.register_all_owners()?;
        first.set_kill_switch(
            tokens.operator,
            false,
            "operator".to_owned(),
            "documented release".to_owned(),
            Some("release-evidence".to_owned()),
        )?;
        first.set_session_open(true)?;
        first.set_snapshot_available(true)?;
        first.set_reconciliation_blocked(
            tokens.reconciliation,
            false,
            "verified replay".to_owned(),
            Some("reconciliation-evidence".to_owned()),
        )?;
        let reservation = first.evaluate_and_reserve(
            intent(1, 2, PRICE, Side::Buy)?,
            observation(&first)?,
            None,
        )?;
        assert_eq!(
            reservation.permit_id, 1,
            "the first issued permit is numbered from one"
        );
        drop(first);

        let permit_digest = permit_digest(
            reservation.permit_id,
            &reservation.assessment.policy_version,
            reservation.snapshot_revision,
            &intent(1, 2, PRICE, Side::Buy)?,
        );
        let mut oms = OmsPermitRecord::new(
            reservation.permit_id,
            reservation.intent_id,
            permit_digest,
            OmsPermitState::Filled,
            reservation.quantity.get(),
            reservation.reserved_notional_paise.0,
        );
        oms.seal(1, genesis_digest());
        let mut account = AccountRecoveryRecord::new(
            "account-event-1".to_owned(),
            AccountId::new(ACCOUNT)?,
            instrument_id()?,
            reservation.quantity.get(),
        );
        account.seal(1, genesis_digest());
        let full_fill = exec_record(
            &reservation,
            "exec-full",
            1,
            ExecutionEventKind::Fill,
            None,
            reservation.quantity.get(),
            0,
            reservation.reserved_notional_paise.0,
            0,
            0,
        );

        let (c2, d2) = file_stores(&control, &decisions);
        let second = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms],
                executions: seal_executions(vec![full_fill]),
                account: vec![account],
                ..Default::default()
            },
            c2,
            d2,
        )?;
        let snapshot = second.snapshot()?;
        assert_eq!(
            snapshot.positions.get(&instrument_id()?).copied(),
            Some(reservation.quantity.get()),
            "the verified account journal reconstructs the position"
        );
        assert_eq!(
            snapshot.gross_exposure_paise,
            Paise(2_000),
            "a consumed and filled permit moves pending exposure into gross"
        );
        assert_eq!(
            snapshot.pending_gross_exposure_paise,
            Paise(0),
            "no pending exposure survives a verified fill"
        );
        assert_eq!(
            snapshot.open_orders, 0,
            "a verified fill closes the pending order"
        );
        assert!(
            !snapshot.kill_switch_engaged,
            "the durably released kill switch is restored"
        );
        assert!(
            !snapshot.reconciliation_blocked,
            "the authenticated reconciliation resolution is restored"
        );
        assert_eq!(
            snapshot.reconciliation_reason.as_deref(),
            Some("verified replay"),
            "the reconciliation reason is restored rather than discarded"
        );
        assert_eq!(
            snapshot.reconciliation_evidence_ref.as_deref(),
            Some("reconciliation-evidence"),
            "the reconciliation evidence reference is restored"
        );

        let replay = second.evaluate_and_reserve(
            intent(1, 2, PRICE, Side::Buy)?,
            observation(&second)?,
            None,
        );
        assert!(
            matches!(replay, Err(AuthorityError::AlreadyConsumed(_))),
            "a recovered consumed intent cannot issue a second permit"
        );

        let next = second.evaluate_and_reserve(
            intent(2, 2, PRICE, Side::Buy)?,
            observation(&second)?,
            None,
        )?;
        assert_eq!(
            next.permit_id, 2,
            "the permit counter continues from the recovered stream"
        );
        drop(second);
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);
        Ok(())
    }

    /// An OPEN OMS permit may already have executed part of its authorized
    /// quantity. Recovery must turn that filled part into position exposure and
    /// keep only the leaves quantity reserved; treating the fill as zero or
    /// keeping the full order reserved misrepresents both figures.
    #[test]
    fn recovery_applies_partial_fill_to_position_and_leaves_reservation() -> TestResult {
        let control = temp_path("control-partial-fill");
        let decisions = temp_path("decisions-partial-fill");
        let lock = temp_path("lock-partial-fill");
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);

        let first = ready_authority(&control, &decisions, &lock)?;
        let target = intent(1, 4, PRICE, Side::Buy)?;
        let reservation = first.evaluate_and_reserve(target.clone(), observation(&first)?, None)?;
        drop(first);

        let digest = permit_digest(
            reservation.permit_id,
            &reservation.assessment.policy_version,
            reservation.snapshot_revision,
            &target,
        );
        let mut oms = OmsPermitRecord::new(
            reservation.permit_id,
            reservation.intent_id,
            digest,
            OmsPermitState::Open,
            1,
            PRICE,
        );
        oms.seal(1, genesis_digest());
        let mut account = AccountRecoveryRecord::new(
            "partial-fill-1".to_owned(),
            AccountId::new(ACCOUNT)?,
            instrument_id()?,
            1,
        );
        account.seal(1, genesis_digest());
        let partial_fill = exec_record(
            &reservation,
            "exec-partial",
            1,
            ExecutionEventKind::Fill,
            None,
            1,
            3,
            PRICE,
            0,
            0,
        );

        let (c, d) = file_stores(&control, &decisions);
        let recovered = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms],
                executions: seal_executions(vec![partial_fill]),
                account: vec![account],
                ..Default::default()
            },
            c,
            d,
        )?;
        let snapshot = recovered.snapshot()?;
        assert_eq!(
            snapshot.positions.get(&instrument_id()?).copied(),
            Some(1),
            "the executed part becomes a position on restart"
        ); // first fill
        assert_eq!(
            snapshot.gross_exposure_paise,
            Paise(PRICE),
            "the executed notional becomes gross exposure"
        ); // execution evidence
        assert_eq!(
            snapshot.pending_gross_exposure_paise,
            Paise(3 * PRICE),
            "only the leaves notional stays reserved"
        ); // execution evidence
        let retained = recovered.reservation(reservation.intent_id)?;
        assert_eq!(
            retained.quantity.get(),
            3,
            "the open permit reserves only the leaves quantity"
        ); // execution evidence
        assert_eq!(
            retained.reserved_notional_paise,
            Paise(3 * PRICE),
            "the open permit reserves only the leaves notional"
        ); // execution evidence
        assert_eq!(snapshot.open_orders, 1, "the permit is still open");

        drop(recovered);
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);
        Ok(())
    }

    /// Issue one durable permit and release its authority so a restart can be
    /// driven from the recorded decision stream.
    fn consumed_permit(
        control: &std::path::Path,
        decisions: &std::path::Path,
        lock: &std::path::Path,
        quantity: i64,
    ) -> Result<(OrderIntent, Reservation), Box<dyn std::error::Error>> {
        let first = ready_authority(control, decisions, lock)?;
        let target = intent(1, quantity, PRICE, Side::Buy)?;
        let reservation = first.evaluate_and_reserve(target.clone(), observation(&first)?, None)?;
        drop(first);
        Ok((target, reservation))
    }

    fn digest_for(target: &OrderIntent, reservation: &Reservation) -> String {
        permit_digest(
            reservation.permit_id,
            &reservation.assessment.policy_version,
            reservation.snapshot_revision,
            target,
        )
    }

    fn oms_record(
        reservation: &Reservation,
        digest: String,
        state: OmsPermitState,
        filled: i64,
        notional: i64,
        leaves: Option<i64>,
    ) -> OmsPermitRecord {
        let mut record = OmsPermitRecord::new(
            reservation.permit_id,
            reservation.intent_id,
            digest,
            state,
            filled,
            notional,
        );
        if let Some(leaves) = leaves {
            record = record.with_leaves_quantity(leaves);
        }
        record.seal(1, genesis_digest());
        record
    }

    #[allow(clippy::too_many_arguments)] // fixture mirrors every execution fact under test
    fn exec_record(
        reservation: &Reservation,
        id: &str,
        report_sequence: u64,
        kind: ExecutionEventKind,
        corrects: Option<&str>,
        cum_quantity: i64,
        leaves: i64,
        cum_notional: i64,
        fees: i64,
        costs: i64,
    ) -> ExecutionRecoveryRecord {
        ExecutionRecoveryRecord::new(
            reservation.permit_id,
            reservation.intent_id,
            id.to_owned(),
            report_sequence,
            kind,
            corrects.map(str::to_owned),
            reservation.quantity.get(),
            cum_quantity,
            leaves,
            cum_notional,
            fees,
            costs,
        )
    }

    fn seal_executions(records: Vec<ExecutionRecoveryRecord>) -> Vec<ExecutionRecoveryRecord> {
        let mut sealed = records;
        let mut previous = genesis_digest().to_owned();
        for (index, record) in sealed.iter_mut().enumerate() {
            record.seal(
                u64::try_from(index + 1).expect("fixture sequence"),
                &previous,
            );
            previous = record.digest.clone();
        }
        sealed
    }

    fn inconsistent(error: Box<dyn std::error::Error>) -> bool {
        matches!(
            error.downcast::<RecoveryError>(),
            Ok(error) if matches!(*error, RecoveryError::Inconsistent(_))
        )
    }

    fn account_delta(
        quantity: i64,
    ) -> Result<Vec<AccountRecoveryRecord>, Box<dyn std::error::Error>> {
        let mut record = AccountRecoveryRecord::new(
            "execution-delta".to_owned(),
            AccountId::new(ACCOUNT)?,
            instrument_id()?,
            quantity,
        );
        record.seal(1, genesis_digest());
        Ok(vec![record])
    }

    /// Whether the recovered authority still grants a permit for a known-good
    /// probe intent. Used to prove a restart reproduces the same risk result.
    fn risk_probe(authority: &RiskAuthority) -> Result<bool, Box<dyn std::error::Error>> {
        authority.register_all_owners()?;
        Ok(authority
            .evaluate_and_reserve(
                intent(9, 1, PRICE, Side::Buy)?,
                observation(authority)?,
                None,
            )
            .is_ok())
    }

    #[test]
    fn recovery_replays_partial_fill_then_fill_then_cancel_across_restarts() -> TestResult {
        let control = temp_path("control-lifecycle");
        let decisions = temp_path("decisions-lifecycle");
        let lock = temp_path("lock-lifecycle");
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);
        let (target, reservation) = consumed_permit(&control, &decisions, &lock, 4)?;
        let digest = digest_for(&target, &reservation);
        let first_fill = exec_record(
            &reservation,
            "exec-1",
            1,
            ExecutionEventKind::Fill,
            None,
            1,
            3,
            PRICE,
            10,
            5,
        );
        let second_fill = exec_record(
            &reservation,
            "exec-2",
            2,
            ExecutionEventKind::Fill,
            None,
            2,
            2,
            2 * PRICE,
            20,
            10,
        );

        let (c1, d1) = file_stores(&control, &decisions);
        let stage1 = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &reservation,
                    digest.clone(),
                    OmsPermitState::Open,
                    1,
                    PRICE,
                    Some(3),
                )],
                executions: seal_executions(vec![first_fill.clone()]),
                account: account_delta(1)?,
                ..Default::default()
            },
            c1,
            d1,
        )?;
        let snapshot = stage1.snapshot()?;
        assert_eq!(
            snapshot.positions.get(&instrument_id()?).copied(),
            Some(1),
            "the reconstructed position equals the executed quantity"
        ); // direct position // restart position
        assert_eq!(
            snapshot.gross_exposure_paise,
            Paise(PRICE),
            "gross exposure equals the executed notional"
        );
        assert_eq!(
            snapshot.pending_gross_exposure_paise,
            Paise(3 * PRICE),
            "gross exposure equals the executed notional"
        );
        assert_eq!(
            snapshot
                .executions
                .get(&reservation.intent_id)
                .map(|value| value.filled_quantity),
            Some(1),
            "the first execution is visible as cumulative filled quantity"
        );
        drop(stage1);

        let (c2, d2) = file_stores(&control, &decisions);
        let stage2 = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &reservation,
                    digest.clone(),
                    OmsPermitState::Open,
                    2,
                    2 * PRICE,
                    Some(2),
                )],
                executions: seal_executions(vec![first_fill.clone(), second_fill.clone()]),
                account: account_delta(2)?,
                ..Default::default()
            },
            c2,
            d2,
        )?;
        let snapshot = stage2.snapshot()?;
        assert_eq!(
            snapshot.positions.get(&instrument_id()?).copied(),
            Some(2),
            "the reconstructed position equals the executed quantity"
        );
        assert_eq!(
            snapshot.gross_exposure_paise,
            Paise(2 * PRICE),
            "gross exposure equals the executed notional"
        );
        assert_eq!(
            snapshot.pending_gross_exposure_paise,
            Paise(2 * PRICE),
            "gross exposure equals the executed notional"
        );
        assert_eq!(
            stage2.reservation(reservation.intent_id)?.quantity.get(),
            2,
            "the second fill further reduces the leaves reservation"
        );
        assert_eq!(
            snapshot
                .executions
                .get(&reservation.intent_id)
                .map(|value| value.execution_ids.clone()),
            Some(vec!["exec-1".to_owned(), "exec-2".to_owned()]),
            "execution identities are deduplicated and canonically ordered"
        ); // both execution ids
        drop(stage2);

        // A terminal cancellation keeps the executed part as exposure and
        // releases only the unexecuted remainder. Reported LeavesQty may be
        // non-zero here: terminal states are exempt from the FIX identity.
        let (c3, d3) = file_stores(&control, &decisions);
        let stage3 = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &reservation,
                    digest.clone(),
                    OmsPermitState::Cancelled,
                    2,
                    2 * PRICE,
                    Some(2),
                )],
                executions: seal_executions(vec![first_fill.clone(), second_fill.clone()]),
                account: account_delta(2)?,
                ..Default::default()
            },
            c3,
            d3,
        )?;
        let restarted = stage3.snapshot()?;
        assert_eq!(
            restarted.positions.get(&instrument_id()?).copied(),
            Some(2),
            "the reconstructed position equals the executed quantity"
        );
        assert_eq!(
            restarted.gross_exposure_paise,
            Paise(2 * PRICE),
            "gross exposure equals the executed notional"
        ); // execution evidence
        assert_eq!(
            restarted.pending_gross_exposure_paise,
            Paise(0),
            "gross exposure equals the executed notional"
        ); // execution evidence
        assert_eq!(restarted.reservation_count, 0, "the remainder is released");
        assert_eq!(
            restarted.open_orders, 0,
            "the permit lifecycle is reflected in the open order count"
        ); // execution evidence
        let restarted_probe = risk_probe(&stage3)?;

        // One uninterrupted restart with the complete journal must reproduce
        // the same position, reservation, exposure and risk result.
        let other_control = temp_path("control-lifecycle-direct");
        let other_decisions = temp_path("decisions-lifecycle-direct");
        let other_lock = temp_path("lock-lifecycle-direct");
        let _ = std::fs::remove_file(&other_control);
        let _ = std::fs::remove_file(&other_decisions);
        let _ = std::fs::remove_file(&other_lock);
        let (direct_target, direct_reservation) =
            consumed_permit(&other_control, &other_decisions, &other_lock, 4)?;
        let (c4, d4) = file_stores(&other_control, &other_decisions);
        let direct = build_recovered(
            &other_lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &direct_reservation,
                    digest_for(&direct_target, &direct_reservation),
                    OmsPermitState::Cancelled,
                    2,
                    2 * PRICE,
                    Some(2),
                )],
                executions: seal_executions(vec![first_fill, second_fill]),
                account: account_delta(2)?,
                ..Default::default()
            },
            c4,
            d4,
        )?;
        let direct_snapshot = direct.snapshot()?;
        assert_eq!(
            direct_snapshot.positions, restarted.positions,
            "the reconstructed position equals the executed quantity"
        ); // execution evidence
        assert_eq!(
            direct_snapshot.gross_exposure_paise, restarted.gross_exposure_paise,
            "gross exposure equals the executed notional"
        );
        assert_eq!(
            direct_snapshot.pending_gross_exposure_paise, restarted.pending_gross_exposure_paise,
            "gross exposure equals the executed notional"
        );
        assert_eq!(
            direct_snapshot.reservation_count, restarted.reservation_count,
            "the permit count matches the terminal outcome"
        );
        assert_eq!(
            direct_snapshot.executions, restarted.executions,
            "the reconciled execution exposure is exact"
        );
        assert_eq!(
            risk_probe(&direct)?,
            restarted_probe,
            "the risk result is identical"
        );

        drop(stage3);
        drop(direct);
        for path in [
            &control,
            &decisions,
            &lock,
            &other_control,
            &other_decisions,
            &other_lock,
        ] {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }

    #[test]
    fn recovery_deduplicates_executions_and_orders_reports_by_sequence() -> TestResult {
        let control = temp_path("control-exec-order");
        let decisions = temp_path("decisions-exec-order");
        let lock = temp_path("lock-exec-order");
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);
        let (target, reservation) = consumed_permit(&control, &decisions, &lock, 4)?;
        let digest = digest_for(&target, &reservation);
        let first = exec_record(
            &reservation,
            "exec-1",
            1,
            ExecutionEventKind::Fill,
            None,
            1,
            3,
            PRICE,
            0,
            0,
        );
        let second = exec_record(
            &reservation,
            "exec-2",
            2,
            ExecutionEventKind::Fill,
            None,
            2,
            2,
            2 * PRICE,
            0,
            0,
        );

        let (c1, d1) = file_stores(&control, &decisions);
        let duplicate = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &reservation,
                    digest.clone(),
                    OmsPermitState::Open,
                    1,
                    PRICE,
                    Some(3),
                )],
                executions: seal_executions(vec![first.clone(), first.clone()]),
                account: account_delta(1)?,
                ..Default::default()
            },
            c1,
            d1,
        )?;
        let exposures = duplicate.snapshot()?.executions;
        assert_eq!(
            exposures
                .get(&reservation.intent_id)
                .map(|value| value.execution_ids.clone()),
            Some(vec!["exec-1".to_owned()]),
            "a replayed execution identity is counted once"
        ); // duplicate identity
        drop(duplicate);

        let mut conflicting = first.clone();
        conflicting.cumulative_notional_paise = 2 * PRICE;
        let (c2, d2) = file_stores(&control, &decisions);
        let conflict = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &reservation,
                    digest.clone(),
                    OmsPermitState::Open,
                    1,
                    PRICE,
                    Some(3),
                )],
                executions: seal_executions(vec![first.clone(), conflicting]),
                account: account_delta(1)?,
                ..Default::default()
            },
            c2,
            d2,
        );
        assert!(
            conflict.err().is_some_and(inconsistent),
            "one execution identity with two different facts fails closed"
        );

        let (c3, d3) = file_stores(&control, &decisions);
        let reordered = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &reservation,
                    digest,
                    OmsPermitState::Open,
                    2,
                    2 * PRICE,
                    Some(2),
                )],
                executions: seal_executions(vec![second, first]),
                account: account_delta(2)?,
                ..Default::default()
            },
            c3,
            d3,
        )?;
        let snapshot = reordered.snapshot()?;
        assert_eq!(
            snapshot.positions.get(&instrument_id()?).copied(),
            Some(2),
            "the reconstructed position equals the executed quantity"
        ); // execution evidence
        assert_eq!(
            snapshot
                .executions
                .get(&reservation.intent_id)
                .map(|value| value.execution_ids.clone()),
            Some(vec!["exec-1".to_owned(), "exec-2".to_owned()]),
            "report order in the journal does not change the reconciled set"
        ); // execution evidence
        assert_eq!(
            reordered.reservation(reservation.intent_id)?.quantity.get(),
            2,
            "leaves quantity follows CumQty, not the last report seen"
        ); // execution evidence
        drop(reordered);
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);
        Ok(())
    }

    #[test]
    fn recovery_applies_execution_correction_and_trade_cancel() -> TestResult {
        let control = temp_path("control-exec-correct");
        let decisions = temp_path("decisions-exec-correct");
        let lock = temp_path("lock-exec-correct");
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);
        let (target, reservation) = consumed_permit(&control, &decisions, &lock, 4)?;
        let digest = digest_for(&target, &reservation);
        let fill = exec_record(
            &reservation,
            "exec-1",
            1,
            ExecutionEventKind::Fill,
            None,
            1,
            3,
            PRICE,
            10,
            5,
        );
        let correction = exec_record(
            &reservation,
            "exec-2",
            2,
            ExecutionEventKind::Correct,
            Some("exec-1"),
            1,
            3,
            900,
            5,
            2,
        );
        let (c1, d1) = file_stores(&control, &decisions);
        let corrected = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &reservation,
                    digest.clone(),
                    OmsPermitState::Open,
                    1,
                    900,
                    Some(3),
                )],
                executions: seal_executions(vec![fill.clone(), correction]),
                account: account_delta(1)?,
                ..Default::default()
            },
            c1,
            d1,
        )?;
        let exposure = corrected.snapshot()?.executions;
        let exposure = exposure
            .get(&reservation.intent_id)
            .expect("corrected exposure");
        assert_eq!(
            exposure.filled_quantity, 1,
            "cumulative filled and remaining leaves quantities stay distinct"
        );
        assert_eq!(
            exposure.leaves_quantity, 3,
            "cumulative filled and remaining leaves quantities stay distinct"
        );
        assert_eq!(
            exposure.filled_notional_paise,
            Paise(900),
            "the reconstructed risk state matches the durable execution facts"
        );
        assert_eq!(
            exposure.average_fill_price_paise,
            Some(Paise(900)),
            "average fill price is derived from cumulative executed notional"
        );
        assert_eq!(
            exposure.fees_paise,
            Paise(5),
            "corrected fees replace the original claim"
        );
        assert_eq!(
            exposure.costs_paise,
            Paise(2),
            "corrected costs are kept distinct"
        );
        assert_eq!(
            exposure.execution_ids,
            vec!["exec-1".to_owned()],
            "execution identities are deduplicated and canonically ordered"
        );
        assert_eq!(
            corrected.snapshot()?.gross_exposure_paise,
            Paise(900),
            "gross exposure equals the executed notional"
        );
        drop(corrected);

        let cancel = exec_record(
            &reservation,
            "exec-3",
            3,
            ExecutionEventKind::TradeCancel,
            Some("exec-1"),
            0,
            4,
            0,
            0,
            0,
        );
        let (c2, d2) = file_stores(&control, &decisions);
        let cancelled = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &reservation,
                    digest,
                    OmsPermitState::Open,
                    0,
                    0,
                    Some(4),
                )],
                executions: seal_executions(vec![fill, cancel]),
                account: account_delta(0)?,
                ..Default::default()
            },
            c2,
            d2,
        )?;
        let snapshot = cancelled.snapshot()?;
        assert!(
            !snapshot.positions.contains_key(&instrument_id()?),
            "a trade cancel removes the execution from the position"
        );
        assert_eq!(
            snapshot.pending_gross_exposure_paise,
            Paise(4 * PRICE),
            "gross exposure equals the executed notional"
        );
        assert!(
            snapshot.executions.is_empty(),
            "no partial fill survives its trade cancel"
        );
        assert_eq!(
            cancelled.reservation(reservation.intent_id)?.quantity.get(),
            4,
            "the reservation retains only the leaves quantity"
        );
        drop(cancelled);
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);
        Ok(())
    }

    #[test]
    fn recovery_latches_reconciliation_for_unreconcilable_partial_fills() -> TestResult {
        let control = temp_path("control-exec-uncertain");
        let decisions = temp_path("decisions-exec-uncertain");
        let lock = temp_path("lock-exec-uncertain");
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);
        let (target, reservation) = consumed_permit(&control, &decisions, &lock, 4)?;
        let digest = digest_for(&target, &reservation);

        // A fill with no notional cannot be priced. The execution-level report
        // is still required; recovery books the executed quantity and shrinks
        // the reservation, but stays NOT-READY.
        let (c1, d1) = file_stores(&control, &decisions);
        let unpriced = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &reservation,
                    digest.clone(),
                    OmsPermitState::Open,
                    1,
                    0,
                    Some(3),
                )],
                executions: seal_executions(vec![exec_record(
                    &reservation,
                    "exec-unpriced",
                    1,
                    ExecutionEventKind::Fill,
                    None,
                    1,
                    3,
                    0,
                    0,
                    0,
                )]),
                account: account_delta(1)?,
                ..Default::default()
            },
            c1,
            d1,
        )?;
        let snapshot = unpriced.snapshot()?;
        assert!(
            snapshot.reconciliation_blocked,
            "uncertain money latches reconciliation"
        );
        assert!(
            snapshot
                .reconciliation_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("no notional")),
            "the latch carries an authored reason"
        );
        assert_eq!(
            snapshot.positions.get(&instrument_id()?).copied(),
            Some(1),
            "the reconstructed position equals the executed quantity"
        );
        assert_eq!(
            unpriced.reservation(reservation.intent_id)?.quantity.get(),
            3,
            "the reservation retains only the leaves quantity"
        );
        assert!(
            !risk_probe(&unpriced)?,
            "a NOT-READY authority denies new risk"
        );
        drop(unpriced);

        // A correction that references an unknown execution is also uncertain,
        // never silently ignored; the reported fill is still accounted for.
        let orphan = exec_record(
            &reservation,
            "exec-orphan",
            1,
            ExecutionEventKind::Correct,
            Some("exec-missing"),
            1,
            3,
            PRICE,
            0,
            0,
        );
        let (c2, d2) = file_stores(&control, &decisions);
        let uncertain = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &reservation,
                    digest,
                    OmsPermitState::Open,
                    1,
                    PRICE,
                    Some(3),
                )],
                executions: seal_executions(vec![orphan]),
                account: account_delta(1)?,
                ..Default::default()
            },
            c2,
            d2,
        )?;
        let snapshot = uncertain.snapshot()?;
        assert!(
            snapshot.reconciliation_blocked,
            "uncertain execution latches reconciliation with an authored reason"
        );
        assert!(
            snapshot
                .reconciliation_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("unknown execution")),
            "uncertain execution latches reconciliation with an authored reason"
        );
        assert_eq!(
            snapshot.positions.get(&instrument_id()?).copied(),
            Some(1),
            "the reconstructed position equals the executed quantity"
        );
        assert_eq!(
            uncertain.reservation(reservation.intent_id)?.quantity.get(),
            3,
            "the reservation retains only the leaves quantity"
        );
        drop(uncertain);
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);
        Ok(())
    }

    #[test]
    fn recovery_rejects_partial_execution_contradictions_and_allows_terminal_exceptions()
    -> TestResult {
        let control = temp_path("control-exec-contradiction");
        let decisions = temp_path("decisions-exec-contradiction");
        let lock = temp_path("lock-exec-contradiction");
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);
        let (target, reservation) = consumed_permit(&control, &decisions, &lock, 4)?;
        let digest = digest_for(&target, &reservation);
        let fill = exec_record(
            &reservation,
            "exec-1",
            1,
            ExecutionEventKind::Fill,
            None,
            1,
            3,
            PRICE,
            0,
            0,
        );

        let (c1, d1) = file_stores(&control, &decisions);
        let account_disagreement = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &reservation,
                    digest.clone(),
                    OmsPermitState::Open,
                    1,
                    PRICE,
                    Some(3),
                )],
                executions: seal_executions(vec![fill.clone()]),
                account: account_delta(2)?,
                ..Default::default()
            },
            c1,
            d1,
        );
        assert!(
            account_disagreement.err().is_some_and(inconsistent),
            "an account position disagreeing with OMS cumulative quantity fails closed"
        );

        let (c2, d2) = file_stores(&control, &decisions);
        let journal_disagreement = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &reservation,
                    digest.clone(),
                    OmsPermitState::Open,
                    1,
                    PRICE,
                    Some(3),
                )],
                executions: seal_executions(vec![fill.clone()]),
                account: account_delta(2)?,
                ..Default::default()
            },
            c2,
            d2,
        );
        assert!(
            journal_disagreement.err().is_some_and(inconsistent),
            "an OMS summary disagreeing with the execution journal fails closed"
        );

        let (c3, d3) = file_stores(&control, &decisions);
        let broken_leaves = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &reservation,
                    digest.clone(),
                    OmsPermitState::Open,
                    1,
                    PRICE,
                    Some(2),
                )],
                executions: seal_executions(vec![fill.clone()]),
                account: account_delta(1)?,
                ..Default::default()
            },
            c3,
            d3,
        );
        assert!(
            broken_leaves.err().is_some_and(inconsistent),
            "OrderQty = CumQty + LeavesQty is enforced on an open permit"
        );

        let mut broken_execution = fill.clone();
        broken_execution.leaves_quantity = 2;
        let (c4, d4) = file_stores(&control, &decisions);
        let broken_execution = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &reservation,
                    digest.clone(),
                    OmsPermitState::Open,
                    1,
                    PRICE,
                    Some(3),
                )],
                executions: seal_executions(vec![broken_execution]),
                account: account_delta(1)?,
                ..Default::default()
            },
            c4,
            d4,
        );
        assert!(
            broken_execution.err().is_some_and(inconsistent),
            "an execution report violating the FIX identity fails closed"
        );

        // Terminal states are exempt: a cancellation may still report leaves,
        // and a full fill may carry a stale leaves figure.
        let (c5, d5) = file_stores(&control, &decisions);
        let cancelled = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &reservation,
                    digest.clone(),
                    OmsPermitState::Cancelled,
                    1,
                    PRICE,
                    Some(3),
                )],
                executions: seal_executions(vec![exec_record(
                    &reservation,
                    "exec-cancelled",
                    1,
                    ExecutionEventKind::Fill,
                    None,
                    1,
                    3,
                    PRICE,
                    0,
                    0,
                )]),
                account: account_delta(1)?,
                ..Default::default()
            },
            c5,
            d5,
        )?;
        let snapshot = cancelled.snapshot()?;
        assert_eq!(
            snapshot.positions.get(&instrument_id()?).copied(),
            Some(1),
            "the reconstructed position equals the executed quantity"
        );
        assert_eq!(
            snapshot.pending_gross_exposure_paise,
            Paise(0),
            "gross exposure equals the executed notional"
        );
        assert_eq!(
            snapshot.reservation_count, 0,
            "the permit count matches the terminal outcome"
        );
        drop(cancelled);

        let (c6, d6) = file_stores(&control, &decisions);
        let filled = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &reservation,
                    digest,
                    OmsPermitState::Filled,
                    4,
                    4 * PRICE,
                    Some(2),
                )],
                executions: seal_executions(vec![exec_record(
                    &reservation,
                    "exec-filled",
                    1,
                    ExecutionEventKind::Fill,
                    None,
                    4,
                    0,
                    4 * PRICE,
                    0,
                    0,
                )]),
                account: account_delta(4)?,
                ..Default::default()
            },
            c6,
            d6,
        )?;
        let snapshot = filled.snapshot()?;
        assert_eq!(
            snapshot.positions.get(&instrument_id()?).copied(),
            Some(4),
            "the reconstructed position equals the executed quantity"
        );
        assert_eq!(
            snapshot.gross_exposure_paise,
            Paise(4 * PRICE),
            "gross exposure equals the executed notional"
        );
        assert_eq!(
            snapshot.pending_gross_exposure_paise,
            Paise(0),
            "gross exposure equals the executed notional"
        );
        assert_eq!(
            snapshot.reservation_count, 0,
            "the permit count matches the terminal outcome"
        );
        drop(filled);
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);
        Ok(())
    }

    #[test]
    fn recovery_rejects_broken_sequences_digests_and_position_mismatch() -> TestResult {
        let control = temp_path("control-negative");
        let decisions = temp_path("decisions-negative");
        let lock = temp_path("lock-negative");
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);

        let mut oms = OmsPermitRecord::new(
            1,
            IntentId::new(1)?,
            "a".repeat(64),
            OmsPermitState::Open,
            2,
            2_000,
        );
        oms.seal(2, genesis_digest());
        let (c1, d1) = file_stores(&control, &decisions);
        let gap = recover_with(
            &lock,
            FixtureRecords {
                oms: vec![oms],
                account: Vec::new(),
                ..Default::default()
            },
            c1,
            d1,
        );
        assert!(
            matches!(gap, Err(RecoveryError::Sequence(_))),
            "a non-contiguous OMS journal sequence fails startup closed"
        );

        let mut decision = RiskDecisionRecord::new(
            intent(1, 2, PRICE, Side::Buy)?,
            DecisionOutcome::Accepted,
            1,
            2_000,
            1,
            0,
        );
        decision.seal(1, genesis_digest());
        let mut tampered = decision.clone();
        tampered.digest = "f".repeat(64);
        std::fs::write(
            &decisions,
            format!("{}\n", serde_json::to_string(&tampered)?),
        )?;
        let (c2, d2) = file_stores(&control, &decisions);
        let digest_failure = recover_with(&lock, FixtureRecords::default(), c2, d2);
        assert!(
            matches!(digest_failure, Err(RecoveryError::DecisionStore(_))),
            "a broken decision digest chain fails startup closed"
        );

        let _ = std::fs::remove_file(&decisions);
        let store = FileDecisionStore::new(&decisions);
        let accepted = RiskDecisionRecord::new(
            intent(1, 2, PRICE, Side::Buy)?,
            DecisionOutcome::Accepted,
            1,
            2_000,
            1,
            0,
        );
        store.append_decision(&accepted)?;
        let sealed = store.load_decisions()?.remove(0);
        let expected_digest = permit_digest(
            sealed.permit_id,
            "risk-v1",
            sealed.revision,
            &intent(1, 2, PRICE, Side::Buy)?,
        );
        let mut filled = OmsPermitRecord::new(
            sealed.permit_id,
            sealed.intent_id,
            expected_digest,
            OmsPermitState::Filled,
            2,
            2_000,
        );
        filled.seal(1, genesis_digest());
        let mut full_fill = ExecutionRecoveryRecord::new(
            sealed.permit_id,
            sealed.intent_id,
            "exec-mismatch".to_owned(),
            1,
            ExecutionEventKind::Fill,
            None,
            2,
            2,
            0,
            2_000,
            0,
            0,
        );
        full_fill.seal(1, genesis_digest());
        let mut account = AccountRecoveryRecord::new(
            "account-event".to_owned(),
            AccountId::new(ACCOUNT)?,
            instrument_id()?,
            3,
        );
        account.seal(1, genesis_digest());
        let (c3, d3) = file_stores(&control, &decisions);
        let mismatch = recover_with(
            &lock,
            FixtureRecords {
                oms: vec![filled],
                executions: vec![full_fill],
                account: vec![account],
                ..Default::default()
            },
            c3,
            d3,
        );
        assert!(
            matches!(mismatch, Err(RecoveryError::Inconsistent(_))),
            "an account position that disagrees with the derived fill fails startup closed"
        );

        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);
        Ok(())
    }

    #[test]
    fn recovery_rejects_snapshots_bound_to_a_foreign_journal_or_account() -> TestResult {
        let control = temp_path("control-foreign-snapshot");
        let decisions = temp_path("decisions-foreign-snapshot");
        let lock = temp_path("lock-foreign-snapshot");
        let permits = temp_path("foreign-permits");
        let executions = temp_path("foreign-executions");
        for path in [&control, &decisions, &lock, &permits, &executions] {
            let _ = std::fs::remove_file(path);
        }
        write_json_lines(&permits, &Vec::<OmsPermitRecord>::new())?;
        write_json_lines(&executions, &Vec::<ExecutionRecoveryRecord>::new())?;
        let account = verified_records(FixtureRecords::default())?.account;

        let foreign_account = VerifiedOmsSnapshot::verify(
            &permits,
            &executions,
            &JournalIdentity::new(ObservationSource::Oms, AccountId::new(99)?, OMS_JOURNAL_ID),
            &RecoveryAuthenticity::OwnerSignature(owner_key()),
        )?;
        let (c1, d1) = file_stores(&control, &decisions);
        let wrong_account = RiskAuthority::recover(RecoveryConfig {
            account_id: AccountId::new(ACCOUNT)?,
            policy: policy()?,
            instruments: vec![instrument()?],
            trade_date: date()?,
            control: c1,
            decisions: d1,
            oms: foreign_account,
            account: account.clone(),
            sectors: BTreeMap::new(),
            ownership_path: lock.to_path_buf(),
        });
        assert!(
            matches!(wrong_account, Err(RecoveryError::Snapshot(_))),
            "a snapshot sealed for another account is denied at startup"
        );

        let foreign_journal = VerifiedOmsSnapshot::verify(
            &permits,
            &executions,
            &JournalIdentity::new(
                ObservationSource::Oms,
                AccountId::new(ACCOUNT)?,
                "some-other-journal",
            ),
            &RecoveryAuthenticity::OwnerSignature(owner_key()),
        )?;
        let (c2, d2) = file_stores(&control, &decisions);
        let wrong_journal = RiskAuthority::recover(RecoveryConfig {
            account_id: AccountId::new(ACCOUNT)?,
            policy: policy()?,
            instruments: vec![instrument()?],
            trade_date: date()?,
            control: c2,
            decisions: d2,
            oms: foreign_journal,
            account,
            sectors: BTreeMap::new(),
            ownership_path: lock.to_path_buf(),
        });
        assert!(
            matches!(wrong_journal, Err(RecoveryError::Snapshot(_))),
            "a snapshot sealed from another journal is denied at startup"
        );

        for path in [&control, &decisions, &lock, &permits, &executions] {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }

    #[test]
    fn noop_control_store_is_not_a_durable_startup_default() -> TestResult {
        let lock = temp_path("lock-noop");
        let _ = std::fs::remove_file(&lock);
        let failures = recover_with(
            &lock,
            FixtureRecords::default(),
            Box::new(NoopControlStore),
            Box::new(InMemoryDecisionStore::new()),
        );
        assert!(
            matches!(failures, Err(RecoveryError::NonDurableStore)),
            "an in-memory control store cannot establish durable startup state"
        );
        let simulation = RiskAuthority::new_simulation(
            AccountId::new(ACCOUNT)?,
            policy()?,
            vec![instrument()?],
            date()?,
        );
        assert!(
            simulation.is_simulation(),
            "the simulation constructor is explicitly labelled"
        );
        let _ = std::fs::remove_file(&lock);
        Ok(())
    }

    #[test]
    fn reconciliation_reason_and_evidence_survive_restart() -> TestResult {
        let path = temp_path("control-reconciliation");
        let _ = std::fs::remove_file(&path);
        let first = RiskAuthority::new_with_control_store(
            AccountId::new(ACCOUNT)?,
            policy()?,
            vec![instrument()?],
            date()?,
            Box::new(FileControlStore::new(&path)),
        );
        let tokens = first.register_all_owners()?;
        first.set_reconciliation_blocked(
            tokens.reconciliation,
            false,
            "evidence reviewed".to_owned(),
            Some("evidence-ref".to_owned()),
        )?;
        assert_eq!(
            first.reconciliation_decision().map(|value| value.reason),
            Some("evidence reviewed".to_owned()),
            "the authored reason is available in memory"
        );
        drop(first);

        let second = RiskAuthority::new_with_control_store(
            AccountId::new(ACCOUNT)?,
            policy()?,
            vec![instrument()?],
            date()?,
            Box::new(FileControlStore::new(&path)),
        );
        assert!(
            !second.snapshot()?.reconciliation_blocked,
            "the persisted resolution is restored across a restart"
        );
        assert_eq!(
            second
                .reconciliation_decision()
                .and_then(|value| value.evidence_ref),
            Some("evidence-ref".to_owned()),
            "the persisted evidence reference is audible after a restart"
        );
        let _ = std::fs::remove_file(&path);
        Ok(())
    }

    fn ready_authority(
        control: &std::path::Path,
        decisions: &std::path::Path,
        lock: &std::path::Path,
    ) -> Result<RiskAuthority, Box<dyn std::error::Error>> {
        let (c, d) = file_stores(control, decisions);
        let value = build_recovered(lock, FixtureRecords::default(), c, d)?;
        let tokens = value.register_all_owners()?;
        value.set_kill_switch(
            tokens.operator,
            false,
            "operator".to_owned(),
            "documented release".to_owned(),
            Some("release-evidence".to_owned()),
        )?;
        value.set_session_open(true)?;
        value.set_snapshot_available(true)?;
        value.set_reconciliation_blocked(
            tokens.reconciliation,
            false,
            "verified replay".to_owned(),
            Some("reconciliation-evidence".to_owned()),
        )?;
        Ok(value)
    }

    #[test]
    fn recovery_latches_unresolved_oms_permits() -> TestResult {
        let control = temp_path("control-unresolved");
        let decisions = temp_path("decisions-unresolved");
        let lock = temp_path("lock-unresolved");
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);

        let first = ready_authority(&control, &decisions, &lock)?;
        let reservation = first.evaluate_and_reserve(
            intent(1, 2, PRICE, Side::Buy)?,
            observation(&first)?,
            None,
        )?;
        drop(first);

        let digest = permit_digest(
            reservation.permit_id,
            &reservation.assessment.policy_version,
            reservation.snapshot_revision,
            &intent(1, 2, PRICE, Side::Buy)?,
        );
        let mut oms = OmsPermitRecord::new(
            reservation.permit_id,
            reservation.intent_id,
            digest.clone(),
            OmsPermitState::Unknown,
            0,
            0,
        );
        oms.seal(1, genesis_digest());
        let (c, d) = file_stores(&control, &decisions);
        let second = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms],
                account: Vec::new(),
                ..Default::default()
            },
            c,
            d,
        )?;
        let snapshot = second.snapshot()?;
        assert_eq!(
            snapshot.unresolved_permits, 1,
            "an UNKNOWN OMS outcome stays determinable after restart"
        );
        assert!(
            snapshot.reconciliation_blocked,
            "an unresolved permit latches reconciliation"
        );
        assert_eq!(
            snapshot.reservation_count, 1,
            "the unsettled reservation is retained"
        );
        let recovered = second.reservation(reservation.intent_id)?;
        assert_eq!(
            second.permit_digest(&recovered, &intent(1, 2, PRICE, Side::Buy)?)?,
            digest,
            "the recovered permit still binds the exact authorized intent"
        );
        let tokens = second.register_all_owners()?;
        second.set_reconciliation_blocked(
            tokens.reconciliation,
            false,
            "broker evidence resolved the unknown order".to_owned(),
            Some("resolution-evidence".to_owned()),
        )?;
        assert!(
            !second.snapshot()?.reconciliation_blocked,
            "an authenticated resolution clears the latch"
        );
        assert_eq!(
            second.snapshot()?.unresolved_permits,
            1,
            "the recovered unresolved fact remains visible"
        );
        drop(second);
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);
        Ok(())
    }

    #[test]
    fn recovery_remembers_rejected_intents() -> TestResult {
        let control = temp_path("control-rejected");
        let decisions = temp_path("decisions-rejected");
        let lock = temp_path("lock-rejected");
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);

        let first = ready_authority(&control, &decisions, &lock)?;
        let oversized = intent(1, 11, PRICE, Side::Buy)?;
        let rejected = first.evaluate_and_reserve(oversized.clone(), observation(&first)?, None);
        assert!(
            matches!(rejected, Err(AuthorityError::Rejected(_))),
            "the oversized intent is rejected"
        );
        drop(first);

        let (c, d) = file_stores(&control, &decisions);
        let second = build_recovered(&lock, FixtureRecords::default(), c, d)?;
        let replay = second.evaluate_and_reserve(oversized, observation(&second)?, None);
        assert!(
            matches!(replay, Err(AuthorityError::PreviouslyRejected(_))),
            "the durable rejection survives the restart"
        );
        drop(second);
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);
        Ok(())
    }

    #[test]
    fn recovery_requires_exclusive_and_readable_state() -> TestResult {
        let control = temp_path("control-exclusive");
        let decisions = temp_path("decisions-exclusive");
        let lock = temp_path("lock-exclusive");
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);

        let first = ready_authority(&control, &decisions, &lock)?;
        let (c, d) = file_stores(&control, &decisions);
        let competing = recover_with(&lock, FixtureRecords::default(), c, d);
        assert!(
            matches!(competing, Err(RecoveryError::StateOwnership(_))),
            "a second startup cannot own the same authority state"
        );
        drop(first);

        std::fs::write(&control, "{broken\n")?;
        let (c2, d2) = file_stores(&control, &decisions);
        let corrupt = recover_with(&lock, FixtureRecords::default(), c2, d2);
        assert!(
            matches!(corrupt, Err(RecoveryError::Control(_))),
            "a corrupt control store is an explicit startup error"
        );
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);
        Ok(())
    }

    /// Startup cannot invent zero P&L, and a real recovered loss must bind the
    /// daily-loss limit on the very first reservation after restart.
    #[test]
    fn recovery_requires_daily_pnl_and_reconstructs_a_real_loss() -> TestResult {
        let control = temp_path("control-daily-pnl");
        let decisions = temp_path("decisions-daily-pnl");
        let lock = temp_path("lock-daily-pnl");
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);

        let (c1, d1) = file_stores(&control, &decisions);
        let missing = recover_with_no_pnl_fill(&lock, FixtureRecords::default(), c1, d1);
        assert!(
            matches!(missing, Err(RecoveryError::DailyPnl(_))),
            "recovery without sealed daily P&L fails closed"
        );

        let calendar = daily_calendar()?;
        let mut account = AccountRecoveryRecord::new(
            "recovered-loss".to_owned(),
            AccountId::new(ACCOUNT)?,
            instrument_id()?,
            0,
        )
        .with_pnl_effects(-5_000, 0, 0, 0);
        account.seal(1, genesis_digest());
        let mut record = DailyPnlRecord::new(
            AccountId::new(ACCOUNT)?,
            date()?,
            "INR".to_owned(),
            DailyPnlBasis::RealizedPlusUnrealized,
            &calendar,
            0,
        )
        .with_journal(1, account.digest.clone())
        .with_digests(
            daily_pnl_positions_digest(&BTreeMap::new()),
            genesis_digest().to_owned(),
        )
        .with_components(-5_000, 0, 0, 0, 0);
        record.seal(1, genesis_digest());
        let (c2, d2) = file_stores(&control, &decisions);
        let recovered = build_recovered(
            &lock,
            FixtureRecords {
                oms: Vec::new(),
                executions: Vec::new(),
                account: vec![account],
                daily_pnl: DailyPnlRecovery {
                    record: Some(record),
                    valuations: Vec::new(),
                    calendar: Some(calendar),
                },
            },
            c2,
            d2,
        )?;
        assert!(
            recovered.snapshot()?.daily_pnl_available,
            "recovery installed an authoritative P&L snapshot"
        );
        release_switch(&recovered)?;
        clear_reconciliation(&recovered)?;
        let denied = recovered.evaluate_and_reserve(
            intent(1, 2, PRICE, Side::Buy)?,
            observation(&recovered)?,
            None,
        );
        match &denied {
            Err(error @ AuthorityError::Rejected(_)) => assert!(
                rejected_check(error, RiskCheck::DailyLossLimit),
                "a recovered real loss is not silently replaced by zero"
            ),
            other => panic!("expected a daily-loss rejection, got {other:?}"),
        }
        drop(recovered);
        let _ = std::fs::remove_file(&control);
        let _ = std::fs::remove_file(&decisions);
        let _ = std::fs::remove_file(&lock);
        Ok(())
    }

    /// An OMS cumulative summary is not execution-level evidence. Without a
    /// sealed execution report there is no execution identity and no fee or
    /// cost fact, so recovery may not fabricate a zero-fee exposure or grant
    /// fresh risk on the strength of the aggregate alone.
    #[test]
    fn recovery_does_not_synthesize_execution_evidence_from_oms_aggregates() -> TestResult {
        let control = temp_path("control-no-evidence");
        let decisions = temp_path("decisions-no-evidence");
        let lock = temp_path("lock-no-evidence");
        for path in [&control, &decisions, &lock] {
            let _ = std::fs::remove_file(path);
        }
        let (target, reservation) = consumed_permit(&control, &decisions, &lock, 4)?;
        let digest = digest_for(&target, &reservation);

        let (c, d) = file_stores(&control, &decisions);
        let recovered = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms_record(
                    &reservation,
                    digest,
                    OmsPermitState::Open,
                    1,
                    PRICE,
                    Some(3),
                )],
                account: account_delta(1)?,
                ..Default::default()
            },
            c,
            d,
        )?;
        let snapshot = recovered.snapshot()?;
        assert!(
            !snapshot.executions.contains_key(&reservation.intent_id),
            "no execution exposure is synthesized without execution-level evidence"
        );
        assert!(
            snapshot.reconciliation_blocked,
            "an uncertifiable aggregate-only fill latches reconciliation"
        );
        assert!(
            snapshot
                .reconciliation_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("execution-level evidence")),
            "the latch names the missing evidence"
        );
        assert!(
            !risk_probe(&recovered)?,
            "a NOT-READY authority denies new risk"
        );
        drop(recovered);
        for path in [&control, &decisions, &lock] {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }

    /// A FIX OrderQty change is an amendment of the authorization, not a new
    /// reading of the original permit. An increase borrows approval for
    /// exposure that was never decided; a decrease needs a new, explicit
    /// decision and permit identity. Neither is implicit in an execution report.
    #[test]
    fn recovery_rejects_unauthorised_order_quantity_amendments() -> TestResult {
        let control = temp_path("control-qty-amend");
        let decisions = temp_path("decisions-qty-amend");
        let lock = temp_path("lock-qty-amend");
        for path in [&control, &decisions, &lock] {
            let _ = std::fs::remove_file(path);
        }
        // Seed the original authorization directly so the fixture is about
        // amendment evidence alone, not the evaluation policy's gross cap.
        let mut decision = RiskDecisionRecord::new(
            intent(1, 10, PRICE, Side::Buy)?,
            DecisionOutcome::Accepted,
            1,
            10 * PRICE,
            1,
            0,
        );
        decision.seal(1, genesis_digest());
        let permit_id = decision.permit_id;
        let intent_id = decision.intent_id;
        let digest = permit_digest(permit_id, "risk-v1", decision.revision, &decision.intent);
        let decision_store = || -> Box<dyn DecisionStore + Send + Sync> {
            write_json_lines(&decisions, std::slice::from_ref(&decision))
                .expect("durable decision");
            Box::new(FileDecisionStore::new(&decisions))
        };
        let oms = |leaves: i64| {
            let mut record = OmsPermitRecord::new(
                permit_id,
                intent_id,
                digest.clone(),
                OmsPermitState::Open,
                1,
                PRICE,
            );
            record = record.with_leaves_quantity(leaves);
            record.seal(1, genesis_digest());
            record
        };
        let exec = |id: &str, order_quantity: i64, leaves: i64| {
            ExecutionRecoveryRecord::new(
                permit_id,
                intent_id,
                id.to_owned(),
                1,
                ExecutionEventKind::Fill,
                None,
                order_quantity,
                1,
                leaves,
                PRICE,
                0,
                0,
            )
        };

        let (c1, _) = file_stores(&control, &decisions);
        let rejected = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms(11)], // Original OrderQty 10, CumQty 1, LeavesQty 11
                executions: seal_executions(vec![exec("exec-increase", 12, 11)]),
                account: account_delta(1)?,
                ..Default::default()
            },
            c1,
            decision_store(),
        );
        assert!(
            rejected.err().is_some_and(inconsistent),
            "OrderQty 12 above the original authorization of 10 is explicitly rejected"
        );

        let (c2, _) = file_stores(&control, &decisions);
        let rejected = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms(8)],
                executions: seal_executions(vec![exec("exec-reduce", 9, 8)]),
                account: account_delta(1)?,
                ..Default::default()
            },
            c2,
            decision_store(),
        );
        assert!(
            rejected.err().is_some_and(inconsistent),
            "a reduced OrderQty without a new decision and permit identity is rejected"
        );
        for path in [&control, &decisions, &lock] {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }

    /// A trade cancel removes one execution from the effective set. All four
    /// cumulative facts must agree with the remaining executions, and a later
    /// correction may not rewrite an execution that no longer exists.
    #[test]
    fn recovery_validates_trade_cancel_money_and_late_corrections() -> TestResult {
        let control = temp_path("control-trade-cancel");
        let decisions = temp_path("decisions-trade-cancel");
        let lock = temp_path("lock-trade-cancel");
        for path in [&control, &decisions, &lock] {
            let _ = std::fs::remove_file(path);
        }
        let (target, reservation) = consumed_permit(&control, &decisions, &lock, 4)?;
        let digest = digest_for(&target, &reservation);
        let fill1 = exec_record(
            &reservation,
            "exec-1",
            1,
            ExecutionEventKind::Fill,
            None,
            1,
            3,
            PRICE,
            10,
            5,
        );
        let fill2 = exec_record(
            &reservation,
            "exec-2",
            2,
            ExecutionEventKind::Fill,
            None,
            2,
            2,
            2 * PRICE,
            25,
            12,
        );
        let cancel = exec_record(
            &reservation,
            "exec-cancel",
            3,
            ExecutionEventKind::TradeCancel,
            Some("exec-2"),
            1,
            3,
            PRICE,
            10,
            5,
        );
        let oms = || {
            oms_record(
                &reservation,
                digest.clone(),
                OmsPermitState::Open,
                1,
                PRICE,
                Some(3),
            )
        };

        let (c1, d1) = file_stores(&control, &decisions);
        let valid = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms()],
                executions: seal_executions(vec![fill1.clone(), fill2.clone(), cancel.clone()]),
                account: account_delta(1)?,
                ..Default::default()
            },
            c1,
            d1,
        )?;
        let exposure = valid
            .snapshot()?
            .executions
            .get(&reservation.intent_id)
            .cloned()
            .expect("the remaining execution stays exposure");
        assert_eq!(
            exposure.filled_quantity, 1,
            "a cancelled execution leaves only the remaining fill"
        );
        assert_eq!(
            exposure.fees_paise,
            Paise(10),
            "the remaining execution keeps its own fees"
        );
        assert_eq!(
            exposure.costs_paise,
            Paise(5),
            "the remaining execution keeps its own costs"
        );
        assert_eq!(
            exposure.execution_ids,
            vec!["exec-1".to_owned()],
            "the cancelled execution identity is excluded"
        );
        drop(valid);

        let mut bad_fees = cancel.clone();
        bad_fees.fees_paise = 11;
        let (c2, d2) = file_stores(&control, &decisions);
        let rejected = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms()],
                executions: seal_executions(vec![fill1.clone(), fill2.clone(), bad_fees]),
                account: account_delta(1)?,
                ..Default::default()
            },
            c2,
            d2,
        );
        assert!(
            rejected.err().is_some_and(inconsistent),
            "a cancel contradicting the remaining fees fails closed"
        );

        let mut bad_costs = cancel.clone();
        bad_costs.costs_paise = 6;
        let (c3, d3) = file_stores(&control, &decisions);
        let rejected = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms()],
                executions: seal_executions(vec![fill1.clone(), fill2.clone(), bad_costs]),
                account: account_delta(1)?,
                ..Default::default()
            },
            c3,
            d3,
        );
        assert!(
            rejected.err().is_some_and(inconsistent),
            "a cancel contradicting the remaining costs fails closed"
        );

        let late_correction = exec_record(
            &reservation,
            "exec-late",
            4,
            ExecutionEventKind::Correct,
            Some("exec-2"),
            1,
            3,
            PRICE,
            10,
            5,
        );
        let (c4, d4) = file_stores(&control, &decisions);
        let rejected = build_recovered(
            &lock,
            FixtureRecords {
                oms: vec![oms()],
                executions: seal_executions(vec![fill1, fill2, cancel, late_correction]),
                account: account_delta(1)?,
                ..Default::default()
            },
            c4,
            d4,
        );
        assert!(
            rejected.err().is_some_and(inconsistent),
            "a correction cannot rewrite an execution already cancelled"
        );
        for path in [&control, &decisions, &lock] {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }
}

/// Recording a rejected decision mutates authority state, so it is versioned
/// once and an idempotent replay of that decision is a no-op.
#[test]
fn risk_rejection_is_a_versioned_transition() -> TestResult {
    let authority = authority()?;
    let before = authority.revision()?;
    let oversized = intent(1, 11, PRICE, Side::Buy)?;
    let error = authority
        .evaluate_and_reserve(oversized.clone(), observation(&authority)?, None)
        .expect_err("an oversized intent is refused by policy");
    assert!(
        rejected_check(&error, RiskCheck::QuantityLimit),
        "the quantity limit is the binding check"
    );
    assert_eq!(
        authority.revision()?,
        before + 1,
        "a recorded rejection advances revision exactly once"
    );
    let replay = authority.evaluate_and_reserve(oversized, observation(&authority)?, None);
    assert!(
        matches!(replay, Err(AuthorityError::PreviouslyRejected(_))),
        "the rejection is remembered"
    );
    assert_eq!(
        authority.revision()?,
        before + 1,
        "replaying a rejection does not mutate state"
    );
    Ok(())
}

/// A missing daily P&L must fail closed: zero is not an authoritative fact.
#[test]
fn missing_daily_pnl_is_not_silent_zero() -> TestResult {
    let authority = RiskAuthority::new(
        AccountId::new(ACCOUNT)?,
        policy()?,
        vec![instrument()?],
        date()?,
    );
    authority.set_session_open(true)?;
    authority.set_snapshot_available(true)?;
    release_switch(&authority)?;
    clear_reconciliation(&authority)?;
    let result = authority.evaluate_and_reserve(
        intent(1, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    );
    assert!(
        result.is_err(),
        "an authority with no authoritative daily P&L must deny new risk"
    );
    Ok(())
}

/// Below, at and above the configured daily-loss limit are distinct,
/// falsifiable outcomes of the owner-derived P&L snapshot.
#[test]
fn daily_loss_limits_below_at_and_above() -> TestResult {
    for (loss, accepted) in [(-4_999_i64, true), (-5_000, false), (-5_001, false)] {
        let authority = bare_authority()?;
        let calendar = daily_calendar()?;
        let accounts = daily_accounts(loss, 0, 0, 0)?;
        let record = sealed_daily_record(
            &accounts,
            &[],
            &calendar,
            DailyPnlBasis::RealizedPlusUnrealized,
            loss,
            0,
            0,
            0,
            0,
        )?;
        publish_daily(&authority, record, &accounts, &[], &calendar)?;
        let result = authority.evaluate_and_reserve(
            intent(1, 2, PRICE, Side::Buy)?,
            observation(&authority)?,
            None,
        );
        if accepted {
            assert!(
                result.is_ok(),
                "a loss of {loss} is inside the daily-loss limit"
            );
        } else {
            let error = result.expect_err("a loss at or beyond the limit is rejected");
            assert!(
                rejected_check(&error, RiskCheck::DailyLossLimit),
                "a loss of {loss} is rejected by the daily-loss limit"
            );
        }
        assert_eq!(
            authority.snapshot()?.daily_pnl_limit_paise,
            Some(Paise(loss)),
            "the exact authoritative P&L reaches the limit check"
        );
    }
    Ok(())
}

/// The daily-limit basis is an explicit record field. With a -4_000 realized
/// and -1_000 unrealized loss, realized+unrealized is at the -5_000 limit and
/// is rejected, while realized-only remains inside and is accepted.
#[test]
fn daily_limit_basis_is_explicit_and_testable_at_both_boundaries() -> TestResult {
    for (basis, accepted) in [
        (DailyPnlBasis::RealizedPlusUnrealized, false),
        (DailyPnlBasis::RealizedOnly, true),
    ] {
        let authority = authority()?;
        let reservation = authority.evaluate_and_reserve(
            intent(1, 2, PRICE, Side::Buy)?,
            observation(&authority)?,
            None,
        )?;
        authority.apply_fill(reservation.intent_id)?;
        let positions = authority.snapshot()?.positions;

        let calendar = daily_calendar()?;
        let accounts = daily_accounts(-4_000, 0, 0, 0)?;
        let mut valuation =
            ValuationRecord::new(AccountId::new(ACCOUNT)?, instrument_id()?, 2, 500, 1_000, 0);
        valuation.seal(1, genesis_digest());
        let valuations = vec![valuation];
        let record = sealed_daily_record_for_positions(
            &positions,
            &accounts,
            &valuations,
            &calendar,
            basis,
            -4_000,
            -1_000,
            0,
            0,
            0,
        )?;
        publish_daily(&authority, record, &accounts, &valuations, &calendar)?;
        assert_eq!(
            authority.snapshot()?.daily_pnl_basis,
            Some(basis),
            "the selected basis is visible in authority evidence"
        );
        let result = authority.evaluate_and_reserve(
            intent(2, 1, PRICE, Side::Buy)?,
            observation(&authority)?,
            None,
        );
        if accepted {
            assert!(result.is_ok(), "realized-only ignores the open mark loss");
        } else {
            let error = result.expect_err("the open mark loss consumes the limit");
            assert!(
                rejected_check(&error, RiskCheck::DailyLossLimit),
                "realized+unrealized rejects at the limit"
            );
        }
    }
    Ok(())
}

/// Revised valuations must move the limit: a mark cut turns an accepted
/// realized-only loss into an at-limit rejection under the chosen basis.
#[test]
fn revised_valuations_change_the_daily_limit() -> TestResult {
    let authority = authority()?;
    let reservation = authority.evaluate_and_reserve(
        intent(1, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    )?;
    authority.apply_fill(reservation.intent_id)?;
    let positions = authority.snapshot()?.positions;
    let calendar = daily_calendar()?;
    let accounts = daily_accounts(-4_000, 0, 0, 0)?;

    let mut initial = ValuationRecord::new(
        AccountId::new(ACCOUNT)?,
        instrument_id()?,
        2,
        1_000,
        1_000,
        0,
    );
    initial.seal(1, genesis_digest());
    let record = sealed_daily_record_for_positions(
        &positions,
        &accounts,
        &[initial.clone()],
        &calendar,
        DailyPnlBasis::RealizedPlusUnrealized,
        -4_000,
        0,
        0,
        0,
        0,
    )?;
    publish_daily(&authority, record, &accounts, &[initial], &calendar)?;
    assert!(
        authority
            .evaluate_and_reserve(
                intent(2, 1, PRICE, Side::Buy)?,
                observation(&authority)?,
                None,
            )
            .is_ok(),
        "an unchanged mark keeps the loss inside the limit"
    );

    let mut revised =
        ValuationRecord::new(AccountId::new(ACCOUNT)?, instrument_id()?, 2, 500, 1_000, 0);
    revised.seal(1, genesis_digest());
    let record = sealed_daily_record_for_positions(
        &positions,
        &accounts,
        &[revised.clone()],
        &calendar,
        DailyPnlBasis::RealizedPlusUnrealized,
        -4_000,
        -1_000,
        0,
        0,
        0,
    )?;
    publish_daily(&authority, record, &accounts, &[revised], &calendar)?;
    let error = authority
        .evaluate_and_reserve(
            intent(3, 1, PRICE, Side::Buy)?,
            observation(&authority)?,
            None,
        )
        .expect_err("the revised mark reaches the daily-loss limit");
    assert!(
        rejected_check(&error, RiskCheck::DailyLossLimit),
        "revised valuation inputs move the daily-loss decision"
    );
    Ok(())
}

/// Corrected fills are a new sealed journal head and must change the limit
/// outcome without any caller assertion about P&L.
#[test]
fn corrected_fills_change_the_daily_limit() -> TestResult {
    let authority = bare_authority()?;
    let calendar = daily_calendar()?;
    let accounts = daily_accounts(-4_000, 0, 0, 0)?;
    let record = sealed_daily_record(
        &accounts,
        &[],
        &calendar,
        DailyPnlBasis::RealizedPlusUnrealized,
        -4_000,
        0,
        0,
        0,
        0,
    )?;
    publish_daily(&authority, record, &accounts, &[], &calendar)?;
    assert!(
        authority
            .evaluate_and_reserve(
                intent(1, 2, PRICE, Side::Buy)?,
                observation(&authority)?,
                None,
            )
            .is_ok(),
        "the first fill set is inside the limit"
    );

    let corrected = daily_accounts(-5_000, 0, 0, 0)?;
    let record = sealed_daily_record(
        &corrected,
        &[],
        &calendar,
        DailyPnlBasis::RealizedPlusUnrealized,
        -5_000,
        0,
        0,
        0,
        0,
    )?;
    publish_daily(&authority, record, &corrected, &[], &calendar)?;
    let error = authority
        .evaluate_and_reserve(
            intent(2, 2, PRICE, Side::Buy)?,
            observation(&authority)?,
            None,
        )
        .expect_err("the corrected fill set reaches the limit");
    assert!(
        rejected_check(&error, RiskCheck::DailyLossLimit),
        "corrected journal facts change the accepted decision"
    );
    Ok(())
}

/// A P&L older than the authority-owned journal freshness window denies risk.
#[test]
fn stale_daily_pnl_denies_new_risk() -> TestResult {
    let authority = authority()?;
    let now = LogicalNanos::new(1_000_000_001);
    authority.set_now(now)?;
    let error = authority
        .evaluate_and_reserve(
            intent(1, 2, PRICE, Side::Buy)?,
            observations_for_feed(
                &authority,
                FeedObservation {
                    received_at: now,
                    ..feed()?
                },
            )?,
            None,
        )
        .expect_err("stale P&L is a typed denial");
    assert!(
        matches!(&error, AuthorityError::ControlFailure(message) if message.contains("stale")),
        "the denial names stale P&L rather than treating it as zero"
    );
    Ok(())
}

/// A new business date invalidates both the boundary and the snapshot's date.
#[test]
fn new_business_date_invalidates_daily_pnl() -> TestResult {
    let authority = authority()?;
    let next_date = TradeDate::new(2026, 10, 29)?;
    let next_calendar = DailyPnlCalendar::new(
        "Asia/Kolkata".to_owned(),
        next_date,
        86_400_000_000_000,
        172_800_000_000_000,
    );
    let accounts = daily_accounts(0, 0, 0, 0)?;
    let mut record = DailyPnlRecord::new(
        AccountId::new(ACCOUNT)?,
        next_date,
        "INR".to_owned(),
        DailyPnlBasis::RealizedPlusUnrealized,
        &next_calendar,
        86_400_000_000_000,
    )
    .with_journal(1, accounts[0].digest.clone())
    .with_digests(
        daily_pnl_positions_digest(&BTreeMap::new()),
        genesis_digest().to_owned(),
    );
    record.seal(1, genesis_digest());
    let wrong_date = authority.publish_daily_pnl(
        authority.register_all_owners()?.account_journal,
        record,
        &accounts,
        &[],
        &next_calendar,
    );
    assert!(
        matches!(wrong_date, Err(DailyPnlError::WrongBusinessDate)),
        "a snapshot for the next business date cannot be bound to this authority"
    );

    let now = LogicalNanos::new(86_400_000_000_000);
    authority.set_now(now)?;
    let error = authority
        .evaluate_and_reserve(
            intent(1, 2, PRICE, Side::Buy)?,
            observations_for_feed(
                &authority,
                FeedObservation {
                    received_at: now,
                    ..feed()?
                },
            )?,
            None,
        )
        .expect_err("the previous day's snapshot is outside the new boundary");
    assert!(
        matches!(&error, AuthorityError::ControlFailure(message) if message.contains("business-day boundary")),
        "a new business day is an explicit denial"
    );
    Ok(())
}

/// The account ledger head and sequence bound to the snapshot must match the
/// fresh owner observation and the sealed journal records.
#[test]
fn account_ledger_digest_mismatch_denies_new_risk() -> TestResult {
    let authority = authority()?;
    let mut observations = observation(&authority)?;
    observations
        .account_journal
        .as_mut()
        .expect("journal")
        .payload
        .head_hash_hex = "e".repeat(64);
    let error = authority
        .evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, observations, None)
        .expect_err("a different ledger head is a typed denial");
    assert!(
        matches!(&error, AuthorityError::ControlFailure(message) if message.contains("journal head or sequence")),
        "the account ledger mismatch is explicit"
    );

    let calendar = daily_calendar()?;
    let accounts = daily_accounts(0, 0, 0, 0)?;
    let mut record = sealed_daily_record(
        &accounts,
        &[],
        &calendar,
        DailyPnlBasis::RealizedPlusUnrealized,
        0,
        0,
        0,
        0,
        0,
    )?;
    record = record.with_journal(1, "e".repeat(64));
    record.seal(1, genesis_digest());
    let mismatch = authority.publish_daily_pnl(
        authority.register_all_owners()?.account_journal,
        record,
        &accounts,
        &[],
        &calendar,
    );
    assert!(
        matches!(mismatch, Err(DailyPnlError::WrongJournal(_))),
        "publication rejects a digest that is not the sealed journal head"
    );
    Ok(())
}

/// A snapshot and its records must belong to this account, and publication
/// must be authenticated by the registered account-journal owner.
#[test]
fn wrong_account_binding_and_unauthenticated_publication_are_denied() -> TestResult {
    let authority = authority()?;
    let calendar = daily_calendar()?;
    let mut account = AccountRecoveryRecord::new(
        "foreign".to_owned(),
        AccountId::new(999)?,
        instrument_id()?,
        0,
    );
    account.seal(1, genesis_digest());
    let mut record = DailyPnlRecord::new(
        AccountId::new(999)?,
        date()?,
        "INR".to_owned(),
        DailyPnlBasis::RealizedPlusUnrealized,
        &calendar,
        0,
    )
    .with_journal(1, account.digest.clone())
    .with_digests(
        daily_pnl_positions_digest(&BTreeMap::new()),
        genesis_digest().to_owned(),
    );
    record.seal(1, genesis_digest());
    let foreign = authority.publish_daily_pnl(
        authority.register_all_owners()?.account_journal,
        record,
        std::slice::from_ref(&account),
        &[],
        &calendar,
    );
    assert!(
        matches!(foreign, Err(DailyPnlError::WrongAccount)),
        "another account's P&L cannot be installed here"
    );

    let accounts = daily_accounts(0, 0, 0, 0)?;
    let record = sealed_daily_record(
        &accounts,
        &[],
        &calendar,
        DailyPnlBasis::RealizedPlusUnrealized,
        0,
        0,
        0,
        0,
        0,
    )?;
    let unauthenticated = authority.publish_daily_pnl(
        authority.register_all_owners()?.ops,
        record,
        &accounts,
        &[],
        &calendar,
    );
    assert!(
        matches!(unauthenticated, Err(DailyPnlError::Unauthorised)),
        "a non-journal owner token cannot publish P&L"
    );
    Ok(())
}

/// Reconciliation is a precondition of P&L evidence, not an implied status.
#[test]
fn daily_pnl_requires_matched_reconciliation() -> TestResult {
    let authority = authority()?;
    let calendar = daily_calendar()?;
    let accounts = daily_accounts(0, 0, 0, 0)?;
    let mut record = sealed_daily_record(
        &accounts,
        &[],
        &calendar,
        DailyPnlBasis::RealizedPlusUnrealized,
        0,
        0,
        0,
        0,
        0,
    )?;
    record = record.with_reconciliation_status(ReconciliationStatus::Unavailable);
    record.seal(1, genesis_digest());
    let unmatched = authority.publish_daily_pnl(
        authority.register_all_owners()?.account_journal,
        record,
        &accounts,
        &[],
        &calendar,
    );
    assert!(
        matches!(unmatched, Err(DailyPnlError::WrongReconciliation)),
        "an unmatched P&L record is rejected at publication"
    );

    let mut observations = observation(&authority)?;
    observations
        .reconciliation
        .as_mut()
        .expect("reconciliation")
        .payload
        .report
        .status = ReconciliationStatus::Unavailable;
    let error = authority
        .evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, observations, None)
        .expect_err("a changed reconciliation status is a typed denial");
    assert!(
        matches!(&error, AuthorityError::ReconciliationMismatch { .. })
            || matches!(&error, AuthorityError::ControlFailure(message) if message.contains("reconciliation")),
        "risk evaluation never uses P&L with unmatched reconciliation"
    );
    Ok(())
}

/// The unowned public P&L setter must not exist in production code.
#[test]
fn no_unauthenticated_daily_pnl_setter() {
    let source = include_str!("../src/authority.rs");
    assert!(
        !source.contains("pub fn set_daily_pnl"),
        "a caller cannot assert a P&L figure directly"
    );
}

// ---------------------------------------------------------------------------
// P&L hardening fixtures: two business dates, exact valuation coverage,
// per-instrument mark freshness and an explicitly neutral currency unit.
// ---------------------------------------------------------------------------

const SECOND_ID: &str = "nse.cash.synth2";
const NEXT_DAY_START: i64 = 86_400_000_000_000;

fn second_instrument_id() -> Result<InstrumentId, Box<dyn std::error::Error>> {
    Ok(InstrumentId::new(SECOND_ID)?)
}

fn second_instrument() -> Result<Instrument, Box<dyn std::error::Error>> {
    let mut value = instrument()?;
    value.instrument_id = second_instrument_id()?;
    if let Some(listing) = value.listings.first_mut() {
        listing.venue_symbol = "SYN2".to_owned();
    }
    Ok(value)
}

fn policy_for(
    ids: BTreeSet<InstrumentId>,
) -> Result<ValidatedRiskPolicy, Box<dyn std::error::Error>> {
    let raw = RiskPolicy {
        version: "risk-v1-pnl-scope".to_owned(),
        allowed_accounts: BTreeSet::from([AccountId::new(ACCOUNT)?]),
        allowed_strategies: BTreeSet::from([StrategyId::new(STRATEGY)?]),
        allowed_algos: BTreeSet::from([AlgoId::new(ALGO)?]),
        allowed_instruments: ids,
        max_order_quantity: Quantity::new(10)?,
        max_order_notional_paise: Paise(10_000),
        max_position_quantity: Quantity::new(100)?,
        max_gross_exposure_paise: Paise(MAX_GROSS),
        max_daily_loss_paise: Paise(5_000),
        max_open_orders: 5,
        max_orders_per_second: 5,
        max_price_deviation_ppm: 100_000,
        max_quote_age_ns: 1_000,
        allow_short: false,
        max_strategy_exposure_paise: Paise(1_000_000),
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

fn two_instrument_authority() -> Result<RiskAuthority, Box<dyn std::error::Error>> {
    let ids = BTreeSet::from([instrument_id()?, second_instrument_id()?]);
    let authority = RiskAuthority::new(
        AccountId::new(ACCOUNT)?,
        policy_for(ids)?,
        vec![instrument()?, second_instrument()?],
        date()?,
    );
    authority.set_session_open(true)?;
    authority.set_snapshot_available(true)?;
    release_switch(&authority)?;
    clear_reconciliation(&authority)?;
    publish_zero_daily(&authority)?;
    Ok(authority)
}

fn intent_for(
    id: InstrumentId,
    n: u64,
    q: i64,
    side: Side,
) -> Result<OrderIntent, Box<dyn std::error::Error>> {
    let mut value = intent(n, q, PRICE, side)?;
    value.instrument_id = id;
    Ok(value)
}

fn second_feed() -> Result<FeedObservation, Box<dyn std::error::Error>> {
    let mut quote = quote()?;
    quote.instrument_id = second_instrument_id()?;
    Ok(FeedObservation {
        quote: Some(quote),
        ..feed()?
    })
}

fn fill_two_positions(
    authority: &RiskAuthority,
) -> Result<BTreeMap<InstrumentId, i64>, Box<dyn std::error::Error>> {
    let first = authority.evaluate_and_reserve(
        intent(1, 2, PRICE, Side::Buy)?,
        observation(authority)?,
        None,
    )?;
    authority.apply_fill(first.intent_id)?;
    let second = authority.evaluate_and_reserve(
        intent_for(second_instrument_id()?, 2, 3, Side::Buy)?,
        observations_for_feed(authority, second_feed()?)?,
        None,
    )?;
    authority.apply_fill(second.intent_id)?;
    Ok(authority.snapshot()?.positions)
}

fn next_day_calendar() -> Result<DailyPnlCalendar, Box<dyn std::error::Error>> {
    Ok(DailyPnlCalendar::new(
        "Asia/Kolkata".to_owned(),
        TradeDate::new(2026, 10, 29)?,
        NEXT_DAY_START,
        172_800_000_000_000,
    ))
}

/// One independent sealed money effect per business date in a continuous
/// account journal.
fn two_day_accounts() -> Result<Vec<AccountRecoveryRecord>, Box<dyn std::error::Error>> {
    let mut day1 = AccountRecoveryRecord::new(
        "day-one".to_owned(),
        AccountId::new(ACCOUNT)?,
        instrument_id()?,
        0,
    )
    .with_event_at_ns(0)
    .with_pnl_effects(1_000, 50, 10, 5);
    day1.seal(1, genesis_digest());
    let mut day2 = AccountRecoveryRecord::new(
        "day-two".to_owned(),
        AccountId::new(ACCOUNT)?,
        instrument_id()?,
        0,
    )
    .with_event_at_ns(NEXT_DAY_START)
    .with_pnl_effects(2_000, 70, 20, 6);
    day2.seal(2, &day1.digest);
    Ok(vec![day1, day2])
}

/// H1: a daily result must contain only the declared business period's effects,
/// even when a continuous journal already holds the next day's records.
#[test]
fn daily_pnl_depends_only_on_the_declared_business_period() -> TestResult {
    let calendar = daily_calendar()?;
    let next = next_day_calendar()?;
    let accounts = two_day_accounts()?;
    let empty_positions = BTreeMap::new();

    let mut day1 = DailyPnlRecord::new(
        AccountId::new(ACCOUNT)?,
        date()?,
        "INR".to_owned(),
        DailyPnlBasis::RealizedPlusUnrealized,
        &calendar,
        0,
    )
    .with_journal(2, accounts[1].digest.clone())
    .with_digests(
        daily_pnl_positions_digest(&empty_positions),
        genesis_digest().to_owned(),
    )
    .with_components(1_000, 0, 50, 10, 5);
    day1.seal(1, genesis_digest());
    let authority = bare_authority()?;
    let first = authority.publish_daily_pnl(
        authority.register_all_owners()?.account_journal,
        day1.clone(),
        &accounts,
        &[],
        &calendar,
    )?;
    assert_eq!(
        (
            first.realized_paise,
            first.fees_paise,
            first.funding_paise,
            first.fx_paise
        ),
        (1_000, 50, 10, 5),
        "day one uses only its own journal effects"
    );

    // Re-deriving day one from the same journal must be a pure function of the
    // declared period and its opening checkpoint.
    let scoped = bare_authority()?;
    let first_again = scoped.publish_daily_pnl(
        scoped.register_all_owners()?.account_journal,
        day1,
        &accounts,
        &[],
        &calendar,
    )?;
    assert_eq!(first, first_again, "later-day records do not move day one");

    let next_authority = RiskAuthority::new(
        AccountId::new(ACCOUNT)?,
        policy()?,
        vec![instrument()?],
        next.business_date,
    );
    next_authority.set_now(LogicalNanos::new(u64::try_from(NEXT_DAY_START)?))?;
    let mut day2 = DailyPnlRecord::new(
        AccountId::new(ACCOUNT)?,
        next.business_date,
        "INR".to_owned(),
        DailyPnlBasis::RealizedPlusUnrealized,
        &next,
        NEXT_DAY_START,
    )
    .with_journal(2, accounts[1].digest.clone())
    .with_opening_checkpoint(accounts[0].digest.clone())
    .with_digests(
        daily_pnl_positions_digest(&empty_positions),
        genesis_digest().to_owned(),
    )
    .with_components(2_000, 0, 70, 20, 6);
    day2.seal(1, genesis_digest());
    let second = next_authority.publish_daily_pnl(
        next_authority.register_all_owners()?.account_journal,
        day2,
        &accounts,
        &[],
        &next,
    )?;
    assert_eq!(
        (
            second.realized_paise,
            second.fees_paise,
            second.funding_paise,
            second.fx_paise
        ),
        (2_000, 70, 20, 6),
        "day two uses only its own journal effects"
    );
    Ok(())
}

/// H2: a non-zero position with no valuation must fail closed at publication.
#[test]
fn non_zero_position_without_a_valuation_is_rejected() -> TestResult {
    let authority = two_instrument_authority()?;
    let reservation = authority.evaluate_and_reserve(
        intent(1, 2, PRICE, Side::Buy)?,
        observation(&authority)?,
        None,
    )?;
    authority.apply_fill(reservation.intent_id)?;
    let positions = authority.snapshot()?.positions;
    let calendar = daily_calendar()?;
    let accounts = daily_accounts(0, 0, 0, 0)?;
    let record = sealed_daily_record_for_positions(
        &positions,
        &accounts,
        &[],
        &calendar,
        DailyPnlBasis::RealizedPlusUnrealized,
        0,
        0,
        0,
        0,
        0,
    )?;
    let error = authority
        .publish_daily_pnl(
            authority.register_all_owners()?.account_journal,
            record,
            &accounts,
            &[],
            &calendar,
        )
        .expect_err("an unmarked position cannot establish a daily P&L");
    assert!(
        matches!(error, DailyPnlError::WrongValuation(_)),
        "the denial names the valuation coverage failure"
    );
    Ok(())
}

/// H2: exact valuation coverage across a multi-instrument account. A missing
/// instrument and a foreign mark both fail closed.
#[test]
fn valuation_coverage_must_be_exact() -> TestResult {
    let authority = two_instrument_authority()?;
    let positions = fill_two_positions(&authority)?;
    let calendar = daily_calendar()?;
    let accounts = daily_accounts(0, 0, 0, 0)?;
    let token = authority.register_all_owners()?.account_journal;

    let mut first = ValuationRecord::new(
        AccountId::new(ACCOUNT)?,
        instrument_id()?,
        2,
        1_000,
        1_000,
        0,
    );
    first.seal(1, genesis_digest());

    let missing = sealed_daily_record_for_positions(
        &positions,
        &accounts,
        &[first.clone()],
        &calendar,
        DailyPnlBasis::RealizedPlusUnrealized,
        0,
        0,
        0,
        0,
        0,
    )?;
    let error = authority
        .publish_daily_pnl(token, missing, &accounts, &[first.clone()], &calendar)
        .expect_err("a missing second instrument is not covered");
    assert!(
        matches!(error, DailyPnlError::WrongValuation(_)),
        "the missing mark is a typed denial"
    );

    let mut second = ValuationRecord::new(
        AccountId::new(ACCOUNT)?,
        second_instrument_id()?,
        3,
        1_000,
        1_000,
        0,
    );
    second.seal(2, &first.digest);
    let mut foreign = ValuationRecord::new(
        AccountId::new(ACCOUNT)?,
        InstrumentId::new("nse.cash.foreign")?,
        0,
        2_000,
        1_000,
        0,
    );
    foreign.seal(3, &second.digest);
    let marked = [first.clone(), second, foreign.clone()];
    let with_foreign = sealed_daily_record_for_positions(
        &positions,
        &accounts,
        &marked,
        &calendar,
        DailyPnlBasis::RealizedPlusUnrealized,
        0,
        0,
        0,
        0,
        0,
    )?;
    let error = authority
        .publish_daily_pnl(token, with_foreign, &accounts, &marked, &calendar)
        .expect_err("a mark for an instrument with no position is foreign");
    assert!(
        matches!(error, DailyPnlError::WrongValuation(_)),
        "the foreign mark is a typed denial"
    );
    Ok(())
}

/// H3: every retained per-instrument mark must stay fresh, not only the newest.
#[test]
fn one_stale_valuation_denies_new_risk_while_another_is_fresh() -> TestResult {
    let authority = two_instrument_authority()?;
    let positions = fill_two_positions(&authority)?;
    let calendar = daily_calendar()?;
    let accounts = daily_accounts(0, 0, 0, 0)?;
    let fresh_ns: i64 = 900_000_000;
    authority.set_now(LogicalNanos::new(u64::try_from(fresh_ns)?))?;

    let mut older = ValuationRecord::new(
        AccountId::new(ACCOUNT)?,
        instrument_id()?,
        2,
        1_000,
        1_000,
        0,
    );
    older.seal(1, genesis_digest());
    let mut newer = ValuationRecord::new(
        AccountId::new(ACCOUNT)?,
        second_instrument_id()?,
        3,
        1_000,
        1_000,
        fresh_ns,
    );
    newer.seal(2, &older.digest);
    let valuations = [older, newer];
    let mut record = DailyPnlRecord::new(
        AccountId::new(ACCOUNT)?,
        date()?,
        "INR".to_owned(),
        DailyPnlBasis::RealizedPlusUnrealized,
        &calendar,
        fresh_ns,
    )
    .with_journal(
        u64::try_from(accounts.len())?,
        accounts.last().expect("account").digest.clone(),
    )
    .with_digests(
        daily_pnl_positions_digest(&positions),
        valuations.last().expect("valuation").digest.clone(),
    );
    record.seal(1, genesis_digest());
    publish_daily(&authority, record, &accounts, &valuations, &calendar)?;

    let now = LogicalNanos::new(1_500_000_000);
    authority.set_now(now)?;
    let error = authority
        .evaluate_and_reserve(
            intent(3, 1, PRICE, Side::Buy)?,
            observations_for_feed(
                &authority,
                FeedObservation {
                    received_at: now,
                    ..feed()?
                },
            )?,
            None,
        )
        .expect_err("an older stale mark denies risk even though the newest is fresh");
    assert!(
        matches!(&error, AuthorityError::ControlFailure(message) if message.contains("stale")),
        "the denial names the stale valuation"
    );
    Ok(())
}

/// H4: money is Paise; no neutral unit exists, so only INR is admissible.
#[test]
fn unsupported_base_currency_is_rejected() -> TestResult {
    let authority = authority()?;
    let calendar = daily_calendar()?;
    let accounts = daily_accounts(0, 0, 0, 0)?;
    let mut record = sealed_daily_record(
        &accounts,
        &[],
        &calendar,
        DailyPnlBasis::RealizedPlusUnrealized,
        0,
        0,
        0,
        0,
        0,
    )?;
    record.base_currency = "USD".to_owned();
    record.seal(1, genesis_digest());
    let error = authority
        .publish_daily_pnl(
            authority.register_all_owners()?.account_journal,
            record,
            &accounts,
            &[],
            &calendar,
        )
        .expect_err("a non-INR base currency cannot be interpreted as Paise");
    assert!(
        matches!(&error, DailyPnlError::Contradictory(message) if message.contains("currency")),
        "the denial is explicit about the unsupported unit"
    );
    Ok(())
}
