// SPDX-License-Identifier: Apache-2.0
//! Venue-neutral financial units and effective-dated instrument terms.
//!
//! Invariants enforced by this module:
//! - authoritative values are integer coefficients with an explicit decimal
//!   scale; floating point is not accepted by any constructor;
//! - every arithmetic operation is checked and overflow is an error;
//! - currency, scale and unit travel with the value, so mismatched operands
//!   are rejected rather than coerced;
//! - scale, unit, tick and FX conversions are exact unless the caller supplies
//!   an explicit rounding rule, and every rounding returns a receipt;
//! - FX rates carry a source and an observation timestamp, and the result of a
//!   conversion preserves that provenance;
//! - instrument terms are effective-dated and hot-path code refers to them by
//!   a versioned id plus an effective time, never by an inlined term record.
//!
//! No venue-specific minor unit names are defined here. A two-decimal currency
//! is expressed as scale two; domain crates remain free to name their own
//! minor units.

use std::cmp::Ordering;
use std::error::Error;
use std::fmt;

#[rustfmt::skip]
use serde::{
    Deserialize,
    Serialize,
};

/// Largest decimal scale supported by an integer coefficient.
pub const MAX_SCALE: u8 = 18;

/// Number of decimal places carried by a monetary or price coefficient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Scale(u8);

impl Scale {
    pub const ZERO: Self = Self(0);
    pub const TWO: Self = Self(2);
    pub const SIX: Self = Self(6);

    #[must_use]
    pub const fn new(value: u8) -> Option<Self> {
        if value <= MAX_SCALE {
            Some(Self(value))
        } else {
            None
        }
    }

    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn factor(self) -> i128 {
        10i128.pow(self.0 as u32)
    }
}

impl TryFrom<u8> for Scale {
    type Error = UnitsError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::new(value).ok_or_else(|| {
            UnitsError::Invalid(format!("scale {value} exceeds maximum {MAX_SCALE}"))
        })
    }
}

impl From<Scale> for u8 {
    fn from(value: Scale) -> Self {
        value.0
    }
}

/// Three-letter currency code. It is a label, not an exchange rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Currency([u8; 3]);

impl Currency {
    pub const INR: Self = Self(*b"INR");
    pub const USD: Self = Self(*b"USD");
    pub const EUR: Self = Self(*b"EUR");
    pub const JPY: Self = Self(*b"JPY");

    pub fn new(code: &str) -> Result<Self, UnitsError> {
        let bytes = code.as_bytes();
        if bytes.len() != 3 || !bytes.iter().all(|byte| byte.is_ascii_uppercase()) {
            return Err(UnitsError::Invalid(format!(
                "invalid currency code {code}; expected three uppercase ASCII letters"
            )));
        }
        let mut normalized = [0_u8; 3];
        normalized.copy_from_slice(bytes);
        Ok(Self(normalized))
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 3] {
        &self.0
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Ok(text) = std::str::from_utf8(&self.0) else {
            return f.write_str("INVALID");
        };
        f.write_str(text)
    }
}

impl Serialize for Currency {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Currency {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let code = String::deserialize(deserializer)?;
        Self::new(&code).map_err(serde::de::Error::custom)
    }
}

/// Unit of measure for a price or a quantity. It is deliberately venue-neutral.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Unit(String);

impl Unit {
    pub fn new(symbol: &str) -> Result<Self, UnitsError> {
        let valid = !symbol.is_empty()
            && symbol.len() <= 32
            && symbol
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'/'));
        if !valid {
            return Err(UnitsError::Invalid(format!("invalid unit symbol {symbol}")));
        }
        Ok(Self(symbol.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Unit {
    type Error = UnitsError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<Unit> for String {
    fn from(value: Unit) -> Self {
        value.0
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How an inexact conversion may be rounded. Exact is the default everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundingRule {
    Exact,
    Floor,
    Ceil,
    HalfAwayFromZero,
    HalfEven,
}

/// Disclosure attached to any conversion that was not exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundingReceipt {
    rule: RoundingRule,
    remainder: i128,
    denominator: i128,
    rounded: bool,
}

impl RoundingReceipt {
    #[must_use]
    pub const fn rule(self) -> RoundingRule {
        self.rule
    }

    #[must_use]
    pub const fn remainder(self) -> i128 {
        self.remainder
    }

    #[must_use]
    pub const fn denominator(self) -> i128 {
        self.denominator
    }

    #[must_use]
    pub const fn was_rounded(self) -> bool {
        self.rounded
    }
}

/// Explicit error returned when an inexact conversion was requested as exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundingError {
    numerator: i128,
    denominator: i128,
    rule: RoundingRule,
}

impl RoundingError {
    #[must_use]
    pub const fn numerator(self) -> i128 {
        self.numerator
    }

    #[must_use]
    pub const fn denominator(self) -> i128 {
        self.denominator
    }
}

impl fmt::Display for RoundingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "inexact conversion {}/{} under rule {:?}",
            self.numerator, self.denominator, self.rule
        )
    }
}

impl Error for RoundingError {}

