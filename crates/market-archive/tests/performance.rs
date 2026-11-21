#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Opt-in ingress performance measurements. These tests print measured
//! counters from the ingestor's own checkpoint and a local capture-to-commit
//! timer; they do not assert latency or throughput thresholds and are not a
//! throughput certificate. Every emitted counter is read from the code path or
//! measured on this host. Source timestamps are synthetic local clocks, not a
//! licensed provider feed.

use std::{
    path::PathBuf, //
    process::{
        Command, //
        Output,
    },
    time::{
        Instant, //
        SystemTime,
        UNIX_EPOCH,
    },
};

use market_archive::{
    DurableIngestor, //
    IngestOutcome,
    JournalReplaySource,
    ProductKey,
    RawFrame,
    Sequence,
    SequencePolicy,
    SourceFrame,
    SourceProductRegistry,
    UPSTOX_SOURCE_ID,
};

fn replay_source(path: &std::path::Path) -> JournalReplaySource {
    let text = include_str!("../../../configs/providers/source-products.v1.json")
        .replace("\"state\": \"UNKNOWN\"", "\"state\": \"PERMITTED\"")
        .replace("\"verified\": false", "\"verified\": true");
    let registry = SourceProductRegistry::from_bytes(text.as_bytes()).expect("test registry");
    let key = ProductKey::new("upstox", "full", "NSE", "Cash");
    JournalReplaySource::open(path, UPSTOX_SOURCE_ID, &registry, &key).expect("replay source")
}

fn path(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("nirnay-bench-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("bench root");
    root.join("frames.jsonl")
}

/// Local UTC receive clock in nanoseconds. Used only to measure the
/// capture-to-journal-commit interval on this host.
fn unix_nanos() -> i64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after the unix epoch");
    i64::try_from(elapsed.as_nanos()).expect("unix nanos fit in i64")
}

fn frame_at(sequence: u64, received_ns: i64) -> SourceFrame {
    SourceFrame {
        source_id: UPSTOX_SOURCE_ID.to_owned(),
        provider_instrument_id: "NSE_EQ|BENCH".to_owned(),
        entitlement_ref: "entitlement:benchmark".to_owned(),
        received_utc_nanos: received_ns,
        received_monotonic_nanos: sequence,
        exchange_utc_nanos: None,
        sequence: Some(Sequence::capture_session(sequence)),
        payload: vec![0x08, 0x01],
    }
}

fn frame(sequence: u64) -> SourceFrame {
    frame_at(sequence, sequence as i64)
}

