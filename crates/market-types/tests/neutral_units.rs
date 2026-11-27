// SPDX-License-Identifier: Apache-2.0
//! Neutral financial units: public-API contract tests.
//!
//! Fixtures are neutral integers only; no market-shaped price sequences.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test assertions only

use market_types::units::{
    Charge, ChargeKind, ContractMultiplier, Currency, DimensionError, FxDirection, FxRate,
    InstrumentSpec, InstrumentSpecId, InstrumentSpecRef, InstrumentSpecRegistry, Money,
    NumericError, Price, Quantity, RateSource, Rational, RoundingError, RoundingRule, Scale,
    SettlementMetadata, SettlementRegime, SpecError, TickSize, Unit, UnitsError,
};

#[test]
fn inr_paise_scale_is_two_decimal_places_without_a_named_minor_type() {
    let amount = Money::new(12_345, Currency::INR, Scale::TWO);
    assert_eq!(
        amount.units(),
        12_345,
        "paise-like coefficient is kept exactly"
    );
    assert_eq!(
        amount.currency(),
        Currency::INR,
        "currency is carried by the value"
    );
    assert_eq!(amount.scale(), Scale::TWO, "scale is carried by the value");
    assert_eq!(
        amount.to_decimal_string(),
        "123.45 INR",
        "two decimals render without a venue-specific type"
    );
    assert!(
        amount.rescale_exact(Scale::ZERO).is_err(),
        "a 2-decimal amount is not 0-decimal exactly"
    );
    let whole = Money::new(123, Currency::INR, Scale::ZERO)
        .rescale_exact(Scale::TWO)
        .unwrap();
    assert_eq!(whole.units(), 12_300, "widening an exact scale is lossless");
}

#[test]
fn non_decimal_quantity_units_convert_only_exactly() {
    let dozen = Unit::new("dozen").unwrap();
    let item = Unit::new("item").unwrap();
    let items = Quantity::new(2, dozen.clone())
        .unwrap()
        .convert_exact(item.clone(), Rational::new(12, 1).unwrap())
        .unwrap();
    assert_eq!(items.units(), 24, "2 dozens is 24 items");
    assert_eq!(items.unit(), &item, "target unit is retained");

    let inexact = Quantity::new(1, item.clone())
        .unwrap()
        .convert_exact(dozen.clone(), Rational::new(1, 12).unwrap());
    assert!(
        matches!(
            inexact,
            Err(UnitsError::Numeric(NumericError::Rounding(
                RoundingError { .. }
            )))
        ),
        "a fractional dozen is an explicit rounding error"
    );

    let mismatch = items.checked_add(Quantity::new(1, dozen).unwrap());
    assert!(
        matches!(mismatch, Err(UnitsError::Dimension(_))),
        "quantity units are checked on arithmetic"
    );
}

#[test]
fn rational_contract_multiplier_is_exact_and_dimension_checked() {
    let point = Unit::new("index_point").unwrap();
    let contract = Unit::new("contract").unwrap();
    let multiplier = ContractMultiplier::new(
        Rational::new(50, 1).unwrap(),
        point.clone(),
        contract.clone(),
    )
    .unwrap();
    let price = Price::new(10_000, Currency::INR, point, Scale::TWO).unwrap();
    let quantity = Quantity::new(2, contract).unwrap();
    let notional = multiplier.checked_notional(price, quantity).unwrap();
    assert_eq!(notional.units(), 1_000_000, "100 * 2 * 50 at scale 2");
    assert_eq!(
        notional.currency(),
        Currency::INR,
        "quote currency flows through"
    );
    assert_eq!(notional.scale(), Scale::TWO, "money scale is explicit");

    let share = Unit::new("share").unwrap();
    let fractional =
        ContractMultiplier::new(Rational::new(2, 3).unwrap(), share.clone(), share.clone())
            .unwrap();
    let unit_price = Price::new(10, Currency::USD, share.clone(), Scale::ZERO).unwrap();
    let three = Quantity::new(3, share.clone()).unwrap();
    let exact = fractional
        .checked_notional(unit_price.clone(), three)
        .unwrap();
    assert_eq!(exact.units(), 20, "10 * 3 * 2/3 is exact");

    let one = Quantity::new(1, share.clone()).unwrap();
    assert!(
        matches!(
            fractional.checked_notional(unit_price.clone(), one.clone()),
            Err(UnitsError::Numeric(NumericError::Rounding(_)))
        ),
        "10 * 1 * 2/3 is not exact"
    );
    let (rounded, receipt) = fractional
        .checked_notional_with_rounding(unit_price.clone(), one, RoundingRule::Floor)
        .unwrap();
    assert_eq!(rounded.units(), 6, "explicit floor rounds 20/3 down");
    assert!(
        receipt.was_rounded(),
        "a receipt records the explicit rounding"
    );

    let other = Unit::new("lot").unwrap();
    let wrong_quantity = Quantity::new(3, other).unwrap();
    assert!(
        matches!(
            fractional.checked_notional(unit_price, wrong_quantity),
            Err(UnitsError::Dimension(DimensionError::Unit { .. }))
        ),
        "a mismatched quantity unit is rejected"
    );
}

