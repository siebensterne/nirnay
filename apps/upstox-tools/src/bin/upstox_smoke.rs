#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Token-ready, read-only Upstox Analytics API smoke test.
//!
//! This binary checks Historical Candle V3, Full Market Quote V3 and Market
//! Data Feed V3. WebSocket protobuf frames are archived byte-for-byte; this
//! smoke test does not normalize them or claim historical tick replay.

use std::{
    fmt,
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use jiff::{Timestamp, civil::Date};
use reqwest::{Client, StatusCode, Url, header, redirect};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::time::{Duration, Instant, timeout_at};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use upstox_feed_proto::decode::{
    FeedResponseType as ProtoFeedResponseType, decode_feed_response_diagnostic,
};

const API_BASE: &str = "https://api.upstox.com/";
const TOKEN_ENV: &str = "UPSTOX_ANALYTICS_TOKEN";
const MAX_HTTP_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_WS_FRAME_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Parser)]
#[command(
    name = "upstox-smoke",
    about = "Read-only Upstox Analytics API connectivity and capture test"
)]
struct Args {
    /// Upstox instrument key, not an internal platform instrument id.
    #[arg(long, default_value = "NSE_EQ|INE848E01016")]
    instrument_key: String,

    /// Inclusive start date for one-minute historical candles (YYYY-MM-DD).
    #[arg(long, value_parser = parse_date)]
    from_date: Date,

    /// Inclusive end date for one-minute historical candles (YYYY-MM-DD).
    #[arg(long, value_parser = parse_date)]
    to_date: Date,

    /// Output directory. A unique run directory is created beneath this path.
    #[arg(long, default_value = "var/upstox")]
    output_dir: PathBuf,

    /// Number of binary feed frames to capture before closing the socket.
    #[arg(long, default_value_t = 5)]
    websocket_frames: usize,

    /// Maximum time to wait for WebSocket frames after subscription.
    #[arg(long, default_value_t = 20)]
    websocket_timeout_secs: u64,

    /// NSE session date for this live capture (YYYY-MM-DD). Defaults to
    /// `--to-date` when omitted.
    #[arg(long, value_parser = parse_date)]
    session_date: Option<Date>,
}

fn parse_date(value: &str) -> Result<Date, String> {
    Date::strptime("%Y-%m-%d", value)
        .map_err(|_| format!("invalid date {value:?}; expected YYYY-MM-DD"))
}

/// A token type whose Debug implementation never reveals the secret.
struct AnalyticsToken(String);

impl AnalyticsToken {
    fn from_environment() -> anyhow::Result<Self> {
        let value = std::env::var(TOKEN_ENV)
            .with_context(|| format!("set {TOKEN_ENV} in the process environment"))?;
        ensure!(!value.trim().is_empty(), "{TOKEN_ENV} is empty");
        ensure!(
            value == value.trim(),
            "{TOKEN_ENV} contains leading or trailing whitespace"
        );
        Ok(Self(value))
    }
}

impl fmt::Debug for AnalyticsToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AnalyticsToken([REDACTED])")
    }
}

#[derive(Clone)]
struct UpstoxClient {
    http: Client,
    token: String,
}

impl UpstoxClient {
    fn new(token: AnalyticsToken) -> anyhow::Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            // Fail closed on redirects: never let a credential-bearing request
            // be re-issued against a different host.
            .redirect(redirect::Policy::none())
            .user_agent(concat!(
                "nirnay/upstox-smoke/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .context("building Upstox HTTP client")?;
        Ok(Self {
            http,
            token: token.0,
        })
    }

    fn endpoint(&self, segments: &[&str]) -> anyhow::Result<Url> {
        let mut url = Url::parse(API_BASE).context("parsing Upstox API base URL")?;
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Upstox API base URL cannot accept path segments"))?;
            path.pop_if_empty();
            for segment in segments {
                path.push(segment);
            }
        }
        Ok(url)
    }

    async fn get_json_bytes(&self, url: Url) -> anyhow::Result<Vec<u8>> {
        let mut response = self
            .http
            .get(url)
            .bearer_auth(&self.token)
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .context("sending authenticated Upstox GET request")?;

        let status = response.status();
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unavailable")
            .to_owned();
        let retry_after = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unavailable")
            .to_owned();

        ensure!(
            response.content_length().unwrap_or(0) <= MAX_HTTP_BODY_BYTES as u64,
            "Upstox response exceeds the {} MiB body limit",
            MAX_HTTP_BODY_BYTES / (1024 * 1024)
        );
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .context("reading Upstox response body")?
        {
            ensure!(
                body.len().saturating_add(chunk.len()) <= MAX_HTTP_BODY_BYTES,
                "Upstox response exceeds body limit"
            );
            body.extend_from_slice(&chunk);
        }

        if !status.is_success() {
            let detail = match status {
                StatusCode::UNAUTHORIZED => {
                    "token is invalid, expired or not authorized for this API"
                }
                StatusCode::FORBIDDEN => "account or token does not have access to this API",
                StatusCode::TOO_MANY_REQUESTS => {
                    "rate limit exceeded; honour the Retry-After header"
                }
                _ => "provider request failed",
            };
            bail!(
                "Upstox HTTP {status}: {detail}; request_id={request_id}; retry_after={retry_after}"
            );
        }
        Ok(body)
    }

    async fn historical_candles(
        &self,
        instrument_key: &str,
        from_date: Date,
        to_date: Date,
    ) -> anyhow::Result<Vec<u8>> {
        let url = self.endpoint(&[
            "v3",
            "historical-candle",
            instrument_key,
            "minutes",
            "1",
            &to_date.strftime("%Y-%m-%d").to_string(),
            &from_date.strftime("%Y-%m-%d").to_string(),
        ])?;
        self.get_json_bytes(url).await
    }

    async fn full_market_quote(&self, instrument_key: &str) -> anyhow::Result<Vec<u8>> {
        let mut url = self.endpoint(&["v3", "market-quote", "quotes"])?;
        url.query_pairs_mut()
            .append_pair("instrument_key", instrument_key);
        self.get_json_bytes(url).await
    }

    async fn authorize_market_feed(&self) -> anyhow::Result<String> {
        let url = self.endpoint(&["v3", "feed", "market-data-feed", "authorize"])?;
        let bytes = self.get_json_bytes(url).await?;
        let response: WebsocketAuthorization = serde_json::from_slice(&bytes)
            .context("decoding Upstox WebSocket authorization response")?;
        ensure!(
            response.status == "success",
            "Upstox WebSocket authorization status was not success"
        );
        let authorized_uri = Url::parse(&response.data.authorized_redirect_uri)
            .context("Upstox returned an invalid WebSocket authorization URI")?;
        ensure!(
            authorized_uri.scheme() == "wss",
            "Upstox returned a non-secure WebSocket URI"
        );
        let host = authorized_uri.host_str().unwrap_or_default();
        ensure!(
            host == "upstox.com" || host.ends_with(".upstox.com"),
            "Upstox returned a WebSocket host outside upstox.com"
        );
        ensure!(
            authorized_uri.username().is_empty() && authorized_uri.password().is_none(),
            "Upstox returned WebSocket URI user information"
        );
        Ok(authorized_uri.to_string())
    }
}

#[derive(Debug, Deserialize)]
struct WebsocketAuthorization {
    status: String,
    data: WebsocketAuthorizationData,
}

#[derive(Debug, Deserialize)]
struct WebsocketAuthorizationData {
    authorized_redirect_uri: String,
}

