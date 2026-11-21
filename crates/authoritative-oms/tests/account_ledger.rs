#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M3.5.2 / M3.5.3 deterministic projection and reconciliation tests.
//! Fixtures are synthetic fills; no broker, live venue or account data exists.

use std::collections::{
    BTreeMap, //
    BTreeSet,
};

use account_journal::{
    CommitFault, //
    Journal,
    JournalError,
};
use authoritative_oms::{
    ChargeStatus, ClientOrderId, FillEvent, LEDGER_STREAM_ID, OmsError, OmsStore, PermitRef,
    ProjectionError, Projector, ReconciliationInput, Resolution, reconcile,
    store::fill_content_digest,
};
use trading_core::{
    AccountId, AlgoId, ChargeComponents, DomainError, EventEnvelope, EventId, EventPayload,
    EventTimes, Fill, FillId, InstrumentId, IntentId, JournalRef, LedgerChargeStatus,
    LedgerPostingKind, LogicalNanos, OpeningBalanceEvidence, OrderId, OrderIntent, Paise,
    PricePaise, Product, Quantity, ReconciliationStatus, Side, StoredBody, StrategyId, TradeDate,
    UtcNanos, Venue,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn account() -> AccountId {
    AccountId::new(1).expect("account")
}

fn instrument() -> InstrumentId {
    InstrumentId::new("nse.cash.test").expect("instrument")
}

fn date() -> TradeDate {
    TradeDate::new(2026, 10, 28).expect("date")
}

fn evidence(hash_hex: &str) -> OpeningBalanceEvidence {
    OpeningBalanceEvidence {
        statement_ref: "synthetic-statement".to_owned(),
        journal_ref: JournalRef {
            sequence: 1,
            hash_hex: hash_hex.to_owned(),
        },
        observed_at_ns: 1,
    }
}

fn projector(dir: &std::path::Path) -> Result<Projector, ProjectionError> {
    let mut projector = Projector::open(dir, account())?;
    projector.open_account("paper", Paise(10_000), date(), evidence(&"0".repeat(64)))?;
    Ok(projector)
}

fn fill(id: &str, quantity: i64, price: i64, charges: i64) -> FillEvent {
    FillEvent {
        schema_version: 1,
        execution_id: id.to_owned(),
        broker_event_id: format!("broker-{id}"),
        client_order_id: ClientOrderId::new(1).expect("client"),
        oms_order_id: "1".to_owned(),
        side: Side::Buy,
        quantity,
        price_paise: price,
        currency: "INR".to_owned(),
        broker_timestamp_ns: 10,
        received_at_ns: 10,
        source: "synthetic-test".to_owned(),
        account_scope: "paper".to_owned(),
        charges_paise: charges,
        charge_status: ChargeStatus::Estimated,
        provenance: "synthetic test fill".to_owned(),
    }
}

#[test]
fn execution_identity_is_content_addressed_and_stable_across_restart() -> TestResult {
    let directory = tempfile::tempdir()?;
    let event = fill("exec-1", 2, 100, 0);
    let before = fill_content_digest(&event);
    {
        let mut ledger = projector(directory.path())?;
        assert!(
            ledger.project_fill(&event, &instrument())?,
            "first projection"
        );
        assert_eq!(
            ledger.verification_report().record_count,
            2,
            "one opening and one posting"
        );
    }
    let reopened = projector(directory.path())?;
    assert_eq!(
        reopened.verification_report().record_count,
        2,
        "reopen does not duplicate the posting"
    );
    assert_eq!(
        reopened.state().cash,
        Paise(9_800),
        "cash survives a restart exactly"
    );
    let stored = reopened
        .verify_replay()?
        .cursor
        .last_content_digest
        .expect("digest");
    assert_eq!(stored, before, "the semantic digest is content-addressed");
    Ok(())
}

#[test]
fn duplicate_delivery_is_idempotent() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut ledger = projector(directory.path())?;
    let event = fill("exec-1", 2, 100, 0);
    assert!(
        ledger.project_fill(&event, &instrument())?,
        "first delivery"
    );
    let digest_before = ledger.state().state_digest();
    let records_before = ledger.verification_report().record_count;
    assert!(
        !ledger.project_fill(&event, &instrument())?,
        "duplicate delivery is a no-op"
    );
    assert_eq!(
        ledger.verification_report().record_count,
        records_before,
        "no second record was appended"
    );
    assert_eq!(
        ledger.state().state_digest(),
        digest_before,
        "the state digest is unchanged"
    );
    Ok(())
}

