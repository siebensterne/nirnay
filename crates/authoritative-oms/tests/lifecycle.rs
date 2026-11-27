#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.5 authoritative OMS tests. Broker behaviour is simulated entirely in
//! these tests; no real endpoint or live routing exists.

use authoritative_oms::{
    BrokerOrderId, //
    ClientOrderId,
    FencingError,
    FencingToken,
    OmsError,
    OmsOrderState,
    OmsStore,
    OutboxState,
    PermitRef,
    Resolution,
};
use trading_core::{
    AccountId, //
    AlgoId,
    InstrumentId,
    IntentId,
    JournalRef,
    LogicalNanos,
    OrderIntent,
    Paise,
    PricePaise,
    Product,
    Quantity,
    Side,
    StrategyId,
    Venue,
    permit_digest,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn fence(store: &mut OmsStore) -> FencingToken {
    store.fence("lifecycle-owner").expect("fence the OMS")
}

fn intent(id: u64, quantity: i64) -> Result<OrderIntent, Box<dyn std::error::Error>> {
    Ok(OrderIntent {
        intent_id: IntentId::new(id)?,
        account_id: AccountId::new(1)?,
        strategy_id: StrategyId::new(2)?,
        strategy_version: "strategy-v1".to_owned(),
        algo_id: AlgoId::new(3)?,
        instrument_id: InstrumentId::new("nse.cash.synidx")?,
        venue: Venue::Nse,
        side: Side::Buy,
        product: Product::Intraday,
        quantity: Quantity::new(quantity)?,
        limit_price: PricePaise::new(Paise(1_000))?,
        reason: "oms fixture".to_owned(),
        evidence: vec![JournalRef {
            sequence: 1,
            hash_hex: "ab".repeat(32),
        }],
        expires_at: LogicalNanos::new(100_000),
    })
}

fn permit() -> PermitRef {
    let canonical = intent(1, 10).expect("canonical fixture intent");
    PermitRef {
        permit_id: 7,
        policy_version: "risk-v1".to_owned(),
        snapshot_revision: 3,
        digest: permit_digest(7, "risk-v1", 3, &canonical),
    }
}

fn broker_id() -> BrokerOrderId {
    BrokerOrderId::new("BROKER-1").expect("broker id")
}

fn fresh(
    store: &mut OmsStore,
    token: &FencingToken,
    client: ClientOrderId,
    quantity: i64,
) -> TestResult {
    store.create_fenced(&token, client, &intent(1, quantity)?, &permit(), 1_000)?;
    store.accept_risk_approved_fenced(&token, client, 1_010)?;
    Ok(())
}

#[test]
fn accepted_order_is_pending_submission_with_one_outbox_command() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.sqlite"))?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    fresh(&mut store, &token, client, 10)?;

    let order = store.order(client)?.expect("order exists");
    assert_eq!(
        order.state,
        OmsOrderState::PendingSubmission,
        "risk acceptance enqueues submission"
    );
    assert_eq!(order.permit, permit(), "the exact permit is bound");
    let outbox = store.outbox()?;
    assert_eq!(outbox.len(), 1, "one submission command exists");
    assert_eq!(
        outbox[0].command_type, "SUBMIT",
        "the first command is a submission"
    );
    assert_eq!(
        outbox[0].state,
        OutboxState::Pending,
        "the submission command is queued"
    );
    Ok(())
}

#[test]
fn client_order_id_is_idempotent_and_conflicts_are_denied() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.sqlite"))?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    store.create_fenced(&token, client, &intent(1, 10)?, &permit(), 1_000)?;
    store.create_fenced(&token, client, &intent(1, 10)?, &permit(), 2_000)?;
    assert_eq!(
        store.outbox()?.len(),
        0,
        "repeated create before acceptance adds no command"
    );

    let conflicting = intent(1, 11)?;
    let result = store.create_fenced(&token, client, &conflicting, &permit(), 3_000);
    assert!(
        matches!(
            result,
            Err(FencingError::Oms(OmsError::ConflictingClientOrder(_)))
        ),
        "the same client id with different order terms is refused"
    );
    Ok(())
}

