//! Read-only Upstox Market Data Feed V3 recorder.
//!
//! The recorder stores each original binary WebSocket payload in the raw-frame
//! journal before projecting protobuf diagnostics. Local frame sequence is not
//! an Upstox provider sequence. No order route is present in this binary.

use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, bail, ensure};
use clap::{Parser, ValueEnum};
use futures_util::{SinkExt, StreamExt};
use jiff::Timestamp;
use market_archive::{
    DataOperation,
    DurableIngestor, //
    IngestOutcome,
    MAX_PAYLOAD_BYTES,
    ProductKey,
    Sequence,
    SequencePolicy,
    SourceFrame,
    SourceProductRegistry,
    UPSTOX_SOURCE_ID,
    verify_journal,
};
use reqwest::{Client, redirect};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::{signal, time::sleep};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use upstox_feed_proto::decode::decode_feed_response_diagnostic;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const AUTHORIZE_URL: &str = "https://api.upstox.com/v3/feed/market-data-feed/authorize";

/// A payload the archive cannot accept. This is a terminal capture outcome, not
/// a network failure: retrying the connection cannot make it valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OversizedFrame {
    size: usize,
    limit: usize,
}

impl std::fmt::Display for OversizedFrame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Upstox frame of {} bytes exceeds the archive payload limit of {} bytes",
            self.size, self.limit
        )
    }
}

impl std::error::Error for OversizedFrame {}

/// The shared ingress port refused a frame. Retrying the connection cannot
/// make a refused observation acceptable, so this is terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IngressRefusal;

impl std::fmt::Display for IngressRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("shared ingress refused an Upstox frame")
    }
}

impl std::error::Error for IngressRefusal {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConnectionFailure {
    OversizedFrame,
    IngressRefusal,
    Retryable,
}

fn classify_failure(error: &anyhow::Error) -> ConnectionFailure {
    if error.downcast_ref::<OversizedFrame>().is_some() {
        ConnectionFailure::OversizedFrame
    } else if error.downcast_ref::<IngressRefusal>().is_some() {
        ConnectionFailure::IngressRefusal
    } else {
        ConnectionFailure::Retryable
    }
}

fn validate_frame_size(size: usize) -> Result<(), OversizedFrame> {
    if size > MAX_PAYLOAD_BYTES {
        return Err(OversizedFrame {
            size,
            limit: MAX_PAYLOAD_BYTES,
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum FeedMode {
    Ltpc,
    Full,
}

impl std::fmt::Display for FeedMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FeedMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ltpc => "ltpc",
            Self::Full => "full",
        }
    }

    fn max_symbols(self) -> usize {
        match self {
            Self::Ltpc => 5_000,
            Self::Full => 2_000,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "upstox-recorder",
    about = "Capture Upstox Market Data Feed V3 binary frames"
)]
struct Args {
    /// Explicit path to the source-product rights registry. No default is inferred.
    #[arg(long)]
    registry: PathBuf,
    /// Text file with one Upstox instrument_key per line.
    #[arg(long, default_value = "configs/providers/upstox-watchlist.txt")]
    instrument_file: PathBuf,
    #[arg(long, value_enum, default_value_t = FeedMode::Full)]
    mode: FeedMode,
    #[arg(long, default_value = "data/market-sessions/upstox")]
    output_root: PathBuf,
    /// Maximum reconnect attempts after the first connection.
    #[arg(long, default_value_t = 10)]
    max_reconnects: u32,
    /// Non-secret reference to the applicable Upstox market-data entitlement.
    #[arg(long, env = "NIRNAY_UPSTOX_ENTITLEMENT_REF")]
    entitlement_ref: String,
}

#[derive(Serialize)]
struct Manifest {
    schema_version: u32,
    provider: &'static str,
    feed_protocol: &'static str,
    mode: String,
    instrument_keys: Vec<String>,
    entitlement_ref: String,
    started_at_utc: String,
    ended_at_utc: String,
    duration_ns: u128,
    connection_count: u64,
    frame_count: u64,
    decode_error_count: u64,
    terminal_error: Option<String>,
    raw_journal: String,
    raw_journal_sha256: String,
    raw_journal_frame_count: u64,
    provider_sequence_available: bool,
    capture_sequence_semantics: &'static str,
    limitations: Vec<&'static str>,
}

#[derive(Serialize)]
struct Observation<'a> {
    schema_version: u32,
    capture_sequence: u64,
    received_at_utc_ns: i64,
    provider_current_ts_ms: i64,
    response_type_raw: i32,
    feed_keys: &'a [String],
    feeds: Vec<FeedObservation<'a>>,
}

#[derive(Serialize)]
struct FeedObservation<'a> {
    key: &'a str,
    kind: &'static str,
    request_mode_raw: i32,
}

fn create_private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)
}