#[test]
fn same_execution_id_different_content_conflicts() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut ledger = projector(directory.path())?;
    ledger.project_fill(&fill("exec-1", 2, 100, 0), &instrument())?;
    let mut changed = fill("exec-1", 2, 100, 0);
    changed.charges_paise = 50;
    assert!(
        matches!(
            ledger.project_fill(&changed, &instrument()),
            Err(ProjectionError::ContentConflict { .. })
        ),
        "a reused execution id with different charges is a hard conflict"
    );
    assert_eq!(
        ledger.verification_report().record_count,
        2,
        "no conflicting record was appended"
    );

    // The OMS layer must reject the same anomaly rather than silently drop it.
    let oms_dir = tempfile::tempdir()?;
    let mut store = OmsStore::open(oms_dir.path().join("oms.journal"))?;
    setup_order(&mut store, 1, 10)?;
    assert!(
        store.apply_typed_fill(fill_for_order("exec-1", 1, 2, 100, 0))?,
        "the OMS accepts the conflicting fixture fill"
    );
    let mut oms_changed = fill_for_order("exec-1", 1, 2, 100, 0);
    oms_changed.price_paise = 101;
    assert!(
        matches!(
            store.apply_typed_fill(oms_changed),
            Err(OmsError::FillContentConflict { .. })
        ),
        "OMS raises a typed content conflict"
    );
    Ok(())
}

#[test]
fn partial_fills_accumulate_exactly() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut split = projector(directory.path())?;
    split.project_fill(&fill("exec-a", 2, 100, 10), &instrument())?;
    split.project_fill(&fill("exec-b", 3, 100, 20), &instrument())?;
    let other = tempfile::tempdir()?;
    let mut whole = projector(other.path())?;
    whole.project_fill(&fill("exec-c", 5, 100, 30), &instrument())?;

    assert_eq!(split.state().cash, whole.state().cash, "cash is exact");
    assert_eq!(
        split.state().positions["paper"][&instrument()],
        5,
        "partial quantities accumulate"
    );
    assert_eq!(
        split.state().cost_lots["paper"][&instrument()].cost_paise,
        whole.state().cost_lots["paper"][&instrument()].cost_paise,
        "weighted-average cost is independent of fill count"
    );
    assert_eq!(
        split.state().charges_estimated,
        Paise(30),
        "charges are summed separately"
    );
    Ok(())
}

#[test]
fn bust_reverses_exactly() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut ledger = projector(directory.path())?;
    ledger.project_fill(&fill("exec-1", 3, 100, 30), &instrument())?;
    assert!(ledger.bust("exec-1")?, "the bust is appended");
    assert_eq!(
        ledger.state().positions["paper"][&instrument()],
        0,
        "a bust leaves no residual position"
    );
    assert_eq!(
        ledger.state().cash,
        Paise(10_000),
        "cash returns to opening"
    );
    assert_eq!(
        ledger.state().charges_estimated,
        Paise(0),
        "no estimated charges remain"
    );
    assert!(
        ledger.verify_replay()?.matched,
        "replay reproduces the reversed state"
    );
    assert!(
        !ledger.bust("exec-1")?,
        "a duplicate bust is an idempotent no-op"
    );
    assert_eq!(
        ledger.state().cash,
        Paise(10_000),
        "a repeated bust leaves no residual"
    );
    Ok(())
}

