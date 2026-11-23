//! Risk-limit boundary and feed-health contract tests.

use std::collections::BTreeSet;

use risk_engine::{
    RiskContext, //
    RiskPolicy,
    ValidatedRiskPolicy,
    evaluate_risk,
};
use trading_core::{
    AccountId, AlgoId, Continuity, DomainError, FeedHealth, Instrument, InstrumentId,
    InstrumentType, IntentId, JournalRef, Listing, ListingStatus, ListingTerms, LogicalNanos,
    OrderIntent, Paise, PricePaise, Product, Quantity, QuoteSnapshot, RiskAssessment, RiskCheck,
    Segment, SettlementRegime, Side, StrategyId, TickAssignment, TradeDate, Venue,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn date() -> Result<TradeDate, DomainError> {
    TradeDate::new(2026, 10, 28)
        .ok()
        .ok_or_else(|| DomainError::InvalidEventTime("invalid date".to_owned()))
}

fn instrument_id() -> Result<InstrumentId, DomainError> {
    InstrumentId::new("nse.cash.synidx")
        .map_err(|error| DomainError::InvalidInstrument(error.to_string()))
}

fn instrument() -> Result<Instrument, DomainError> {
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
            effective_from: TradeDate::new(2026, 1, 1)
                .ok()
                .ok_or_else(|| DomainError::InvalidEventTime("invalid date".to_owned()))?,
            effective_to: Some(
                TradeDate::new(2026, 12, 31)
                    .ok()
                    .ok_or_else(|| DomainError::InvalidEventTime("invalid date".to_owned()))?,
            ),
            status: ListingStatus::Active,
            settlement_regime: SettlementRegime::Standard,
            z_category: false,
            settlement_group: None,
            terms: vec![ListingTerms {
                effective_from: TradeDate::new(2026, 1, 1)
                    .ok()
                    .ok_or_else(|| DomainError::InvalidEventTime("invalid date".to_owned()))?,
                effective_to: Some(
                    TradeDate::new(2026, 12, 31)
                        .ok()
                        .ok_or_else(|| DomainError::InvalidEventTime("invalid date".to_owned()))?,
                ),
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

fn validated_policy() -> Result<ValidatedRiskPolicy, DomainError> {
    ValidatedRiskPolicy::from_policy(&policy()?)
}

fn policy() -> Result<RiskPolicy, DomainError> {
    Ok(RiskPolicy {
        version: "risk-v1".to_owned(),
        allowed_accounts: BTreeSet::from([AccountId::new(1)?]),
        allowed_strategies: BTreeSet::from([StrategyId::new(2)?]),
        allowed_algos: BTreeSet::from([AlgoId::new(3)?]),
        allowed_instruments: BTreeSet::from([instrument_id()?]),
        max_order_quantity: Quantity::new(10)?,
        max_order_notional_paise: Paise(100_000),
        max_position_quantity: Quantity::new(10)?,
        max_gross_exposure_paise: Paise(100_000),
        max_daily_loss_paise: Paise(5_000),
        max_open_orders: 5,
        max_orders_per_second: 5,
        max_price_deviation_ppm: 100_000,
        max_quote_age_ns: 1_000,
        allow_short: false,
        max_strategy_exposure_paise: Paise(100_000),
        max_sector_exposure_paise: None,
        stress_scenario: None,
        max_stress_loss_paise: None,
        max_participation_ppm: None,
        greek_limits: None,
        max_borrow_quantity: None,
        max_financing_cost_paise: None,
    })
}

fn intent(quantity: i64, price: i64, side: Side) -> Result<OrderIntent, DomainError> {
    Ok(OrderIntent {
        intent_id: IntentId::new(1)?,
        account_id: AccountId::new(1)?,
        strategy_id: StrategyId::new(2)?,
        strategy_version: "strategy-v1".to_owned(),
        algo_id: AlgoId::new(3)?,
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
        expires_at: LogicalNanos::new(100),
    })
}

fn quote(bid: i64, ask: i64) -> Result<QuoteSnapshot, DomainError> {
    Ok(QuoteSnapshot {
        instrument_id: instrument_id()?,
        venue: Venue::Nse,
        bid_price: Some(PricePaise::new(Paise(bid))?),
        bid_quantity: Quantity::new(1)?,
        ask_price: Some(PricePaise::new(Paise(ask))?),
        ask_quantity: Quantity::new(1)?,
        continuity: Continuity::Snapshot,
    })
}

fn context() -> Result<RiskContext, DomainError> {
    Ok(RiskContext {
        account_id: AccountId::new(1)?,
        strategy_id: StrategyId::new(2)?,
        venue: Venue::Nse,
        instrument: Some(instrument()?),
        quote: Some(quote(995, 1_005)?),
        feed_health: FeedHealth::Healthy { age_ns: 100 },
        trade_date: date()?,
        now: LogicalNanos::new(50),
        position_quantity: 0,
        pending_buy_quantity: Quantity::ZERO,
        pending_sell_quantity: Quantity::ZERO,
        gross_exposure_paise: Paise(0),
        pending_gross_exposure_paise: Paise(0),
        strategy_exposure_paise: Paise(0),
        sector_label: None,
        sector_exposure_paise: Paise(0),
        positions: Default::default(),
        marks_paise: Default::default(),
        depth: None,
        greeks: None,
        is_derivative: false,
        borrow: None,
        daily_pnl: Paise(0),
        open_orders: 0,
        orders_last_second: 0,
        session_open: true,
        account_snapshot_available: true,
        kill_switch_engaged: false,
        reconciliation_blocked: false,
        duplicate_intent: false,
    })
}

fn passed(assessment: &RiskAssessment, check: RiskCheck) -> bool {
    assessment
        .checks
        .iter()
        .find(|result| result.check == check)
        .is_some_and(|result| result.passed)
}

#[test]
fn baseline_intent_passes_every_check() -> TestResult {
    let assessment = evaluate_risk(
        &intent(1, 1_000, Side::Buy)?,
        &validated_policy()?,
        &context()?,
    );
    assert!(assessment.accepted);
    assert!(assessment.checks.iter().all(|check| check.passed));
    assert_eq!(assessment.policy_version, "risk-v1");
    Ok(())
}

#[test]
fn quantity_and_notional_boundaries_are_inclusive() -> TestResult {
    let policy = validated_policy()?;
    let base = context()?;

    let at_limit = evaluate_risk(&intent(10, 1_000, Side::Buy)?, &policy, &base);
    assert!(passed(&at_limit, RiskCheck::QuantityLimit));
    let above = evaluate_risk(&intent(11, 1_000, Side::Buy)?, &policy, &base);
    assert!(!passed(&above, RiskCheck::QuantityLimit));

    // 10,000 paise * 10 lots = 100,000 paise, exactly the notional limit.
    let mut near_reference = context()?;
    near_reference.quote = Some(quote(9_995, 10_005)?);
    let notional_at_limit =
        evaluate_risk(&intent(10, 10_000, Side::Buy)?, &policy, &near_reference);
    assert!(passed(&notional_at_limit, RiskCheck::OrderNotional));
    assert!(passed(&notional_at_limit, RiskCheck::PriceCollar));

    let notional_above = evaluate_risk(&intent(10, 10_005, Side::Buy)?, &policy, &near_reference);
    assert!(!passed(&notional_above, RiskCheck::OrderNotional));
    assert!(passed(&notional_above, RiskCheck::PriceCollar));
    Ok(())
}

#[test]
fn position_limit_boundaries_apply_pending_orders_and_short_policy() -> TestResult {
    let policy = validated_policy()?;

    let mut at_limit = context()?;
    at_limit.position_quantity = 6;
    at_limit.pending_buy_quantity = Quantity::new(2)?;
    assert!(passed(
        &evaluate_risk(&intent(2, 1_000, Side::Buy)?, &policy, &at_limit),
        RiskCheck::PositionLimit
    ));

    let mut above = context()?;
    above.position_quantity = 6;
    above.pending_buy_quantity = Quantity::new(2)?;
    assert!(!passed(
        &evaluate_risk(&intent(3, 1_000, Side::Buy)?, &policy, &above),
        RiskCheck::PositionLimit
    ));

    let mut no_short = context()?;
    no_short.position_quantity = 0;
    assert!(!passed(
        &evaluate_risk(&intent(1, 1_000, Side::Sell)?, &policy, &no_short),
        RiskCheck::PositionLimit
    ));

    // Flipping allow_short requires re-validation; the validated policy
    // cannot be mutated into a different policy in place.
    let mut raw = (*policy).clone();
    raw.allow_short = true;
    let short_allowed = ValidatedRiskPolicy::from_policy(&raw)?;
    assert!(passed(
        &evaluate_risk(&intent(1, 1_000, Side::Sell)?, &short_allowed, &no_short),
        RiskCheck::PositionLimit
    ));
    Ok(())
}

#[test]
fn daily_loss_at_the_limit_blocks_new_risk() -> TestResult {
    let policy = validated_policy()?;
    let intent = intent(1, 1_000, Side::Buy)?;

    let mut at_limit = context()?;
    at_limit.daily_pnl = Paise(-5_000);
    assert!(!passed(
        &evaluate_risk(&intent, &policy, &at_limit),
        RiskCheck::DailyLossLimit
    ));

    let mut inside = context()?;
    inside.daily_pnl = Paise(-4_999);
    assert!(passed(
        &evaluate_risk(&intent, &policy, &inside),
        RiskCheck::DailyLossLimit
    ));
    Ok(())
}

#[test]
fn kill_switch_duplicate_intent_and_expiry_block_submission() -> TestResult {
    let policy = validated_policy()?;
    let intent = intent(1, 1_000, Side::Buy)?;

    let mut killed = context()?;
    killed.kill_switch_engaged = true;
    assert!(!passed(
        &evaluate_risk(&intent, &policy, &killed),
        RiskCheck::KillSwitch
    ));

    let mut duplicate = context()?;
    duplicate.duplicate_intent = true;
    assert!(!passed(
        &evaluate_risk(&intent, &policy, &duplicate),
        RiskCheck::Idempotency
    ));

    let mut expired_context = context()?;
    expired_context.now = LogicalNanos::new(100);
    assert!(!passed(
        &evaluate_risk(&intent, &policy, &expired_context),
        RiskCheck::IntentExpiry
    ));
    Ok(())
}

#[test]
fn policy_validation_rejects_empty_and_zero_limits() -> TestResult {
    let mut value = policy()?;
    assert!(value.validate().is_ok());

    value.max_order_quantity = Quantity::ZERO;
    assert!(value.validate().is_err());

    value = policy()?;
    value.allowed_accounts.clear();
    assert!(value.validate().is_err());

    value = policy()?;
    value.max_price_deviation_ppm = 1_000_001;
    assert!(value.validate().is_err());

    let encoded = serde_json::to_string(&policy()?)?;
    let decoded: RiskPolicy = serde_json::from_str(&encoded)?;
    assert_eq!(decoded, policy()?);
    Ok(())
}

#[test]
fn invalid_policies_are_unrepresentable_for_evaluation() -> TestResult {
    // A zero-limit policy (invalid per RiskPolicy::validate) cannot be
    // wrapped: evaluation is only reachable through the checked
    // constructor, so there is no call path for an invalid policy.
    let mut invalid = policy()?;
    invalid.max_daily_loss_paise = Paise(0);
    assert!(ValidatedRiskPolicy::from_policy(&invalid).is_err());
    let mut empty = policy()?;
    empty.allowed_instruments.clear();
    assert!(ValidatedRiskPolicy::from_policy(&empty).is_err());
    Ok(())
}

#[test]
fn negative_gross_exposure_is_rejected_not_netting() -> TestResult {
    // The P0 counterexample: a negative current exposure must never
    // understate the projected total (current -100k + order 100k = 0
    // previously "fit" a 100k limit).
    let counter = context()?;
    let mut counter = counter;
    counter.gross_exposure_paise = Paise(-100_000);
    let order = intent(10, 10_000, Side::Buy)?;
    let assessment = evaluate_risk(&order, &validated_policy()?, &counter);
    assert!(!assessment.accepted);
    assert!(!passed(&assessment, RiskCheck::GrossExposure));
    Ok(())
}

#[test]
fn negative_pending_exposure_is_rejected() -> TestResult {
    let mut counter = context()?;
    counter.pending_gross_exposure_paise = Paise(-100_000);
    let order = intent(10, 10_000, Side::Buy)?;
    let assessment = evaluate_risk(&order, &validated_policy()?, &counter);
    assert!(!assessment.accepted);
    assert!(!passed(&assessment, RiskCheck::GrossExposure));
    Ok(())
}

#[test]
fn invalid_context_detail_names_the_failure() -> TestResult {
    let mut counter = context()?;
    counter.gross_exposure_paise = Paise(i64::MIN / 2);
    let detail = evaluate_risk(
        &intent(1, 1_000, Side::Buy)?,
        &validated_policy()?,
        &counter,
    )
    .checks
    .iter()
    .find(|result| result.check == RiskCheck::GrossExposure)
    .map(|result| result.detail.clone())
    .unwrap_or_default();
    assert!(
        detail.contains("non-negative"),
        "unexpected detail: {detail}"
    );
    Ok(())
}

#[test]
fn exposure_monotonicity_holds() -> TestResult {
    // Increasing the current gross exposure must never turn a rejection
    // into an acceptance (projected = current + pending + order).
    let base = context()?;
    let policy = validated_policy()?;
    let order = intent(1, 1_000, Side::Buy)?;
    let mut rejected_seen = false;
    for exposure in [10_000, 50_000, 90_000, 99_000, 100_000, 101_000, 150_000] {
        let mut counter = base.clone();
        counter.gross_exposure_paise = Paise(exposure);
        let assessment = evaluate_risk(&order, &policy, &counter);
        if rejected_seen {
            assert!(
                !assessment.accepted,
                "monotonicity violated at exposure {exposure}"
            );
        } else if !assessment.accepted && !passed(&assessment, RiskCheck::GrossExposure) {
            rejected_seen = true;
        }
    }
    Ok(())
}

#[test]
fn pending_exposure_cannot_reduce_projected_total() -> TestResult {
    let base = context()?;
    let policy = validated_policy()?;
    let order = intent(1, 1_000, Side::Buy)?;
    let mut low = base.clone();
    low.pending_gross_exposure_paise = Paise(0);
    let mut high = base.clone();
    high.pending_gross_exposure_paise = Paise(99_000);
    let low_result = evaluate_risk(&order, &policy, &low);
    let high_result = evaluate_risk(&order, &policy, &high);
    if low_result.accepted {
        assert!(
            high_result.accepted == (99_999 <= 100_000),
            "pending exposure must monotonically tighten the result"
        );
    }
    Ok(())
}

#[test]
fn overflowing_notional_never_produces_acceptance() -> TestResult {
    let counter = context()?;
    let big_order = intent(1_i64, i64::MAX, Side::Buy)?;
    let assessment = evaluate_risk(&big_order, &validated_policy()?, &counter);
    assert!(!assessment.accepted);
    assert!(!passed(&assessment, RiskCheck::OrderNotional));
    assert!(!passed(&assessment, RiskCheck::GrossExposure));
    Ok(())
}

#[test]
fn exact_collar_uses_unrounded_midpoint() -> TestResult {
    // Odd midpoint 100.5 (bid 100 + ask 101): with a one-ppm collar no
    // integer limit can be within tolerance, so every order is rejected
    // rather than silently rounded to a whole-paise reference.
    let mut tight = policy()?;
    tight.max_price_deviation_ppm = 1;
    let policy = ValidatedRiskPolicy::from_policy(&tight)?;
    let mut tight_context = context()?;
    tight_context.quote = Some(quote(100, 101)?);
    assert!(!passed(
        &evaluate_risk(&intent(1, 100, Side::Buy)?, &policy, &tight_context),
        RiskCheck::PriceCollar
    ));
    assert!(!passed(
        &evaluate_risk(&intent(1, 101, Side::Buy)?, &policy, &tight_context),
        RiskCheck::PriceCollar
    ));
    Ok(())
}

#[test]
fn collar_boundary_is_inclusive_and_overflow_safe() -> TestResult {
    let policy = validated_policy()?;
    // Midpoint exactly 1000 (bid 995 + ask 1005); 10% collar: 950 = 0.9 * M
    // exactly at the boundary, 949 is one paise outside.
    // Midpoint exactly 1000 (bid 995 + ask 1005); 10% collar on M₂=2000:
    // |2p − 2000|·10⁶ ≤ 2000·100000; limit 900 gives 200·10⁶ = exactly
    // 2000·100000, so it is accepted at the boundary; 899 is outside.
    let mut boundary = context()?;
    boundary.quote = Some(quote(995, 1_005)?);
    assert!(passed(
        &evaluate_risk(&intent(1, 900, Side::Buy)?, &policy, &boundary),
        RiskCheck::PriceCollar
    ));
    assert!(!passed(
        &evaluate_risk(&intent(1, 899, Side::Buy)?, &policy, &boundary),
        RiskCheck::PriceCollar
    ));
    // Extreme paise values: arithmetic must not overflow or false-accept.
    let mut huge = context()?;
    huge.quote = Some(quote(i64::MAX, 1)?);
    let assessment = evaluate_risk(&intent(1, i64::MAX, Side::Buy)?, &policy, &huge);
    assert!(!assessment.accepted);
    assert!(!passed(&assessment, RiskCheck::PriceCollar));
    Ok(())
}

#[test]
fn strategy_exposure_aggregates_two_intents_under_the_order_cap() -> TestResult {
    let mut raw = policy()?;
    raw.max_strategy_exposure_paise = Paise(9_000);
    let policy = ValidatedRiskPolicy::from_policy(&raw)?;
    // Each intent alone is 5,000 paise, well under the 100,000 order cap.
    let first = evaluate_risk(&intent(5, 1_000, Side::Buy)?, &policy, &context()?);
    assert!(
        passed(&first, RiskCheck::StrategyExposure),
        "first intent passes"
    );

    let mut second_context = context()?;
    second_context.strategy_exposure_paise = Paise(5_000);
    let second = evaluate_risk(&intent(5, 1_000, Side::Buy)?, &policy, &second_context);
    assert!(
        !passed(&second, RiskCheck::StrategyExposure),
        "10,000 paise projected strategy notional exceeds the 9,000 cap"
    );
    assert!(!second.accepted, "the over-cap strategy intent is denied");
    Ok(())
}