/// Result of checked integer arithmetic or an exactness requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericError {
    Overflow,
    Rounding(RoundingError),
}

impl From<RoundingError> for NumericError {
    fn from(value: RoundingError) -> Self {
        Self::Rounding(value)
    }
}

impl fmt::Display for NumericError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Overflow => f.write_str("checked arithmetic overflow"),
            Self::Rounding(error) => error.fmt(f),
        }
    }
}

impl Error for NumericError {}

/// Mismatched dimensions of two typed operands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DimensionError {
    Currency { left: Currency, right: Currency },
    Scale { left: Scale, right: Scale },
    Unit { left: String, right: String },
}

impl fmt::Display for DimensionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Currency { left, right } => {
                write!(f, "currency mismatch {left} versus {right}")
            }
            Self::Scale { left, right } => {
                write!(f, "scale mismatch {} versus {}", left.get(), right.get())
            }
            Self::Unit { left, right } => write!(f, "unit mismatch {left} versus {right}"),
        }
    }
}

impl Error for DimensionError {}

/// Failure to resolve an effective-dated instrument specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecError {
    NotEffective,
    Overlap,
    InvalidInterval,
    MissingField,
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotEffective => "no spec is effective at the requested time",
            Self::Overlap => "effective windows overlap",
            Self::InvalidInterval => "effective interval is empty or inverted",
            Self::MissingField => "required spec field is missing",
        })
    }
}

impl Error for SpecError {}

/// Error type shared by all operations in this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitsError {
    Dimension(DimensionError),
    Numeric(NumericError),
    Direction { expected: Currency, found: Currency },
    Invalid(String),
    Spec(SpecError),
}

impl From<DimensionError> for UnitsError {
    fn from(value: DimensionError) -> Self {
        Self::Dimension(value)
    }
}

impl From<NumericError> for UnitsError {
    fn from(value: NumericError) -> Self {
        Self::Numeric(value)
    }
}

impl From<RoundingError> for UnitsError {
    fn from(value: RoundingError) -> Self {
        Self::Numeric(NumericError::Rounding(value))
    }
}

impl From<SpecError> for UnitsError {
    fn from(value: SpecError) -> Self {
        Self::Spec(value)
    }
}

impl fmt::Display for UnitsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dimension(error) => error.fmt(f),
            Self::Numeric(error) => error.fmt(f),
            Self::Direction { expected, found } => {
                write!(f, "direction expects {expected}, found {found}")
            }
            Self::Invalid(message) => f.write_str(message),
            Self::Spec(error) => error.fmt(f),
        }
    }
}

impl Error for UnitsError {}

fn gcd_u128(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    if left == 0 { 1 } else { left }
}

fn as_i128(value: u128) -> Result<i128, NumericError> {
    i128::try_from(value).map_err(|_| NumericError::Overflow)
}

fn divide_and_round(
    numerator: i128,
    denominator: i128,
    rule: RoundingRule,
) -> Result<(i128, RoundingReceipt), NumericError> {
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    let receipt = RoundingReceipt {
        rule,
        remainder,
        denominator,
        rounded: remainder != 0,
    };
    if remainder == 0 {
        return Ok((quotient, receipt));
    }
    if rule == RoundingRule::Exact {
        return Err(NumericError::Rounding(RoundingError {
            numerator,
            denominator,
            rule,
        }));
    }
    let rounded = match rule {
        RoundingRule::Floor => {
            if remainder < 0 {
                quotient.checked_sub(1).ok_or(NumericError::Overflow)?
            } else {
                quotient
            }
        }
        RoundingRule::Ceil => {
            if remainder > 0 {
                quotient.checked_add(1).ok_or(NumericError::Overflow)?
            } else {
                quotient
            }
        }
        RoundingRule::HalfAwayFromZero | RoundingRule::HalfEven => {
            let doubled = remainder
                .unsigned_abs()
                .checked_mul(2)
                .ok_or(NumericError::Overflow)?;
            let magnitude = denominator.unsigned_abs();
            let comparison = doubled.cmp(&magnitude);
            let away = match comparison {
                Ordering::Greater => true,
                Ordering::Less => false,
                Ordering::Equal => {
                    rule == RoundingRule::HalfAwayFromZero || quotient.unsigned_abs() % 2 == 1
                }
            };
            if !away {
                quotient
            } else if remainder > 0 {
                quotient.checked_add(1).ok_or(NumericError::Overflow)?
            } else {
                quotient.checked_sub(1).ok_or(NumericError::Overflow)?
            }
        }
        RoundingRule::Exact => quotient,
    };
    Ok((rounded, receipt))
}

/// Positive rational number in lowest terms, with a positive denominator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "(i128, i128)", into = "(i128, i128)")]
pub struct Rational {
    numerator: i128,
    denominator: i128,
}