#[test]
fn estimated_to_confirmed_charges_rebooks_only_delta() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut ledger = projector(directory.path())?;
    ledger.project_fill(&fill("exec-1", 2, 100, 100), &instrument())?;
    assert_eq!(
        ledger.state().charges_estimated,
        Paise(100),
        "the estimated charge bucket is recorded"
    );
    assert_eq!(ledger.state().cash, Paise(9_700), "fill cash is exact");
    assert!(
        ledger.confirm_charges("exec-1", 150)?,
        "charges are confirmed"
    );
    assert_eq!(
        ledger.state().charges_estimated,
        Paise(0),
        "no estimated charges remain"
    );
    assert_eq!(
        ledger.state().charges_confirmed,
        Paise(150),
        "the confirmed charge bucket is recorded"
    );
    assert_eq!(
        ledger.state().cash,
        Paise(9_650),
        "the net cash effect changes only by the 50-paise charge delta"
    );
    assert_eq!(
        ledger.state().positions["paper"][&instrument()],
        2,
        "the position is unchanged"
    );
    Ok(())
}

#[test]
fn opening_balance_requires_evidence_and_positive_cash() -> TestResult {
    let directory = tempfile::tempdir()?;
    let journal = Journal::open(directory.path(), LEDGER_STREAM_ID)?;
    let mut ledger = Projector::new(journal, account())?;
    let invalid = OpeningBalanceEvidence {
        statement_ref: String::new(),
        journal_ref: JournalRef {
            sequence: 1,
            hash_hex: "a".repeat(64),
        },
        observed_at_ns: 1,
    };
    assert!(
        matches!(
            ledger.open_account("paper", Paise(1_000), date(), invalid),
            Err(ProjectionError::Domain(DomainError::InvalidInstrument(_)))
        ),
        "an opening without evidence is refused"
    );
    assert!(
        matches!(
            ledger.open_account("paper", Paise(0), date(), evidence(&"0".repeat(64))),
            Err(ProjectionError::Overflow(_))
        ),
        "a non-positive opening balance is refused"
    );
    Ok(())
}

#[test]
fn crash_at_each_append_boundary() -> TestResult {
    for fault in [
        CommitFault::BeforeHeader,
        CommitFault::MidBody,
        CommitFault::AfterBodyBeforeSync,
        CommitFault::AfterSync,
        CommitFault::Rollover,
    ] {
        let directory = tempfile::tempdir()?;
        let segment_limit = if fault == CommitFault::Rollover {
            1_108
        } else {
            64 * 1024 * 1024
        };
        let journal =
            Journal::open_with_segment_limit(directory.path(), LEDGER_STREAM_ID, segment_limit)?;
        let mut ledger = Projector::new(journal, account())?;
        ledger.open_account("paper", Paise(10_000), date(), evidence(&"0".repeat(64)))?;
        let opening_records = ledger.verification_report().record_count;
        let mut crashing_fill = fill("exec-1", 2, 100, 0);
        if fault == CommitFault::Rollover {
            crashing_fill.provenance = "x".repeat(2_000);
        }
        ledger.force_commit_fault(Some(fault));
        let outcome = ledger.project_fill(&crashing_fill, &instrument());
        assert!(
            outcome.is_err(),
            "{fault:?} must surface an uncertain commit"
        );
        assert!(
            matches!(
                ledger.project_fill(&fill("exec-2", 2, 100, 0), &instrument()),
                Err(ProjectionError::Journal(JournalError::Poisoned))
                    | Err(ProjectionError::Journal(JournalError::CommitOutcomeUnknown))
            ),
            "the poisoned writer refuses further writes after {fault:?}"
        );
        drop(ledger);

        let reopened = projector(directory.path())?;
        let count = reopened.verification_report().record_count;
        let expected = if matches!(
            fault,
            CommitFault::AfterSync | CommitFault::AfterBodyBeforeSync
        ) {
            opening_records + 1
        } else {
            opening_records
        };
        assert_eq!(
            count, expected,
            "only the committed prefix survives {fault:?}"
        );
        assert!(reopened.verify_replay()?.matched, "replay after {fault:?}");
    }
    Ok(())
}

