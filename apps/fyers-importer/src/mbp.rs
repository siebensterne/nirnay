//! Typed five-level MBP projection from FYERS Data Socket SDK callbacks.
//!
//! The captured item is a Python SDK callback observation, not an original
//! exchange packet. That origin is carried by the kernel envelope; this module
//! only converts the decoded depth fields into typed MBP levels.

use market_event::{
    MarketByPriceSnapshot,
    MarketDataEvent,
    MbpLevel, //
};
use serde_json::Value;
use thiserror::Error;

/// Five levels for the standard Data Socket product. The separate 50-level TBT
/// product is not accepted here.
pub const DATA_SOCKET_DEPTH_LEVELS: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FyersMbpError {
    #[error("callback payload is not an object")]
    NotObject,
    #[error("depth level {0} has an invalid price")]
    InvalidPrice(usize),
    #[error("depth level {0} has an invalid quantity")]
    InvalidQuantity(usize),
    #[error("depth level {0} has an invalid order count")]
    InvalidOrderCount(usize),
    #[error("price exceeds the paise range")]
    PriceOverflow,
    #[error("snapshot validation failed: {0}")]
    InvalidSnapshot(String),
}

fn decimal_string_paise(text: &str) -> Result<i64, FyersMbpError> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if fraction.len() > 2 || fraction.bytes().any(|byte| !byte.is_ascii_digit()) {
        return Err(FyersMbpError::PriceOverflow);
    }
    if whole.is_empty() || whole.bytes().any(|byte| !byte.is_ascii_digit()) {
        return Err(FyersMbpError::PriceOverflow);
    }
    let whole: i64 = whole.parse().map_err(|_| FyersMbpError::PriceOverflow)?;
    let fraction: i64 = if fraction.is_empty() {
        0
    } else {
        format!("{fraction:0<2}")
            .parse()
            .map_err(|_| FyersMbpError::PriceOverflow)?
    };
    whole
        .checked_mul(100)
        .and_then(|value| value.checked_add(fraction))
        .ok_or(FyersMbpError::PriceOverflow)
}

fn price_paise(value: &Value) -> Result<i64, FyersMbpError> {
    if let Some(text) = value.as_str() {
        return decimal_string_paise(text.trim());
    }
    let number = value.as_f64().ok_or(FyersMbpError::PriceOverflow)?;
    if !number.is_finite() || number < 0.0 || number * 100.0 >= i64::MAX as f64 {
        return Err(FyersMbpError::PriceOverflow);
    }
    Ok((number * 100.0).round() as i64)
}

fn quantity(value: Option<&Value>, index: usize) -> Result<i64, FyersMbpError> {
    let Some(value) = value else { return Ok(0) };
    let number = value
        .as_i64()
        .ok_or(FyersMbpError::InvalidQuantity(index))?;
    if number < 0 {
        return Err(FyersMbpError::InvalidQuantity(index));
    }
    Ok(number)
}

fn order_count(value: Option<&Value>, index: usize) -> Result<Option<u32>, FyersMbpError> {
    let Some(value) = value else { return Ok(None) };
    if value.is_null() {
        return Ok(None);
    }
    let number = value
        .as_u64()
        .ok_or(FyersMbpError::InvalidOrderCount(index))?;
    u32::try_from(number)
        .map(Some)
        .map_err(|_| FyersMbpError::InvalidOrderCount(index))
}

fn levels(payload: &Value, side: &str, index: usize) -> Result<MbpLevel, FyersMbpError> {
    let price = payload
        .get(format!("{side}_price{index}"))
        .ok_or(FyersMbpError::InvalidPrice(index))?;
    let price_paise = price_paise(price).map_err(|_| FyersMbpError::InvalidPrice(index))?;
    Ok(MbpLevel {
        price_paise,
        quantity: quantity(payload.get(format!("{side}_size{index}")), index)?,
        order_count: order_count(payload.get(format!("{side}_order{index}")), index)?,
    })
}

/// Project a `dp` callback into a typed five-level MBP snapshot. Non-depth
/// callback types return `None` rather than fabricating a book.
pub fn callback_to_mbp(
    payload: &Value,
    instrument_id: impl Into<String>,
    venue: impl Into<String>,
    product: impl Into<String>,
    received_utc_ns: i64,
) -> Result<Option<MarketDataEvent>, FyersMbpError> {
    if !payload.is_object() {
        return Err(FyersMbpError::NotObject);
    }
    if payload.get("type").and_then(Value::as_str) != Some("dp") {
        return Ok(None);
    }
    let mut bids = Vec::new();
    let mut asks = Vec::new();
    for index in 1..=DATA_SOCKET_DEPTH_LEVELS {
        let bid = levels(payload, "bid", index as usize)?;
        let ask = levels(payload, "ask", index as usize)?;
        if bid.quantity > 0 {
            bids.push(bid);
        }
        if ask.quantity > 0 {
            asks.push(ask);
        }
    }
    if bids.is_empty() || asks.is_empty() {
        return Ok(None);
    }
    let source_time_ns = match payload.get("exch_feed_time") {
        Some(value) if !value.is_null() => {
            let seconds = value.as_f64().ok_or(FyersMbpError::InvalidPrice(0))?;
            if !seconds.is_finite() || seconds < 0.0 {
                return Err(FyersMbpError::InvalidPrice(0));
            }
            let nanos = seconds * 1_000_000_000.0;
            if nanos >= i64::MAX as f64 {
                return Err(FyersMbpError::PriceOverflow);
            }
            Some(nanos as i64)
        }
        _ => None,
    };
    let snapshot = MarketByPriceSnapshot {
        instrument_id: instrument_id.into(),
        venue: venue.into(),
        source_id: "fyers.data_socket".to_owned(),
        product: product.into(),
        depth_levels: DATA_SOCKET_DEPTH_LEVELS,
        bids,
        asks,
        source_time_ns,
        last_trade_time_ns: None,
        received_utc_ns,
    };
    snapshot
        .validate()
        .map_err(|error| FyersMbpError::InvalidSnapshot(error.to_string()))?;
    Ok(Some(MarketDataEvent::MarketByPriceSnapshot(snapshot)))
}