impl Rational {
    pub fn new(numerator: i128, denominator: i128) -> Result<Self, UnitsError> {
        if denominator == 0 {
            return Err(UnitsError::Invalid("zero denominator".to_owned()));
        }
        let negative = (numerator < 0) != (denominator < 0);
        let divisor = gcd_u128(numerator.unsigned_abs(), denominator.unsigned_abs());
        let numerator_magnitude = numerator.unsigned_abs() / divisor;
        let denominator_magnitude = denominator.unsigned_abs() / divisor;
        let denominator = as_i128(denominator_magnitude)
            .map_err(|_| UnitsError::Invalid("rational denominator out of range".to_owned()))?;
        let numerator = if negative && numerator_magnitude == (i128::MIN as u128) {
            i128::MIN
        } else {
            let magnitude = as_i128(numerator_magnitude)
                .map_err(|_| UnitsError::Invalid("rational numerator out of range".to_owned()))?;
            if negative { -magnitude } else { magnitude }
        };
        Ok(Self {
            numerator,
            denominator,
        })
    }

    #[must_use]
    pub const fn numerator(self) -> i128 {
        self.numerator
    }

    #[must_use]
    pub const fn denominator(self) -> i128 {
        self.denominator
    }

    pub fn reciprocal(self) -> Result<Self, UnitsError> {
        Self::new(self.denominator, self.numerator)
    }

    /// Multiply by a positive numerator and denominator factor, cancelling
    /// common factors before any checked multiplication.
    fn with_factors(
        self,
        numerator_factor: i128,
        denominator_factor: i128,
    ) -> Result<Self, NumericError> {
        let left = gcd_u128(
            self.numerator.unsigned_abs(),
            denominator_factor.unsigned_abs(),
        );
        let right = gcd_u128(
            self.denominator.unsigned_abs(),
            numerator_factor.unsigned_abs(),
        );
        let numerator = as_i128(self.numerator.unsigned_abs() / left)?
            .checked_mul(as_i128(numerator_factor.unsigned_abs() / right)?)
            .ok_or(NumericError::Overflow)?;
        let denominator = as_i128(denominator_factor.unsigned_abs() / left)?
            .checked_mul(as_i128(self.denominator.unsigned_abs() / right)?)
            .ok_or(NumericError::Overflow)?;
        if (self.numerator < 0) != (numerator_factor < 0) {
            return Ok(Self {
                numerator: -numerator,
                denominator,
            });
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }
}

impl TryFrom<(i128, i128)> for Rational {
    type Error = UnitsError;

    fn try_from(value: (i128, i128)) -> Result<Self, Self::Error> {
        Self::new(value.0, value.1)
    }
}

impl From<Rational> for (i128, i128) {
    fn from(value: Rational) -> Self {
        (value.numerator, value.denominator)
    }
}

fn apply_rational(
    value: i128,
    rational: Rational,
    rule: RoundingRule,
) -> Result<(i128, RoundingReceipt), NumericError> {
    if value == 0 || rational.numerator == 0 {
        return Ok((
            0,
            RoundingReceipt {
                rule,
                remainder: 0,
                denominator: rational.denominator,
                rounded: false,
            },
        ));
    }
    let shared = gcd_u128(value.unsigned_abs(), rational.denominator.unsigned_abs());
    let reduced_value = as_i128(value.unsigned_abs() / shared)?;
    let reduced_denominator = rational.denominator.unsigned_abs() / shared;
    let numerator_shared = gcd_u128(rational.numerator.unsigned_abs(), reduced_denominator);
    let reduced_numerator = as_i128(rational.numerator.unsigned_abs() / numerator_shared)?;
    let denominator = as_i128(reduced_denominator / numerator_shared)?;
    let magnitude = reduced_value
        .checked_mul(reduced_numerator)
        .ok_or(NumericError::Overflow)?;
    let signed = if (value < 0) != (rational.numerator < 0) {
        -magnitude
    } else {
        magnitude
    };
    divide_and_round(signed, denominator, rule)
}

fn rescale_units(
    units: i128,
    from: Scale,
    to: Scale,
    rule: RoundingRule,
) -> Result<(i128, RoundingReceipt), NumericError> {
    let factor = Rational::new(to.factor(), from.factor()).map_err(|_| NumericError::Overflow)?;
    apply_rational(units, factor, rule)
}

/// Exact amount of money in a named currency at an explicit decimal scale.
///
/// There is deliberately no conversion from an integer and no arithmetic trait
/// implementation: a bare integer or a different currency cannot cross the
/// boundary without an explicit constructor or an explicit FX conversion.
///
/// ```compile_fail
/// use market_types::units::{Currency, Money, Scale};
/// let _ = Money::new(1.5, Currency::INR, Scale::TWO);
/// ```
///
/// ```compile_fail
/// use market_types::units::{Currency, Money, Scale};
/// let amount = Money::new(1, Currency::INR, Scale::TWO);
/// let _ = amount + 1;
/// ```
///
/// ```compile_fail
/// use market_types::units::{Currency, Money, Scale};
/// let inr = Money::new(1, Currency::INR, Scale::TWO);
/// let usd = Money::new(1, Currency::USD, Scale::TWO);
/// let _ = inr + usd;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Money {
    units: i128,
    currency: Currency,
    scale: Scale,
}