#[test]
fn restart_replay_digest_matches() -> TestResult {
    let directory = tempfile::tempdir()?;
    let expected;
    {
        let mut ledger = projector(directory.path())?;
        ledger.project_fill(&fill("exec-1", 2, 100, 10), &instrument())?;
        expected = ledger.verify_replay()?.cursor;
    }
    let reopened = projector(directory.path())?;
    let verification = reopened.verify_replay()?;
    assert!(
        verification.matched,
        "recomputed state equals committed state"
    );
    assert_eq!(
        verification.cursor, expected,
        "the replay cursor is derived from the journal"
    );
    Ok(())
}

#[test]
fn out_of_order_delivery_is_confluent() -> TestResult {
    let first_dir = tempfile::tempdir()?;
    let second_dir = tempfile::tempdir()?;
    let mut first = projector(first_dir.path())?;
    let mut second = projector(second_dir.path())?;
    let a = fill("exec-a", 2, 100, 7);
    let mut b = fill("exec-b", 3, 120, 11);
    b.broker_timestamp_ns = 5;
    first.project_fill(&a, &instrument())?;
    first.project_fill(&b, &instrument())?;
    second.project_fill(&b, &instrument())?;
    second.project_fill(&a, &instrument())?;
    assert_eq!(
        first.state().state_digest(),
        second.state().state_digest(),
        "delivery order cannot change the final state digest"
    );
    Ok(())
}

#[test]
fn currency_and_scope_mismatch_fails_closed() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut ledger = projector(directory.path())?;
    let mut foreign = fill("exec-1", 1, 100, 0);
    foreign.currency = "USD".to_owned();
    assert!(
        matches!(
            ledger.project_fill(&foreign, &instrument()),
            Err(ProjectionError::CurrencyNotInr { .. })
        ),
        "a non-INR fill is refused"
    );
    let mut other_scope = fill("exec-2", 1, 100, 0);
    other_scope.account_scope = "other".to_owned();
    assert!(
        matches!(
            ledger.project_fill(&other_scope, &instrument()),
            Err(ProjectionError::AccountNotOpen { .. })
        ),
        "an unopened scope is refused"
    );
    Ok(())
}

fn order_intent(id: u64, quantity: i64) -> OrderIntent {
    OrderIntent {
        intent_id: IntentId::new(id).expect("intent"),
        account_id: account(),
        strategy_id: StrategyId::new(2).expect("strategy"),
        strategy_version: "test-v1".to_owned(),
        algo_id: AlgoId::new(3).expect("algo"),
        instrument_id: instrument(),
        venue: Venue::Nse,
        side: Side::Buy,
        product: Product::Intraday,
        quantity: Quantity::new(quantity).expect("quantity"),
        limit_price: PricePaise::new(Paise(1_000)).expect("price"),
        reason: "synthetic reconciliation fixture".to_owned(),
        evidence: vec![JournalRef {
            sequence: 1,
            hash_hex: "a".repeat(64),
        }],
        expires_at: LogicalNanos::new(1_000_000),
    }
}

fn setup_order(store: &mut OmsStore, id: u64, quantity: i64) -> Result<(), OmsError> {
    let client = ClientOrderId::new(id)?;
    let order = order_intent(id, quantity);
    let permit = PermitRef {
        permit_id: 1,
        policy_version: "test".to_owned(),
        snapshot_revision: 1,
        digest: trading_core::permit_digest(1, "test", 1, &order),
    };
    store.create(client, &order, &permit, 1)?;
    store.accept_risk_approved(client, 2)?;
    let claim = store
        .claim("test-dispatcher", 3, 100)?
        .ok_or_else(|| OmsError::CorruptRecord("no claim".to_owned()))?;
    store.mark_dispatching(claim.attempt_id, 4)?;
    store.acknowledge(
        claim.attempt_id,
        &authoritative_oms::BrokerOrderId::new("BROKER-1")?,
        "ack-1",
        5,
    )?;
    Ok(())
}

fn fill_for_order(id: &str, order_id: u64, quantity: i64, price: i64, charges: i64) -> FillEvent {
    let mut event = fill(id, quantity, price, charges);
    event.client_order_id = ClientOrderId::new(order_id).expect("client");
    event
}

