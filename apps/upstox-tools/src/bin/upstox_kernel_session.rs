#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Build a verified kernel session from an offline decoded Upstox capture.
//!
//! One-way projection for real-source certification. The raw frame journal
//! remains the evidence authority; decoded JSON is an input projection. No
//! network, credential or order capability exists here.

use std::{
    //
    io::Write,
    path::PathBuf,
};

use anyhow::{
    Context, //
    bail,
};
use clap::Parser;
use market_event::{
    ContentId,             //
    MarketByPriceSnapshot, //
    MarketDataEvent,
    MbpLevel,
    OriginClass, //
    SequenceScope,
};
use market_kernel::{
    KernelSession, //
    SourceObservation,
};
use upstox_feed_proto::{
    millis_to_ns, //
    rupees_to_paise,
};

#[derive(Debug, Parser)]
#[command(
    name = "upstox-kernel-session",
    about = "Project a decoded Upstox capture into a verified kernel session"
)]
struct Args {
    #[arg(long)]
    decoded: PathBuf,
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    output: PathBuf,
    /// Non-secret session identity recorded in the kernel envelope.
    #[arg(long)]
    session_id: String,
    /// Optional JSON map of provider instrument key to canonical instrument id.
    #[arg(long)]
    instrument_map: Option<PathBuf>,
}

fn paise(value: Option<&serde_json::Value>) -> anyhow::Result<i64> {
    let number = value
        .and_then(serde_json::Value::as_f64)
        .context("missing level price")?;
    rupees_to_paise(number).map_err(|error| anyhow::anyhow!(error.to_string()))
}

fn quantity(value: Option<&serde_json::Value>) -> anyhow::Result<i64> {
    let text = value
        .and_then(serde_json::Value::as_str)
        .context("level quantity is not a string")?;
    text.parse::<i64>().context("invalid level quantity")
}