impl Money {
    #[must_use]
    pub const fn new(units: i128, currency: Currency, scale: Scale) -> Self {
        Self {
            units,
            currency,
            scale,
        }
    }

    #[must_use]
    pub const fn units(self) -> i128 {
        self.units
    }

    #[must_use]
    pub const fn currency(self) -> Currency {
        self.currency
    }

    #[must_use]
    pub const fn scale(self) -> Scale {
        self.scale
    }

    #[must_use]
    pub fn to_decimal_string(self) -> String {
        let divisor = self.scale.factor().unsigned_abs();
        let magnitude = self.units.unsigned_abs();
        let whole = magnitude / divisor;
        let fraction = magnitude % divisor;
        let sign = if self.units < 0 { "-" } else { "" };
        if self.scale.get() == 0 || fraction == 0 {
            return format!("{sign}{whole} {}", self.currency);
        }
        let width = usize::from(self.scale.get());
        let padded = format!("{fraction:0width$}");
        let trimmed = padded.trim_end_matches('0');
        format!("{sign}{whole}.{trimmed} {}", self.currency)
    }

    fn require_same_dimensions(self, other: Self) -> Result<(), UnitsError> {
        if self.currency != other.currency {
            return Err(DimensionError::Currency {
                left: self.currency,
                right: other.currency,
            }
            .into());
        }
        if self.scale != other.scale {
            return Err(DimensionError::Scale {
                left: self.scale,
                right: other.scale,
            }
            .into());
        }
        Ok(())
    }

    pub fn checked_add(self, other: Self) -> Result<Self, UnitsError> {
        self.require_same_dimensions(other)?;
        let units = self
            .units
            .checked_add(other.units)
            .ok_or(NumericError::Overflow)?;
        Ok(Self { units, ..self })
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, UnitsError> {
        self.require_same_dimensions(other)?;
        let units = self
            .units
            .checked_sub(other.units)
            .ok_or(NumericError::Overflow)?;
        Ok(Self { units, ..self })
    }

    pub fn checked_neg(self) -> Result<Self, UnitsError> {
        let units = self.units.checked_neg().ok_or(NumericError::Overflow)?;
        Ok(Self { units, ..self })
    }

    /// Change decimal scale only when the value is exactly representable.
    pub fn rescale_exact(self, target: Scale) -> Result<Self, UnitsError> {
        let (units, _) = rescale_units(self.units, self.scale, target, RoundingRule::Exact)?;
        Ok(Self {
            units,
            ..self.with_scale(target)
        })
    }

    /// Convert through an explicitly supplied rate with provenance.
    pub fn convert(
        self,
        to: Currency,
        target_scale: Scale,
        rate: &FxRate,
        direction: FxDirection,
        rule: RoundingRule,
    ) -> Result<FxConversion, UnitsError> {
        let (source, target) = match direction {
            FxDirection::BaseToQuote => (rate.base, rate.quote),
            FxDirection::QuoteToBase => (rate.quote, rate.base),
        };
        if self.currency != source {
            return Err(UnitsError::Direction {
                expected: source,
                found: self.currency,
            });
        }
        if to != target {
            return Err(UnitsError::Direction {
                expected: target,
                found: to,
            });
        }
        let effective = match direction {
            FxDirection::BaseToQuote => *rate.quote_per_base(),
            FxDirection::QuoteToBase => rate.quote_per_base().reciprocal()?,
        };
        let combined = effective.with_factors(target_scale.factor(), self.scale.factor())?;
        let (units, receipt) = apply_rational(self.units, combined, rule)?;
        Ok(FxConversion {
            amount: Money::new(units, to, target_scale),
            receipt,
            rate: rate.clone(),
            direction,
        })
    }

    const fn with_scale(self, scale: Scale) -> Self {
        Self { scale, ..self }
    }
}

/// Signed price quoted in one currency per one unit of measure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Price {
    units: i128,
    quote: Currency,
    unit: Unit,
    scale: Scale,
}

impl Price {
    pub fn new(units: i128, quote: Currency, unit: Unit, scale: Scale) -> Result<Self, UnitsError> {
        Ok(Self {
            units,
            quote,
            unit,
            scale,
        })
    }

    #[must_use]
    pub const fn units(&self) -> i128 {
        self.units
    }

    #[must_use]
    pub const fn quote(&self) -> Currency {
        self.quote
    }

    #[must_use]
    pub fn unit(&self) -> &Unit {
        &self.unit
    }

    #[must_use]
    pub const fn scale(&self) -> Scale {
        self.scale
    }

    fn require_same_dimensions(&self, other: &Self) -> Result<(), UnitsError> {
        if self.quote != other.quote {
            return Err(DimensionError::Currency {
                left: self.quote,
                right: other.quote,
            }
            .into());
        }
        if self.unit != other.unit {
            return Err(DimensionError::Unit {
                left: self.unit.to_string(),
                right: other.unit.to_string(),
            }
            .into());
        }
        if self.scale != other.scale {
            return Err(DimensionError::Scale {
                left: self.scale,
                right: other.scale,
            }
            .into());
        }
        Ok(())
    }