#[derive(Debug, Serialize)]
struct SmokeManifest {
    schema_version: u32,
    provider: &'static str,
    mode: &'static str,
    started_at_utc: String,
    completed_at_utc: String,
    instrument_key: String,
    historical: HistoricalArtifact,
    quote: QuoteArtifact,
    websocket: WebsocketArtifact,
    credential_source: &'static str,
    credential_value_recorded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum CandleOrder {
    Ascending,
    Descending,
    Single,
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HistoricalValidation {
    candle_count: usize,
    provider_order: CandleOrder,
}

#[derive(Debug, Serialize)]
struct HistoricalArtifact {
    endpoint: &'static str,
    from_date: String,
    to_date: String,
    interval: &'static str,
    candle_count: usize,
    provider_order: CandleOrder,
    body_sha256: String,
    file: String,
}

#[derive(Debug, Serialize)]
struct QuoteArtifact {
    endpoint: &'static str,
    body_sha256: String,
    file: String,
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TerminationReason {
    FrameLimit,
    Timeout,
    RemoteClose,
    StreamEnd,
}

#[derive(Debug, Serialize)]
struct WebsocketArtifact {
    endpoint: &'static str,
    mode: &'static str,
    connected: bool,
    binary_frame_count: usize,
    timeout_secs: u64,
    max_frames: usize,
    session_date: String,
    subscription_sent_at_utc: String,
    capture_started_at_utc: String,
    capture_completed_at_utc: String,
    termination_reason: TerminationReason,
    raw_file: String,
    protobuf_decode: ProtobufDecodeSummary,
}

#[derive(Debug, Serialize)]
struct ProtobufDecodeSummary {
    enabled: bool,
    diagnostics_file: String,
    decoded_frame_count: usize,
    decode_error_count: usize,
    initial_feed_count: usize,
    live_feed_count: usize,
    market_info_count: usize,
    unknown_type_count: usize,
    requested_instrument_feed_frames: usize,
    requested_instrument_live_feed_frames: usize,
}

#[derive(Debug, Serialize)]
struct SmokeReport {
    output_directory: PathBuf,
    instrument_key: String,
    historical_candles: usize,
    quote_saved: bool,
    websocket_connected: bool,
    websocket_binary_frames: usize,
    websocket_decoded_frames: usize,
    websocket_decode_errors: usize,
    session_date: String,
    max_frames: usize,
    capture_started_at_utc: String,
    capture_completed_at_utc: String,
    termination_reason: TerminationReason,
    manifest: PathBuf,
}

#[derive(Debug, Serialize)]
struct ArchivedBinaryFrame<'a> {
    schema_version: u32,
    sequence: usize,
    received_at_utc: String,
    byte_len: usize,
    sha256: String,
    encoding: &'static str,
    payload_base64: &'a str,
}

#[derive(Debug, Serialize)]
struct DiagnosticRecord {
    schema_version: u32,
    sequence: usize,
    received_at_utc: String,
    byte_len: usize,
    sha256: String,
    outcome: DiagnosticOutcome,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum DiagnosticOutcome {
    Decoded {
        response_type_raw: i32,
        response_type: String,
        current_ts_raw: i64,
        has_market_info: bool,
        feed_keys: Vec<String>,
        feeds: Vec<DiagnosticFeed>,
    },
    DecodeError {
        error: String,
    },
}

#[derive(Debug, Serialize)]
struct DiagnosticFeed {
    key: String,
    kind: String,
    request_mode_raw: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameDecodeOutcome {
    Decoded,
    DecodeError,
}

#[derive(Debug, Clone, Default)]
struct ProtobufDecodeCounters {
    decoded_frame_count: usize,
    decode_error_count: usize,
    initial_feed_count: usize,
    live_feed_count: usize,
    market_info_count: usize,
    unknown_type_count: usize,
    requested_instrument_feed_frames: usize,
    requested_instrument_live_feed_frames: usize,
}

#[derive(Debug, Clone)]
struct CaptureObservability {
    subscription_sent_at_utc: String,
    capture_started_at_utc: String,
    capture_completed_at_utc: String,
    termination_reason: TerminationReason,
}

fn determine_termination_reason(
    sequence: usize,
    max_frames: usize,
    timed_out: bool,
    remote_close: bool,
    stream_end: bool,
) -> TerminationReason {
    if sequence >= max_frames {
        return TerminationReason::FrameLimit;
    }
    if timed_out {
        return TerminationReason::Timeout;
    }
    if remote_close {
        return TerminationReason::RemoteClose;
    }
    if stream_end {
        return TerminationReason::StreamEnd;
    }
    TerminationReason::StreamEnd
}

async fn run(args: Args) -> anyhow::Result<SmokeReport> {
    validate_inputs(&args)?;
    let token = AnalyticsToken::from_environment()?;
    let client = UpstoxClient::new(token)?;
    let started_at = Timestamp::now();
    let run_directory = create_run_directory(&args.output_dir, started_at)?;

    // Historical Candle V3: preserve the exact provider response before validation.
    let historical_raw = client
        .historical_candles(&args.instrument_key, args.from_date, args.to_date)
        .await
        .context("fetching Upstox Historical Candle V3")?;
    let historical_file = "historical-candles-v3.json";
    write_new(&run_directory.join(historical_file), &historical_raw)?;
    let historical_sha256 = sha256_hex(&historical_raw);
    let historical_validation = validate_historical_candles(&historical_raw)?;
    let candle_count = historical_validation.candle_count;
    let provider_order = historical_validation.provider_order;

    // Full Market Quote V3: preserve the exact provider response before validation.
    let quote_raw = client
        .full_market_quote(&args.instrument_key)
        .await
        .context("fetching Upstox Full Market Quote V3")?;
    let quote_file = "full-market-quote-v3.json";
    write_new(&run_directory.join(quote_file), &quote_raw)?;
    let quote_sha256 = sha256_hex(&quote_raw);
    validate_quote(&quote_raw, &args.instrument_key)?;

    // Market Data Feed V3: the authorized redirect is single-use. Never put
    // it in a report, log, manifest or archive.
    let authorized_uri = client
        .authorize_market_feed()
        .await
        .context("authorizing Upstox Market Data Feed V3")?;
    let websocket_file = "market-data-feed-v3.jsonl";
    let diagnostics_file = "market-data-feed-v3-diagnostics.jsonl";
    let session_date = args
        .session_date
        .unwrap_or(args.to_date)
        .strftime("%Y-%m-%d")
        .to_string();
    let (connected, binary_frames, decode_counters, observability) = capture_ltpc_frames(
        &authorized_uri,
        &args.instrument_key,
        &run_directory.join(websocket_file),
        &run_directory.join(diagnostics_file),
        args.websocket_frames,
        Duration::from_secs(args.websocket_timeout_secs),
    )
    .await
    .context("capturing Upstox Market Data Feed V3")?;

    let completed_at = Timestamp::now();
    let manifest = SmokeManifest {
        schema_version: 4,
        provider: "upstox",
        mode: "read_only_analytics_token",
        started_at_utc: started_at.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        completed_at_utc: completed_at.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        instrument_key: args.instrument_key.clone(),
        historical: HistoricalArtifact {
            endpoint: "GET /v3/historical-candle/{instrument_key}/minutes/1/{to_date}/{from_date}",
            from_date: args.from_date.strftime("%Y-%m-%d").to_string(),
            to_date: args.to_date.strftime("%Y-%m-%d").to_string(),
            interval: "1 minute",
            candle_count,
            provider_order,
            body_sha256: historical_sha256,
            file: historical_file.to_owned(),
        },
        quote: QuoteArtifact {
            endpoint: "GET /v3/market-quote/quotes",
            body_sha256: quote_sha256,
            file: quote_file.to_owned(),
        },
        websocket: WebsocketArtifact {
            endpoint: "GET /v3/feed/market-data-feed/authorize -> WSS Market Data Feed V3",
            mode: "ltpc",
            connected,
            binary_frame_count: binary_frames,
            timeout_secs: args.websocket_timeout_secs,
            max_frames: args.websocket_frames,
            session_date: session_date.clone(),
            subscription_sent_at_utc: observability.subscription_sent_at_utc.clone(),
            capture_started_at_utc: observability.capture_started_at_utc.clone(),
            capture_completed_at_utc: observability.capture_completed_at_utc.clone(),
            termination_reason: observability.termination_reason,
            raw_file: websocket_file.to_owned(),
            protobuf_decode: ProtobufDecodeSummary {
                enabled: true,
                diagnostics_file: diagnostics_file.to_owned(),
                decoded_frame_count: decode_counters.decoded_frame_count,
                decode_error_count: decode_counters.decode_error_count,
                initial_feed_count: decode_counters.initial_feed_count,
                live_feed_count: decode_counters.live_feed_count,
                market_info_count: decode_counters.market_info_count,
                unknown_type_count: decode_counters.unknown_type_count,
                requested_instrument_feed_frames: decode_counters.requested_instrument_feed_frames,
                requested_instrument_live_feed_frames: decode_counters
                    .requested_instrument_live_feed_frames,
            },
        },
        credential_source: "environment:UPSTOX_ANALYTICS_TOKEN",
        credential_value_recorded: false,
    };
    let manifest_path = run_directory.join("manifest.json");
    let manifest_bytes =
        serde_json::to_vec_pretty(&manifest).context("serializing smoke manifest")?;
    write_new(&manifest_path, &manifest_bytes)?;

    Ok(SmokeReport {
        output_directory: run_directory,
        instrument_key: args.instrument_key,
        historical_candles: candle_count,
        quote_saved: true,
        websocket_connected: connected,
        websocket_binary_frames: binary_frames,
        websocket_decoded_frames: decode_counters.decoded_frame_count,
        websocket_decode_errors: decode_counters.decode_error_count,
        session_date: session_date.clone(),
        max_frames: args.websocket_frames,
        capture_started_at_utc: observability.capture_started_at_utc.clone(),
        capture_completed_at_utc: observability.capture_completed_at_utc.clone(),
        termination_reason: observability.termination_reason,
        manifest: manifest_path,
    })
}

fn validate_inputs(args: &Args) -> anyhow::Result<()> {
    ensure!(
        !args.instrument_key.trim().is_empty(),
        "instrument key is required"
    );
    ensure!(
        args.instrument_key == args.instrument_key.trim(),
        "instrument key has surrounding whitespace"
    );
    ensure!(
        args.from_date <= args.to_date,
        "from-date must be <= to-date"
    );
    let maximum_to_date = args
        .from_date
        .checked_add(jiff::Span::new().months(1))
        .context("could not calculate one-month request boundary")?;
    ensure!(
        args.to_date <= maximum_to_date,
        "one-minute Historical Candle V3 requests are limited to one calendar month; split the range"
    );
    ensure!(
        args.websocket_frames > 0 && args.websocket_frames <= 10_000,
        "websocket-frames must be in 1..=10000"
    );
    ensure!(
        args.websocket_timeout_secs > 0 && args.websocket_timeout_secs <= 300,
        "websocket-timeout-secs must be in 1..=300"
    );
    Ok(())
}

fn validate_historical_candles(raw: &[u8]) -> anyhow::Result<HistoricalValidation> {
    let value: Value = serde_json::from_slice(raw).context("historical response is not JSON")?;
    ensure!(
        value.get("status").and_then(Value::as_str) == Some("success"),
        "historical response status is not success"
    );
    let candles = value
        .pointer("/data/candles")
        .and_then(Value::as_array)
        .context("historical response has no data.candles array")?;

    if candles.is_empty() {
        return Ok(HistoricalValidation {
            candle_count: 0,
            provider_order: CandleOrder::Empty,
        });
    }

    let mut previous_timestamp_nanos: Option<i64> = None;
    let mut observed_order: Option<CandleOrder> = None;
    for (index, candle) in candles.iter().enumerate() {
        let row = candle
            .as_array()
            .with_context(|| format!("candle {index} is not an array"))?;
        ensure!(
            (6..=7).contains(&row.len()),
            "candle {index} has {} fields; expected 6 or 7",
            row.len()
        );
        let timestamp = row[0]
            .as_str()
            .with_context(|| format!("candle {index} timestamp is not a string"))?;
        let parsed_timestamp = timestamp
            .parse::<Timestamp>()
            .with_context(|| format!("candle {index} timestamp is not RFC3339"))?;
        let timestamp_nanos = i64::try_from(parsed_timestamp.as_nanosecond())
            .ok()
            .context("candle timestamp is outside the supported nanosecond range")?;
        if let Some(previous) = previous_timestamp_nanos {
            ensure!(
                timestamp_nanos != previous,
                "historical candle has duplicate timestamp at row {index}"
            );
            let direction = if timestamp_nanos > previous {
                CandleOrder::Ascending
            } else {
                CandleOrder::Descending
            };
            match observed_order {
                None => observed_order = Some(direction),
                Some(existing) => ensure!(
                    existing == direction,
                    "historical candle has mixed or inconsistent timestamp ordering at row {index}"
                ),
            }
        }
        previous_timestamp_nanos = Some(timestamp_nanos);

        let open = finite_number(&row[1], index, "open")?;
        let high = finite_number(&row[2], index, "high")?;
        let low = finite_number(&row[3], index, "low")?;
        let close = finite_number(&row[4], index, "close")?;
        ensure!(low <= high, "candle {index} has low > high");
        ensure!(
            low <= open && open <= high,
            "candle {index} open is outside [low, high]"
        );
        ensure!(
            low <= close && close <= high,
            "candle {index} close is outside [low, high]"
        );
        let volume = row[5]
            .as_i64()
            .with_context(|| format!("candle {index} volume is not an integer"))?;
        ensure!(volume >= 0, "candle {index} volume is negative");
        if row.len() == 7 {
            let open_interest = row[6]
                .as_i64()
                .with_context(|| format!("candle {index} open interest is not an integer"))?;
            ensure!(
                open_interest >= 0,
                "candle {index} open interest is negative"
            );
        }
    }
    let provider_order = if candles.len() == 1 {
        CandleOrder::Single
    } else {
        observed_order.context("historical candle ordering could not be determined")?
    };
    Ok(HistoricalValidation {
        candle_count: candles.len(),
        provider_order,
    })
}

fn finite_number(value: &Value, row: usize, field: &str) -> anyhow::Result<f64> {
    let number = value
        .as_f64()
        .with_context(|| format!("candle {row} {field} is not numeric"))?;
    ensure!(number.is_finite(), "candle {row} {field} is not finite");
    Ok(number)
}

fn validate_quote(raw: &[u8], requested_instrument_key: &str) -> anyhow::Result<()> {
    let value: Value = serde_json::from_slice(raw).context("quote response is not JSON")?;
    ensure!(
        value.get("status").and_then(Value::as_str) == Some("success"),
        "quote response status is not success"
    );
    let data = value
        .get("data")
        .and_then(Value::as_object)
        .context("quote response has no data object")?;
    ensure!(!data.is_empty(), "quote response data object is empty");

    let mut matching: Vec<&Value> = Vec::new();
    for record in data.values() {
        let object = record
            .as_object()
            .context("quote record is not an object")?;
        if object.get("instrument_token").and_then(Value::as_str) == Some(requested_instrument_key)
        {
            matching.push(record);
        }
    }
    ensure!(
        matching.len() == 1,
        "quote response must contain exactly one record for requested instrument"
    );
    let record = matching[0]
        .as_object()
        .context("quote matched record is not an object")?;

    let instrument_token = record
        .get("instrument_token")
        .and_then(Value::as_str)
        .context("quote record has no instrument_token")?;
    ensure!(
        !instrument_token.is_empty(),
        "quote instrument_token is empty"
    );
    let symbol = record
        .get("symbol")
        .and_then(Value::as_str)
        .context("quote record has no symbol")?;
    ensure!(!symbol.is_empty(), "quote symbol is empty");

    let timestamp = record
        .get("timestamp")
        .and_then(Value::as_str)
        .context("quote record has no timestamp")?;
    let parsed_timestamp = timestamp
        .parse::<Timestamp>()
        .context("quote timestamp is not RFC3339")?;
    ensure!(
        i64::try_from(parsed_timestamp.as_nanosecond()).is_ok(),
        "quote timestamp is outside the supported range"
    );

    if let Some(last_trade_raw) = record.get("last_trade_time") {
        let last_trade_str = last_trade_raw
            .as_str()
            .context("quote last_trade_time is not a string")?;
        let last_trade_ms: i64 = last_trade_str
            .parse()
            .context("quote last_trade_time is not decimal milliseconds")?;
        ensure!(last_trade_ms >= 0, "quote last_trade_time is negative");
    }

    let last_price_value = record
        .get("last_price")
        .context("quote record has no last_price")?;
    non_negative_finite(last_price_value, "last_price")?;
    let volume_value = record.get("volume").context("quote record has no volume")?;
    non_negative_integer(volume_value, "volume")?;

    let ohlc = record
        .get("ohlc")
        .and_then(Value::as_object)
        .context("quote record has no ohlc object")?;
    let open = non_negative_finite(&ohlc["open"], "ohlc.open")?;
    let high = non_negative_finite(&ohlc["high"], "ohlc.high")?;
    let low = non_negative_finite(&ohlc["low"], "ohlc.low")?;
    let close = non_negative_finite(&ohlc["close"], "ohlc.close")?;
    ensure!(low <= high, "quote ohlc has low > high");
    ensure!(
        low <= open && open <= high,
        "quote ohlc open is outside [low, high]"
    );
    ensure!(
        low <= close && close <= high,
        "quote ohlc close is outside [low, high]"
    );
    let _ohlc_volume = non_negative_integer(&ohlc["volume"], "ohlc.volume")?;
    let _ohlc_ts = non_negative_integer(&ohlc["ts"], "ohlc.ts")?;

    let depth = record
        .get("depth")
        .and_then(Value::as_object)
        .context("quote record has no depth object")?;
    for side in ["buy", "sell"] {
        let levels = depth
            .get(side)
            .and_then(Value::as_array)
            .with_context(|| format!("quote depth {side} is not an array"))?;
        ensure!(
            levels.len() <= 5,
            "quote depth {side} has more than five levels"
        );
        for (index, level) in levels.iter().enumerate() {
            let level_object = level
                .as_object()
                .with_context(|| format!("quote depth {side} level {index} is not an object"))?;
            let _quantity = non_negative_integer(&level_object["quantity"], "depth.quantity")?;
            let _price = non_negative_finite(&level_object["price"], "depth.price")?;
            let _orders = non_negative_integer(&level_object["orders"], "depth.orders")?;
        }
    }

    Ok(())
}

fn non_negative_finite(value: &Value, field: &str) -> anyhow::Result<f64> {
    let number = value
        .as_f64()
        .with_context(|| format!("quote {field} is not numeric"))?;
    ensure!(number.is_finite(), "quote {field} is not finite");
    ensure!(number >= 0.0, "quote {field} is negative");
    Ok(number)
}

fn non_negative_integer(value: &Value, field: &str) -> anyhow::Result<i64> {
    let integer = value
        .as_i64()
        .with_context(|| format!("quote {field} is not an integer"))?;
    ensure!(integer >= 0, "quote {field} is negative");
    Ok(integer)
}

fn subscription_frame(instrument_key: &str) -> anyhow::Result<Vec<u8>> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_nanos();
    let guid = format!("{}-{nanos}", std::process::id());
    let request = serde_json::json!({
        "guid": guid,
        "method": "sub",
        "data": {
            "mode": "ltpc",
            "instrumentKeys": [instrument_key]
        }
    });
    serde_json::to_vec(&request).context("encoding WebSocket subscription")
}

fn frame_kind_name(kind: upstox_feed_proto::decode::FeedKind) -> &'static str {
    match kind {
        upstox_feed_proto::decode::FeedKind::Ltpc => "ltpc",
        upstox_feed_proto::decode::FeedKind::FullFeed => "full_feed",
        upstox_feed_proto::decode::FeedKind::FirstLevelWithGreeks => "first_level_with_greeks",
        upstox_feed_proto::decode::FeedKind::Unset => "unset",
    }
}

fn response_type_name(response_type: ProtoFeedResponseType) -> &'static str {
    match response_type {
        ProtoFeedResponseType::InitialFeed => "initial_feed",
        ProtoFeedResponseType::LiveFeed => "live_feed",
        ProtoFeedResponseType::MarketInfo => "market_info",
        ProtoFeedResponseType::Unknown(_) => "unknown",
    }
}

fn archive_and_diagnose_frame<R: std::io::Write, D: std::io::Write>(
    payload: &[u8],
    sequence: usize,
    received_at_utc: String,
    instrument_key: &str,
    raw_writer: &mut R,
    diagnostic_writer: &mut D,
) -> anyhow::Result<(FrameDecodeOutcome, ProtobufDecodeCounters)> {
    let mut counters = ProtobufDecodeCounters::default();
    ensure!(
        payload.len() <= MAX_WS_FRAME_BYTES,
        "Upstox WebSocket frame exceeds 4 MiB limit"
    );
    let byte_len = payload.len();
    let sha256 = sha256_hex(payload);
    let encoded = BASE64.encode(payload);
    let row = ArchivedBinaryFrame {
        schema_version: 1,
        sequence,
        received_at_utc: received_at_utc.clone(),
        byte_len,
        sha256: sha256.clone(),
        encoding: "protobuf-v3-base64",
        payload_base64: &encoded,
    };
    serde_json::to_writer(&mut *raw_writer, &row).context("serializing captured feed frame")?;
    raw_writer
        .write_all(b"\n")
        .context("writing captured feed frame")?;
    raw_writer.flush().context("flushing captured feed frame")?;

    let outcome_record = match decode_feed_response_diagnostic(payload) {
        Ok(diagnostic) => {
            counters.decoded_frame_count += 1;
            match diagnostic.response_type {
                ProtoFeedResponseType::InitialFeed => counters.initial_feed_count += 1,
                ProtoFeedResponseType::LiveFeed => counters.live_feed_count += 1,
                ProtoFeedResponseType::MarketInfo => counters.market_info_count += 1,
                ProtoFeedResponseType::Unknown(_) => counters.unknown_type_count += 1,
            }
            if diagnostic.feed_keys.iter().any(|k| k == instrument_key) {
                counters.requested_instrument_feed_frames += 1;
                if diagnostic.response_type == ProtoFeedResponseType::LiveFeed {
                    counters.requested_instrument_live_feed_frames += 1;
                }
            }
            DiagnosticOutcome::Decoded {
                response_type_raw: diagnostic.response_type_raw,
                response_type: response_type_name(diagnostic.response_type).to_owned(),
                current_ts_raw: diagnostic.current_ts,
                has_market_info: diagnostic.has_market_info,
                feed_keys: diagnostic.feed_keys.clone(),
                feeds: diagnostic
                    .feeds
                    .iter()
                    .map(|f| DiagnosticFeed {
                        key: f.key.clone(),
                        kind: frame_kind_name(f.kind).to_owned(),
                        request_mode_raw: f.request_mode_raw,
                    })
                    .collect(),
            }
        }
        Err(error) => {
            counters.decode_error_count += 1;
            DiagnosticOutcome::DecodeError {
                error: error.to_string(),
            }
        }
    };
    let outcome = match &outcome_record {
        DiagnosticOutcome::Decoded { .. } => FrameDecodeOutcome::Decoded,
        DiagnosticOutcome::DecodeError { .. } => FrameDecodeOutcome::DecodeError,
    };
    let diagnostic = DiagnosticRecord {
        schema_version: 1,
        sequence,
        received_at_utc,
        byte_len,
        sha256,
        outcome: outcome_record,
    };
    serde_json::to_writer(&mut *diagnostic_writer, &diagnostic)
        .context("serializing diagnostic frame")?;
    diagnostic_writer
        .write_all(b"\n")
        .context("writing diagnostic frame")?;
    diagnostic_writer
        .flush()
        .context("flushing diagnostic frame")?;
    Ok((outcome, counters))
}

fn flush_and_sync(writer: &mut BufWriter<fs::File>) -> anyhow::Result<()> {
    let flush_result = writer.flush().context("flushing capture");
    let sync_result = writer.get_ref().sync_all().context("syncing capture");
    flush_result?;
    sync_result?;
    Ok(())
}

async fn capture_ltpc_frames(
    authorized_uri: &str,
    instrument_key: &str,
    output: &Path,
    diagnostics_output: &Path,
    max_frames: usize,
    max_wait: Duration,
) -> anyhow::Result<(bool, usize, ProtobufDecodeCounters, CaptureObservability)> {
    let (mut socket, _response) = connect_async(authorized_uri).await.map_err(|_| {
        anyhow::anyhow!(
            "Upstox WebSocket handshake failed; the one-time authorization URI is redacted"
        )
    })?;
    let subscription = subscription_frame(instrument_key)?;
    socket
        .send(Message::Binary(subscription.into()))
        .await
        .context("sending binary LTPC subscription")?;
    let subscription_sent_at_utc = Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    let capture_started_at_utc = subscription_sent_at_utc.clone();
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .with_context(|| format!("creating WebSocket capture {}", output.display()))?;
    let mut writer = BufWriter::new(file);
    let diag_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(diagnostics_output)
        .with_context(|| {
            format!(
                "creating diagnostics capture {}",
                diagnostics_output.display()
            )
        })?;
    let mut diag_writer = BufWriter::new(diag_file);
    let mut counters = ProtobufDecodeCounters::default();
    let deadline = Instant::now() + max_wait;
    let mut sequence = 0usize;
    let mut remote_close = false;
    let mut stream_end = false;
    let mut timed_out = false;

    let capture_result: anyhow::Result<()> = async {
        while sequence < max_frames && Instant::now() < deadline {
            let next = timeout_at(deadline, socket.next()).await;
            match next {
                Err(_) => {
                    timed_out = true;
                    break;
                }
                Ok(None) => {
                    stream_end = true;
                    break;
                }
                Ok(Some(Err(error))) => {
                    return Err(error).context("reading Upstox WebSocket frame");
                }
                Ok(Some(Ok(Message::Binary(payload)))) => {
                    sequence += 1;
                    let received = Timestamp::now()
                        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
                        .to_string();
                    let (_outcome, frame_counters) = archive_and_diagnose_frame(
                        payload.as_ref(),
                        sequence,
                        received,
                        instrument_key,
                        &mut writer,
                        &mut diag_writer,
                    )?;
                    counters.decoded_frame_count += frame_counters.decoded_frame_count;
                    counters.decode_error_count += frame_counters.decode_error_count;
                    counters.initial_feed_count += frame_counters.initial_feed_count;
                    counters.live_feed_count += frame_counters.live_feed_count;
                    counters.market_info_count += frame_counters.market_info_count;
                    counters.unknown_type_count += frame_counters.unknown_type_count;
                    counters.requested_instrument_feed_frames +=
                        frame_counters.requested_instrument_feed_frames;
                    counters.requested_instrument_live_feed_frames +=
                        frame_counters.requested_instrument_live_feed_frames;
                }
                Ok(Some(Ok(Message::Ping(payload)))) => {
                    socket
                        .send(Message::Pong(payload))
                        .await
                        .context("responding to WebSocket ping")?;
                }
                Ok(Some(Ok(Message::Close(_)))) => {
                    remote_close = true;
                    break;
                }
                Ok(Some(Ok(Message::Text(_))))
                | Ok(Some(Ok(Message::Pong(_))))
                | Ok(Some(Ok(Message::Frame(_)))) => {}
            }
        }
        if !timed_out && !remote_close && !stream_end && sequence >= max_frames {
            // Frame-limit exit is distinguished by the post-loop check below.
        }
        Ok(())
    }
    .await;

    let raw_finalize = flush_and_sync(&mut writer);
    let diagnostic_finalize = flush_and_sync(&mut diag_writer);

    capture_result?;
    raw_finalize?;
    diagnostic_finalize?;
    let capture_completed_at_utc = Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    let termination_reason =
        determine_termination_reason(sequence, max_frames, timed_out, remote_close, stream_end);
    let _ = socket.close(None).await;
    Ok((
        true,
        sequence,
        counters,
        CaptureObservability {
            subscription_sent_at_utc,
            capture_started_at_utc,
            capture_completed_at_utc,
            termination_reason,
        },
    ))
}

fn create_run_directory(root: &Path, started_at: Timestamp) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(root).with_context(|| format!("creating output root {}", root.display()))?;
    let stamp = started_at.strftime("%Y%m%dT%H%M%S%.3fZ");
    let run_directory = root.join(format!("upstox-smoke-{stamp}-{}", std::process::id()));
    fs::create_dir(&run_directory)
        .with_context(|| format!("creating unique run directory {}", run_directory.display()))?;
    Ok(run_directory)
}