#[test]
fn duplicate_fill_does_not_double_project() -> TestResult {
    let oms_dir = tempfile::tempdir()?;
    let ledger_dir = tempfile::tempdir()?;
    let mut store = OmsStore::open(oms_dir.path().join("oms.journal"))?;
    setup_order(&mut store, 1, 10)?;
    let event = fill_for_order("exec-1", 1, 4, 100, 10);
    assert!(
        store.apply_typed_fill(event.clone())?,
        "the OMS accepts the first fill"
    );
    assert!(!store.apply_typed_fill(event.clone())?, "OMS dedups");
    let mut ledger = projector(ledger_dir.path())?;
    ledger.project_fill(&event, &instrument())?;
    let outcome = reconcile(
        &store,
        &ledger,
        &ReconciliationInput {
            deliveries: vec![event.clone(), event],
            instruments: BTreeMap::from([("exec-1".to_owned(), instrument())]),
        },
    )?;
    assert_eq!(
        outcome.report.status,
        ReconciliationStatus::Matched,
        "a duplicate delivery is not a mismatch"
    );
    assert_eq!(
        ledger.state().positions["paper"][&instrument()],
        4,
        "the duplicate did not double the position"
    );
    assert!(
        outcome
            .differences
            .iter()
            .all(|difference| !difference.is_blocking()),
        "duplicate evidence is informational"
    );
    Ok(())
}

#[test]
fn same_id_different_content_raises_mismatch() -> TestResult {
    let oms_dir = tempfile::tempdir()?;
    let ledger_dir = tempfile::tempdir()?;
    let mut store = OmsStore::open(oms_dir.path().join("oms.journal"))?;
    setup_order(&mut store, 1, 10)?;
    let event = fill_for_order("exec-1", 1, 4, 100, 10);
    store.apply_typed_fill(event.clone())?;
    let mut ledger = projector(ledger_dir.path())?;
    ledger.project_fill(&event, &instrument())?;
    let mut conflicting = event.clone();
    conflicting.charges_paise = 500;
    let outcome = reconcile(
        &store,
        &ledger,
        &ReconciliationInput {
            deliveries: vec![event, conflicting],
            instruments: BTreeMap::from([("exec-1".to_owned(), instrument())]),
        },
    )?;
    assert_eq!(
        outcome.report.status,
        ReconciliationStatus::Mismatch,
        "the anomaly is reported as a mismatch"
    );
    assert!(
        outcome
            .differences
            .iter()
            .any(|difference| difference.is_blocking()),
        "a content conflict is a blocking difference"
    );
    Ok(())
}

#[test]
fn partial_fill_sum_matches_order_aggregate() -> TestResult {
    let oms_dir = tempfile::tempdir()?;
    let ledger_dir = tempfile::tempdir()?;
    let mut store = OmsStore::open(oms_dir.path().join("oms.journal"))?;
    setup_order(&mut store, 1, 10)?;
    let first = fill_for_order("exec-1", 1, 4, 100, 0);
    let second = fill_for_order("exec-2", 1, 3, 100, 0);
    store.apply_typed_fill(first.clone())?;
    store.apply_typed_fill(second.clone())?;
    let mut ledger = projector(ledger_dir.path())?;
    ledger.project_fill(&first, &instrument())?;
    ledger.project_fill(&second, &instrument())?;
    let outcome = reconcile(
        &store,
        &ledger,
        &ReconciliationInput {
            deliveries: vec![first, second],
            instruments: BTreeMap::from([
                ("exec-1".to_owned(), instrument()),
                ("exec-2".to_owned(), instrument()),
            ]),
        },
    )?;
    assert_eq!(
        outcome.report.status,
        ReconciliationStatus::Matched,
        "a consistent pair of sources is matched"
    );
    assert_eq!(
        store
            .order(ClientOrderId::new(1).expect("client"))?
            .expect("order")
            .filled_quantity,
        7,
        "OMS keeps the aggregate"
    );
    Ok(())
}