fn build(input: &Args) -> anyhow::Result<KernelSession> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&input.manifest)?).context("reading manifest")?;
    if manifest.get("provider").and_then(serde_json::Value::as_str) != Some("Upstox") {
        bail!("manifest is not an Upstox session");
    }
    let raw_digest = manifest
        .get("raw_journal_sha256")
        .and_then(serde_json::Value::as_str)
        .context("manifest has no raw journal digest")?;
    let instrument_map: std::collections::BTreeMap<String, String> = match &input.instrument_map {
        Some(path) => serde_json::from_slice(&std::fs::read(path)?)?,
        None => std::collections::BTreeMap::new(),
    };

    let text = std::fs::read_to_string(&input.decoded).context("reading decoded capture")?;
    let mut observations = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let row: serde_json::Value = serde_json::from_str(line).context("parsing decoded row")?;
        let capture_sequence = row
            .get("capture_sequence")
            .and_then(serde_json::Value::as_u64)
            .context("row has no capture sequence")?;
        let frame_hash = ContentId::parse(
            row.get("payload_sha256")
                .and_then(serde_json::Value::as_str)
                .context("row has no payload digest")?,
        )
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let received_utc_ns = row
            .get("received_at_utc_ns")
            .and_then(serde_json::Value::as_i64)
            .context("row has no receive time")?;
        let received_monotonic_ns = row
            .get("received_monotonic_ns")
            .and_then(serde_json::Value::as_u64)
            .context("row has no monotonic receive time")?;
        let current_ms = row
            .get("message")
            .and_then(|value| value.get("currentTs"))
            .and_then(serde_json::Value::as_str)
            .context("row has no provider currentTs")?
            .parse::<i64>()
            .context("invalid provider currentTs")?;
        let source_time_ns =
            millis_to_ns(current_ms).map_err(|error| anyhow::anyhow!(error.to_string()))?;

        let Some(feeds) = row
            .get("message")
            .and_then(|value| value.get("feeds"))
            .and_then(serde_json::Value::as_object)
        else {
            continue;
        };
        for (provider_key, feed) in feeds {
            let Some(quotes) = feed
                .get("fullFeed")
                .and_then(|value| value.get("marketFF"))
                .and_then(|value| value.get("marketLevel"))
                .and_then(|value| value.get("bidAskQuote"))
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };
            let mut bids = Vec::new();
            let mut asks = Vec::new();
            for quote in quotes {
                bids.push(MbpLevel {
                    price_paise: paise(quote.get("bidP"))?,
                    quantity: quantity(quote.get("bidQ"))?,
                    order_count: None,
                });
                asks.push(MbpLevel {
                    price_paise: paise(quote.get("askP"))?,
                    quantity: quantity(quote.get("askQ"))?,
                    order_count: None,
                });
            }
            let book = MarketByPriceSnapshot {
                instrument_id: instrument_map
                    .get(provider_key)
                    .cloned()
                    .unwrap_or_else(|| provider_key.clone()),
                venue: "NSE".to_owned(),
                source_id: "upstox".to_owned(),
                product: manifest
                    .get("mode")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("full")
                    .to_owned(),
                depth_levels: u32::try_from(bids.len().max(asks.len()))?,
                bids,
                asks,
                source_time_ns: Some(source_time_ns),
                last_trade_time_ns: None,
                received_utc_ns,
            };
            book.validate()
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let canonical = format!("{provider_key}:{capture_sequence}").into_bytes();
            observations.push(SourceObservation {
                raw_frame_hash: frame_hash.clone(),
                source_id: "upstox".to_owned(),
                source_session_id: input.session_id.clone(),
                provider_instrument_id: provider_key.clone(),
                canonical_payload: canonical,
                origin_class: OriginClass::RawWireFrame,
                receive_utc_ns: received_utc_ns,
                receive_monotonic_ns: received_monotonic_ns,
                source_time_ns: Some(source_time_ns),
                source_clock_domain: Some("provider_current_ts_ms".to_owned()),
                channel_id: Some("upstox-v3-feed".to_owned()),
                connection_epoch: 1,
                receive_sequence: capture_sequence,
                // Capture-local receive order, not an exchange sequence.
                sequence: Some(capture_sequence),
                sequence_scope: SequenceScope::Connection,
                reference_revision: Some(raw_digest.to_owned()),
                instrument_id: instrument_map.get(provider_key).cloned(),
                evidence_token: None,
                payload: Some(MarketDataEvent::MarketByPriceSnapshot(book)),
            });
        }
    }
    if observations.is_empty() {
        bail!("decoded capture has no five-level full-feed observations");
    }
    Ok(KernelSession {
        schema_version: market_kernel::KERNEL_SESSION_SCHEMA_VERSION,
        source_scope: "NSE".to_owned(),
        session_reference_revision: Some(raw_digest.to_owned()),
        observations,
    })
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let session = build(&args)?;
    let encoded = serde_json::to_vec(&session)?;
    let mut file = std::fs::File::create(&args.output).context("creating kernel session")?;
    file.write_all(&encoded)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    eprintln!(
        "observations={} source=upstox session={}",
        session.observations.len(),
        args.session_id
    );
    Ok(())
}

#[cfg(all(test, feature = "market-shaped-tests"))]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn provider_prices_convert_exactly_to_paise() {
        let level = json!({"bidP": 76.44, "bidQ": "4103", "askP": 76.48, "askQ": "1495"});
        assert_eq!(
            paise(level.get("bidP")).expect("bid"),
            7_644,
            "rupees convert to paise"
        );
        assert_eq!(
            paise(level.get("askP")).expect("ask"),
            7_648,
            "rupees convert to paise"
        );
        assert_eq!(
            quantity(level.get("bidQ")).expect("qty"),
            4_103,
            "quantity parses as integer"
        );
        assert!(
            paise(Some(&json!(f64::NAN))).is_err(),
            "non-finite price is refused"
        );
    }
}
