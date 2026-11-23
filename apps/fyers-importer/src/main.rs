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

#[cfg(all(test, feature = "market-shaped-tests"))]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/session")
    }

    #[test]
    fn verifies_a_real_python_hash_chain_session() {
        let (manifest, records) = verify_session(&fixture()).expect("verified session");
        assert_eq!(
            manifest.get("run_id").and_then(|value| value.as_str()),
            Some("synthetic-fyers-session"),
            "the verified session keeps its run id"
        );
        assert_eq!(records.len(), 4, "all four records are verified");
        assert!(
            records.iter().any(
                |record| record.get("event_type").and_then(|value| value.as_str()) == Some("gap")
            ),
            "gap records are preserved in the verified set"
        );
    }

    #[test]
    fn import_is_byte_faithful_and_idempotent() {
        let directory = tempfile::tempdir().expect("tempdir");
        let output = directory.path().join("fyers-shared.jsonl");
        let first = import_session(&fixture(), &output).expect("first import");
        assert_eq!(first.imported, 4, "all verified records are imported");
        let frames = market_archive::scan_journal(&output).expect("scan").frames;
        assert_eq!(frames.len(), 4, "the common journal has four records");
        let stored_bytes = base64::engine::general_purpose::STANDARD
            .decode(&frames[0].payload_base64)
            .expect("decode stored payload");
        let first_payload: Value = serde_json::from_slice(&stored_bytes).expect("stored payload");
        assert_eq!(
            first_payload
                .get("event_type")
                .and_then(|value| value.as_str()),
            Some("socket_connected"),
            "event type survives canonical re-encoding"
        );
        assert!(
            frames.iter().any(|frame| {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(&frame.payload_base64)
                    .unwrap_or_default();
                decoded
                    .windows(b"first_missing_local_sequence".len())
                    .any(|window| window == b"first_missing_local_sequence")
            }),
            "gap metadata survives import"
        );
        let second = import_session(&fixture(), &output).expect("repeat import");
        assert_eq!(second.imported, 0, "a repeated session import adds nothing");
        assert_eq!(
            second.duplicates, 4,
            "all four records are recognised as already imported"
        );
        assert_eq!(
            market_archive::scan_journal(&output)
                .expect("scan")
                .frames
                .len(),
            4,
            "the journal is unchanged by a repeat import"
        );
    }

    #[test]
    fn tampered_manifest_or_chain_fails_closed() {
        let root = tempfile::tempdir().expect("tempdir");
        let session = root.path().join("session");
        std::fs::create_dir_all(&session).expect("session dir");
        for name in ["manifest.json", "fyers-sdk-observations.jsonl"] {
            std::fs::copy(fixture().join(name), session.join(name)).expect("copy fixture");
        }
        let manifest_path = session.join("manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).expect("manifest"))
                .expect("manifest JSON");
        manifest["journal_sha256"] = Value::String("0".repeat(64));
        std::fs::write(
            &manifest_path,
            canonical_json(&manifest).expect("canonical"),
        )
        .expect("tampered manifest");
        assert!(
            verify_session(&session).is_err(),
            "a manifest digest mismatch is refused"
        );

        std::fs::copy(
            fixture().join("manifest.json"),
            session.join("manifest.json"),
        )
        .expect("restore manifest");
        let journal_path = session.join("fyers-sdk-observations.jsonl");
        let mut lines: Vec<String> = std::fs::read_to_string(&journal_path)
            .expect("journal")
            .lines()
            .map(str::to_owned)
            .collect();
        let mut record: Value = serde_json::from_str(&lines[1]).expect("record");
        record["payload"]["ltp"] = Value::from(1);
        lines[1] = serde_json::to_string(&record).expect("tampered record");
        std::fs::write(&journal_path, lines.join("\n") + "\n").expect("tampered journal");
        assert!(
            verify_session(&session).is_err(),
            "a broken hash chain is refused"
        );
    }
    #[test]
    fn interrupted_import_resumes_without_dropping_records() {
        let directory = tempfile::tempdir().expect("tempdir");
        let output = directory.path().join("resumable.jsonl");
        import_session(&fixture(), &output).expect("first import");
        let original = std::fs::read_to_string(&output).expect("journal");
        let lines: Vec<&str> = original.lines().collect();
        std::fs::write(&output, format!("{}\n", lines[..2].join("\n")))
            .expect("simulate interruption after two frames");
        let resumed = import_session(&fixture(), &output).expect("resumed import");
        assert_eq!(resumed.imported, 2, "the missing tail is imported");
        assert_eq!(
            market_archive::scan_journal(&output)
                .expect("scan")
                .frames
                .len(),
            4,
            "no verified record is lost after an interrupted import"
        );
    }

    #[test]
    fn two_sessions_with_overlapping_local_sequences_share_the_journal() {
        let directory = tempfile::tempdir().expect("tempdir");
        let output = directory.path().join("two-sessions.jsonl");
        let second_session = directory.path().join("second-session");
        std::fs::create_dir_all(&second_session).expect("session dir");
        for name in ["manifest.json", "fyers-sdk-observations.jsonl"] {
            std::fs::copy(fixture().join(name), second_session.join(name)).expect("copy fixture");
        }
        let manifest_path = second_session.join("manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).expect("manifest"))
                .expect("manifest JSON");
        manifest["run_id"] = Value::String("second-synthetic-session".to_owned());
        std::fs::write(
            &manifest_path,
            canonical_json(&manifest).expect("canonical"),
        )
        .expect("second manifest");

        let first = import_session(&fixture(), &output).expect("first session");
        let second = import_session(&second_session, &output).expect("second session");
        assert_eq!(first.imported, 4, "the first session imports all records");
        assert_eq!(
            second.imported, 4,
            "overlapping local sequences do not collide"
        );
        assert_eq!(
            market_archive::scan_journal(&output)
                .expect("scan")
                .frames
                .len(),
            8,
            "both sessions are preserved in one shared journal"
        );
    }
}