fn set_private_permissions(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

struct Token(String);

impl Token {
    fn from_environment() -> anyhow::Result<Self> {
        let value = std::env::var("UPSTOX_ACCESS_TOKEN")
            .context("set UPSTOX_ACCESS_TOKEN in the process environment")?;
        ensure!(
            !value.is_empty() && value.trim() == value,
            "UPSTOX_ACCESS_TOKEN is empty or has surrounding whitespace"
        );
        Ok(Self(value))
    }
}

struct Session {
    run_dir: PathBuf,
    journal_path: PathBuf,
    ingestor: DurableIngestor,
    observations: BufWriter<File>,
    lifecycle: BufWriter<File>,
    started_utc: String,
    started_instant: Instant,
    instrument_keys: Vec<String>,
    mode: FeedMode,
    entitlement_ref: String,
    connection_count: u64,
    frame_count: u64,
    decode_error_count: u64,
    terminal_error: Option<String>,
}

impl Session {
    fn create(args: &Args, instrument_keys: Vec<String>) -> anyhow::Result<Self> {
        let started = Timestamp::now();
        let stamp = started.strftime("%Y%m%dT%H%M%S%.fZ");
        let run_dir = args.output_root.join(format!("upstox-{stamp}"));
        fs::create_dir_all(&args.output_root)
            .with_context(|| format!("creating output root {}", args.output_root.display()))?;
        set_private_permissions(&args.output_root)?;
        fs::create_dir(&run_dir)
            .with_context(|| format!("creating unique session directory {}", run_dir.display()))?;
        set_private_permissions(&run_dir)?;
        let journal_path = run_dir.join("raw-frames.jsonl");
        // One authoritative journal, owned by the shared ingress port. The
        // recorder-local sequence is a capture-session sequence, never a
        // provider sequence.
        let ingestor = DurableIngestor::create_new(
            &journal_path,
            UPSTOX_SOURCE_ID,
            SequencePolicy::CaptureSession,
        )
        .context("creating shared-ingress raw-frame journal")?;
        let observations = BufWriter::new(create_private_file(
            &run_dir.join("decoded-observations.jsonl"),
        )?);
        let lifecycle = BufWriter::new(create_private_file(
            &run_dir.join("connection-events.jsonl"),
        )?);
        Ok(Self {
            run_dir,
            journal_path,
            ingestor,
            observations,
            lifecycle,
            started_utc: started.strftime("%Y-%m-%dT%H:%M:%S%.9fZ").to_string(),
            started_instant: Instant::now(),
            instrument_keys,
            mode: args.mode,
            entitlement_ref: args.entitlement_ref.clone(),
            connection_count: 0,
            frame_count: 0,
            decode_error_count: 0,
            terminal_error: None,
        })
    }

    fn lifecycle(&mut self, event: &str, attempt: u64, detail: Option<&str>) -> anyhow::Result<()> {
        let row = json!({
            "schema_version": 1,
            "event": event,
            "attempt": attempt,
            "at_utc": Timestamp::now()
                .strftime("%Y-%m-%dT%H:%M:%S%.9fZ")
                .to_string(),
            "detail": detail,
        });
        serde_json::to_writer(&mut self.lifecycle, &row)?;
        self.lifecycle.write_all(b"\n")?;
        self.lifecycle.flush()?;
        Ok(())
    }

    fn frame(
        &mut self,
        payload: &[u8],
        received_utc_ns: i64,
        received_monotonic_ns: u64,
    ) -> anyhow::Result<()> {
        validate_frame_size(payload.len())?;
        let capture_sequence = self
            .frame_count
            .checked_add(1)
            .context("Upstox capture sequence exhausted")?;
        let source_frame = SourceFrame {
            source_id: UPSTOX_SOURCE_ID.to_owned(),
            provider_instrument_id: "__multi_instrument_frame__".to_owned(),
            entitlement_ref: self.entitlement_ref.clone(),
            received_utc_nanos: received_utc_ns,
            received_monotonic_nanos: received_monotonic_ns,
            exchange_utc_nanos: None,
            sequence: Some(Sequence::capture_session(capture_sequence)),
            payload: payload.to_vec(),
        };
        match self
            .ingestor
            .ingest_one(source_frame)
            .context("durably appending Upstox raw frame through shared ingress")?
        {
            IngestOutcome::Committed { .. } => {}
            IngestOutcome::DuplicateSkipped
            | IngestOutcome::ConflictRejected
            | IngestOutcome::MalformedRejected => {
                return Err(IngressRefusal.into());
            }
        }
        self.frame_count = self.ingestor.checkpoint().committed_frames;

        match decode_feed_response_diagnostic(payload) {
            Ok(diagnostic) => {
                let feeds = diagnostic
                    .feeds
                    .iter()
                    .map(|feed| FeedObservation {
                        key: &feed.key,
                        kind: match feed.kind {
                            upstox_feed_proto::decode::FeedKind::Ltpc => "ltpc",
                            upstox_feed_proto::decode::FeedKind::FullFeed => "full_feed",
                            upstox_feed_proto::decode::FeedKind::FirstLevelWithGreeks => {
                                "first_level_with_greeks"
                            }
                            upstox_feed_proto::decode::FeedKind::Unset => "unset",
                        },
                        request_mode_raw: feed.request_mode_raw,
                    })
                    .collect();
                let row = Observation {
                    schema_version: 1,
                    capture_sequence: self.frame_count,
                    received_at_utc_ns: received_utc_ns,
                    provider_current_ts_ms: diagnostic.current_ts,
                    response_type_raw: diagnostic.response_type_raw,
                    feed_keys: &diagnostic.feed_keys,
                    feeds,
                };
                serde_json::to_writer(&mut self.observations, &row)?;
                self.observations.write_all(b"\n")?;
            }
            Err(error) => {
                self.decode_error_count += 1;
                let row = json!({
                    "schema_version": 1,
                    "capture_sequence": self.frame_count,
                    "received_at_utc_ns": received_utc_ns,
                    "decode_error": error.to_string(),
                });
                serde_json::to_writer(&mut self.observations, &row)?;
                self.observations.write_all(b"\n")?;
            }
        }
        Ok(())
    }

    fn finish(mut self) -> anyhow::Result<PathBuf> {
        self.observations.flush()?;
        self.observations.get_ref().sync_all()?;
        self.lifecycle.flush()?;
        self.lifecycle.get_ref().sync_all()?;
        drop(self.ingestor);
        let summary = verify_journal(&self.journal_path).context("verifying Upstox raw journal")?;
        let mut source = File::open(&self.journal_path)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 1024 * 1024];
        loop {
            let count = source.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        let digest = hasher.finalize();
        let raw_journal_sha256 = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let manifest = Manifest {
            schema_version: 1,
            provider: "Upstox",
            feed_protocol: "Market Data Feed V3; protobuf binary",
            mode: self.mode.as_str().to_owned(),
            instrument_keys: self.instrument_keys,
            entitlement_ref: self.entitlement_ref,
            started_at_utc: self.started_utc,
            ended_at_utc: Timestamp::now()
                .strftime("%Y-%m-%dT%H:%M:%S%.9fZ")
                .to_string(),
            duration_ns: self.started_instant.elapsed().as_nanos(),
            connection_count: self.connection_count,
            frame_count: self.frame_count,
            decode_error_count: self.decode_error_count,
            raw_journal: "raw-frames.jsonl".to_owned(),
            raw_journal_sha256,
            raw_journal_frame_count: summary.frame_count,
            provider_sequence_available: false,
            capture_sequence_semantics: "local receive-order counter; not a provider sequence",
            terminal_error: self.terminal_error,
            limitations: vec![
                "A successful connection does not verify data redistribution rights.",
                "The provider envelope currentTs is retained in decoded diagnostics; per-instrument exchange timestamps are not inferred.",
                "Unknown protobuf fields remain in raw frames but may not appear in the diagnostic projection.",
                "The first V4 recorder does not normalize instrument identities, prices or quantities into trading-core events.",
            ],
        };
        let manifest_path = self.run_dir.join("manifest.json");
        let temp_path = self.run_dir.join("manifest.json.tmp");
        {
            let file = create_private_file(&temp_path)?;
            let mut writer = BufWriter::new(file);
            serde_json::to_writer_pretty(&mut writer, &manifest)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
            writer.get_ref().sync_all()?;
        }
        fs::rename(&temp_path, &manifest_path)?;
        let dir = File::open(&self.run_dir)?;
        dir.sync_all()?;
        Ok(self.run_dir)
    }
}

fn registry_key(mode: FeedMode) -> ProductKey {
    let product = match mode {
        FeedMode::Ltpc => "ltpc",
        FeedMode::Full => "full",
    };
    ProductKey::new("upstox", product, "NSE", "Cash")
}

/// The only production path to a session. Authorization happens before any
/// directory, journal or manifest file is created; a denial leaves no files.
fn create_authorized_session(args: &Args, instrument_keys: Vec<String>) -> anyhow::Result<Session> {
    let registry = SourceProductRegistry::load(&args.registry)
        .with_context(|| format!("loading rights registry {}", args.registry.display()))?;
    let key = registry_key(args.mode);
    registry
        .authorize(&key, DataOperation::Capture)
        .context("capture is not authorized by the registry")?;
    registry
        .authorize(&key, DataOperation::Store)
        .context("store is not authorized by the registry")?;
    Session::create(args, instrument_keys)
}

fn read_watchlist(path: &Path, mode: FeedMode) -> anyhow::Result<Vec<String>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading watchlist {}", path.display()))?;
    let mut keys = Vec::new();
    let mut seen = BTreeSet::new();
    for (line_number, line) in text.lines().enumerate() {
        let value = line.trim();
        if value.is_empty() || value.starts_with('#') {
            continue;
        }
        ensure!(
            value.contains('|'),
            "watchlist line {} is not an Upstox instrument_key",
            line_number + 1
        );
        ensure!(
            seen.insert(value.to_owned()),
            "duplicate Upstox instrument key at line {}",
            line_number + 1
        );
        keys.push(value.to_owned());
    }
    ensure!(!keys.is_empty(), "Upstox watchlist is empty");
    ensure!(
        keys.len() <= mode.max_symbols(),
        "{} mode allows at most {} symbols",
        mode.as_str(),
        mode.max_symbols()
    );
    Ok(keys)
}

