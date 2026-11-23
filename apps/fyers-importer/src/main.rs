#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Offline FYERS session importer.
//!
//! The Python SDK recorder remains the capture authority. This tool verifies a
//! completed session's manifest and hash chain, validates every record, and
//! re-encodes each callback observation through the shared ingress port. It
//! never connects to FYERS and does not project canonical market events.

use std::{
    fs::File,
    io::{
        BufRead, //
        BufReader,
    },
    path::{
        Path, //
        PathBuf,
    },
};

mod mbp;
use anyhow::{
    Context, //
    bail,
    ensure,
};
use base64::Engine as _;
use clap::Parser;
use market_archive::{
    DurableIngestor, //
    FYERS_SOURCE_ID,
    IngestOutcome,
    JournalReader,
    Sequence,
    SequencePolicy,
    SourceFrame,
};
use serde_json::{
    Map,   //
    Value, //
    json,
};
use sha2::{
    Digest, //
    Sha256,
};

#[derive(Debug, Parser)]
#[command(
    name = "nirnay-fyers-import",
    about = "Import a verified FYERS SDK session into the shared raw journal"
)]
struct Args {
    /// Completed FYERS session directory containing manifest.json and its journal.
    #[arg(long)]
    session: PathBuf,
    /// Authoritative shared raw journal to append to.
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub session_id: String,
    pub records: usize,
    pub imported: usize,
    pub duplicates: usize,
    /// Count of SDK depth callbacks that produced a typed five-level MBP
    /// snapshot. Non-depth callbacks are not counted.
    pub depth_snapshots: usize,
}

pub fn canonical_json(value: &Value) -> anyhow::Result<Vec<u8>> {
    serde_json::to_vec(value).context("canonical JSON serialization")
}

fn hex_bytes(text: &str) -> anyhow::Result<Vec<u8>> {
    ensure!(
        text.len().is_multiple_of(2),
        "odd-length hexadecimal string"
    );
    let mut output = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    for index in (0..bytes.len()).step_by(2) {
        let high = (bytes[index] as char)
            .to_digit(16)
            .context("invalid hex digit")?;
        let low = (bytes[index + 1] as char)
            .to_digit(16)
            .context("invalid hex digit")?;
        output.push((high * 16 + low) as u8);
    }
    Ok(output)
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Verify the session manifest and the full hash chain before any import.
pub fn verify_session(session_dir: &Path) -> anyhow::Result<(Value, Vec<Value>)> {
    let manifest_path = session_dir.join("manifest.json");
    let manifest_bytes = std::fs::read(&manifest_path).context("reading session manifest")?;
    let manifest: Value =
        serde_json::from_slice(&manifest_bytes).context("parsing manifest JSON")?;
    ensure!(
        manifest.get("provider").and_then(|value| value.as_str()) == Some("FYERS"),
        "session manifest is not a FYERS capture"
    );
    ensure!(
        manifest
            .get("journal_chain_valid")
            .and_then(|value| value.as_bool())
            == Some(true),
        "session manifest does not claim a valid journal chain"
    );
    ensure!(
        manifest.get("writer_error").is_none_or(Value::is_null),
        "session ended with a writer error"
    );
    ensure!(
        manifest
            .get("provider_sequence_available")
            .and_then(|value| value.as_bool())
            == Some(false),
        "FYERS local sequence must be declared as capture-session sequence"
    );

    let journal_name = manifest
        .get("journal_file")
        .and_then(|value| value.as_str())
        .context("manifest has no journal_file")?;
    let journal_path = session_dir.join(journal_name);
    let journal_bytes = std::fs::read(&journal_path).context("reading FYERS journal")?;
    let declared_digest = manifest
        .get("journal_sha256")
        .and_then(|value| value.as_str())
        .context("manifest has no journal_sha256")?;
    ensure!(
        sha256_hex(&journal_bytes) == declared_digest,
        "FYERS journal bytes do not match the manifest digest"
    );

    let mut previous = [0_u8; 32];
    let mut records = Vec::new();
    let mut declared_head = None;
    for line in BufReader::new(File::open(&journal_path)?).split(b'\n') {
        let line = line.context("reading journal line")?;
        if line.is_empty() {
            continue;
        }
        let mut record: Map<String, Value> =
            serde_json::from_slice(&line).context("parsing journal record")?;
        let hash_value = record
            .remove("record_hash")
            .context("journal record has no record_hash")?;
        let declared_hash = hash_value
            .as_str()
            .context("record_hash is not a string")?
            .to_owned();
        let canonical = canonical_json(&Value::Object(record.clone()))
            .context("canonicalizing journal record")?;
        let mut input = previous.to_vec();
        input.extend_from_slice(&canonical);
        let computed = sha256_hex(&input);
        ensure!(
            computed == declared_hash,
            "FYERS journal hash chain is broken at record {}",
            records.len() + 1
        );
        previous = hex_bytes(&declared_hash)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid record hash length"))?;
        declared_head = Some(declared_hash);
        records.push(Value::Object(record));
    }
    let declared_count = manifest
        .get("record_count")
        .and_then(|value| value.as_u64())
        .context("manifest has no record_count")?;
    ensure!(
        records.len() as u64 == declared_count,
        "journal record count does not match manifest"
    );
    ensure!(
        manifest
            .get("journal_head_hash")
            .and_then(|value| value.as_str())
            == declared_head.as_deref(),
        "journal head hash does not match manifest"
    );
    Ok((manifest, records))
}

/// Highest capture sequence already committed for this session. Records are
/// imported in ascending sequence order, so this is a resumable prefix fence:
/// an interrupted import continues at the next record instead of being
/// mistaken for a completed one.
fn imported_high_water(output: &Path, session_id: &str) -> anyhow::Result<Option<u64>> {
    if !output.exists() {
        return Ok(None);
    }
    let mut reader = JournalReader::new(BufReader::new(File::open(output)?));
    let marker = format!("fyers-session:{session_id}");
    let mut highest = None;
    while let Some(frame) = reader.next_frame()? {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&frame.payload_base64)
            .context("decoding shared journal payload")?;
        let payload: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if payload
            .get("session_marker")
            .and_then(|value| value.as_str())
            != Some(marker.as_str())
        {
            continue;
        }
        if let Some(sequence) = payload
            .get("capture_sequence")
            .and_then(|value| value.as_u64())
        {
            highest = Some(highest.map_or(sequence, |prior: u64| prior.max(sequence)));
        }
    }
    Ok(highest)
}