#[test]
fn resolution_filled_without_event_is_a_mismatch() -> TestResult {
    let oms_dir = tempfile::tempdir()?;
    let ledger_dir = tempfile::tempdir()?;
    let mut store = OmsStore::open(oms_dir.path().join("oms.journal"))?;
    let client = ClientOrderId::new(1).expect("client");
    let order = order_intent(1, 4);
    let permit = PermitRef {
        permit_id: 1,
        policy_version: "test".to_owned(),
        snapshot_revision: 1,
        digest: trading_core::permit_digest(1, "test", 1, &order),
    };
    store.create(client, &order, &permit, 1)?;
    store.accept_risk_approved(client, 2)?;
    let claim = store.claim("test", 3, 100)?.expect("claim");
    store.mark_dispatching(claim.attempt_id, 4)?;
    store.expire_dispatched(5, 1)?;
    store.begin_reconciliation(client, 6)?;
    store.resolve_unknown(
        client,
        Resolution::Filled {
            broker_order_id: authoritative_oms::BrokerOrderId::new("B-1")?,
            filled_quantity: 4,
            filled_notional: 400,
        },
        "resolution-evidence",
        7,
    )?;
    let ledger = projector(ledger_dir.path())?;
    let outcome = reconcile(&store, &ledger, &ReconciliationInput::default())?;
    assert_eq!(
        outcome.report.status,
        ReconciliationStatus::Mismatch,
        "a filled resolution with no FillEvent is visible as a mismatch"
    );
    Ok(())
}

#[test]
fn cancel_ack_then_late_fill_is_mismatch() -> TestResult {
    let oms_dir = tempfile::tempdir()?;
    let ledger_dir = tempfile::tempdir()?;
    let mut store = OmsStore::open(oms_dir.path().join("oms.journal"))?;
    setup_order(&mut store, 1, 10)?;
    let client = ClientOrderId::new(1).expect("client");
    store.request_cancel(client, 20)?;
    store.apply_cancel(client, "cancel-evidence", 21)?;
    let mut late = fill_for_order("exec-late", 1, 1, 100, 0);
    late.broker_timestamp_ns = 30;
    let ledger = projector(ledger_dir.path())?;
    let outcome = reconcile(
        &store,
        &ledger,
        &ReconciliationInput {
            deliveries: vec![late],
            instruments: BTreeMap::new(),
        },
    )?;
    assert_eq!(
        outcome.report.status,
        ReconciliationStatus::Mismatch,
        "a late fill after cancellation is a hard mismatch"
    );
    Ok(())
}

#[test]
fn reconciliation_unavailable_blocks_like_mismatch() -> TestResult {
    let oms_dir = tempfile::tempdir()?;
    let ledger_dir = tempfile::tempdir()?;
    let mut store = OmsStore::open(oms_dir.path().join("oms.journal"))?;
    setup_order(&mut store, 1, 10)?;
    store.force_commit_fault(authoritative_oms::CommitFault::SyncFailure);
    let client = ClientOrderId::new(1).expect("client");
    let _ = store.request_cancel(client, 30);
    assert!(store.is_poisoned(), "the OMS writer is poisoned");
    let ledger = projector(ledger_dir.path())?;
    let outcome = reconcile(&store, &ledger, &ReconciliationInput::default())?;
    assert_eq!(
        outcome.report.status,
        ReconciliationStatus::Unavailable,
        "a poisoned OMS yields Unavailable"
    );
    assert!(
        outcome.report.observed_positions.is_none(),
        "Unavailable must not present an empty observed view as a match"
    );
    Ok(())
}

#[test]
fn mismatch_does_not_auto_clear() -> TestResult {
    let oms_dir = tempfile::tempdir()?;
    let ledger_dir = tempfile::tempdir()?;
    let mut store = OmsStore::open(oms_dir.path().join("oms.journal"))?;
    setup_order(&mut store, 1, 10)?;
    let event = fill_for_order("exec-1", 1, 4, 100, 0);
    store.apply_typed_fill(event.clone())?;
    let mut ledger = projector(ledger_dir.path())?;
    ledger.project_fill(&event, &instrument())?;
    let mut conflicting = event.clone();
    conflicting.charges_paise = 999;
    let input = ReconciliationInput {
        deliveries: vec![event.clone(), conflicting],
        instruments: BTreeMap::from([("exec-1".to_owned(), instrument())]),
    };
    let first = reconcile(&store, &ledger, &input)?;
    let second = reconcile(&store, &ledger, &input)?;
    assert_eq!(
        first.report.status,
        ReconciliationStatus::Mismatch,
        "the initial report is a mismatch"
    );
    assert_eq!(
        second.report.status,
        ReconciliationStatus::Mismatch,
        "repeated observation does not silently clear a mismatch"
    );
    Ok(())
}

