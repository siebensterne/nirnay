// SPDX-License-Identifier: Apache-2.0
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Public-boundary tests for mandatory OMS and account projection fencing.
//!
//! The crate is used exactly as an external caller would use it: tokens come
//! only from `fence`, every durable mutation goes through the fenced facade,
//! and a token from a displaced owner must be refused for every mutation
//! class. Fixtures carry order identity only; no market observations, quotes,
//! provider feed or live execution path is involved.

use std::collections::BTreeMap;

use authoritative_oms::{
    BrokerOrderId, //
    ChargeStatus,
    ClientOrderId,
    FencingError,
    FillEvent,
    JournalFencingToken,
    OmsStore,
    PermitRef,
    Projector,
    Resolution,
};
use trading_core::{
    AccountId, //
    AlgoId,
    InstrumentId,
    IntentId,
    JournalRef,
    LogicalNanos,
    OpeningBalanceEvidence,
    OrderIntent,
    Paise,
    PricePaise,
    Product,
    Quantity,
    Side,
    StrategyId,
    TradeDate,
    Venue,
    permit_digest,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const NOW: i64 = 1_000;

fn intent(id: u64, quantity: i64) -> OrderIntent {
    OrderIntent {
        intent_id: IntentId::new(id).expect("intent id"),
        account_id: AccountId::new(1).expect("account id"),
        strategy_id: StrategyId::new(2).expect("strategy id"),
        strategy_version: "fencing-boundary-v1".to_owned(),
        algo_id: AlgoId::new(3).expect("algo id"),
        instrument_id: InstrumentId::new("paper.fencing-boundary").expect("instrument id"),
        venue: Venue::Nse,
        side: Side::Buy,
        product: Product::Intraday,
        quantity: Quantity::new(quantity).expect("quantity"),
        limit_price: PricePaise::new(Paise(1_000)).expect("price"),
        reason: "fencing boundary fixture".to_owned(),
        evidence: vec![JournalRef {
            sequence: 1,
            hash_hex: "ab".repeat(32),
        }],
        expires_at: LogicalNanos::new(100_000),
    }
}

fn permit(id: u64, order: &OrderIntent) -> PermitRef {
    PermitRef {
        permit_id: id,
        policy_version: "risk-v1".to_owned(),
        snapshot_revision: 1,
        digest: permit_digest(id, "risk-v1", 1, order),
    }
}

fn fill(execution_id: &str, broker_event_id: &str, quantity: i64) -> FillEvent {
    FillEvent {
        schema_version: 1,
        execution_id: execution_id.to_owned(),
        broker_event_id: broker_event_id.to_owned(),
        client_order_id: ClientOrderId::new(1).expect("client id"),
        oms_order_id: "1".to_owned(),
        side: Side::Buy,
        quantity,
        price_paise: 1_000,
        currency: "INR".to_owned(),
        broker_timestamp_ns: NOW,
        received_at_ns: NOW,
        source: "fencing-boundary-fixture".to_owned(),
        account_scope: "paper".to_owned(),
        charges_paise: 10,
        charge_status: ChargeStatus::Estimated,
        provenance: "fencing boundary fixture".to_owned(),
    }
}

fn opening_evidence() -> OpeningBalanceEvidence {
    OpeningBalanceEvidence {
        statement_ref: "fencing-boundary-statement".to_owned(),
        journal_ref: JournalRef {
            sequence: 1,
            hash_hex: "0".repeat(64),
        },
        observed_at_ns: NOW,
    }
}

fn assert_stale<T: std::fmt::Debug>(result: Result<T, FencingError>, label: &str) {
    match result {
        Err(FencingError::StaleTerm { .. }) => {}
        other => panic!("{label} must refuse a stale term, got {other:?}"),
    }
}

fn assert_journal_stale<T: std::fmt::Debug, E: std::fmt::Debug>(
    result: Result<T, E>,
    stale: fn(&E) -> bool,
    label: &str,
) {
    match result {
        Err(error) if stale(&error) => {}
        other => panic!("{label} must refuse a stale term, got {other:?}"),
    }
}

#[test]
fn every_oms_mutation_class_refuses_a_stale_term() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.journal"))?;
    let stale = store.fence("owner-a")?;
    let current = store.fence("owner-b")?;
    let client = ClientOrderId::new(1)?;
    let order = intent(1, 10);
    let order_permit = permit(1, &order);
    store.create_fenced(&current, client, &order, &order_permit, NOW)?;
    let digest_before = store.state_digest().to_owned();

    assert_stale(
        store.create_fenced(
            &stale,
            ClientOrderId::new(2)?,
            &intent(2, 10),
            &permit(2, &intent(2, 10)),
            NOW,
        ),
        "creation",
    );
    assert_stale(
        store.accept_risk_approved_fenced(&stale, client, NOW),
        "risk approval",
    );
    assert_stale(store.claim_fenced(&stale, "worker", NOW, 1_000), "claim");
    assert_stale(store.mark_dispatching_fenced(&stale, 1, NOW), "dispatch");
    assert_stale(
        store.acknowledge_fenced(&stale, 1, &BrokerOrderId::new("B-1")?, "ack-evt", NOW),
        "acknowledgement",
    );
    assert_stale(
        store.reject_submission_fenced(&stale, 1, "rejected", "reject-evt", NOW),
        "rejection",
    );
    assert_stale(
        store.apply_typed_fill_fenced(&stale, fill("exec-1", "fill-evt-1", 1)),
        "typed fill",
    );
    assert_stale(
        store.apply_fill_fenced(&stale, client, "exec-2", "fill-evt-2", 1, 1_000, NOW),
        "simulated fill",
    );
    assert_stale(
        store.request_cancel_fenced(&stale, client, NOW),
        "cancel request",
    );
    assert_stale(
        store.apply_cancel_fenced(&stale, client, "cancel-evt", NOW),
        "cancel apply",
    );
    assert_stale(
        store.begin_reconciliation_fenced(&stale, client, NOW),
        "reconciliation begin",
    );
    assert_stale(
        store.resolve_unknown_fenced(
            &stale,
            client,
            Resolution::Working {
                broker_order_id: BrokerOrderId::new("B-2")?,
            },
            "resolve-evt",
            NOW,
        ),
        "reconciliation resolution",
    );
    assert_stale(
        store.expire_dispatched_fenced(&stale, NOW, 0),
        "dispatch expiry",
    );

    assert_eq!(
        store.state_digest(),
        digest_before,
        "no refused mutation changed durable OMS state"
    );
    assert_eq!(
        store.orders()?.len(),
        1,
        "the stale owner created no extra order"
    );
    Ok(())
}