/// Import a verified session. Repeat imports are no-ops.
pub fn import_session(session_dir: &Path, output: &Path) -> anyhow::Result<ImportReport> {
    let (manifest, records) = verify_session(session_dir)?;
    let session_id = manifest
        .get("run_id")
        .and_then(|value| value.as_str())
        .context("manifest has no run_id")?
        .to_owned();
    let high_water = imported_high_water(output, &session_id)?;
    let mut ingestor = if output.exists() {
        DurableIngestor::resume(output, FYERS_SOURCE_ID, SequencePolicy::CaptureSession)
            .context("resuming shared journal")?
    } else {
        DurableIngestor::create_new(output, FYERS_SOURCE_ID, SequencePolicy::CaptureSession)
            .context("creating shared journal")?
    };
    let marker = format!("fyers-session:{session_id}");
    let mut report = ImportReport {
        session_id: session_id.clone(),
        records: records.len(),
        imported: 0,
        duplicates: 0,
        depth_snapshots: 0,
    };
    for record in records {
        let sequence = record
            .get("capture_sequence")
            .and_then(|value| value.as_u64())
            .context("journal record has no capture_sequence")?;
        ensure!(
            record
                .get("event_type")
                .and_then(|value| value.as_str())
                .is_some_and(|value| !value.is_empty()),
            "verified record has no event type"
        );
        ensure!(
            record.get("payload").is_some_and(Value::is_object),
            "verified record has no callback payload object"
        );
        if high_water.is_some_and(|committed| sequence <= committed) {
            report.duplicates += 1;
            continue;
        }
        // Typed projection of depth callbacks. The kernel envelope keeps the
        // SDK-callback origin; this does not promote it to an exchange packet.
        if record.get("event_type").and_then(|value| value.as_str()) == Some("market_data") {
            let symbol = record.get("symbol").and_then(Value::as_str).unwrap_or("");
            let projected = mbp::callback_to_mbp(
                record.get("payload").unwrap_or(&Value::Null),
                symbol,
                "NSE",
                "data_socket",
                record
                    .get("received_utc_ns")
                    .and_then(Value::as_i64)
                    .unwrap_or_default(),
            )
            .context("projecting FYERS callback to typed MBP")?;
            if projected.is_some() {
                report.depth_snapshots += 1;
            }
        }
        // Canonical re-encoding of the SDK-decoded callback record. The
        // original Python journal remains the authoritative capture artifact.
        let payload = canonical_json(&json!({
            "schema_version": 1,
            "event_type": record.get("event_type"),
            "payload": record.get("payload"),
            "capture_sequence": sequence,
            "session_marker": marker,
        }))?;
        let frame = SourceFrame {
            source_id: FYERS_SOURCE_ID.to_owned(),
            // Scope delivery identity by session so two sessions with the
            // same local sequence cannot collide in one shared journal.
            provider_instrument_id: format!(
                "{}:{}",
                session_id,
                record
                    .get("symbol")
                    .and_then(|value| value.as_str())
                    .unwrap_or("__fyers_session_event__")
            ),
            entitlement_ref: manifest
                .get("entitlement_ref")
                .and_then(|value| value.as_str())
                .context("manifest has no entitlement_ref")?
                .to_owned(),
            received_utc_nanos: 0,
            received_monotonic_nanos: sequence,
            exchange_utc_nanos: None,
            sequence: Some(Sequence::capture_session(sequence)),
            payload,
        };
        match ingestor
            .ingest_one(frame)
            .context("importing FYERS record")?
        {
            IngestOutcome::Committed { .. } => report.imported += 1,
            IngestOutcome::DuplicateSkipped => report.duplicates += 1,
            IngestOutcome::ConflictRejected | IngestOutcome::MalformedRejected => {
                bail!("shared ingress refused a verified FYERS record")
            }
        }
    }
    Ok(report)
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let report = import_session(&args.session, &args.output)?;
    eprintln!(
        "fyers_import session={} records={} imported={} duplicates={}",
        report.session_id, report.records, report.imported, report.duplicates
    );
    Ok(())
}