#[test]
fn projection_payload_contract_rejects_bad_accounting() -> TestResult {
    // A projected posting whose cash delta contradicts its inputs never
    // reaches the journal: domain validation is the last line of defence.
    let event = fill("exec-1", 2, 100, 0);
    let bad_fill = Fill {
        fill_id: FillId::new(1)?,
        order_id: OrderId::new(1)?,
        side: Side::Buy,
        quantity: Quantity::new(2)?,
        price: PricePaise::new(Paise(100))?,
        charges: ChargeComponents::default(),
        cash_delta: Paise(-1),
    };
    assert!(
        matches!(
            bad_fill.validate_accounting(),
            Err(DomainError::InvalidInstrument(_))
        ),
        "a fill with contradictory cash is refused"
    );
    let body = StoredBody {
        envelope: EventEnvelope {
            event_id: EventId::parse("a".repeat(48))?,
            schema_version: 2,
            stream_id: LEDGER_STREAM_ID.to_owned(),
            source_id: Some("test".to_owned()),
            source_clock_domain: Some("test-clock".to_owned()),
            session_id: None,
            source_sequence: None,
            correlation_id: None,
            causation_id: Some(EventId::parse("b".repeat(48))?),
            times: EventTimes {
                source_time_utc: Some(UtcNanos::new(1)),
                received_time_utc: UtcNanos::new(1),
                available_time: LogicalNanos::new(1),
                recorded_time_utc: UtcNanos::new(1),
            },
            quality: BTreeSet::new(),
        },
        payload: EventPayload::ExecutionProjected {
            kind: LedgerPostingKind::Posting,
            execution_id: "exec-1".to_owned(),
            content_digest: "0".repeat(64),
            broker_event_id: "broker".to_owned(),
            client_order_id: 1,
            oms_order_id: "1".to_owned(),
            charge_status: LedgerChargeStatus::Estimated,
            account_scope: "paper".to_owned(),
            instrument_id: instrument(),
            provenance: "test".to_owned(),
            fill: bad_fill,
            cash_delta: Paise(-1),
        },
    };
    assert!(
        matches!(body.validate(), Err(DomainError::InvalidInstrument(_))),
        "the projected payload validates its accounting"
    );
    assert_eq!(fill_content_digest(&event).len(), 64, "digest is SHA-256");
    Ok(())
}

/// Review scenario: fill, confirm charges, then bust. The bust must reverse
/// the active (confirmed) posting, leaving cash and charge buckets exact.
#[test]
fn bust_after_confirm_charges_is_exact() -> TestResult {
    let directory = tempfile::tempdir()?;
    {
        let mut ledger = projector(directory.path())?;
        ledger.project_fill(&fill("exec-1", 2, 100, 100), &instrument())?;
        assert_eq!(
            ledger.state().cash,
            Paise(9_700),
            "filled with estimated charges"
        );
        assert!(ledger.confirm_charges("exec-1", 150)?, "charges confirmed");
        assert_eq!(
            ledger.state().cash,
            Paise(9_650),
            "only the charge delta moved"
        );
        assert!(ledger.bust("exec-1")?, "bust the active confirmed posting");
        assert_eq!(
            ledger.state().cash,
            Paise(10_000),
            "the bust nets back to the opening balance"
        );
        assert_eq!(
            ledger.state().positions["paper"][&instrument()],
            0,
            "no residual position"
        );
        assert_eq!(
            ledger.state().charges_estimated,
            Paise(0),
            "no negative estimate"
        );
        assert_eq!(
            ledger.state().charges_confirmed,
            Paise(0),
            "no residual confirmation"
        );
        assert!(
            ledger.verify_replay()?.matched,
            "replay reproduces the exact state"
        );
    }
    let reopened = projector(directory.path())?;
    assert_eq!(
        reopened.state().cash,
        Paise(10_000),
        "the exact state survives restart"
    );
    assert_eq!(
        reopened.state().charges_estimated,
        Paise(0),
        "no estimated charges survive the reopen"
    );
    assert_eq!(
        reopened.state().charges_confirmed,
        Paise(0),
        "no confirmed charges survive the bust"
    );
    Ok(())
}