#[test]
fn tick_boundaries_and_minimum_increment_are_exact() {
    let tick = TickSize::new(5).unwrap();
    assert!(tick.is_aligned(100), "100 is on a 5-unit grid");
    assert!(tick.is_aligned(-105), "negative multiples are on the grid");
    assert!(!tick.is_aligned(101), "101 is off grid");

    let (up, receipt) = tick.checked_snap(101, RoundingRule::Ceil).unwrap();
    assert_eq!(up, 105, "ceil snaps to the next tick");
    assert!(receipt.was_rounded(), "snapping records its remainder");
    let (down, _) = tick.checked_snap(-101, RoundingRule::Floor).unwrap();
    assert_eq!(down, -105, "floor snaps toward negative infinity");
    assert!(
        matches!(
            tick.checked_snap(101, RoundingRule::Exact),
            Err(UnitsError::Numeric(NumericError::Rounding(_)))
        ),
        "exact snapping rejects off-grid values"
    );
    assert!(
        TickSize::new(0).is_err(),
        "minimum increment must be positive"
    );
}

#[test]
fn checked_arithmetic_and_rational_normalization_reject_overflow() {
    let max = Money::new(i128::MAX, Currency::INR, Scale::ZERO);
    let one = Money::new(1, Currency::INR, Scale::ZERO);
    assert!(
        matches!(
            max.checked_add(one),
            Err(UnitsError::Numeric(NumericError::Overflow))
        ),
        "money addition is checked"
    );
    assert!(
        matches!(
            max.rescale_exact(Scale::TWO),
            Err(UnitsError::Numeric(NumericError::Overflow))
        ),
        "rescaling is checked"
    );
    let normalized = Rational::new(i128::MAX, i128::MAX).unwrap();
    assert_eq!(normalized.numerator(), 1, "rationals are normalized");
    assert_eq!(normalized.denominator(), 1, "rationals are normalized");
    assert!(Rational::new(1, 0).is_err(), "zero denominator is rejected");
    let unit = Unit::new("item").unwrap();
    let overflow = Quantity::new(i128::MAX, unit.clone())
        .unwrap()
        .checked_add(Quantity::new(1, unit).unwrap());
    assert!(
        matches!(overflow, Err(UnitsError::Numeric(NumericError::Overflow))),
        "quantity addition is checked"
    );
}

#[test]
fn rounding_rules_are_deterministic_at_ties() {
    let tick = TickSize::new(2).unwrap();
    let (even, even_receipt) = tick.checked_snap(5, RoundingRule::HalfEven).unwrap();
    assert_eq!(even, 4, "half-even rounds a positive tie down");
    assert!(even_receipt.was_rounded(), "a tie is disclosed as rounding");
    let (away, _) = tick
        .checked_snap(5, RoundingRule::HalfAwayFromZero)
        .unwrap();
    assert_eq!(away, 6, "half-away rounds a positive tie up");
    let (negative, _) = tick.checked_snap(-5, RoundingRule::HalfEven).unwrap();
    assert_eq!(negative, -4, "half-even is symmetric about zero");
}

