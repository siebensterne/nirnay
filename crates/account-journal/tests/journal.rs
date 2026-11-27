#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Account-journal contract tests: recovery, hash chain, idempotency,
//! deterministic identity, exclusive ownership and segment continuity.

use std::{
    collections::BTreeSet,
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
};

use account_journal::{
    FencingToken, //
    Journal,
    JournalError,
};
use trading_core::{
    AccountId, Continuity, DomainError, EventEnvelope, EventId, EventPayload, EventTimes,
    InstrumentId, LogicalNanos, Paise, PricePaise, Quantity, QuoteSnapshot, RunId, StoredBody,
    TradeDate, UtcNanos, Venue,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn fence(journal: &mut Journal) -> FencingToken {
    journal
        .fence("journal-contract-owner")
        .expect("fence the journal writer")
}

const TIMESTAMP: i64 = 1_793_145_600_000_000_000;
const RUN: RunId = RunId::from_bytes([
    0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
]);

fn envelope(stream: &str, event_id: EventId) -> EventEnvelope {
    EventEnvelope {
        event_id,
        schema_version: 1,
        stream_id: stream.to_owned(),
        source_id: None,
        source_clock_domain: None,
        session_id: Some("2026-10-28".to_owned()),
        source_sequence: None,
        correlation_id: None,
        causation_id: None,
        times: EventTimes {
            source_time_utc: None,
            received_time_utc: UtcNanos::new(TIMESTAMP),
            available_time: LogicalNanos::new(1),
            recorded_time_utc: UtcNanos::new(TIMESTAMP),
        },
        quality: BTreeSet::new(),
    }
}

fn account_opened(stream: &str, event_id: EventId) -> Result<StoredBody, DomainError> {
    Ok(StoredBody {
        envelope: envelope(stream, event_id),
        payload: EventPayload::AccountOpened {
            account_id: AccountId::new(1)?,
            starting_cash: Paise(1_000_000),
            trade_date: TradeDate::new(2026, 10, 28)
                .ok()
                .ok_or_else(|| DomainError::InvalidEventTime("invalid date".to_owned()))?,
        },
    })
}

fn kill_switch(stream: &str, event_id: EventId, reason: &str) -> StoredBody {
    StoredBody {
        envelope: envelope(stream, event_id),
        payload: EventPayload::KillSwitchChanged {
            enabled: true,
            actor_id: 7,
            reason: reason.to_owned(),
        },
    }
}

fn quote_body(
    stream: &str,
    event_id: EventId,
    schema_version: u16,
    venue: Venue,
) -> Result<StoredBody, DomainError> {
    Ok(StoredBody {
        envelope: EventEnvelope {
            schema_version,
            source_id: Some("synthetic-nse".to_owned()),
            source_clock_domain: Some("utc".to_owned()),
            source_sequence: Some(1),
            ..envelope(stream, event_id)
        },
        payload: EventPayload::QuoteReceived {
            quote: QuoteSnapshot {
                instrument_id: InstrumentId::new("nse.cash.synidx")
                    .map_err(|error| DomainError::InvalidInstrument(error.to_string()))?,
                venue,
                bid_price: Some(PricePaise::new(Paise(995))?),
                bid_quantity: Quantity::new(1)?,
                ask_price: Some(PricePaise::new(Paise(1_005))?),
                ask_quantity: Quantity::new(1)?,
                continuity: Continuity::Snapshot,
            },
        },
    })
}

#[test]
fn venue_bearing_payloads_require_and_preserve_schema_version_three() -> TestResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("ledger");
    let stream = "venue-schema-version";
    let event_id = EventId::for_run(RUN, 1);
    {
        let mut journal = Journal::open(&root, stream)?;
        let token = fence(&mut journal);
        // A v1 body would replay as a legacy record without a venue.
        let legacy = quote_body(stream, event_id.clone(), 1, Venue::Nse)?;
        match journal.append_fenced(&token, legacy) {
            Err(JournalError::SchemaVersionTooOld {
                required: 3,
                declared: 1,
            }) => {}
            other => panic!("expected a schema version rejection, got {other:?}"),
        }
        // Nothing was committed by the rejected append.
        assert_eq!(journal.verification_report().record_count, 0);

        let body = quote_body(stream, event_id, 3, Venue::Bse)?;
        journal.append_fenced(&token, body)?;
        assert_eq!(journal.verification_report().record_count, 1);
    }
    let journal = Journal::open(&root, stream)?;
    let record = journal
        .records()
        .first()
        .ok_or("expected the quote record")?;
    match &record.body.payload {
        EventPayload::QuoteReceived { quote } => {
            assert_eq!(quote.venue, Venue::Bse, "the venue survives reopen");
        }
        other => panic!("unexpected payload {other:?}"),
    }
    Ok(())
}

#[test]
fn reopen_repairs_an_incomplete_final_frame_and_preserves_committed_history() -> TestResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("ledger");
    let stream = "journal-recovery-contract";
    let body = account_opened(stream, EventId::for_run(RUN, 1))?;
    let first_hash;
    {
        let mut journal = Journal::open(&root, stream)?;
        let token = fence(&mut journal);
        let record = journal.append_fenced(&token, body)?;
        first_hash = record.hash.hex();
        assert_eq!(journal.verification_report().record_count, 1);
    }

    let segment = root
        .join("segments")
        .join("segment-00000000000000000001.nrj");
    let mut file = OpenOptions::new().append(true).open(segment)?;
    file.write_all(&[0x4e, 0x52, 0x4a])?;
    file.sync_all()?;
    drop(file);

    let journal = Journal::open(&root, stream)?;
    let report = journal.verification_report();
    assert_eq!(report.record_count, 1);
    assert_eq!(report.head_hash_hex, first_hash);
    assert_eq!(report.repaired_tail_bytes, 3);
    assert_eq!(journal.records().len(), 1);
    Ok(())
}