    pub fn checked_add(self, other: Self) -> Result<Self, UnitsError> {
        self.require_same_dimensions(&other)?;
        let units = self
            .units
            .checked_add(other.units)
            .ok_or(NumericError::Overflow)?;
        Ok(Self { units, ..self })
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, UnitsError> {
        self.require_same_dimensions(&other)?;
        let units = self
            .units
            .checked_sub(other.units)
            .ok_or(NumericError::Overflow)?;
        Ok(Self { units, ..self })
    }

    pub fn rescale_exact(self, target: Scale) -> Result<Self, UnitsError> {
        let (units, _) = rescale_units(self.units, self.scale, target, RoundingRule::Exact)?;
        Ok(Self {
            units,
            scale: target,
            ..self
        })
    }
}

/// Non-negative quantity measured in an explicit unit of measure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quantity {
    units: i128,
    unit: Unit,
}

impl Quantity {
    pub fn new(units: i128, unit: Unit) -> Result<Self, UnitsError> {
        if units < 0 {
            return Err(UnitsError::Invalid(format!(
                "negative quantity {units} in {}",
                unit.as_str()
            )));
        }
        Ok(Self { units, unit })
    }

    #[must_use]
    pub const fn units(&self) -> i128 {
        self.units
    }

    #[must_use]
    pub fn unit(&self) -> &Unit {
        &self.unit
    }

    pub fn checked_add(self, other: Self) -> Result<Self, UnitsError> {
        self.require_same_unit(&other)?;
        let units = self
            .units
            .checked_add(other.units)
            .ok_or(NumericError::Overflow)?;
        Ok(Self { units, ..self })
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, UnitsError> {
        self.require_same_unit(&other)?;
        let units = self
            .units
            .checked_sub(other.units)
            .ok_or(NumericError::Overflow)?;
        Self::new(units, self.unit)
    }

    /// Convert with an explicit factor of target units per source unit.
    pub fn convert_exact(self, target: Unit, factor: Rational) -> Result<Self, UnitsError> {
        if factor.numerator <= 0 {
            return Err(UnitsError::Invalid(
                "quantity conversion factor must be positive".to_owned(),
            ));
        }
        let (units, _) = apply_rational(self.units, factor, RoundingRule::Exact)?;
        Self::new(units, target)
    }

    fn require_same_unit(&self, other: &Self) -> Result<(), UnitsError> {
        if self.unit != other.unit {
            return Err(DimensionError::Unit {
                left: self.unit.to_string(),
                right: other.unit.to_string(),
            }
            .into());
        }
        Ok(())
    }
}

/// Positive minimum price increment, expressed in price minor units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "i128", into = "i128")]
pub struct TickSize(i128);

impl TickSize {
    pub fn new(units: i128) -> Result<Self, UnitsError> {
        if units <= 0 {
            return Err(UnitsError::Invalid(format!(
                "tick size {units} must be positive"
            )));
        }
        Ok(Self(units))
    }

    #[must_use]
    pub const fn units(self) -> i128 {
        self.0
    }

    #[must_use]
    pub fn is_aligned(self, price_units: i128) -> bool {
        price_units % self.0 == 0
    }

    pub fn checked_snap(
        self,
        price_units: i128,
        rule: RoundingRule,
    ) -> Result<(i128, RoundingReceipt), UnitsError> {
        let steps = Rational::new(1, self.0)?;
        let (aligned_steps, receipt) = apply_rational(price_units, steps, rule)?;
        let snapped = aligned_steps
            .checked_mul(self.0)
            .ok_or(NumericError::Overflow)?;
        Ok((snapped, receipt))
    }
}

impl TryFrom<i128> for TickSize {
    type Error = UnitsError;

    fn try_from(value: i128) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<TickSize> for i128 {
    fn from(value: TickSize) -> Self {
        value.0
    }
}

/// Rational number of quote currency units per price unit and quantity unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractMultiplier {
    value: Rational,
    price_unit: Unit,
    quantity_unit: Unit,
}

impl ContractMultiplier {
    pub fn new(value: Rational, price_unit: Unit, quantity_unit: Unit) -> Result<Self, UnitsError> {
        if value.numerator <= 0 {
            return Err(UnitsError::Invalid(
                "contract multiplier must be positive".to_owned(),
            ));
        }
        Ok(Self {
            value,
            price_unit,
            quantity_unit,
        })
    }

    #[must_use]
    pub const fn value(&self) -> Rational {
        self.value
    }

    #[must_use]
    pub fn price_unit(&self) -> &Unit {
        &self.price_unit
    }

    #[must_use]
    pub fn quantity_unit(&self) -> &Unit {
        &self.quantity_unit
    }

    pub fn checked_notional(&self, price: Price, quantity: Quantity) -> Result<Money, UnitsError> {
        let (money, _) =
            self.checked_notional_with_rounding(price, quantity, RoundingRule::Exact)?;
        Ok(money)
    }