#[test]
fn acknowledgement_binds_broker_identity_and_moves_to_working() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.sqlite"))?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    fresh(&mut store, &token, client, 10)?;

    let claim = store
        .claim_fenced(&token, "dispatcher-a", 2_000, 1_000)?
        .expect("claim");
    assert_eq!(
        claim.intent,
        intent(1, 10)?,
        "the complete typed order request is carried"
    );
    store.mark_dispatching_fenced(&token, claim.attempt_id, 2_100)?;
    assert!(
        store.acknowledge_fenced(&token, claim.attempt_id, &broker_id(), "evt-ack-1", 2_200)?,
        "the first acknowledgement is accepted"
    );

    let order = store.order(client)?.expect("order");
    assert_eq!(
        order.state,
        OmsOrderState::Working,
        "acknowledgement moves to working"
    );
    assert_eq!(
        order.broker_order_id,
        Some(broker_id()),
        "broker identity is bound"
    );
    assert_eq!(
        store.outbox()?[0].state,
        OutboxState::Acknowledged,
        "the attempt is acknowledged"
    );
    Ok(())
}

/// A dispatch with no acknowledgement becomes UNKNOWN, never a retry.
#[test]
fn dispatch_timeout_is_unknown_with_no_automatic_retry() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.sqlite"))?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    fresh(&mut store, &token, client, 10)?;
    let claim = store
        .claim_fenced(&token, "dispatcher-a", 2_000, 1_000)?
        .expect("claim");
    store.mark_dispatching_fenced(&token, claim.attempt_id, 2_100)?;

    let expired = store.expire_dispatched_fenced(&token, 5_000, 1_000)?;
    assert_eq!(expired, 1, "the dispatch timed out");
    let order = store.order(client)?.expect("order");
    assert_eq!(
        order.state,
        OmsOrderState::Unknown,
        "timeout yields UNKNOWN"
    );
    assert_eq!(
        store.outbox()?[0].state,
        OutboxState::Unknown,
        "the attempt is marked unknown"
    );
    assert_eq!(store.outbox()?.len(), 1, "no retry command was created");
    assert!(
        store
            .claim_fenced(&token, "dispatcher-b", 6_000, 1_000)?
            .is_none(),
        "there is no pending command to claim"
    );
    Ok(())
}

#[test]
fn unknown_requires_evidence_and_is_resolved_by_reconciliation() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.sqlite"))?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    fresh(&mut store, &token, client, 10)?;
    let claim = store
        .claim_fenced(&token, "dispatcher-a", 2_000, 1_000)?
        .expect("claim");
    store.mark_dispatching_fenced(&token, claim.attempt_id, 2_100)?;
    store.expire_dispatched_fenced(&token, 5_000, 1_000)?;

    store.begin_reconciliation_fenced(&token, client, 5_100)?;
    assert_eq!(
        store.order(client)?.expect("order").state,
        OmsOrderState::Reconciling,
        "reconciliation is started"
    );
    let missing_evidence = store.resolve_unknown_fenced(
        &token,
        client,
        Resolution::Working {
            broker_order_id: broker_id(),
        },
        "   ",
        5_200,
    );
    assert!(
        matches!(
            missing_evidence,
            Err(FencingError::Oms(OmsError::UnresolvedUnknown(_)))
        ),
        "resolution without broker evidence is refused"
    );

    store.resolve_unknown_fenced(
        &token,
        client,
        Resolution::Working {
            broker_order_id: broker_id(),
        },
        "evt-reconcile-1",
        5_300,
    )?;
    let order = store.order(client)?.expect("order");
    assert_eq!(
        order.state,
        OmsOrderState::Working,
        "reconciliation resolves the order"
    );
    assert_eq!(
        order.broker_order_id,
        Some(broker_id()),
        "broker identity is bound after reconciliation"
    );
    assert!(
        store.unresolved()?.is_empty(),
        "no unresolved orders remain"
    );
    Ok(())
}