#[test]
fn deserialization_cannot_bypass_typed_invariants() {
    assert!(
        serde_json::from_str::<Scale>("19").is_err(),
        "an unsupported scale is rejected on input"
    );
    assert!(
        serde_json::from_str::<Currency>("\"inr\"").is_err(),
        "currency codes must be uppercase"
    );
    assert!(
        serde_json::from_str::<Rational>("[1,0]").is_err(),
        "a zero denominator is rejected on input"
    );
    assert!(
        serde_json::from_str::<TickSize>("0").is_err(),
        "a zero tick is rejected on input"
    );
    let money: Money =
        serde_json::from_str("{\"units\":100,\"currency\":\"INR\",\"scale\":2}").unwrap();
    assert_eq!(
        money.to_decimal_string(),
        "1 INR",
        "valid typed data round-trips"
    );
}

fn spec_base(id: u64) -> InstrumentSpec {
    let unit = Unit::new("contract").unwrap();
    let settlement = SettlementMetadata::new(
        SettlementRegime::Cash,
        Currency::INR,
        3_000,
        "neutral-reference",
    )
    .unwrap();
    InstrumentSpec::builder()
        .id(InstrumentSpecId::new(id))
        .effective_window(1_000, Some(2_000))
        .quote(Currency::INR)
        .price_unit(unit.clone())
        .quantity_unit(unit.clone())
        .price_scale(Scale::TWO)
        .money_scale(Scale::TWO)
        .tick_size(TickSize::new(5).unwrap())
        .multiplier(
            ContractMultiplier::new(Rational::new(1, 1).unwrap(), unit.clone(), unit).unwrap(),
        )
        .fees(vec![
            Charge::new(
                ChargeKind::Fee,
                Money::new(100, Currency::INR, Scale::TWO),
                "setup",
            )
            .unwrap(),
        ])
        .taxes(vec![
            Charge::new(
                ChargeKind::Tax,
                Money::new(50, Currency::INR, Scale::TWO),
                "levy",
            )
            .unwrap(),
        ])
        .funding(Some(
            Charge::new(
                ChargeKind::Funding,
                Money::new(25, Currency::INR, Scale::TWO),
                "carry",
            )
            .unwrap(),
        ))
        .settlement(settlement)
        .build()
        .unwrap()
}

#[test]
fn effective_dated_spec_id_resolves_by_version_and_time() {
    let first = spec_base(7);
    let second = {
        let unit = Unit::new("contract").unwrap();
        InstrumentSpec::builder()
            .id(InstrumentSpecId::new(7))
            .effective_window(2_000, None)
            .quote(Currency::INR)
            .price_unit(unit.clone())
            .quantity_unit(unit.clone())
            .price_scale(Scale::TWO)
            .money_scale(Scale::TWO)
            .tick_size(TickSize::new(10).unwrap())
            .multiplier(
                ContractMultiplier::new(Rational::new(1, 1).unwrap(), unit.clone(), unit).unwrap(),
            )
            .settlement(
                SettlementMetadata::new(
                    SettlementRegime::Physical,
                    Currency::INR,
                    4_000,
                    "neutral-reference",
                )
                .unwrap(),
            )
            .build()
            .unwrap()
    };
    assert_eq!(first.fees().len(), 1, "fee metadata is carried");
    assert_eq!(first.taxes().len(), 1, "tax metadata is carried");
    assert!(first.funding().is_some(), "funding metadata is carried");
    assert_eq!(
        first.settlement().regime(),
        SettlementRegime::Cash,
        "settlement metadata is carried"
    );

    let mut registry = InstrumentSpecRegistry::new();
    registry.insert(first).unwrap();
    registry.insert(second).unwrap();

    let old = InstrumentSpecRef::new(InstrumentSpecId::new(7), 1_999);
    assert_eq!(
        old.resolve(&registry).unwrap().tick_size().units(),
        5,
        "v1 is active before 2000"
    );
    let current = InstrumentSpecRef::new(InstrumentSpecId::new(7), 2_000);
    assert_eq!(
        current.resolve(&registry).unwrap().tick_size().units(),
        10,
        "v2 is active at 2000"
    );
    assert!(
        matches!(
            InstrumentSpecRef::new(InstrumentSpecId::new(7), 999).resolve(&registry),
            Err(UnitsError::Spec(SpecError::NotEffective))
        ),
        "resolution outside all windows is explicit"
    );
    let overlap = spec_base(7);
    assert!(
        matches!(
            registry.insert(overlap),
            Err(UnitsError::Spec(SpecError::Overlap))
        ),
        "effective windows must not overlap"
    );
}