    pub fn checked_notional_with_rounding(
        &self,
        price: Price,
        quantity: Quantity,
        rule: RoundingRule,
    ) -> Result<(Money, RoundingReceipt), UnitsError> {
        self.require_dimensions(&price, &quantity)?;
        let product = price
            .units
            .checked_mul(quantity.units)
            .ok_or(NumericError::Overflow)?;
        let (units, receipt) = apply_rational(product, self.value, rule)?;
        Ok((Money::new(units, price.quote, price.scale), receipt))
    }

    fn require_dimensions(&self, price: &Price, quantity: &Quantity) -> Result<(), UnitsError> {
        if price.unit() != &self.price_unit {
            return Err(DimensionError::Unit {
                left: price.unit().to_string(),
                right: self.price_unit.to_string(),
            }
            .into());
        }
        if quantity.unit() != &self.quantity_unit {
            return Err(DimensionError::Unit {
                left: quantity.unit().to_string(),
                right: self.quantity_unit.to_string(),
            }
            .into());
        }
        Ok(())
    }
}

/// Origin of an FX rate. Both fields are required for provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateSource {
    name: String,
    reference: String,
}

impl RateSource {
    pub fn new(name: &str, reference: &str) -> Result<Self, UnitsError> {
        if name.trim().is_empty() || reference.trim().is_empty() {
            return Err(UnitsError::Invalid(
                "rate source requires a name and a reference".to_owned(),
            ));
        }
        Ok(Self {
            name: name.to_owned(),
            reference: reference.to_owned(),
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }
}

impl fmt::Display for RateSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.name, self.reference)
    }
}

/// Which leg of a rate is being applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FxDirection {
    BaseToQuote,
    QuoteToBase,
}

/// Rational rate of quote major units per one base major unit, with provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FxRate {
    base: Currency,
    quote: Currency,
    quote_per_base: Rational,
    source: RateSource,
    observed_at_us: i64,
}

impl FxRate {
    pub fn new(
        base: Currency,
        quote: Currency,
        quote_per_base: Rational,
        source: RateSource,
        observed_at_us: i64,
    ) -> Result<Self, UnitsError> {
        if base == quote {
            return Err(UnitsError::Invalid(
                "FX base and quote currencies must differ".to_owned(),
            ));
        }
        if quote_per_base.numerator <= 0 {
            return Err(UnitsError::Invalid("FX rate must be positive".to_owned()));
        }
        if observed_at_us <= 0 {
            return Err(UnitsError::Invalid(
                "FX observation timestamp must be positive".to_owned(),
            ));
        }
        Ok(Self {
            base,
            quote,
            quote_per_base,
            source,
            observed_at_us,
        })
    }

    #[must_use]
    pub const fn base(&self) -> Currency {
        self.base
    }

    #[must_use]
    pub const fn quote(&self) -> Currency {
        self.quote
    }

    #[must_use]
    pub const fn quote_per_base(&self) -> &Rational {
        &self.quote_per_base
    }

    #[must_use]
    pub fn source(&self) -> &RateSource {
        &self.source
    }

    #[must_use]
    pub const fn observed_at_us(&self) -> i64 {
        self.observed_at_us
    }
}

/// Result of an explicit FX conversion, including rate provenance and receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FxConversion {
    amount: Money,
    receipt: RoundingReceipt,
    rate: FxRate,
    direction: FxDirection,
}

impl FxConversion {
    #[must_use]
    pub const fn amount(&self) -> Money {
        self.amount
    }

    #[must_use]
    pub const fn receipt(&self) -> RoundingReceipt {
        self.receipt
    }

    #[must_use]
    pub fn rate(&self) -> &FxRate {
        &self.rate
    }

    #[must_use]
    pub const fn direction(&self) -> FxDirection {
        self.direction
    }
}

/// Fee, tax or funding charge denominated in an explicit currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Charge {
    kind: ChargeKind,
    amount: Money,
    code: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeKind {
    Fee,
    Tax,
    Funding,
}

impl Charge {
    pub fn new(kind: ChargeKind, amount: Money, code: &str) -> Result<Self, UnitsError> {
        if code.trim().is_empty() {
            return Err(UnitsError::Invalid("charge code is required".to_owned()));
        }
        Ok(Self {
            kind,
            amount,
            code: code.to_owned(),
        })
    }

    #[must_use]
    pub const fn kind(&self) -> ChargeKind {
        self.kind
    }

    #[must_use]
    pub const fn amount(&self) -> Money {
        self.amount
    }

    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementRegime {
    Cash,
    Physical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementMetadata {
    regime: SettlementRegime,
    currency: Currency,
    settlement_at_us: i64,
    reference: String,
}

impl SettlementMetadata {
    pub fn new(
        regime: SettlementRegime,
        currency: Currency,
        settlement_at_us: i64,
        reference: &str,
    ) -> Result<Self, UnitsError> {
        if settlement_at_us <= 0 {
            return Err(UnitsError::Invalid(
                "settlement timestamp must be positive".to_owned(),
            ));
        }
        if reference.trim().is_empty() {
            return Err(UnitsError::Invalid(
                "settlement reference is required".to_owned(),
            ));
        }
        Ok(Self {
            regime,
            currency,
            settlement_at_us,
            reference: reference.to_owned(),
        })
    }

    #[must_use]
    pub const fn regime(&self) -> SettlementRegime {
        self.regime
    }

    #[must_use]
    pub const fn currency(&self) -> Currency {
        self.currency
    }

    #[must_use]
    pub const fn settlement_at_us(&self) -> i64 {
        self.settlement_at_us
    }

    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }
}

/// Versioned identifier of one effective-dated set of instrument terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InstrumentSpecId(u64);

impl InstrumentSpecId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for InstrumentSpecId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "spec-{}", self.0)
    }
}