#[test]
fn duplicate_broker_events_are_ignored() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.sqlite"))?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    fresh(&mut store, &token, client, 10)?;
    let claim = store
        .claim_fenced(&token, "dispatcher-a", 2_000, 1_000)?
        .expect("claim");
    store.mark_dispatching_fenced(&token, claim.attempt_id, 2_100)?;

    assert!(
        store.acknowledge_fenced(&token, claim.attempt_id, &broker_id(), "evt-ack-1", 2_200)?,
        "a duplicate acknowledgement is ignored"
    );
    assert!(
        !store.acknowledge_fenced(&token, claim.attempt_id, &broker_id(), "evt-ack-1", 2_201)?,
        "duplicate event is ignored"
    );
    assert!(
        !store.apply_fill_fenced(&token, client, "exec-1", "evt-ack-1", 1, 1_000, 2_300)?,
        "no fill is applied on a duplicate event"
    );
    assert_eq!(
        store.order(client)?.expect("order").filled_quantity,
        0,
        "no fill was applied on a duplicate event"
    );
    Ok(())
}

#[test]
fn fills_are_deduplicated_and_overfill_is_refused() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.sqlite"))?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    fresh(&mut store, &token, client, 10)?;
    let claim = store
        .claim_fenced(&token, "dispatcher-a", 2_000, 1_000)?
        .expect("claim");
    store.mark_dispatching_fenced(&token, claim.attempt_id, 2_100)?;
    store.acknowledge_fenced(&token, claim.attempt_id, &broker_id(), "evt-ack-1", 2_200)?;

    assert!(
        store.apply_fill_fenced(&token, client, "exec-1", "evt-fill-1", 4, 1_000, 2_300)?,
        "the first fill is accepted"
    );
    assert_eq!(
        store.order(client)?.expect("order").state,
        OmsOrderState::PartiallyFilled,
        "a partial fill leaves the order partially filled"
    );
    assert!(
        !store.apply_fill_fenced(&token, client, "exec-1", "evt-fill-1", 4, 1_000, 2_300)?,
        "an identical repeated execution id is ignored"
    );
    assert!(
        store.apply_fill_fenced(&token, client, "exec-2", "evt-fill-3", 6, 1_000, 2_500)?,
        "the remaining fill is accepted"
    );
    let order = store.order(client)?.expect("order");
    assert_eq!(
        order.state,
        OmsOrderState::Filled,
        "the order is fully filled"
    );
    assert_eq!(
        order.filled_quantity, 10,
        "filled quantity matches the order"
    );
    assert_eq!(order.filled_notional, 10_000, "fill notional is exact");
    let over = store.apply_fill_fenced(&token, client, "exec-3", "evt-fill-4", 1, 1_000, 2_600);
    assert!(
        matches!(
            over,
            Err(FencingError::Oms(OmsError::IllegalOrderTransition { .. }))
        ),
        "a filled order cannot take another fill"
    );
    Ok(())
}

#[test]
fn cancel_flow_uses_its_own_outbox_command() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.sqlite"))?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    fresh(&mut store, &token, client, 10)?;
    let claim = store
        .claim_fenced(&token, "dispatcher-a", 2_000, 1_000)?
        .expect("claim");
    store.mark_dispatching_fenced(&token, claim.attempt_id, 2_100)?;
    store.acknowledge_fenced(&token, claim.attempt_id, &broker_id(), "evt-ack-1", 2_200)?;

    let cancel_attempt = store.request_cancel_fenced(&token, client, 3_000)?;
    assert_eq!(
        store.order(client)?.expect("order").state,
        OmsOrderState::CancelPending,
        "cancellation moves the order to cancel pending"
    );
    assert!(
        store
            .outbox()?
            .iter()
            .any(|row| row.attempt_id == cancel_attempt && row.command_type == "CANCEL"),
        "a cancel outbox command exists"
    );
    assert!(
        store.apply_cancel_fenced(&token, client, "evt-cancel-1", 3_100)?,
        "the cancellation is accepted"
    );
    assert_eq!(
        store.order(client)?.expect("order").state,
        OmsOrderState::Cancelled,
        "the order is cancelled"
    );
    Ok(())
}