fn write_new(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("creating archive file {}", path.display()))?;
    file.write_all(contents)
        .with_context(|| format!("writing archive file {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing archive file {}", path.display()))?;
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let report = run(args).await?;
    emit_report(&report)?;
    if report.websocket_binary_frames == 0 {
        eprintln!(
            "warning: WebSocket connected but no binary market frames arrived before timeout; retry during market hours"
        );
    }
    Ok(())
}

fn emit_report(report: &SmokeReport) -> anyhow::Result<()> {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, report).context("serializing smoke report")?;
    output.write_all(b"\n").context("writing smoke report")?;
    output.flush().context("flushing smoke report")
}

#[cfg(all(test, feature = "market-shaped-tests"))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_valid_historical_candle_response() {
        let payload = br#"{"status":"success","data":{"candles":[["2026-11-06T09:15:00+05:30",100.0,102.0,99.0,101.0,1500,0],["2026-11-06T09:16:00+05:30",101.0,103.0,100.0,102.0,1750,0]]}}"#;
        let result = validate_historical_candles(payload).expect("valid fixture");
        assert_eq!(result.candle_count, 2);
        assert_eq!(result.provider_order, CandleOrder::Ascending);
    }

    #[test]
    fn accepts_strictly_ascending_historical_candles() {
        let payload = br#"{"status":"success","data":{"candles":[["2026-11-06T09:15:00+05:30",100.0,102.0,99.0,101.0,1500,0],["2026-11-06T09:16:00+05:30",101.0,103.0,100.0,102.0,1750,0],["2026-11-06T09:17:00+05:30",102.0,104.0,101.0,103.0,1800,0]]}}"#;
        let result = validate_historical_candles(payload).expect("ascending fixture");
        assert_eq!(result.candle_count, 3);
        assert_eq!(result.provider_order, CandleOrder::Ascending);
    }

    #[test]
    fn accepts_strictly_descending_historical_candles() {
        let payload = br#"{"status":"success","data":{"candles":[["2026-11-06T15:29:00+05:30",100.0,102.0,99.0,101.0,1500,0],["2026-11-06T15:28:00+05:30",101.0,103.0,100.0,102.0,1750,0],["2026-11-06T15:27:00+05:30",102.0,104.0,101.0,103.0,1800,0]]}}"#;
        let result = validate_historical_candles(payload).expect("descending fixture");
        assert_eq!(result.candle_count, 3);
        assert_eq!(result.provider_order, CandleOrder::Descending);
    }

    #[test]
    fn rejects_duplicate_timestamps() {
        let payload = br#"{"status":"success","data":{"candles":[["2026-11-06T09:16:00+05:30",100,102,99,101,1500,0],["2026-11-06T09:16:00+05:30",101,103,100,102,1750,0]]}}"#;
        assert!(validate_historical_candles(payload).is_err());
    }

    #[test]
    fn rejects_offset_equivalent_duplicate_timestamps() {
        // 2026-11-06T09:15:00+05:30 == 2026-11-06T03:45:00Z (same instant)
        let payload = br#"{"status":"success","data":{"candles":[["2026-11-06T09:15:00+05:30",100.0,102.0,99.0,101.0,1500,0],["2026-11-06T03:45:00Z",101.0,103.0,100.0,102.0,1750,0]]}}"#;
        assert!(validate_historical_candles(payload).is_err());
    }

    #[test]
    fn rejects_duplicate_or_out_of_order_candles() {
        let payload = br#"{"status":"success","data":{"candles":[["2026-11-06T09:16:00+05:30",100,102,99,101,1500,0],["2026-11-06T09:16:00+05:30",101,103,100,102,1750,0]]}}"#;
        assert!(validate_historical_candles(payload).is_err());
    }

    #[test]
    fn rejects_mixed_ordering() {
        let payload = br#"{"status":"success","data":{"candles":[["2026-11-06T15:29:00+05:30",100.0,102.0,99.0,101.0,1500,0],["2026-11-06T15:27:00+05:30",101.0,103.0,100.0,102.0,1750,0],["2026-11-06T15:28:00+05:30",102.0,104.0,101.0,103.0,1800,0],["2026-11-06T15:26:00+05:30",103.0,105.0,102.0,104.0,1850,0]]}}"#;
        assert!(validate_historical_candles(payload).is_err());
    }

    #[test]
    fn accepts_empty_candles() {
        let payload = br#"{"status":"success","data":{"candles":[]}}"#;
        let result = validate_historical_candles(payload).expect("empty fixture");
        assert_eq!(result.candle_count, 0);
        assert_eq!(result.provider_order, CandleOrder::Empty);
    }

    #[test]
    fn accepts_single_candle() {
        let payload = br#"{"status":"success","data":{"candles":[["2026-11-06T09:15:00+05:30",100.0,102.0,99.0,101.0,1500,0]]}}"#;
        let result = validate_historical_candles(payload).expect("single fixture");
        assert_eq!(result.candle_count, 1);
        assert_eq!(result.provider_order, CandleOrder::Single);
    }

    #[test]
    fn rejects_invalid_rfc3339_timestamps() {
        let payload = br#"{"status":"success","data":{"candles":[["not-a-timestamp",100.0,102.0,99.0,101.0,1500,0]]}}"#;
        assert!(validate_historical_candles(payload).is_err());
    }

    #[test]
    fn preserves_original_source_bytes_for_both_orderings() {
        let ascending = br#"{"status":"success","data":{"candles":[["2026-11-06T09:15:00+05:30",100.0,102.0,99.0,101.0,1500,0],["2026-11-06T09:16:00+05:30",101.0,103.0,100.0,102.0,1750,0]]}}"#;
        let descending = br#"{"status":"success","data":{"candles":[["2026-11-06T09:16:00+05:30",101.0,103.0,100.0,102.0,1750,0],["2026-11-06T09:15:00+05:30",100.0,102.0,99.0,101.0,1500,0]]}}"#;
        let ascending_before = ascending.to_vec();
        let descending_before = descending.to_vec();
        assert!(validate_historical_candles(ascending).is_ok());
        assert!(validate_historical_candles(descending).is_ok());
        assert_eq!(ascending, ascending_before.as_slice());
        assert_eq!(descending, descending_before.as_slice());
    }

    #[test]
    fn rejects_ohlc_invariant_violations() {
        let payload = br#"{"status":"success","data":{"candles":[["2026-11-06T09:15:00+05:30",100,98,99,101,1500,0]]}}"#;
        assert!(validate_historical_candles(payload).is_err());
    }

    #[test]
    fn rejects_negative_volume() {
        let payload = br#"{"status":"success","data":{"candles":[["2026-11-06T09:15:00+05:30",100.0,102.0,99.0,101.0,-1,0]]}}"#;
        assert!(validate_historical_candles(payload).is_err());
    }

    const VALID_QUOTE: &[u8] = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[{"quantity":0,"price":0.0,"orders":0},{"quantity":0,"price":0.0,"orders":0},{"quantity":0,"price":0.0,"orders":0},{"quantity":0,"price":0.0,"orders":0},{"quantity":0,"price":0.0,"orders":0}],"sell":[{"quantity":14624,"price":80.39,"orders":15},{"quantity":0,"price":0.0,"orders":0},{"quantity":0,"price":0.0,"orders":0},{"quantity":0,"price":0.0,"orders":0},{"quantity":0,"price":0.0,"orders":0}]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932,"average_price":79.75,"oi":0.0,"net_change":-0.65,"total_buy_quantity":0.0,"total_sell_quantity":14624.0,"lower_circuit_limit":77.72000122070312,"upper_circuit_limit":82.5199966430664,"last_trade_time":"1794219944214","oi_day_high":0.0,"oi_day_low":0.0,"prev_close_price":81.04,"year_high":86.83,"year_low":68.79,"previous_oi":0.0,"indicative_equilibrium_price":0.0,"reference_price":80.12,"indicative_equilibrium_quantity":0,"indicative_imbalance_quantity_total":0,"indicative_imbalance_quantity_market":0,"cas_eligible":true}}}"#;

    #[test]
    fn validates_valid_nhpc_quote_fixture() {
        validate_quote(VALID_QUOTE, "NSE_EQ|INE848E01016").expect("valid captured fixture");
    }

    #[test]
    fn validates_quote_envelope() {
        assert!(validate_quote(VALID_QUOTE, "NSE_EQ|INE848E01016").is_ok());
        assert!(validate_quote(br#"{"status":"error","data":{}}"#, "NSE_EQ|INE848E01016").is_err());
    }

    #[test]
    fn rejects_wrong_instrument_token() {
        let payload = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|WRONG","symbol":"NHPC","last_price":80.39,"volume":17190932}}}"#;
        assert!(validate_quote(payload, "NSE_EQ|INE848E01016").is_err());
    }

    #[test]
    fn rejects_missing_requested_instrument() {
        let payload = br#"{"status":"success","data":{"NSE_EQ:OTHER":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE00000000000","symbol":"OTHER","last_price":80.39,"volume":17190932}}}"#;
        assert!(validate_quote(payload, "NSE_EQ|INE848E01016").is_err());
    }

    #[test]
    fn rejects_duplicate_matching_records() {
        let payload = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932},"NSE_EQ:NHPC2":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932}}}"#;
        assert!(validate_quote(payload, "NSE_EQ|INE848E01016").is_err());
    }

    #[test]
    fn rejects_missing_required_quote_fields() {
        let no_symbol = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","last_price":80.39,"volume":17190932}}}"#;
        assert!(validate_quote(no_symbol, "NSE_EQ|INE848E01016").is_err());
        let no_ohlc = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932}}}"#;
        assert!(validate_quote(no_ohlc, "NSE_EQ|INE848E01016").is_err());
        let no_depth = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932}}}"#;
        assert!(validate_quote(no_depth, "NSE_EQ|INE848E01016").is_err());
    }

    #[test]
    fn rejects_invalid_rfc3339_quote_timestamp() {
        let payload = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"not-a-timestamp","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932}}}"#;
        assert!(validate_quote(payload, "NSE_EQ|INE848E01016").is_err());
    }

    #[test]
    fn rejects_invalid_negative_and_non_finite_numbers() {
        let negative_price = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":-1.0,"volume":17190932}}}"#;
        assert!(validate_quote(negative_price, "NSE_EQ|INE848E01016").is_err());
        let string_price = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":"80.39","volume":17190932}}}"#;
        assert!(validate_quote(string_price, "NSE_EQ|INE848E01016").is_err());
        let negative_ohlc_ts = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":-1},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932}}}"#;
        assert!(validate_quote(negative_ohlc_ts, "NSE_EQ|INE848E01016").is_err());
    }

    #[test]
    fn rejects_negative_or_non_integer_volume() {
        let negative = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":-1}}}"#;
        assert!(validate_quote(negative, "NSE_EQ|INE848E01016").is_err());
        let fractional = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":1.5}}}"#;
        assert!(validate_quote(fractional, "NSE_EQ|INE848E01016").is_err());
        let string_volume = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":"17190932"}}}"#;
        assert!(validate_quote(string_volume, "NSE_EQ|INE848E01016").is_err());
        let missing_volume = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39}}}"#;
        assert!(validate_quote(missing_volume, "NSE_EQ|INE848E01016").is_err());
        let zero_volume = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":0,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":0}}}"#;
        assert!(validate_quote(zero_volume, "NSE_EQ|INE848E01016").is_ok());
    }

    #[test]
    fn rejects_quote_ohlc_invariant_violations() {
        let payload = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":79.0,"low":80.0,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932}}}"#;
        assert!(validate_quote(payload, "NSE_EQ|INE848E01016").is_err());
    }

    #[test]
    fn rejects_missing_or_malformed_depth_arrays() {
        let missing_sell = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932}}}"#;
        assert!(validate_quote(missing_sell, "NSE_EQ|INE848E01016").is_err());
        let malformed = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":"nope","sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932}}}"#;
        assert!(validate_quote(malformed, "NSE_EQ|INE848E01016").is_err());
    }

    #[test]
    fn rejects_more_than_five_depth_levels() {
        let payload = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[{"quantity":0,"price":0.0,"orders":0},{"quantity":0,"price":0.0,"orders":0},{"quantity":0,"price":0.0,"orders":0},{"quantity":0,"price":0.0,"orders":0},{"quantity":0,"price":0.0,"orders":0},{"quantity":0,"price":0.0,"orders":0}],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932}}}"#;
        assert!(validate_quote(payload, "NSE_EQ|INE848E01016").is_err());
    }

    #[test]
    fn rejects_negative_depth_quantities_or_order_counts() {
        let payload = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[{"quantity":-1,"price":80.39,"orders":0}],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932}}}"#;
        assert!(validate_quote(payload, "NSE_EQ|INE848E01016").is_err());
        let orders_payload = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[{"quantity":0,"price":80.39,"orders":-1}],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932}}}"#;
        assert!(validate_quote(orders_payload, "NSE_EQ|INE848E01016").is_err());
    }

    #[test]
    fn rejects_negative_depth_price() {
        let payload = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[{"quantity":0,"price":-1.0,"orders":0}],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932}}}"#;
        assert!(validate_quote(payload, "NSE_EQ|INE848E01016").is_err());
    }

    #[test]
    fn accepts_valid_zero_valued_depth_entries() {
        let payload = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[{"quantity":0,"price":0.0,"orders":0}],"sell":[{"quantity":0,"price":0.0,"orders":0}]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932}}}"#;
        assert!(validate_quote(payload, "NSE_EQ|INE848E01016").is_ok());
    }

    #[test]
    fn accepts_valid_quote_with_optional_fields_absent() {
        let payload = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932}}}"#;
        validate_quote(payload, "NSE_EQ|INE848E01016").expect("minimal quote fixture");
    }

    #[test]
    fn validates_last_trade_time_when_present() {
        // Valid string-millis present in captured fixture (VALID_QUOTE has last_trade_time="1794219944214").
        assert!(validate_quote(VALID_QUOTE, "NSE_EQ|INE848E01016").is_ok());
        // RFC3339 string must be rejected as last_trade_time (not parsed as timestamp).
        let rfc3339 = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932,"last_trade_time":"2026-11-09T18:58:46+05:30"}}}"#;
        assert!(validate_quote(rfc3339, "NSE_EQ|INE848E01016").is_err());
        // Negative decimal string must be rejected.
        let negative = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932,"last_trade_time":"-1"}}}"#;
        assert!(validate_quote(negative, "NSE_EQ|INE848E01016").is_err());
        // Numeric (not string) must be rejected.
        let numeric = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932,"last_trade_time":1794219944214}}}"#;
        assert!(validate_quote(numeric, "NSE_EQ|INE848E01016").is_err());
    }

    #[test]
    fn accepts_absent_last_trade_time() {
        let payload = br#"{"status":"success","data":{"NSE_EQ:NHPC":{"ohlc":{"open":81.3,"high":81.38,"low":79.23,"close":80.39,"volume":17190932,"ts":1794162600000},"depth":{"buy":[],"sell":[]},"timestamp":"2026-11-09T18:58:46.473+05:30","instrument_token":"NSE_EQ|INE848E01016","symbol":"NHPC","last_price":80.39,"volume":17190932}}}"#;
        assert!(validate_quote(payload, "NSE_EQ|INE848E01016").is_ok());
    }

    #[test]
    fn subscription_is_binary_json_for_ltpc_mode() {
        let bytes = subscription_frame("NSE_EQ|INE848E01016").expect("subscription frame");
        let value: Value = serde_json::from_slice(&bytes).expect("valid JSON");
        assert_eq!(value["method"], "sub");
        assert_eq!(value["data"]["mode"], "ltpc");
        assert_eq!(value["data"]["instrumentKeys"][0], "NSE_EQ|INE848E01016");
    }

    #[test]
    fn malformed_frame_does_not_stop_next_frame() {
        let mut raw: Vec<u8> = Vec::new();
        let mut diag: Vec<u8> = Vec::new();
        let malformed = b"not protobuf at all";
        let (outcome1, counters1) = archive_and_diagnose_frame(
            malformed,
            1,
            "2026-11-09T13:28:47.000Z".to_owned(),
            "NSE_EQ|INE848E01016",
            &mut raw,
            &mut diag,
        )
        .expect("malformed must not be fatal");
        assert_eq!(outcome1, FrameDecodeOutcome::DecodeError);
        assert_eq!(counters1.decode_error_count, 1);
        assert_eq!(counters1.decoded_frame_count, 0);

        let valid_b64 =
            "EjQKE05TRV9FUXxJTkU4NDhFMDEwMTYSHQobCSlcj8L1GFRAEJbS3v6bNBgKIcP1KFyPQlRAGKD+/IOcNA==";
        let valid = BASE64.decode(valid_b64).expect("valid b64");
        let (outcome2, counters2) = archive_and_diagnose_frame(
            &valid,
            2,
            "2026-11-09T13:28:47.147Z".to_owned(),
            "NSE_EQ|INE848E01016",
            &mut raw,
            &mut diag,
        )
        .expect("valid frame must decode");
        assert_eq!(outcome2, FrameDecodeOutcome::Decoded);
        assert_eq!(counters2.decoded_frame_count, 1);
        assert_eq!(counters2.initial_feed_count, 1);
        assert_eq!(counters2.requested_instrument_feed_frames, 1);
        assert_eq!(counters2.requested_instrument_live_feed_frames, 0);

        let raw_lines: Vec<&[u8]> = raw
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(raw_lines.len(), 2);
        let diag_lines: Vec<&[u8]> = diag
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(diag_lines.len(), 2);

        let raw1: Value = serde_json::from_slice(raw_lines[0]).expect("raw1 json");
        let raw2: Value = serde_json::from_slice(raw_lines[1]).expect("raw2 json");
        let diag1: Value = serde_json::from_slice(diag_lines[0]).expect("diag1 json");
        let diag2: Value = serde_json::from_slice(diag_lines[1]).expect("diag2 json");
        assert_eq!(diag1["sequence"], 1);
        assert_eq!(diag2["sequence"], 2);
        assert_eq!(diag1["sha256"], raw1["sha256"]);
        assert_eq!(diag2["sha256"], raw2["sha256"]);
        assert_eq!(diag1["byte_len"], raw1["byte_len"]);
        assert_eq!(diag2["byte_len"], raw2["byte_len"]);
        assert_eq!(diag1["received_at_utc"], raw1["received_at_utc"]);
        assert_eq!(diag2["received_at_utc"], raw2["received_at_utc"]);
        assert_eq!(diag1["outcome"]["status"], "decode_error");
        assert_eq!(diag2["outcome"]["status"], "decoded");
        assert_eq!(diag2["outcome"]["response_type"], "initial_feed");
        assert_eq!(diag2["outcome"]["feed_keys"][0], "NSE_EQ|INE848E01016");

        let redecoded1 = BASE64
            .decode(raw1["payload_base64"].as_str().expect("b64"))
            .expect("re-decode b64");
        assert_eq!(
            sha256_hex(&redecoded1),
            raw1["sha256"].as_str().expect("sha").to_owned()
        );
        assert!(decode_feed_response_diagnostic(&redecoded1).is_err());
        let redecoded2 = BASE64
            .decode(raw2["payload_base64"].as_str().expect("b64"))
            .expect("re-decode b64");
        assert_eq!(
            sha256_hex(&redecoded2),
            raw2["sha256"].as_str().expect("sha").to_owned()
        );
        let rediag = decode_feed_response_diagnostic(&redecoded2).expect("re-diagnose");
        assert_eq!(rediag.response_type, ProtoFeedResponseType::InitialFeed);
    }

    #[test]
    fn default_decode_counters_are_zero() {
        let counters = ProtobufDecodeCounters::default();
        assert_eq!(counters.decoded_frame_count, 0);
        assert_eq!(counters.decode_error_count, 0);
        assert_eq!(counters.initial_feed_count, 0);
        assert_eq!(counters.live_feed_count, 0);
        assert_eq!(counters.market_info_count, 0);
        assert_eq!(counters.unknown_type_count, 0);
        assert_eq!(counters.requested_instrument_feed_frames, 0);
        assert_eq!(counters.requested_instrument_live_feed_frames, 0);
    }

    #[test]
    fn archive_writer_failure_returns_error() {
        struct FailingWriter;
        impl std::io::Write for FailingWriter {
            fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::other("flush failed"))
            }
        }
        let mut raw: Vec<u8> = Vec::new();
        let mut diag = FailingWriter;
        let result = archive_and_diagnose_frame(
            b"payload",
            1,
            "2026-11-09T13:28:47.000Z".to_owned(),
            "NSE_EQ|INE848E01016",
            &mut raw,
            &mut diag,
        );
        assert!(result.is_err());
        let mut raw_fail = FailingWriter;
        let mut diag_buf: Vec<u8> = Vec::new();
        let result2 = archive_and_diagnose_frame(
            b"payload",
            1,
            "2026-11-09T13:28:47.000Z".to_owned(),
            "NSE_EQ|INE848E01016",
            &mut raw_fail,
            &mut diag_buf,
        );
        assert!(result2.is_err());
    }

    #[test]
    fn token_debug_is_redacted() {
        let token = AnalyticsToken("secret-token-value".to_owned());
        assert!(!format!("{token:?}").contains("secret-token-value"));
        assert!(format!("{token:?}").contains("REDACTED"));
    }

    #[test]
    fn enforces_the_one_month_intraday_request_bound() {
        let mut args = Args {
            instrument_key: "NSE_EQ|INE848E01016".to_owned(),
            from_date: Date::new(2026, 10, 9).expect("date"),
            to_date: Date::new(2026, 11, 9).expect("date"),
            output_dir: PathBuf::from("var/upstox"),
            websocket_frames: 5,
            websocket_timeout_secs: 20,
            session_date: None,
        };
        assert!(validate_inputs(&args).is_ok());
        args.to_date = Date::new(2026, 11, 10).expect("date");
        assert!(validate_inputs(&args).is_err());
    }

    #[test]
    fn classifies_frame_limit_termination() {
        assert_eq!(
            determine_termination_reason(5, 5, false, false, false),
            TerminationReason::FrameLimit
        );
        assert_eq!(
            determine_termination_reason(10, 10, false, false, false),
            TerminationReason::FrameLimit
        );
    }

    #[test]
    fn classifies_timeout_and_remote_termination() {
        assert_eq!(
            determine_termination_reason(2, 10000, true, false, false),
            TerminationReason::Timeout
        );
        assert_eq!(
            determine_termination_reason(2, 10000, false, true, false),
            TerminationReason::RemoteClose
        );
        assert_eq!(
            determine_termination_reason(2, 10000, false, false, true),
            TerminationReason::StreamEnd
        );
        assert_eq!(
            determine_termination_reason(0, 10000, false, false, false),
            TerminationReason::StreamEnd
        );
    }
}