async fn authorize(client: &Client, token: &Token) -> anyhow::Result<String> {
    #[derive(Deserialize)]
    struct Envelope {
        data: Option<Data>,
        message: Option<String>,
    }
    #[derive(Deserialize)]
    struct Data {
        authorized_redirect_uri: String,
    }
    let response = client
        .get(AUTHORIZE_URL)
        .bearer_auth(&token.0)
        .header("Accept", "application/json")
        .send()
        .await
        .context("requesting Upstox authorized WebSocket URL")?;
    let status = response.status();
    ensure!(
        status.is_success(),
        "Upstox authorize endpoint returned HTTP {status}"
    );
    let body: Envelope = response
        .json()
        .await
        .context("decoding Upstox authorize response")?;
    let Some(data) = body.data else {
        bail!(
            "Upstox authorize response omitted data: {}",
            body.message
                .unwrap_or_else(|| "unknown response".to_owned())
        );
    };
    let parsed = reqwest::Url::parse(&data.authorized_redirect_uri)
        .context("parsing Upstox WebSocket URL")?;
    ensure!(
        parsed.scheme() == "wss",
        "Upstox authorized URL must use wss"
    );
    let host = parsed.host_str().unwrap_or_default();
    ensure!(
        host == "upstox.com" || host.ends_with(".upstox.com"),
        "Upstox authorize returned an unexpected host"
    );
    Ok(data.authorized_redirect_uri)
}