#[test]
fn identical_run_inputs_produce_identical_event_ids_and_hashes() -> TestResult {
    let first_dir = tempfile::tempdir()?;
    let second_dir = tempfile::tempdir()?;
    let stream = "deterministic-replay";

    let mut hashes = Vec::new();
    for directory in [first_dir.path(), second_dir.path()] {
        let root = directory.join("ledger");
        let mut journal = Journal::open(&root, stream)?;
        let token = fence(&mut journal);
        for ordinal in 1..=3_u64 {
            let event_id = journal.next_event_id(RUN)?;
            assert_eq!(event_id, EventId::for_run(RUN, ordinal));
            let record =
                journal.append_fenced(&token, kill_switch(stream, event_id, "identical input"))?;
            assert_eq!(record.sequence, ordinal);
            hashes.push(record.hash.hex());
        }
    }

    assert_eq!(hashes.len(), 6);
    assert_eq!(hashes[0], hashes[3]);
    assert_eq!(hashes[1], hashes[4]);
    assert_eq!(hashes[2], hashes[5]);
    Ok(())
}

#[test]
fn duplicate_append_is_idempotent_and_conflicting_content_is_rejected() -> TestResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("ledger");
    let stream = "idempotency";
    let mut journal = Journal::open(&root, stream)?;
    let token = fence(&mut journal);
    let event_id = journal.next_event_id(RUN)?;
    let body = kill_switch(stream, event_id.clone(), "first");

    let first = journal.append_fenced(&token, body.clone())?;
    let second = journal.append_fenced(&token, body)?;
    assert_eq!(first, second);
    assert_eq!(journal.records().len(), 1);

    let conflicting = kill_switch(stream, event_id, "different content");
    assert!(matches!(
        journal.append_fenced(&token, conflicting),
        Err(JournalError::IdempotencyConflict(_))
    ));
    assert_eq!(journal.records().len(), 1);
    Ok(())
}

#[test]
fn tampered_body_breaks_the_hash_chain() -> TestResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("ledger");
    let stream = "hash-chain";
    {
        let mut journal = Journal::open(&root, stream)?;
        let token = fence(&mut journal);
        let event_id = journal.next_event_id(RUN)?;
        journal.append_fenced(&token, kill_switch(stream, event_id, "before tampering"))?;
    }

    let segment = root
        .join("segments")
        .join("segment-00000000000000000001.nrj");
    let mut file = OpenOptions::new().read(true).write(true).open(segment)?;
    // Flip one byte of the JSON body, leaving the 84-byte frame header intact.
    file.seek(SeekFrom::Start(84 + 4))?;
    let mut byte = [0_u8; 1];
    file.read_exact(&mut byte)?;
    byte[0] ^= 0x01;
    file.seek(SeekFrom::Start(84 + 4))?;
    file.write_all(&byte)?;
    file.sync_all()?;
    drop(file);

    assert!(matches!(
        Journal::open(&root, stream),
        Err(JournalError::Corrupt { .. })
    ));
    Ok(())
}

#[test]
fn a_second_writer_cannot_open_the_same_journal() -> TestResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("ledger");
    let stream = "exclusive-owner";
    let _journal = Journal::open(&root, stream)?;
    assert!(matches!(
        Journal::open(&root, stream),
        Err(JournalError::Locked(_))
    ));
    Ok(())
}

#[test]
fn segment_rollover_preserves_contiguous_sequences() -> TestResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("ledger");
    let stream = "segment-rollover";
    {
        let mut journal = Journal::open_with_segment_limit(&root, stream, 84 + 1024)?;
        let token = fence(&mut journal);
        for _ in 1..=6_u64 {
            let event_id = journal.next_event_id(RUN)?;
            journal.append_fenced(&token, kill_switch(stream, event_id, "rollover record"))?;
        }
        assert!(journal.verification_report().segment_count > 1);
        assert_eq!(journal.verification_report().record_count, 6);
    }

    let journal = Journal::open_with_segment_limit(&root, stream, 84 + 1024)?;
    assert_eq!(journal.verification_report().record_count, 6);
    let sequences: Vec<u64> = journal
        .records()
        .iter()
        .map(|record| record.sequence)
        .collect();
    assert_eq!(sequences, vec![1, 2, 3, 4, 5, 6]);
    Ok(())
}

#[test]
fn stream_mismatch_and_invalid_bodies_are_rejected_before_write() -> TestResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("ledger");
    let mut journal = Journal::open(&root, "expected-stream")?;
    let token = fence(&mut journal);

    let other = kill_switch("other-stream", EventId::for_run(RUN, 1), "wrong stream");
    assert!(matches!(
        journal.append_fenced(&token, other),
        Err(JournalError::StreamMismatch { .. })
    ));

    let mut invalid = account_opened("expected-stream", EventId::for_run(RUN, 2))?;
    invalid.payload = EventPayload::AccountOpened {
        account_id: AccountId::new(1)?,
        starting_cash: Paise(0),
        trade_date: TradeDate::new(2026, 10, 28)
            .ok()
            .ok_or_else(|| DomainError::InvalidEventTime("invalid date".to_owned()))?,
    };
    assert!(matches!(
        journal.append_fenced(&token, invalid),
        Err(JournalError::Domain(_))
    ));
    assert_eq!(journal.records().len(), 0);
    Ok(())
}