/// Hot-path reference: a versioned id and an effective time, never full terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstrumentSpecRef {
    spec_id: InstrumentSpecId,
    effective_at_us: i64,
}

impl InstrumentSpecRef {
    #[must_use]
    pub const fn new(spec_id: InstrumentSpecId, effective_at_us: i64) -> Self {
        Self {
            spec_id,
            effective_at_us,
        }
    }

    #[must_use]
    pub const fn spec_id(&self) -> InstrumentSpecId {
        self.spec_id
    }

    #[must_use]
    pub const fn effective_at_us(&self) -> i64 {
        self.effective_at_us
    }

    pub fn resolve<'a>(
        &self,
        registry: &'a InstrumentSpecRegistry,
    ) -> Result<&'a InstrumentSpec, UnitsError> {
        registry.resolve(self.spec_id, self.effective_at_us)
    }
}

/// Immutable, effective-dated instrument terms referenced by id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstrumentSpec {
    id: InstrumentSpecId,
    effective_from_us: i64,
    effective_to_us: Option<i64>,
    quote: Currency,
    price_unit: Unit,
    quantity_unit: Unit,
    price_scale: Scale,
    money_scale: Scale,
    tick_size: TickSize,
    multiplier: ContractMultiplier,
    fees: Vec<Charge>,
    taxes: Vec<Charge>,
    funding: Option<Charge>,
    settlement: SettlementMetadata,
}

impl InstrumentSpec {
    #[must_use]
    pub fn builder() -> InstrumentSpecBuilder {
        InstrumentSpecBuilder::default()
    }

    #[must_use]
    pub const fn id(&self) -> InstrumentSpecId {
        self.id
    }

    #[must_use]
    pub const fn effective_from_us(&self) -> i64 {
        self.effective_from_us
    }

    #[must_use]
    pub const fn effective_to_us(&self) -> Option<i64> {
        self.effective_to_us
    }

    #[must_use]
    pub const fn quote(&self) -> Currency {
        self.quote
    }

    #[must_use]
    pub fn price_unit(&self) -> &Unit {
        &self.price_unit
    }

    #[must_use]
    pub fn quantity_unit(&self) -> &Unit {
        &self.quantity_unit
    }

    #[must_use]
    pub const fn price_scale(&self) -> Scale {
        self.price_scale
    }

    #[must_use]
    pub const fn money_scale(&self) -> Scale {
        self.money_scale
    }

    #[must_use]
    pub const fn tick_size(&self) -> TickSize {
        self.tick_size
    }

    #[must_use]
    pub fn multiplier(&self) -> &ContractMultiplier {
        &self.multiplier
    }

    #[must_use]
    pub fn fees(&self) -> &[Charge] {
        &self.fees
    }

    #[must_use]
    pub fn taxes(&self) -> &[Charge] {
        &self.taxes
    }

    #[must_use]
    pub fn funding(&self) -> Option<&Charge> {
        self.funding.as_ref()
    }

    #[must_use]
    pub fn settlement(&self) -> &SettlementMetadata {
        &self.settlement
    }

    #[must_use]
    pub fn covers(&self, effective_at_us: i64) -> bool {
        self.effective_from_us <= effective_at_us
            && self.effective_to_us.is_none_or(|end| effective_at_us < end)
    }
}

#[derive(Debug, Clone, Default)]
pub struct InstrumentSpecBuilder {
    id: Option<InstrumentSpecId>,
    effective_from_us: Option<i64>,
    effective_to_us: Option<Option<i64>>,
    quote: Option<Currency>,
    price_unit: Option<Unit>,
    quantity_unit: Option<Unit>,
    price_scale: Option<Scale>,
    money_scale: Option<Scale>,
    tick_size: Option<TickSize>,
    multiplier: Option<ContractMultiplier>,
    fees: Vec<Charge>,
    taxes: Vec<Charge>,
    funding: Option<Charge>,
    settlement: Option<SettlementMetadata>,
}

impl InstrumentSpecBuilder {
    #[must_use]
    pub fn id(mut self, id: InstrumentSpecId) -> Self {
        self.id = Some(id);
        self
    }

    #[must_use]
    pub fn effective_window(mut self, from_us: i64, to_us: Option<i64>) -> Self {
        self.effective_from_us = Some(from_us);
        self.effective_to_us = Some(to_us);
        self
    }

    #[must_use]
    pub fn quote(mut self, quote: Currency) -> Self {
        self.quote = Some(quote);
        self
    }