#[test]
fn spec_rejects_cross_currency_charges() {
    let unit = Unit::new("contract").unwrap();
    let result = InstrumentSpec::builder()
        .id(InstrumentSpecId::new(9))
        .effective_window(1, None)
        .quote(Currency::INR)
        .price_unit(unit.clone())
        .quantity_unit(unit.clone())
        .price_scale(Scale::ZERO)
        .money_scale(Scale::TWO)
        .tick_size(TickSize::new(1).unwrap())
        .multiplier(
            ContractMultiplier::new(Rational::new(1, 1).unwrap(), unit.clone(), unit).unwrap(),
        )
        .fees(vec![
            Charge::new(
                ChargeKind::Fee,
                Money::new(1, Currency::USD, Scale::ZERO),
                "foreign",
            )
            .unwrap(),
        ])
        .settlement(
            SettlementMetadata::new(SettlementRegime::Cash, Currency::INR, 2, "ref").unwrap(),
        )
        .build();
    assert!(
        matches!(
            result,
            Err(UnitsError::Dimension(DimensionError::Currency { .. }))
        ),
        "a fee in a different currency is not silently mixed in"
    );
}

#[test]
fn fx_conversion_requires_provenance_direction_and_explicit_rounding() {
    let source = RateSource::new("neutral-oracle", "series-42").unwrap();
    let observed_at_us = 1_700_000_000_000_000;
    let rate = FxRate::new(
        Currency::INR,
        Currency::USD,
        Rational::new(1, 83).unwrap(),
        source.clone(),
        observed_at_us,
    )
    .unwrap();
    let inr = Money::new(8_300, Currency::INR, Scale::TWO);
    let converted = inr
        .convert(
            Currency::USD,
            Scale::TWO,
            &rate,
            FxDirection::BaseToQuote,
            RoundingRule::Exact,
        )
        .unwrap();
    assert_eq!(converted.amount().units(), 100, "83 INR is 1.00 USD");
    assert_eq!(
        converted.amount().currency(),
        Currency::USD,
        "target currency"
    );
    assert_eq!(
        converted.rate().source(),
        &source,
        "rate source is preserved"
    );
    assert_eq!(
        converted.rate().observed_at_us(),
        observed_at_us,
        "rate timestamp is preserved"
    );
    assert_eq!(
        converted.direction(),
        FxDirection::BaseToQuote,
        "direction is preserved"
    );
    assert!(
        !converted.receipt().was_rounded(),
        "exact conversion is recorded"
    );

    let one_rupee = Money::new(1, Currency::INR, Scale::ZERO);
    assert!(
        matches!(
            one_rupee.convert(
                Currency::USD,
                Scale::TWO,
                &rate,
                FxDirection::BaseToQuote,
                RoundingRule::Exact,
            ),
            Err(UnitsError::Numeric(NumericError::Rounding(_)))
        ),
        "an inexact FX result is an explicit rounding error"
    );
    let floored = one_rupee
        .convert(
            Currency::USD,
            Scale::TWO,
            &rate,
            FxDirection::BaseToQuote,
            RoundingRule::Floor,
        )
        .unwrap();
    assert_eq!(floored.amount().units(), 1, "explicit floor keeps one cent");
    assert!(
        floored.receipt().was_rounded(),
        "explicit rounding is disclosed"
    );

    let usd = Money::new(100, Currency::USD, Scale::TWO);
    let back = usd
        .convert(
            Currency::INR,
            Scale::TWO,
            &rate,
            FxDirection::QuoteToBase,
            RoundingRule::Exact,
        )
        .unwrap();
    assert_eq!(
        back.amount().units(),
        8_300,
        "reverse direction uses reciprocal"
    );

    assert!(
        matches!(
            inr.convert(
                Currency::USD,
                Scale::TWO,
                &rate,
                FxDirection::QuoteToBase,
                RoundingRule::Exact,
            ),
            Err(UnitsError::Direction { .. })
        ),
        "wrong direction is rejected"
    );
    assert!(
        matches!(
            inr.checked_add(Money::new(100, Currency::USD, Scale::TWO)),
            Err(UnitsError::Dimension(DimensionError::Currency { .. }))
        ),
        "cross-currency addition is rejected without an FX rate"
    );
}