#[test]
fn illegal_transitions_are_refused() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.sqlite"))?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    fresh(&mut store, &token, client, 10)?;
    let illegal = store.apply_cancel_fenced(&token, client, "evt-cancel-1", 2_000);
    assert!(
        matches!(
            illegal,
            Err(FencingError::Oms(OmsError::IllegalOrderTransition { .. }))
        ),
        "a pending-submission order cannot be cancelled directly"
    );
    assert_eq!(
        store.order(client)?.expect("order").state,
        OmsOrderState::PendingSubmission,
        "the illegal transition left state unchanged"
    );
    Ok(())
}

/// Reopening the store replays committed state; an UNKNOWN order stays UNKNOWN
/// and is still listed for reconciliation after a process restart.
#[test]
fn committed_state_survives_restart() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("oms.sqlite");
    {
        let mut store = OmsStore::open(&path)?;
        let token = fence(&mut store);
        let client = ClientOrderId::new(1)?;
        fresh(&mut store, &token, client, 10)?;
        let claim = store
            .claim_fenced(&token, "dispatcher-a", 2_000, 1_000)?
            .expect("claim");
        store.mark_dispatching_fenced(&token, claim.attempt_id, 2_100)?;
        store.expire_dispatched_fenced(&token, 5_000, 1_000)?;
    }
    let store = OmsStore::open(&path)?;
    let client = ClientOrderId::new(1)?;
    let order = store.order(client)?.expect("order after restart");
    assert_eq!(
        order.state,
        OmsOrderState::Unknown,
        "UNKNOWN survives restart"
    );
    assert_eq!(
        order.intent,
        intent(1, 10)?,
        "the exact typed request survives restart"
    );
    assert_eq!(
        store.outbox()?[0].state,
        OutboxState::Unknown,
        "UNKNOWN survives restart in the outbox"
    );
    assert_eq!(
        store.unresolved()?.len(),
        1,
        "the order is listed for reconciliation"
    );
    Ok(())
}

#[test]
fn lifecycle_guards_terminal_states_and_unknown_path() -> TestResult {
    assert!(
        OmsOrderState::Unknown.permits(OmsOrderState::Reconciling),
        "UNKNOWN goes only to reconciliation"
    );
    assert!(
        !OmsOrderState::Unknown.permits(OmsOrderState::Working),
        "UNKNOWN cannot resume directly"
    );
    assert!(
        !OmsOrderState::Filled.permits(OmsOrderState::Cancelled),
        "terminal states never move"
    );
    assert!(
        OmsOrderState::Resolved.permits(OmsOrderState::Working),
        "resolution can restore working"
    );
    assert!(
        !OutboxState::Unknown.permits(OutboxState::Pending),
        "an UNKNOWN attempt is never requeued"
    );
    Ok(())
}

#[test]
fn no_broker_sdk_strategy_or_live_coupling() -> TestResult {
    let manifest = include_str!("../Cargo.toml").to_ascii_lowercase();
    for forbidden in [
        "upstox",
        "fyers",
        "kite",
        "strategy-runtime",
        "risk-authority",
        "broker-sdk",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "manifest must not depend on {forbidden}"
        );
    }
    Ok(())
}

#[test]
fn broker_identity_requires_a_real_value() -> TestResult {
    assert!(
        BrokerOrderId::new("").is_err(),
        "an empty broker id is refused"
    );
    assert!(
        ClientOrderId::new(0).is_err(),
        "a zero client id is refused"
    );
    Ok(())
}