/// Mirror scenario: a busted execution is not active, so confirming its
/// charges is refused with a typed error instead of double-reversing.
#[test]
fn confirm_after_bust_is_a_typed_refusal() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut ledger = projector(directory.path())?;
    ledger.project_fill(&fill("exec-1", 2, 100, 100), &instrument())?;
    assert!(ledger.bust("exec-1")?, "bust the posting");
    assert_eq!(
        ledger.state().cash,
        Paise(10_000),
        "cash is back at opening"
    );
    assert!(
        matches!(
            ledger.confirm_charges("exec-1", 150),
            Err(ProjectionError::ExecutionNotActive { .. })
        ),
        "a busted execution cannot be charge-confirmed"
    );
    assert_eq!(
        ledger.state().cash,
        Paise(10_000),
        "the refused confirmation changed nothing"
    );
    assert_eq!(
        ledger.state().charges_estimated,
        Paise(0),
        "no estimated charges remain"
    );
    assert_eq!(
        ledger.state().charges_confirmed,
        Paise(0),
        "no confirmed charges remain"
    );
    Ok(())
}

/// NB-2: a second confirmation with a different amount rebooks the delta
/// instead of silently no-opping on a reused reversal identity.
#[test]
fn repeated_confirm_with_different_amount_rebooks_exactly() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut ledger = projector(directory.path())?;
    ledger.project_fill(&fill("exec-1", 2, 100, 100), &instrument())?;
    assert!(
        ledger.confirm_charges("exec-1", 150)?,
        "charges are confirmed"
    );
    assert_eq!(
        ledger.state().cash,
        Paise(9_650),
        "the charge delta is exact"
    );
    assert!(
        ledger.confirm_charges("exec-1", 200)?,
        "a new amount is a new reversal"
    );
    assert_eq!(
        ledger.state().cash,
        Paise(9_600),
        "the 50-paise delta is rebooked"
    );
    assert_eq!(
        ledger.state().charges_estimated,
        Paise(0),
        "no estimated charges remain"
    );
    assert_eq!(
        ledger.state().charges_confirmed,
        Paise(200),
        "the second confirmation is authoritative"
    );
    assert_eq!(
        ledger.state().positions["paper"][&instrument()],
        2,
        "charge confirmations do not change position"
    );
    assert!(
        ledger.verify_replay()?.matched,
        "replay reproduces the exact state"
    );
    Ok(())
}

/// NB-1: an unknown delivery is a blocking anomaly and must be reflected in
/// the report status, not listed in `differences`.
#[test]
fn unknown_delivery_produces_mismatch_status() -> TestResult {
    let oms_dir = tempfile::tempdir()?;
    let ledger_dir = tempfile::tempdir()?;
    let store = OmsStore::open(oms_dir.path().join("oms.journal"))?;
    let ledger = projector(ledger_dir.path())?;
    let mut unknown = fill("exec-unknown", 1, 100, 0);
    unknown.client_order_id = ClientOrderId::new(1).expect("client");
    let outcome = reconcile(
        &store,
        &ledger,
        &ReconciliationInput {
            deliveries: vec![unknown],
            instruments: BTreeMap::new(),
        },
    )?;
    assert_eq!(
        outcome.report.status,
        ReconciliationStatus::Mismatch,
        "an unknown delivery is consistently blocking"
    );
    assert!(
        outcome
            .differences
            .iter()
            .any(|difference| difference.is_blocking()),
        "the reported difference is marked blocking"
    );
    Ok(())
}