#[cfg(all(test, feature = "market-shaped-tests"))]
mod tests {
    use super::*;
    use serde_json::json;

    fn depth_payload() -> Value {
        let mut payload = json!({"type": "dp", "exch_feed_time": 1_700_000_000.0});
        for index in 1..=5 {
            payload[format!("bid_price{index}")] = json!(format!("812.{}", 50 - index));
            payload[format!("bid_size{index}")] = json!(100 + index);
            payload[format!("bid_order{index}")] = json!(index);
            payload[format!("ask_price{index}")] = json!(format!("812.{}", 60 + index));
            payload[format!("ask_size{index}")] = json!(200 + index);
            payload[format!("ask_order{index}")] = json!(index + 10);
        }
        payload
    }

    #[test]
    fn dp_callback_projects_five_levels_with_order_counts() {
        let event = callback_to_mbp(&depth_payload(), "SBIN-EQ", "NSE", "data_socket", 42)
            .expect("projection")
            .expect("depth event");
        let MarketDataEvent::MarketByPriceSnapshot(snapshot) = event else {
            panic!("expected snapshot");
        };
        snapshot.validate().expect("valid snapshot");
        assert_eq!(snapshot.bids.len(), 5, "five bid levels");
        assert_eq!(snapshot.asks.len(), 5, "five ask levels");
        assert_eq!(
            snapshot.source_id, "fyers.data_socket",
            "SDK callback origin"
        );
        assert_eq!(
            snapshot.bids[0].order_count,
            Some(1),
            "displayed order count is carried"
        );
        assert_eq!(
            snapshot.bids[0].price_paise, 81_249,
            "decimal string converts exactly"
        );
        assert_eq!(
            snapshot.source_time_ns,
            Some(1_700_000_000_000_000_000),
            "exchange seconds convert to ns"
        );
    }

    #[test]
    fn non_depth_callback_does_not_fabricate_a_book() {
        assert_eq!(
            callback_to_mbp(
                &json!({"type": "sf", "ltp": 812.5}),
                "SBIN-EQ",
                "NSE",
                "data_socket",
                1
            )
            .expect("projection"),
            None,
            "last-trade callbacks are not depth"
        );
    }

    #[test]
    fn zero_quantity_levels_are_skipped() {
        let mut payload = depth_payload();
        payload["bid_size1"] = json!(0);
        let event = callback_to_mbp(&payload, "SBIN-EQ", "NSE", "data_socket", 1)
            .expect("projection")
            .expect("depth event");
        let MarketDataEvent::MarketByPriceSnapshot(snapshot) = event else {
            panic!("expected snapshot")
        };
        assert_eq!(snapshot.bids.len(), 4, "empty level is omitted");
    }

    #[test]
    fn invalid_depth_values_are_refused() {
        let mut negative = depth_payload();
        negative["bid_size2"] = json!(-1);
        assert!(
            callback_to_mbp(&negative, "SBIN-EQ", "NSE", "data_socket", 1).is_err(),
            "negative depth quantity is refused"
        );

        let mut bad_price = depth_payload();
        bad_price["ask_price3"] = json!("12.345");
        assert!(
            callback_to_mbp(&bad_price, "SBIN-EQ", "NSE", "data_socket", 1).is_err(),
            "a price with more than two decimal places is refused"
        );
    }

    #[test]
    fn empty_depth_returns_no_event() {
        let mut payload = depth_payload();
        for index in 1..=5 {
            payload[format!("bid_size{index}")] = json!(0);
            payload[format!("ask_size{index}")] = json!(0);
        }
        assert_eq!(
            callback_to_mbp(&payload, "SBIN-EQ", "NSE", "data_socket", 1).expect("projection"),
            None,
            "no populated levels means no book"
        );
        assert!(
            callback_to_mbp(&json!({"type": "dp"}), "SBIN-EQ", "NSE", "data_socket", 1).is_err(),
            "a depth callback missing price fields fails closed"
        );
    }
}