async fn run_connection(
    client: &Client,
    token: &Token,
    session: &mut Session,
    stop: &mut std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<()>> + Send>>,
    attempt: u64,
) -> anyhow::Result<bool> {
    let authorized_url = authorize(client, token).await?;
    let (mut socket, _response) = connect_async(authorized_url.as_str())
        .await
        .map_err(|_| anyhow::anyhow!("connecting to Upstox Market Data Feed V3 failed"))?;
    let subscription = json!({
        "guid": format!("nirnay-{}-{attempt}", Timestamp::now().as_millisecond()),
        "method": "sub",
        "data": {
            "mode": session.mode.as_str(),
            "instrumentKeys": &session.instrument_keys,
        }
    });
    let bytes = serde_json::to_vec(&subscription)?;
    socket
        .send(Message::Binary(bytes.into()))
        .await
        .context("sending Upstox binary subscription")?;
    session.connection_count += 1;
    session.lifecycle(
        "connected_and_subscribed",
        attempt,
        Some("subscription sent as binary JSON control frame"),
    )?;
    loop {
        tokio::select! {
            result = stop.as_mut() => {
                result.context("waiting for SIGINT")?;
                session.lifecycle("operator_stop", attempt, None)?;
                let _ = socket.close(None).await;
                return Ok(true);
            }
            message = socket.next() => {
                match message {
                    Some(Ok(Message::Binary(payload))) => {
                        let utc_ns = i64::try_from(Timestamp::now().as_nanosecond())
                            .ok()
                            .context("UTC timestamp outside nanosecond range")?;
                        let mono_ns = u64::try_from(session.started_instant.elapsed().as_nanos()).context("monotonic duration overflow")?;
                        session.frame(&payload, utc_ns, mono_ns)?;
                    }
                    Some(Ok(Message::Ping(payload))) => socket.send(Message::Pong(payload)).await.context("answering Upstox ping")?,
                    Some(Ok(Message::Close(frame))) => {
                        session.lifecycle("server_closed", attempt, Some(&format!("{frame:?}")))?;
                        return Ok(false);
                    }
                    Some(Ok(Message::Text(_))) => {
                        session.lifecycle("unexpected_text_frame", attempt, Some("text frame preserved only as a lifecycle observation"))?;
                    }
                    Some(Ok(Message::Pong(_))) => (),
                    Some(Ok(Message::Frame(_))) => (),
                    Some(Err(error)) => {
                        session.lifecycle("socket_error", attempt, Some("websocket read failed; diagnostic details omitted"))?;
                        return Err(error).context("Upstox WebSocket read failed");
                    }
                    None => {
                        session.lifecycle("stream_ended", attempt, None)?;
                        return Ok(false);
                    }
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let token = Token::from_environment()?;
    ensure!(
        !args.entitlement_ref.trim().is_empty(),
        "NIRNAY_UPSTOX_ENTITLEMENT_REF is required"
    );
    let instrument_keys = read_watchlist(&args.instrument_file, args.mode)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(redirect::Policy::none())
        .user_agent(concat!(
            "nirnay-upstox-recorder/",
            env!("CARGO_PKG_VERSION")
        ))
        .build()?;
    let mut session = create_authorized_session(&args, instrument_keys)?;
    let mut stop: std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<()>> + Send>> =
        Box::pin(signal::ctrl_c());

    for attempt in 0..=args.max_reconnects {
        if attempt > 0 {
            let backoff = 1_u64.checked_shl(attempt.min(6)).unwrap_or(64);
            session.lifecycle(
                "reconnect_wait",
                u64::from(attempt),
                Some(&format!("backoff_seconds={backoff}")),
            )?;
            tokio::select! {
                result = stop.as_mut() => { result.context("waiting for SIGINT")?; break; }
                _ = sleep(Duration::from_secs(backoff)) => (),
            }
        }
        match run_connection(&client, &token, &mut session, &mut stop, u64::from(attempt)).await {
            Ok(true) => break,
            Ok(false) if attempt < args.max_reconnects => (),
            Ok(false) => {
                session.terminal_error =
                    Some("reconnect budget exhausted after server closed the stream".to_owned());
                session.lifecycle("reconnect_budget_exhausted", u64::from(attempt), None)?;
            }
            Err(error) => {
                if matches!(
                    classify_failure(&error),
                    ConnectionFailure::OversizedFrame | ConnectionFailure::IngressRefusal
                ) {
                    session.terminal_error = Some(format!("terminal capture failure: {error}"));
                    session.lifecycle(
                        "terminal_capture_failure",
                        u64::from(attempt),
                        Some(&error.to_string()),
                    )?;
                    break;
                }
                if attempt < args.max_reconnects {
                    session.lifecycle(
                        "reconnect_after_error",
                        u64::from(attempt),
                        Some("network_or_protocol_error; diagnostic details omitted"),
                    )?;
                } else {
                    session.terminal_error = Some(
                        "reconnect budget exhausted after network or protocol error".to_owned(),
                    );
                    session.lifecycle(
                        "reconnect_budget_exhausted_after_error",
                        u64::from(attempt),
                        Some("network_or_protocol_error; diagnostic details omitted"),
                    )?;
                }
            }
        }
    }
    let terminal_error = session.terminal_error.clone();
    let path = session.finish()?;
    writeln!(io::stdout().lock(), "{}", path.display())?;
    if terminal_error.is_some() {
        bail!(
            "Upstox capture ended with a terminal error; inspect the session events at {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(all(test, feature = "market-shaped-tests"))]
mod tests {
    use super::{
        Args, //
        ConnectionFailure,
        DurableIngestor,
        FeedMode,
        IngressRefusal,
        OversizedFrame,
        SequencePolicy,
        Session,
        UPSTOX_SOURCE_ID,
        classify_failure,
        create_authorized_session,
        validate_frame_size,
    };
    use base64::Engine as _;
    use market_archive::{
        MAX_PAYLOAD_BYTES, //
        ProductKey,
        SourceProductRegistry,
    };
    use std::path::PathBuf;

    const DENY_REGISTRY: &str =
        include_str!("../../../../configs/providers/source-products.v1.json");

    fn registry_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../configs/providers/source-products.v1.json")
    }

    fn replay_source(path: &std::path::Path) -> market_archive::JournalReplaySource {
        let text = DENY_REGISTRY
            .replace("\"state\": \"UNKNOWN\"", "\"state\": \"PERMITTED\"")
            .replace("\"verified\": false", "\"verified\": true");
        let registry = SourceProductRegistry::from_bytes(text.as_bytes()).expect("test registry");
        let key = ProductKey::new("upstox", "full", "NSE", "Cash");
        market_archive::JournalReplaySource::open(path, UPSTOX_SOURCE_ID, &registry, &key)
            .expect("replay source")
    }

    #[test]
    fn payload_limit_is_the_shared_archive_limit() {
        assert_eq!(MAX_PAYLOAD_BYTES, 512 * 1024, "one authoritative limit");
        assert!(
            validate_frame_size(MAX_PAYLOAD_BYTES).is_ok(),
            "a frame at the limit is accepted"
        );
        assert_eq!(
            validate_frame_size(MAX_PAYLOAD_BYTES + 1),
            Err(OversizedFrame {
                size: MAX_PAYLOAD_BYTES + 1,
                limit: MAX_PAYLOAD_BYTES,
            }),
            "one byte over the limit is rejected"
        );
    }

    #[test]
    fn oversized_frame_is_not_retryable() {
        let oversized: anyhow::Error = OversizedFrame { size: 9, limit: 8 }.into();
        assert!(
            classify_failure(&oversized) == ConnectionFailure::OversizedFrame,
            "an oversized frame is terminal"
        );
        let network = anyhow::anyhow!("connection reset by peer");
        assert!(
            classify_failure(&network) == ConnectionFailure::Retryable,
            "network errors remain retryable"
        );
    }

    #[test]
    fn oversized_frame_error_names_both_sizes() {
        let message = OversizedFrame { size: 9, limit: 8 }.to_string();
        assert_eq!(
            message, "Upstox frame of 9 bytes exceeds the archive payload limit of 8 bytes",
            "the diagnostic names both sizes exactly"
        );
    }
    #[test]
    fn recorder_writes_one_shared_ingress_journal() {
        let root = std::env::temp_dir().join(format!(
            "nirnay-upstox-ingress-{}-{}",
            std::process::id(),
            "one-journal"
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp root");
        let args = Args {
            registry: registry_path(),
            instrument_file: root.join("watchlist.txt"),
            mode: FeedMode::Full,
            output_root: root.clone(),
            max_reconnects: 0,
            entitlement_ref: "entitlement:test:upstox".to_owned(),
        };
        let mut session = Session::create(&args, vec!["NSE_EQ|TEST".to_owned()])
            .expect("session with shared ingress");
        session
            .frame(b"original-protobuf-bytes", 100, 1)
            .expect("first frame");
        session
            .frame(b"second-original-bytes", 200, 2)
            .expect("second frame");
        assert_eq!(
            session.frame_count, 2,
            "capture sequence tracks committed frames"
        );
        let frames = market_archive::scan_journal(&session.journal_path)
            .expect("journal scan")
            .frames;
        assert_eq!(frames.len(), 2, "exactly one raw journal has two records");
        assert!(
            frames.iter().all(|frame| frame.source == UPSTOX_SOURCE_ID),
            "journal frames carry the Upstox ingress source identity"
        );
        assert_eq!(
            frames[0].payload_base64,
            base64::engine::general_purpose::STANDARD.encode(b"original-protobuf-bytes"),
            "the exact original protobuf bytes survive in the authoritative journal"
        );
        let files: Vec<_> = std::fs::read_dir(&session.run_dir)
            .expect("session files")
            .filter_map(|entry| entry.ok().map(|entry| entry.file_name()))
            .collect();
        assert_eq!(
            files
                .iter()
                .filter(|name| name.to_string_lossy().ends_with(".jsonl"))
                .count(),
            3,
            "raw, decoded and lifecycle journals are distinct; no duplicate raw journal"
        );
        let path = session.finish().expect("session manifest");
        assert!(
            path.join("manifest.json").exists(),
            "session manifest is preserved"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn recorder_journal_restart_replays_without_new_commits() {
        let root = std::env::temp_dir().join(format!(
            "nirnay-upstox-ingress-{}-{}",
            std::process::id(),
            "restart"
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp root");
        let args = Args {
            registry: registry_path(),
            instrument_file: root.join("watchlist.txt"),
            mode: FeedMode::Ltpc,
            output_root: root.clone(),
            max_reconnects: 0,
            entitlement_ref: "entitlement:test:upstox".to_owned(),
        };
        let mut session = Session::create(&args, vec!["NSE_EQ|TEST".to_owned()])
            .expect("session with shared ingress");
        session.frame(b"first", 100, 1).expect("first frame");
        let journal_path = session.journal_path.clone();
        session.finish().expect("manifest");

        let mut replay = replay_source(&journal_path);
        let mut ingestor = DurableIngestor::resume(
            &journal_path,
            UPSTOX_SOURCE_ID,
            SequencePolicy::CaptureSession,
        )
        .expect("restarted ingestor");
        assert_eq!(
            ingestor.checkpoint().committed_frames,
            1,
            "restart recovers the committed frame count"
        );
        let summary = ingestor.replay(&mut replay).expect("replay");
        assert_eq!(
            summary.committed, 0,
            "restart does not re-commit captured frames"
        );
        assert_eq!(
            summary.duplicates, 1,
            "replay resolves the committed frame as a duplicate"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn ingress_refusal_is_not_retryable() {
        let refusal: anyhow::Error = IngressRefusal.into();
        assert!(
            classify_failure(&refusal) == ConnectionFailure::IngressRefusal,
            "a refused frame terminates capture instead of reconnecting"
        );
    }

    #[test]
    fn deny_all_registry_refuses_recorder_file_creation() {
        let root =
            std::env::temp_dir().join(format!("nirnay-upstox-rights-{}-deny", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp root");
        let registry_file = root.join("deny-registry.json");
        std::fs::write(&registry_file, DENY_REGISTRY).expect("deny registry");
        let args = Args {
            registry: registry_file.clone(),
            instrument_file: root.join("watchlist.txt"),
            mode: FeedMode::Full,
            output_root: root.join("sessions"),
            max_reconnects: 0,
            entitlement_ref: "entitlement:test:upstox".to_owned(),
        };
        let denied = create_authorized_session(&args, vec!["NSE_EQ|TEST".to_owned()]);
        assert!(denied.is_err(), "a deny-all registry refuses the session");
        assert!(
            !args.output_root.exists(),
            "no session directory is created on denial"
        );
        assert_eq!(
            std::fs::read_dir(&root).expect("temp entries").count(),
            1,
            "only the registry file exists; no journal, manifest or observations"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn authorized_registry_allows_recorder_file_creation() {
        let root =
            std::env::temp_dir().join(format!("nirnay-upstox-rights-{}-allow", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp root");
        let permitted = DENY_REGISTRY
            .replace("\"state\": \"UNKNOWN\"", "\"state\": \"PERMITTED\"")
            .replace("\"verified\": false", "\"verified\": true");
        let registry_file = root.join("permitted-registry.json");
        std::fs::write(&registry_file, permitted).expect("test registry");
        let args = Args {
            registry: registry_file,
            instrument_file: root.join("watchlist.txt"),
            mode: FeedMode::Full,
            output_root: root.join("sessions"),
            max_reconnects: 0,
            entitlement_ref: "entitlement:test:upstox".to_owned(),
        };
        let session = create_authorized_session(&args, vec!["NSE_EQ|TEST".to_owned()])
            .expect("test-only permitted registry");
        assert!(
            session.journal_path.exists(),
            "the authorized session opens its authoritative journal"
        );
        let path = session.finish().expect("manifest");
        assert!(
            path.join("manifest.json").exists(),
            "the authorized session writes its manifest"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
