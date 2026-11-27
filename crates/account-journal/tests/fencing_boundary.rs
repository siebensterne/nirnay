// SPDX-License-Identifier: Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Public-boundary tests for mandatory account-journal fencing.
//!
//! These tests compile against the crate's exported API only. They prove that
//! the persistent mutation surface is the fenced facade, that a stale term is
//! refused after reassignment, and that current-term duplicates remain
//! idempotent without lowering the durable term.

use std::collections::BTreeSet;

use account_journal::{
    FencingTerm, //
    Journal,
    JournalError,
};
use trading_core::{
    AccountId, DomainError, EventEnvelope, EventId, EventPayload, EventTimes, LogicalNanos, Paise,
    RunId, StoredBody, TradeDate, UtcNanos,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const STREAM: &str = "account-fencing-boundary";
const TIMESTAMP: i64 = 1_793_145_600_000_000_000;
const RUN: RunId = RunId::from_bytes([
    0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
]);

fn body(sequence: u64) -> Result<StoredBody, DomainError> {
    Ok(StoredBody {
        envelope: EventEnvelope {
            event_id: EventId::for_run(RUN, sequence),
            schema_version: 1,
            stream_id: STREAM.to_owned(),
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
        },
        payload: EventPayload::AccountOpened {
            account_id: AccountId::new(sequence)?,
            starting_cash: Paise(1_000_000),
            trade_date: TradeDate::new(2026, 10, 28)
                .ok()
                .ok_or_else(|| DomainError::InvalidEventTime("invalid date".to_owned()))?,
        },
    })
}

#[test]
fn only_the_fenced_facade_can_append_at_the_public_boundary() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut journal = Journal::open(directory.path().join("journal"), STREAM)?;
    let token = journal.fence("boundary-owner")?;
    assert_eq!(token.term(), FencingTerm::new(1), "first term is one");
    assert_eq!(token.owner(), "boundary-owner", "owner label is preserved");
    journal.append_fenced(&token, body(1)?)?;
    assert_eq!(
        journal.records().len(),
        1,
        "the fenced append persisted one record"
    );
    Ok(())
}

#[test]
fn stale_term_after_reassignment_is_refused_without_a_write() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("journal");
    let stale;
    {
        let mut journal = Journal::open(&path, STREAM)?;
        stale = journal.fence("owner-a")?;
        journal.append_fenced(&stale, body(1)?)?;
    }
    let mut journal = Journal::open(&path, STREAM)?;
    let current = journal.fence("owner-b")?;
    journal.append_fenced(&current, body(2)?)?;
    match journal.append_fenced(&stale, body(3)?) {
        Err(JournalError::StaleFencingTerm {
            presented: 1,
            persisted: 2,
        }) => {}
        other => panic!("expected a stale-term refusal, got {other:?}"),
    }
    assert_eq!(
        journal.records().len(),
        2,
        "a refused append must not change the journal"
    );
    assert_eq!(
        journal.fencing_term(),
        FencingTerm::new(2),
        "a refused append must not lower the term"
    );
    Ok(())
}

#[test]
fn reassignment_stops_accepting_the_displaced_owner() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut journal = Journal::open(directory.path().join("journal"), STREAM)?;
    let displaced = journal.fence("first-owner")?;
    let current = journal.fence("second-owner")?;
    assert_eq!(displaced.term(), FencingTerm::new(1), "first term is one");
    assert_eq!(
        current.term(),
        FencingTerm::new(2),
        "reassignment advances the term"
    );
    assert_eq!(
        current.owner(),
        "second-owner",
        "only the latest owner holds the current term"
    );
    match journal.append_fenced(&displaced, body(1)?) {
        Err(JournalError::StaleFencingTerm { .. }) => {}
        other => panic!("expected the displaced owner to be refused, got {other:?}"),
    }
    journal.append_fenced(&current, body(1)?)?;
    assert_eq!(
        journal.records().len(),
        1,
        "only the current owner's append landed"
    );
    Ok(())
}

#[test]
fn duplicate_at_current_term_is_idempotent_without_lowering_the_term() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("journal");
    let token;
    let replay = body(1)?;
    {
        let mut journal = Journal::open(&path, STREAM)?;
        token = journal.fence("owner-a")?;
        journal.append_fenced(&token, replay.clone())?;
    }
    let mut journal = Journal::open(&path, STREAM)?;
    let first = journal.append_fenced(&token, replay.clone())?;
    let second = journal.append_fenced(&token, replay)?;
    assert_eq!(first.sequence, 1, "the duplicate replays at sequence one");
    assert_eq!(second.sequence, 1, "repeated replay stays idempotent");
    assert_eq!(
        journal.fencing_term(),
        FencingTerm::new(1),
        "duplicates never lower the durable term"
    );
    let _ = journal.fence("owner-b")?;
    match journal.append_fenced(&token, body(2)?) {
        Err(JournalError::StaleFencingTerm { .. }) => {}
        other => panic!("expected the old token to be stale after reassignment, got {other:?}"),
    }
    Ok(())
}