    #[must_use]
    pub fn price_unit(mut self, unit: Unit) -> Self {
        self.price_unit = Some(unit);
        self
    }

    #[must_use]
    pub fn quantity_unit(mut self, unit: Unit) -> Self {
        self.quantity_unit = Some(unit);
        self
    }

    #[must_use]
    pub fn price_scale(mut self, scale: Scale) -> Self {
        self.price_scale = Some(scale);
        self
    }

    #[must_use]
    pub fn money_scale(mut self, scale: Scale) -> Self {
        self.money_scale = Some(scale);
        self
    }

    #[must_use]
    pub fn tick_size(mut self, tick: TickSize) -> Self {
        self.tick_size = Some(tick);
        self
    }

    #[must_use]
    pub fn multiplier(mut self, multiplier: ContractMultiplier) -> Self {
        self.multiplier = Some(multiplier);
        self
    }

    #[must_use]
    pub fn fees(mut self, fees: Vec<Charge>) -> Self {
        self.fees = fees;
        self
    }

    #[must_use]
    pub fn taxes(mut self, taxes: Vec<Charge>) -> Self {
        self.taxes = taxes;
        self
    }

    #[must_use]
    pub fn funding(mut self, funding: Option<Charge>) -> Self {
        self.funding = funding;
        self
    }

    #[must_use]
    pub fn settlement(mut self, settlement: SettlementMetadata) -> Self {
        self.settlement = Some(settlement);
        self
    }

    pub fn build(self) -> Result<InstrumentSpec, UnitsError> {
        let id = self.id.ok_or(SpecError::MissingField)?;
        let effective_from_us = self.effective_from_us.ok_or(SpecError::MissingField)?;
        let effective_to_us = self.effective_to_us.ok_or(SpecError::MissingField)?;
        if effective_to_us.is_some_and(|end| end <= effective_from_us) {
            return Err(SpecError::InvalidInterval.into());
        }
        let quote = self.quote.ok_or(SpecError::MissingField)?;
        let price_unit = self.price_unit.ok_or(SpecError::MissingField)?;
        let quantity_unit = self.quantity_unit.ok_or(SpecError::MissingField)?;
        let multiplier = self.multiplier.ok_or(SpecError::MissingField)?;
        if multiplier.price_unit() != &price_unit || multiplier.quantity_unit() != &quantity_unit {
            return Err(DimensionError::Unit {
                left: format!("{} per {}", price_unit, quantity_unit),
                right: format!(
                    "{} per {}",
                    multiplier.price_unit(),
                    multiplier.quantity_unit()
                ),
            }
            .into());
        }
        let settlement = self.settlement.ok_or(SpecError::MissingField)?;
        check_charge_currency(&self.fees, settlement.currency())?;
        check_charge_currency(&self.taxes, settlement.currency())?;
        if let Some(funding) = &self.funding {
            check_charge_currency(std::slice::from_ref(funding), settlement.currency())?;
        }
        Ok(InstrumentSpec {
            id,
            effective_from_us,
            effective_to_us,
            quote,
            price_unit,
            quantity_unit,
            price_scale: self.price_scale.ok_or(SpecError::MissingField)?,
            money_scale: self.money_scale.ok_or(SpecError::MissingField)?,
            tick_size: self.tick_size.ok_or(SpecError::MissingField)?,
            multiplier,
            fees: self.fees,
            taxes: self.taxes,
            funding: self.funding,
            settlement,
        })
    }
}

fn check_charge_currency(charges: &[Charge], currency: Currency) -> Result<(), UnitsError> {
    for charge in charges {
        if charge.amount().currency() != currency {
            return Err(DimensionError::Currency {
                left: charge.amount().currency(),
                right: currency,
            }
            .into());
        }
    }
    Ok(())
}

fn windows_overlap(left: &InstrumentSpec, right: &InstrumentSpec) -> bool {
    let left_end = left.effective_to_us.unwrap_or(i64::MAX);
    let right_end = right.effective_to_us.unwrap_or(i64::MAX);
    left.effective_from_us < right_end && right.effective_from_us < left_end
}

/// Registry of immutable specs; resolution needs only an id and an instant.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstrumentSpecRegistry {
    specs: Vec<InstrumentSpec>,
}

impl InstrumentSpecRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: InstrumentSpec) -> Result<(), UnitsError> {
        if self
            .specs
            .iter()
            .any(|existing| existing.id == spec.id && windows_overlap(existing, &spec))
        {
            return Err(SpecError::Overlap.into());
        }
        self.specs.push(spec);
        self.specs.sort_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then(left.effective_from_us.cmp(&right.effective_from_us))
        });
        Ok(())
    }

    pub fn resolve(
        &self,
        id: InstrumentSpecId,
        effective_at_us: i64,
    ) -> Result<&InstrumentSpec, UnitsError> {
        let mut matched = self
            .specs
            .iter()
            .filter(|spec| spec.id == id && spec.covers(effective_at_us));
        let first = matched.next().ok_or(SpecError::NotEffective)?;
        if matched.next().is_some() {
            return Err(SpecError::Overlap.into());
        }
        Ok(first)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }
}