#[test]
fn current_term_duplicate_is_idempotent_without_lowering_the_term() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("oms.journal");
    let token;
    {
        let mut store = OmsStore::open(&path)?;
        token = store.fence("owner-a")?;
        let order = intent(1, 10);
        store.create_fenced(
            &token,
            ClientOrderId::new(1)?,
            &order,
            &permit(1, &order),
            NOW,
        )?;
    }
    let mut store = OmsStore::open(&path)?;
    let order = intent(1, 10);
    store.create_fenced(
        &token,
        ClientOrderId::new(1)?,
        &order,
        &permit(1, &order),
        NOW + 1,
    )?;
    assert_eq!(store.fencing_term().get(), 1, "duplicate kept the term");
    assert_eq!(store.orders()?.len(), 1, "duplicate added no order");
    let next = store.fence("owner-b")?;
    assert_eq!(next.term().get(), 2, "reassignment advanced the term");
    assert_stale(
        store.create_fenced(
            &token,
            ClientOrderId::new(2)?,
            &intent(2, 10),
            &permit(2, &intent(2, 10)),
            NOW,
        ),
        "creation after reassignment",
    );
    Ok(())
}

#[test]
fn every_projection_mutation_class_refuses_a_stale_term() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut projector = Projector::open(directory.path().join("ledger"), AccountId::new(1)?)?;
    let stale: JournalFencingToken = projector.fence("owner-a")?;
    let current: JournalFencingToken = projector.fence("owner-b")?;
    let is_stale = |error: &authoritative_oms::ProjectionError| {
        matches!(
            error,
            authoritative_oms::ProjectionError::Journal(
                account_journal::JournalError::StaleFencingTerm { .. }
            )
        )
    };
    assert_journal_stale(
        projector.open_account_fenced(
            &stale,
            "other-scope",
            Paise(1_000_000),
            TradeDate::new(2026, 1, 1)?,
            opening_evidence(),
        ),
        is_stale,
        "opening balance",
    );
    assert_eq!(
        projector.verification_report().record_count,
        0,
        "a refused opening must not append a journal record"
    );
    projector.open_account_fenced(
        &current,
        "paper",
        Paise(1_000_000),
        TradeDate::new(2026, 1, 1)?,
        opening_evidence(),
    )?;
    assert_eq!(
        projector.verification_report().record_count,
        1,
        "only the current owner's opening was appended"
    );

    let event = fill("exec-1", "fill-evt-1", 1);
    assert_journal_stale(
        projector.project_fill_fenced(
            &stale,
            &event,
            &InstrumentId::new("paper.fencing-boundary")?,
        ),
        is_stale,
        "fill projection",
    );
    assert_journal_stale(
        projector.bust_fenced(&stale, "exec-1"),
        is_stale,
        "bust correction",
    );
    assert_journal_stale(
        projector.confirm_charges_fenced(&stale, "exec-1", 20),
        is_stale,
        "charge correction",
    );
    let instruments = BTreeMap::new();
    assert_journal_stale(
        projector.project_store_fills_fenced(
            &stale,
            &OmsStore::open(directory.path().join("oms.journal"))?,
            &instruments,
        ),
        is_stale,
        "bulk projection",
    );
    assert_eq!(
        projector.verification_report().record_count,
        1,
        "no refused mutation appended a journal record"
    );
    Ok(())
}
