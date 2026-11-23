#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
use std::io::{BufRead, Write};

use anyhow::{Context, Result, bail};
use cost_model::{ChargeBook, ChargeRate};
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use trading_core::{Paise, Product, Side};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TradeInput {
    trade_date: Date,
    product: Product,
    side: Side,
    turnover_paise: i64,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChargesOutput {
    pub brokerage: i64,
    pub securities_transaction_tax: i64,
    pub exchange_transaction: i64,
    pub sebi_turnover: i64,
    pub stamp_duty: i64,
    pub gst: i64,
    pub other: i64,
    pub total: i64,
}

pub fn load_book(json: &str) -> Result<ChargeBook> {
    let rates: Vec<ChargeRate> = serde_json::from_str(json)
        .context("schedule must be a JSON array of ChargeRate records")?;
    ChargeBook::new(rates).context("cost schedule rejected by the Rust cost model")
}

fn calculate(book: &ChargeBook, input: &str) -> Result<ChargesOutput> {
    let trade: TradeInput = serde_json::from_str(input).context("invalid trade input")?;
    if trade.turnover_paise < 0 {
        bail!("turnover_paise must not be negative");
    }
    let charges = book
        .calculate(
            trade.trade_date,
            trade.product,
            trade.side,
            Paise(trade.turnover_paise),
        )
        .context("no applicable effective-dated charge schedule")?;
    let parts = [
        charges.brokerage.0,
        charges.securities_transaction_tax.0,
        charges.exchange_transaction.0,
        charges.sebi_turnover.0,
        charges.stamp_duty.0,
        charges.gst.0,
        charges.other.0,
    ];
    let total = parts
        .iter()
        .try_fold(0_i64, |sum, value| sum.checked_add(*value))
        .context("total charge overflow")?;
    Ok(ChargesOutput {
        brokerage: charges.brokerage.0,
        securities_transaction_tax: charges.securities_transaction_tax.0,
        exchange_transaction: charges.exchange_transaction.0,
        sebi_turnover: charges.sebi_turnover.0,
        stamp_duty: charges.stamp_duty.0,
        gst: charges.gst.0,
        other: charges.other.0,
        total,
    })
}

/// Each non-empty input line produces exactly one output line. A bad input
/// aborts with its line number; partial output is not reported as complete.
pub fn run(book: &ChargeBook, input: impl BufRead, mut output: impl Write) -> Result<usize> {
    let mut count = 0usize;
    for (index, line) in input.lines().enumerate() {
        let line = line.context("failed reading input")?;
        if line.trim().is_empty() {
            continue;
        }
        let result = calculate(book, &line).with_context(|| format!("input line {}", index + 1))?;
        serde_json::to_writer(&mut output, &result).context("writing JSON output")?;
        output.write_all(b"\n").context("writing newline")?;
        count += 1;
    }
    Ok(count)
}

#[cfg(all(test, feature = "market-shaped-tests"))]
mod tests {
    use super::*;

    const SCHEDULE: &str = r#"[
      {
        "effective_from": "2026-01-01",
        "product": "intraday",
        "side": "Buy",
        "brokerage_ppb": 100000,
        "stt_ppb": 250000,
        "exchange_ppb": 32500,
        "sebi_ppb": 1000,
        "stamp_ppb": 0,
        "gst_ppb": 180000000
      }
    ]"#;

    #[test]
    fn calls_shared_cost_model_and_outputs_paise() {
        let book = load_book(SCHEDULE).unwrap();
        let input = concat!(
            r#"{"trade_date":"2026-03-02","product":"intraday","side":"Buy","#,
            r#""turnover_paise":10000000}"#,
        );
        let mut output = Vec::new();
        assert_eq!(run(&book, input.as_bytes(), &mut output).unwrap(), 1);
        let parsed: ChargesOutput = serde_json::from_slice(&output).unwrap();
        assert_eq!(parsed.brokerage, 1000);
        assert_eq!(parsed.total, 4075);
    }

    #[test]
    fn missing_schedule_is_an_error_not_zero() {
        let book = load_book(SCHEDULE).unwrap();
        let input = concat!(
            r#"{"trade_date":"2025-01-01","product":"intraday","side":"Buy","#,
            r#""turnover_paise":100}"#,
        );
        let mut output = Vec::new();
        assert!(run(&book, input.as_bytes(), &mut output).is_err());
        assert!(output.is_empty());
    }

    #[test]
    fn negative_turnover_is_rejected() {
        let book = load_book(SCHEDULE).unwrap();
        let input = concat!(
            r#"{"trade_date":"2026-03-02","product":"intraday","side":"Buy","#,
            r#""turnover_paise":-1}"#,
        );
        let mut output = Vec::new();
        assert!(run(&book, input.as_bytes(), &mut output).is_err());
    }
}