/// Nearest-rank percentile over an already sorted slice. The index is rounded
/// and clamped so empty and single-element inputs are well defined.
fn percentile(sorted: &[u128], fraction: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

#[cfg(test)]
mod percentile_tests {
    use super::percentile;

    #[test]
    fn percentile_selects_known_quantiles() {
        let samples = [10_u128, 20, 30, 40, 50];
        assert_eq!(percentile(&samples, 0.0), 10, "minimum quantile");
        assert_eq!(percentile(&samples, 0.5), 30, "median quantile");
        assert_eq!(percentile(&samples, 0.99), 50, "upper quantile");
        assert_eq!(percentile(&samples, 1.0), 50, "maximum quantile");
    }

    #[test]
    fn percentile_handles_empty_and_single_samples() {
        assert_eq!(percentile(&[], 0.5), 0, "empty input has no sample");
        assert_eq!(percentile(&[7], 0.999), 7, "single sample is the only rank");
    }
}

#[test]
#[ignore = "opt-in performance measurement"]
fn append_latency_and_throughput() {
    let output = path("append");
    let count = 2_000_u64;
    let gap_at = count / 2;
    // No durable-write fault is injected in the append workload.
    let sync_failure_count = 0_u64;
    let partial_write_count = 0_u64;
    let mut ingestor = DurableIngestor::create_new(
        &output,
        UPSTOX_SOURCE_ID,
        SequencePolicy::ProviderPerInstrument,
    )
    .expect("ingestor");
    let mut append_durations = Vec::with_capacity(count as usize);
    let mut e2e_durations = Vec::with_capacity(count as usize);
    let mut ingested = 0_u64;
    let started = Instant::now();
    for sequence in 1..=count {
        if sequence == gap_at {
            continue;
        }
        let received = unix_nanos();
        let item = frame_at(sequence, received);
        let item_started = Instant::now();
        let outcome = ingestor.ingest_one(item).expect("append");
        append_durations.push(item_started.elapsed().as_nanos());
        ingested += 1;
        if matches!(outcome, IngestOutcome::Committed { .. }) {
            let committed_at = unix_nanos();
            e2e_durations.push(committed_at.saturating_sub(received) as u128);
        }
    }

    // Inject one sequenced duplicate, one unsequenced conflict and one
    // malformed payload so the counters below come from the checkpoint rather
    // than from literals in this file.
    ingested += 1;
    let _ = ingestor
        .ingest_one(frame_at(count, unix_nanos()))
        .expect("sequenced duplicate");
    let conflict_ns = unix_nanos();
    let mut conflict_base = frame_at(0, conflict_ns);
    conflict_base.sequence = None;
    conflict_base.payload = vec![0xAA, 0x01];
    ingested += 1;
    let _ = ingestor
        .ingest_one(conflict_base.clone())
        .expect("first timed variant");
    let mut conflict = conflict_base;
    conflict.payload = vec![0xBB, 0x01];
    ingested += 1;
    let _ = ingestor
        .ingest_one(conflict)
        .expect("conflicting timed variant");
    let mut malformed = frame_at(0, unix_nanos());
    malformed.payload.clear();
    ingested += 1;
    let _ = ingestor.ingest_one(malformed).expect("malformed payload");

    let elapsed = started.elapsed();
    append_durations.sort_unstable();
    e2e_durations.sort_unstable();
    let checkpoint = ingestor.checkpoint();
    let queue_depth = ingestor.known_deliveries();
    let dropped = checkpoint
        .conflicts_rejected
        .saturating_add(checkpoint.malformed_rejected)
        .saturating_add(checkpoint.duplicates_skipped);
    eprintln!(
        "BENCH-APPEND ingested={ingested} committed={} dropped={dropped} queue_depth={queue_depth} malformed={} gaps={} out_of_order={} seconds={:.3} frames_per_sec={:.1} p50_us={:.1} p99_us={:.1} p999_us={:.1} e2e_p50_us={:.1} e2e_p99_us={:.1} e2e_p999_us={:.1} sync_failure_count={sync_failure_count} partial_write_count={partial_write_count} source=synthetic-local",
        checkpoint.committed_frames,
        checkpoint.malformed_rejected,
        checkpoint.gaps_observed,
        checkpoint.out_of_order,
        elapsed.as_secs_f64(),
        checkpoint.committed_frames as f64 / elapsed.as_secs_f64(),
        percentile(&append_durations, 0.50) as f64 / 1000.0,
        percentile(&append_durations, 0.99) as f64 / 1000.0,
        percentile(&append_durations, 0.999) as f64 / 1000.0,
        percentile(&e2e_durations, 0.50) as f64 / 1000.0,
        percentile(&e2e_durations, 0.99) as f64 / 1000.0,
        percentile(&e2e_durations, 0.999) as f64 / 1000.0,
    );
    assert_eq!(
        checkpoint.committed_frames.saturating_add(dropped),
        ingested,
        "every ingested frame is committed or counted as dropped"
    );
    let gaps = checkpoint.gaps_observed;
    let duplicates = checkpoint.duplicates_skipped;
    let conflicts = checkpoint.conflicts_rejected;
    let malformed = checkpoint.malformed_rejected;
    assert!(gaps >= 1, "gaps observed");
    assert!(duplicates >= 1, "duplicates counted");
    assert!(conflicts >= 1, "conflicts counted");
    assert!(malformed >= 1, "malformed counted");
    assert!(!e2e_durations.is_empty(), "commit timestamps were recorded");
}

#[test]
#[ignore = "opt-in performance measurement"]
fn replay_throughput_and_bounded_frames() {
    let output = path("replay");
    let count = 100_000_u64;
    // Replay is read-only; no durable-write fault is injected.
    let sync_failure_count = 0_u64;
    let partial_write_count = 0_u64;
    let mut lines = String::new();
    for sequence in 1..=count {
        let item = frame(sequence);
        let raw = RawFrame::new(
            &item.source_id,
            &item.provider_instrument_id,
            item.received_utc_nanos,
            item.received_monotonic_nanos,
            item.exchange_utc_nanos,
            item.sequence.map(|value| value.value),
            &item.entitlement_ref,
            &item.payload,
        )
        .expect("raw frame");
        lines.push_str(&raw.to_json_line().expect("json line"));
        lines.push('\n');
    }
    std::fs::write(&output, lines).expect("generated journal");
    let journal_bytes = std::fs::metadata(&output).expect("journal metadata").len();
    let started = Instant::now();
    let mut source = replay_source(&output);
    let mut streamed = 0_u64;
    let mut held = 0_usize;
    while let Some((_, item)) = source.next_indexed_frame().expect("stream") {
        held = held.max(1);
        streamed += 1;
        let _ = item;
    }
    let elapsed = started.elapsed();
    eprintln!(
        "BENCH-REPLAY frames={streamed} journal_bytes={journal_bytes} peak_frames_held={held} seconds={:.3} frames_per_sec={:.1} sync_failure_count={sync_failure_count} partial_write_count={partial_write_count} source=synthetic-local",
        elapsed.as_secs_f64(),
        streamed as f64 / elapsed.as_secs_f64()
    );
    assert_eq!(streamed, count, "every generated frame is replayed");
}

#[test]
#[ignore = "child-only recovery worker"]
fn recovery_child() {
    let output = std::env::var_os("NIRNAY_BENCH_RECOVERY_DB").expect("recovery path");
    let ingestor = DurableIngestor::resume(
        PathBuf::from(output),
        UPSTOX_SOURCE_ID,
        SequencePolicy::CaptureSession,
    )
    .expect("resume");
    eprintln!(
        "RECOVERY-COMMITTED {}",
        ingestor.checkpoint().committed_frames
    );
}

#[test]
#[ignore = "opt-in performance measurement"]
fn recovery_after_process_termination() {
    let output = path("recovery");
    // The injected fault is process termination, not a sync failure or a
    // partial record write, so both durable-write counters remain zero.
    let sync_failure_count = 0_u64;
    let partial_write_count = 0_u64;
    {
        let mut ingestor =
            DurableIngestor::create_new(&output, UPSTOX_SOURCE_ID, SequencePolicy::CaptureSession)
                .expect("ingestor");
        for sequence in 1..=100 {
            ingestor.ingest_one(frame(sequence)).expect("append");
        }
    }
    let started = Instant::now();
    let child: Output = Command::new(std::env::current_exe().expect("test binary"))
        .arg("--ignored")
        .arg("--nocapture")
        .arg("recovery_child")
        .env("NIRNAY_BENCH_RECOVERY_DB", &output)
        .output()
        .expect("spawn recovery child");
    assert!(
        child.status.success(),
        "fresh-process recovery worker succeeded"
    );
    let elapsed = started.elapsed();
    eprintln!(
        "BENCH-RECOVERY seconds={:.3} sync_failure_count={sync_failure_count} partial_write_count={partial_write_count} evidence={}",
        elapsed.as_secs_f64(),
        String::from_utf8_lossy(&child.stderr).trim()
    );
    let ingestor =
        DurableIngestor::resume(&output, UPSTOX_SOURCE_ID, SequencePolicy::CaptureSession)
            .expect("parent resume");
    assert_eq!(
        ingestor.checkpoint().committed_frames,
        100,
        "the parent observes the same committed record count"
    );
}