/// A tampered or truncated journal tail is refused rather than replayed into
/// a plausible but wrong order state.
#[test]
fn journal_sequence_corruption_is_refused() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("oms.sqlite.journal");
    {
        let mut store = OmsStore::open(&path)?;
        let token = fence(&mut store);
        fresh(&mut store, &token, ClientOrderId::new(1)?, 10)?;
    }
    let mut file = std::fs::OpenOptions::new().append(true).open(&path)?;
    use std::io::Write as _;
    file.write_all(b"{\"seq\":99,\"op\":\"tamper\",\"state\":{\"seq\":99,\"next_attempt\":1,\"orders\":{},\"outbox\":{},\"events\":[]}}\n")?;
    file.sync_all()?;

    let reopened = OmsStore::open(&path);
    assert!(
        matches!(reopened, Err(OmsError::CorruptRecord(_))),
        "a broken sequence chain is refused on replay"
    );
    Ok(())
}

/// Changing a state field while leaving the original digest must be detected,
/// not replayed as a plausible order state.
#[test]
fn semantic_tampering_is_refused() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("oms.journal");
    {
        let mut store = OmsStore::open(&path)?;
        let token = fence(&mut store);
        fresh(&mut store, &token, ClientOrderId::new(1)?, 10)?;
    }
    let mut lines: Vec<String> = std::fs::read_to_string(&path)?
        .lines()
        .map(str::to_owned)
        .collect();
    let last = lines.last_mut().ok_or("journal has no records")?;
    let mut value: serde_json::Value = serde_json::from_str(last)?;
    value["state"]["next_attempt"] = serde_json::json!(999);
    *last = serde_json::to_string(&value)?;
    std::fs::write(&path, lines.join("\n") + "\n")?;

    let reopened = OmsStore::open(&path);
    assert!(
        matches!(reopened, Err(OmsError::CorruptRecord(_))),
        "a digest mismatch on a well-formed record is refused"
    );
    Ok(())
}

/// A reconciliation that resolves an order as filled must record the fill
/// quantities, not leave accounting fields at zero.
#[test]
fn filled_resolution_records_fill_accounting() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.journal"))?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    fresh(&mut store, &token, client, 10)?;
    let claim = store
        .claim_fenced(&token, "dispatcher-a", 2_000, 1_000)?
        .expect("claim");
    store.mark_dispatching_fenced(&token, claim.attempt_id, 2_100)?;
    store.expire_dispatched_fenced(&token, 5_000, 1_000)?;
    store.begin_reconciliation_fenced(&token, client, 5_100)?;

    store.resolve_unknown_fenced(
        &token,
        client,
        Resolution::Filled {
            broker_order_id: broker_id(),
            filled_quantity: 10,
            filled_notional: 10_000,
        },
        "evt-reconcile-filled",
        5_200,
    )?;
    let order = store.order(client)?.expect("order");
    assert_eq!(
        order.state,
        OmsOrderState::Filled,
        "the order is resolved as filled"
    );
    assert_eq!(
        order.filled_quantity, 10,
        "the reconciled fill quantity is recorded"
    );
    assert_eq!(
        order.filled_notional, 10_000,
        "the reconciled fill notional is recorded"
    );

    let wrong = store.resolve_unknown_fenced(
        &token,
        client,
        Resolution::Filled {
            broker_order_id: broker_id(),
            filled_quantity: 2,
            filled_notional: 2_000,
        },
        "evt-reconcile-wrong",
        5_300,
    );
    assert!(
        matches!(
            wrong,
            Err(FencingError::Oms(OmsError::IllegalOrderTransition { .. }))
        ),
        "a terminal filled order cannot be resolved again"
    );
    Ok(())
}

/// An invalid typed request is refused before it reaches the journal.
#[test]
fn invalid_typed_request_is_refused() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.journal"))?;
    let token = fence(&mut store);
    let mut invalid = intent(1, 10)?;
    invalid.reason = String::new();
    let result = store.create_fenced(&token, ClientOrderId::new(1)?, &invalid, &permit(), 1_000);
    assert!(
        matches!(
            result,
            Err(FencingError::Oms(OmsError::InvalidIdentifier(_)))
        ),
        "an intent missing decision evidence is refused"
    );
    Ok(())
}
