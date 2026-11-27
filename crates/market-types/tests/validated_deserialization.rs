// SPDX-License-Identifier: Apache-2.0
//! Adversarial deserialization checks for invariant-bearing financial units.
//!
//! Fixtures are neutral integers and labels only; no market-shaped data.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test assertions only

use market_types::units::{
    Charge, ContractMultiplier, Currency, FxDirection, FxRate, InstrumentSpec, InstrumentSpecId,
    InstrumentSpecRef, InstrumentSpecRegistry, Money, Price, Quantity, RateSource, Rational,
    RoundingRule, Scale, SettlementMetadata, TickSize, Unit, UnitsError,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

fn rejected<T: DeserializeOwned>(value: Value, context: &str) {
    assert!(
        serde_json::from_value::<T>(value).is_err(),
        "{context} must be rejected on deserialization"
    );
}

fn money_json(units: i64, currency: &str, scale: u8) -> Value {
    serde_json::json!({"units": units, "currency": currency, "scale": scale})
}

fn charge_json(kind: &str, code: &str, currency: &str) -> Value {
    serde_json::json!({
        "kind": kind,
        "amount": money_json(100, currency, 2),
        "code": code,
    })
}

fn source_json(name: &str, reference: &str) -> Value {
    serde_json::json!({"name": name, "reference": reference})
}

fn settlement_json(regime: &str, currency: &str, at_us: i64, reference: &str) -> Value {
    serde_json::json!({
        "regime": regime,
        "currency": currency,
        "settlement_at_us": at_us,
        "reference": reference,
    })
}

fn multiplier_json(value: Value, price_unit: &str, quantity_unit: &str) -> Value {
    serde_json::json!({
        "value": value,
        "price_unit": price_unit,
        "quantity_unit": quantity_unit,
    })
}

fn fx_json(base: &str, quote: &str, rate: Value, source: Value, at_us: i64) -> Value {
    serde_json::json!({
        "base": base,
        "quote": quote,
        "quote_per_base": rate,
        "source": source,
        "observed_at_us": at_us,
    })
}

fn spec_json(
    id: u64,
    from_us: i64,
    to_us: Option<i64>,
    price_unit: &str,
    quantity_unit: &str,
    settlement_currency: &str,
) -> Value {
    serde_json::json!({
        "id": id,
        "effective_from_us": from_us,
        "effective_to_us": to_us,
        "quote": settlement_currency,
        "price_unit": price_unit,
        "quantity_unit": quantity_unit,
        "price_scale": 2,
        "money_scale": 2,
        "tick_size": 5,
        "multiplier": multiplier_json(serde_json::json!([1, 1]), price_unit, quantity_unit),
        "fees": [charge_json("fee", "setup", settlement_currency)],
        "taxes": [charge_json("tax", "levy", settlement_currency)],
        "funding": charge_json("funding", "carry", settlement_currency),
        "settlement": settlement_json("cash", settlement_currency, 3_000, "neutral-ref"),
    })
}

fn notional_from_wire(value: Value) -> Result<Money, UnitsError> {
    let quantity: Quantity =
        serde_json::from_value(value).map_err(|error| UnitsError::Invalid(error.to_string()))?;
    let unit = Unit::new("contract")?;
    let multiplier = ContractMultiplier::new(Rational::new(1, 1)?, unit.clone(), unit)?;
    let price = Price::new(10, Currency::INR, Unit::new("contract")?, Scale::ZERO)?;
    multiplier.checked_notional(price, quantity)
}

fn conversion_from_wire(value: Value) -> Result<Money, UnitsError> {
    let rate: FxRate =
        serde_json::from_value(value).map_err(|error| UnitsError::Invalid(error.to_string()))?;
    Money::new(83, Currency::INR, Scale::TWO)
        .convert(
            Currency::USD,
            Scale::TWO,
            &rate,
            FxDirection::BaseToQuote,
            RoundingRule::Exact,
        )
        .map(|conversion| conversion.amount())
}

fn resolve_from_wire(value: Value) -> Result<InstrumentSpec, UnitsError> {
    let registry: InstrumentSpecRegistry =
        serde_json::from_value(value).map_err(|error| UnitsError::Invalid(error.to_string()))?;
    InstrumentSpecRef::new(InstrumentSpecId::new(1), 1_750)
        .resolve(&registry)
        .cloned()
}

#[test]
fn quantity_deserialization_rejects_negative_units_and_keeps_wire_schema() {
    let valid = serde_json::json!({"units": 4, "unit": "item"});
    let parsed: Quantity = serde_json::from_value(valid.clone()).expect("valid quantity parses");
    assert_eq!(parsed.units(), 4, "units survive constructor validation");
    assert_eq!(
        parsed.unit(),
        &Unit::new("item").expect("valid unit"),
        "unit survives constructor validation"
    );
    assert_eq!(
        serde_json::to_value(&parsed).expect("quantity serializes"),
        valid,
        "the serialized schema is unchanged"
    );
    rejected::<Quantity>(
        serde_json::json!({"units": -1, "unit": "item"}),
        "negative quantity",
    );
    rejected::<Quantity>(
        serde_json::json!({"units": -40, "unit": "item"}),
        "large negative quantity",
    );
    assert!(
        Quantity::new(-1, Unit::new("item").expect("valid unit")).is_err(),
        "the public constructor also rejects negative quantities"
    );
}

#[test]
fn rejected_quantity_never_reaches_notional_or_accounting() {
    let outcome = notional_from_wire(serde_json::json!({"units": -3, "unit": "contract"}));
    assert!(
        outcome.is_err(),
        "a wire-injected negative quantity cannot produce a notional"
    );
    let valid = notional_from_wire(serde_json::json!({"units": 3, "unit": "contract"}));
    assert_eq!(
        valid.expect("valid notional").units(),
        30,
        "a validated quantity still reaches the notional path"
    );
}

#[test]
fn contract_multiplier_deserialization_rejects_non_positive_values() {
    let valid = multiplier_json(serde_json::json!([2, 3]), "item", "item");
    let parsed: ContractMultiplier =
        serde_json::from_value(valid.clone()).expect("positive multiplier parses");
    assert_eq!(
        parsed.value(),
        Rational::new(2, 3).expect("valid rational"),
        "the multiplier value survives validation"
    );
    assert_eq!(
        serde_json::to_value(&parsed).expect("multiplier serializes"),
        valid,
        "the serialized schema is unchanged"
    );
    rejected::<ContractMultiplier>(
        multiplier_json(serde_json::json!([0, 1]), "item", "item"),
        "zero multiplier",
    );
    rejected::<ContractMultiplier>(
        multiplier_json(serde_json::json!([-1, 1]), "item", "item"),
        "negative multiplier",
    );
    rejected::<ContractMultiplier>(
        multiplier_json(serde_json::json!([1, -1]), "item", "item"),
        "negative normalized multiplier",
    );
}

#[test]
fn rate_source_deserialization_rejects_blank_provenance() {
    let valid = source_json("neutral-oracle", "series-1");
    let parsed: RateSource = serde_json::from_value(valid.clone()).expect("valid source parses");
    assert_eq!(parsed.name(), "neutral-oracle", "name survives validation");
    assert_eq!(
        parsed.reference(),
        "series-1",
        "reference survives validation"
    );
    assert_eq!(
        serde_json::to_value(&parsed).expect("source serializes"),
        valid,
        "the serialized schema is unchanged"
    );
    rejected::<RateSource>(source_json("", "series-1"), "empty source name");
    rejected::<RateSource>(source_json("   ", "series-1"), "blank source name");
    rejected::<RateSource>(source_json("neutral-oracle", ""), "empty source reference");
    rejected::<RateSource>(
        source_json("neutral-oracle", "\t"),
        "blank source reference",
    );
}

#[test]
fn fx_rate_deserialization_rejects_every_constructor_violation() {
    let valid = fx_json(
        "INR",
        "USD",
        serde_json::json!([1, 83]),
        source_json("neutral-oracle", "series-1"),
        1_000,
    );
    let parsed: FxRate = serde_json::from_value(valid.clone()).expect("valid FX rate parses");
    assert_eq!(parsed.base(), Currency::INR, "base survives validation");
    assert_eq!(parsed.quote(), Currency::USD, "quote survives validation");
    assert_eq!(
        parsed.observed_at_us(),
        1_000,
        "timestamp survives validation"
    );
    assert_eq!(
        serde_json::to_value(&parsed).expect("FX rate serializes"),
        valid,
        "the serialized schema is unchanged"
    );

    rejected::<FxRate>(
        fx_json(
            "INR",
            "INR",
            serde_json::json!([1, 1]),
            source_json("neutral-oracle", "series-1"),
            1_000,
        ),
        "same-currency FX rate",
    );
    rejected::<FxRate>(
        fx_json(
            "INR",
            "USD",
            serde_json::json!([0, 1]),
            source_json("neutral-oracle", "series-1"),
            1_000,
        ),
        "zero FX rate",
    );
    rejected::<FxRate>(
        fx_json(
            "INR",
            "USD",
            serde_json::json!([-1, 83]),
            source_json("neutral-oracle", "series-1"),
            1_000,
        ),
        "negative FX rate",
    );
    rejected::<FxRate>(
        fx_json(
            "INR",
            "USD",
            serde_json::json!([1, 83]),
            source_json("neutral-oracle", "series-1"),
            0,
        ),
        "zero FX observation timestamp",
    );
    rejected::<FxRate>(
        fx_json(
            "INR",
            "USD",
            serde_json::json!([1, 83]),
            source_json("neutral-oracle", "series-1"),
            -5,
        ),
        "negative FX observation timestamp",
    );
    rejected::<FxRate>(
        fx_json(
            "INR",
            "USD",
            serde_json::json!([1, 83]),
            source_json("", "series-1"),
            1_000,
        ),
        "FX rate with empty source name",
    );
    rejected::<FxRate>(
        fx_json(
            "INR",
            "USD",
            serde_json::json!([1, 83]),
            source_json("neutral-oracle", ""),
            1_000,
        ),
        "FX rate with empty source reference",
    );
    assert!(
        conversion_from_wire(fx_json(
            "INR",
            "INR",
            serde_json::json!([1, 1]),
            source_json("neutral-oracle", "series-1"),
            1_000,
        ))
        .is_err(),
        "an invalid wire rate cannot reach FX conversion"
    );
    assert_eq!(
        conversion_from_wire(fx_json(
            "INR",
            "USD",
            serde_json::json!([1, 83]),
            source_json("neutral-oracle", "series-1"),
            1_000,
        ))
        .expect("valid conversion")
        .units(),
        1,
        "a validated wire rate still reaches FX conversion"
    );
}

#[test]
fn charge_deserialization_rejects_empty_code() {
    let valid = charge_json("fee", "setup", "INR");
    let parsed: Charge = serde_json::from_value(valid.clone()).expect("valid charge parses");
    assert_eq!(parsed.code(), "setup", "code survives validation");
    assert_eq!(
        serde_json::to_value(&parsed).expect("charge serializes"),
        valid,
        "the serialized schema is unchanged"
    );
    rejected::<Charge>(charge_json("fee", "", "INR"), "empty charge code");
    rejected::<Charge>(charge_json("tax", "  ", "INR"), "blank charge code");
}

#[test]
fn settlement_metadata_deserialization_rejects_invalid_fields() {
    let valid = settlement_json("cash", "INR", 3_000, "neutral-ref");
    let parsed: SettlementMetadata =
        serde_json::from_value(valid.clone()).expect("valid settlement parses");
    assert_eq!(
        parsed.settlement_at_us(),
        3_000,
        "timestamp survives validation"
    );
    assert_eq!(
        parsed.reference(),
        "neutral-ref",
        "reference survives validation"
    );
    assert_eq!(
        serde_json::to_value(&parsed).expect("settlement serializes"),
        valid,
        "the serialized schema is unchanged"
    );
    rejected::<SettlementMetadata>(
        settlement_json("cash", "INR", 0, "neutral-ref"),
        "zero settlement timestamp",
    );
    rejected::<SettlementMetadata>(
        settlement_json("cash", "INR", -1, "neutral-ref"),
        "negative settlement timestamp",
    );
    rejected::<SettlementMetadata>(
        settlement_json("cash", "INR", 3_000, ""),
        "empty settlement reference",
    );
    rejected::<SettlementMetadata>(
        settlement_json("cash", "INR", 3_000, "\n"),
        "blank settlement reference",
    );
}

#[test]
fn instrument_spec_deserialization_rejects_every_builder_violation() {
    let valid = spec_json(1, 1_000, Some(2_000), "item", "item", "INR");
    let parsed: InstrumentSpec = serde_json::from_value(valid.clone()).expect("valid spec parses");
    assert_eq!(
        parsed.id(),
        InstrumentSpecId::new(1),
        "id survives builder validation"
    );
    assert!(parsed.covers(1_500), "validated window covers its interior");
    assert_eq!(
        serde_json::to_value(&parsed).expect("spec serializes"),
        valid,
        "the serialized schema is unchanged"
    );

    rejected::<InstrumentSpec>(
        spec_json(1, 2_000, Some(1_000), "item", "item", "INR"),
        "reversed effective window",
    );
    rejected::<InstrumentSpec>(
        spec_json(1, 1_000, Some(1_000), "item", "item", "INR"),
        "empty effective window",
    );
    let mut price_unit_mismatch = spec_json(1, 1_000, Some(2_000), "item", "item", "INR");
    price_unit_mismatch["multiplier"]["price_unit"] = serde_json::json!("unit-a");
    rejected::<InstrumentSpec>(price_unit_mismatch, "spec price-unit mismatch");

    let mut quantity_unit_mismatch = spec_json(1, 1_000, Some(2_000), "item", "item", "INR");
    quantity_unit_mismatch["multiplier"]["quantity_unit"] = serde_json::json!("unit-b");
    rejected::<InstrumentSpec>(quantity_unit_mismatch, "spec quantity-unit mismatch");

    let mut fee_mismatch = spec_json(1, 1_000, Some(2_000), "item", "item", "INR");
    fee_mismatch["fees"][0]["amount"]["currency"] = serde_json::json!("USD");
    rejected::<InstrumentSpec>(fee_mismatch, "fee currency mismatch");

    let mut tax_mismatch = spec_json(1, 1_000, Some(2_000), "item", "item", "INR");
    tax_mismatch["taxes"][0]["amount"]["currency"] = serde_json::json!("USD");
    rejected::<InstrumentSpec>(tax_mismatch, "tax currency mismatch");

    let mut funding_mismatch = spec_json(1, 1_000, Some(2_000), "item", "item", "INR");
    funding_mismatch["funding"]["amount"]["currency"] = serde_json::json!("USD");
    rejected::<InstrumentSpec>(funding_mismatch, "funding currency mismatch");

    let mut missing_settlement = spec_json(1, 1_000, Some(2_000), "item", "item", "INR");
    missing_settlement
        .as_object_mut()
        .expect("spec is an object")
        .remove("settlement");
    rejected::<InstrumentSpec>(missing_settlement, "missing settlement field");
}

#[test]
fn registry_deserialization_rebuilds_through_insert_and_rejects_overlap() {
    let disjoint = serde_json::json!({
        "specs": [
            spec_json(1, 1_000, Some(2_000), "item", "item", "INR"),
            spec_json(1, 2_000, None, "item", "item", "INR"),
        ],
    });
    let registry: InstrumentSpecRegistry =
        serde_json::from_value(disjoint).expect("disjoint specs parse");
    assert_eq!(registry.len(), 2, "both validated specs are inserted");
    assert_eq!(
        InstrumentSpecRef::new(InstrumentSpecId::new(1), 1_500)
            .resolve(&registry)
            .expect("first window resolves")
            .tick_size()
            .units(),
        5,
        "resolved specs come from the rebuilt registry"
    );

    rejected::<InstrumentSpecRegistry>(
        serde_json::json!({
            "specs": [
                spec_json(1, 1_000, Some(2_000), "item", "item", "INR"),
                spec_json(1, 1_500, None, "item", "item", "INR"),
            ],
        }),
        "overlapping effective periods",
    );
    rejected::<InstrumentSpecRegistry>(
        serde_json::json!({
            "specs": [spec_json(1, 2_000, Some(1_000), "item", "item", "INR")],
        }),
        "invalid spec inside a registry",
    );
    assert!(
        resolve_from_wire(serde_json::json!({
            "specs": [
                spec_json(1, 1_000, Some(2_000), "item", "item", "INR"),
                spec_json(1, 1_500, None, "item", "item", "INR"),
            ],
        }))
        .is_err(),
        "an overlapping wire registry cannot reach instrument lookup"
    );
}

#[test]
fn deserialization_cannot_bypass_typed_invariants() {
    // Primitive types that already validate through TryFrom/custom impls.
    rejected::<Scale>(serde_json::json!(19), "scale above maximum");
    rejected::<Currency>(serde_json::json!("inr"), "lowercase currency code");
    rejected::<Rational>(serde_json::json!([1, 0]), "zero rational denominator");
    rejected::<TickSize>(serde_json::json!(0), "zero tick size");

    // Composite types now re-enter their constructors/builders on input.
    rejected::<Quantity>(
        serde_json::json!({"units": -1, "unit": "item"}),
        "negative quantity",
    );
    rejected::<ContractMultiplier>(
        multiplier_json(serde_json::json!([0, 1]), "item", "item"),
        "zero contract multiplier",
    );
    rejected::<ContractMultiplier>(
        multiplier_json(serde_json::json!([-1, 1]), "item", "item"),
        "negative contract multiplier",
    );
    rejected::<RateSource>(source_json("", "series-1"), "empty rate source name");
    rejected::<RateSource>(
        source_json("neutral-oracle", ""),
        "empty rate source reference",
    );
    rejected::<FxRate>(
        fx_json(
            "INR",
            "INR",
            serde_json::json!([1, 1]),
            source_json("neutral-oracle", "series-1"),
            1_000,
        ),
        "same-currency FX rate",
    );
    rejected::<FxRate>(
        fx_json(
            "INR",
            "USD",
            serde_json::json!([0, 1]),
            source_json("neutral-oracle", "series-1"),
            1_000,
        ),
        "non-positive FX rate",
    );
    rejected::<FxRate>(
        fx_json(
            "INR",
            "USD",
            serde_json::json!([1, 83]),
            source_json("neutral-oracle", "series-1"),
            -1,
        ),
        "non-positive FX timestamp",
    );
    rejected::<FxRate>(
        fx_json(
            "INR",
            "USD",
            serde_json::json!([1, 83]),
            source_json("", "series-1"),
            1_000,
        ),
        "FX rate with blank source",
    );
    rejected::<Charge>(charge_json("fee", "", "INR"), "empty charge code");
    rejected::<SettlementMetadata>(
        settlement_json("cash", "INR", 0, "neutral-ref"),
        "invalid settlement timestamp",
    );
    rejected::<SettlementMetadata>(
        settlement_json("cash", "INR", 3_000, ""),
        "invalid settlement reference",
    );
    rejected::<InstrumentSpec>(
        spec_json(1, 2_000, Some(1_000), "item", "item", "INR"),
        "reversed spec window",
    );
    rejected::<InstrumentSpec>(
        spec_json(1, 1_000, Some(1_000), "item", "item", "INR"),
        "empty spec window",
    );
    let mut price_unit_mismatch = spec_json(1, 1_000, Some(2_000), "item", "item", "INR");
    price_unit_mismatch["multiplier"]["price_unit"] = serde_json::json!("unit-a");
    rejected::<InstrumentSpec>(price_unit_mismatch, "spec price-unit mismatch");

    let mut quantity_unit_mismatch = spec_json(1, 1_000, Some(2_000), "item", "item", "INR");
    quantity_unit_mismatch["multiplier"]["quantity_unit"] = serde_json::json!("unit-b");
    rejected::<InstrumentSpec>(quantity_unit_mismatch, "spec quantity-unit mismatch");
    rejected::<InstrumentSpecRegistry>(
        serde_json::json!({
            "specs": [
                spec_json(1, 1_000, Some(2_000), "item", "item", "INR"),
                spec_json(1, 1_500, None, "item", "item", "INR"),
            ],
        }),
        "overlapping registry windows",
    );

    let money: Money = serde_json::from_str("{\"units\":100,\"currency\":\"INR\",\"scale\":2}")
        .expect("valid typed data parses");
    assert_eq!(
        money.to_decimal_string(),
        "1 INR",
        "valid typed data still round-trips"
    );
}
