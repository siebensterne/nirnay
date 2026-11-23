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
    },
    thread,
};

use risk_authority::{
    AccountJournalObservation, //
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

fn authority() -> Result<RiskAuthority, Box<dyn std::error::Error>> {
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
    Ok(observations_with(feed()?, authority.register_all_owners()?))
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
        reservation.snapshot_revision, 1,
        "the permit records its state revision"
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
        observations_with(gapped, authority.register_all_owners()?),
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
        let mut observations = observations_with(fresh.clone(), authority.register_all_owners()?);
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
    authority.set_now(LogicalNanos::new(2_000))?;
    let barrier = Arc::new(Barrier::new(2));
    let mut fresh = feed()?;
    fresh.received_at = LogicalNanos::new(2_000);
    let mut handles = Vec::new();
    for (id, stale) in [(1_u64, false), (2_u64, true)] {
        let authority = Arc::clone(&authority);
        let barrier = Arc::clone(&barrier);
        let mut observations = observations_with(fresh.clone(), authority.register_all_owners()?);
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
