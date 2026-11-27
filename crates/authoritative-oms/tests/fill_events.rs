#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M3.5.1 typed execution-event tests. All fills are simulated in-process.

use authoritative_oms::{
    BrokerOrderId, //
    ChargeStatus,
    ClientOrderId,
    FencingError,
    FencingToken,
    FillEvent,
    OmsError,
    OmsStore,
    PermitRef,
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
    store.fence("fill-events-owner").expect("fence the OMS")
}

fn intent() -> Result<OrderIntent, Box<dyn std::error::Error>> {
    Ok(OrderIntent {
        intent_id: IntentId::new(1)?,
        account_id: AccountId::new(1)?,
        strategy_id: StrategyId::new(2)?,
        strategy_version: "strategy-v1".to_owned(),
        algo_id: AlgoId::new(3)?,
        instrument_id: InstrumentId::new("nse.cash.synidx")?,
        venue: Venue::Nse,
        side: Side::Buy,
        product: Product::Intraday,
        quantity: Quantity::new(10)?,
        limit_price: PricePaise::new(Paise(1_000))?,
        reason: "typed fill fixture".to_owned(),
        evidence: vec![JournalRef {
            sequence: 1,
            hash_hex: "ab".repeat(32),
        }],
        expires_at: LogicalNanos::new(100_000),
    })
}

fn permit() -> PermitRef {
    let canonical = intent().expect("canonical fixture intent");
    PermitRef {
        permit_id: 7,
        policy_version: "risk-v1".to_owned(),
        snapshot_revision: 3,
        digest: permit_digest(7, "risk-v1", 3, &canonical),
    }
}

fn working(store: &mut OmsStore, token: &FencingToken, client: ClientOrderId) -> TestResult {
    store.create_fenced(&token, client, &intent()?, &permit(), 1_000)?;
    store.accept_risk_approved_fenced(&token, client, 1_010)?;
    let claim = store
        .claim_fenced(&token, "dispatcher-a", 2_000, 1_000)?
        .expect("claim");
    store.mark_dispatching_fenced(&token, claim.attempt_id, 2_100)?;
    store.acknowledge_fenced(
        &token,
        claim.attempt_id,
        &BrokerOrderId::new("B-1")?,
        "evt-ack",
        2_200,
    )?;
    Ok(())
}

fn event(execution_id: &str, broker_event_id: &str, quantity: i64) -> FillEvent {
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
        broker_timestamp_ns: 2_300,
        received_at_ns: 2_310,
        source: "simulated-broker".to_owned(),
        account_scope: "paper-account-1".to_owned(),
        charges_paise: 20,
        charge_status: ChargeStatus::BrokerConfirmed,
        provenance: "typed fill fixture".to_owned(),
    }
}

#[test]
fn each_fill_is_persisted_as_a_typed_event() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("oms.journal");
    let mut store = OmsStore::open(&path)?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    working(&mut store, &token, client)?;

    assert!(
        store.apply_typed_fill_fenced(&token, event("exec-1", "evt-fill-1", 4))?,
        "first fill is accepted"
    );
    assert!(
        store.apply_typed_fill_fenced(&token, event("exec-2", "evt-fill-2", 6))?,
        "second fill is accepted"
    );
    let fills = store.fills();
    assert_eq!(fills.len(), 2, "each execution has its own durable record");
    assert_eq!(fills[0].execution_id, "exec-1", "first execution id");
    assert_eq!(fills[1].execution_id, "exec-2", "second execution id");
    let first = store.fill("exec-1").expect("persisted execution");
    assert_eq!(first.charges_paise, 20, "charge value is retained");
    assert_eq!(
        first.charge_status,
        ChargeStatus::BrokerConfirmed,
        "confirmed charges are distinguished from estimates"
    );
    assert_eq!(first.currency, "INR", "currency is retained");
    assert_eq!(first.received_at_ns, 2_310, "fill event field");
    assert_eq!(first.broker_timestamp_ns, 2_300, "fill event field");
    assert_eq!(first.account_scope, "paper-account-1", "scope is retained");
    Ok(())
}

#[test]
fn duplicate_execution_is_idempotent_and_different_content_conflicts() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.journal"))?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    working(&mut store, &token, client)?;
    assert!(
        store.apply_typed_fill_fenced(&token, event("exec-1", "evt-fill-1", 4))?,
        "first fill is accepted"
    );
    assert!(
        !store.apply_typed_fill_fenced(&token, event("exec-1", "evt-fill-1", 4))?,
        "an identical duplicate is idempotent"
    );
    assert_eq!(
        store.fills().len(),
        1,
        "no second economic event is recorded"
    );
    let mut conflicting = event("exec-1", "evt-fill-1", 4);
    conflicting.price_paise = 1_001;
    assert!(
        matches!(
            store.apply_typed_fill_fenced(&token, conflicting),
            Err(FencingError::Oms(OmsError::FillContentConflict { .. }))
        ),
        "a reused execution id with different content is a hard conflict"
    );
    assert_eq!(
        store.fills().len(),
        1,
        "the conflicting event did not replace the original"
    );
    Ok(())
}

#[test]
fn typed_events_survive_restart() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("oms.journal");
    {
        let mut store = OmsStore::open(&path)?;
        let token = fence(&mut store);
        working(&mut store, &token, ClientOrderId::new(1)?)?;
        store.apply_typed_fill_fenced(&token, event("exec-1", "evt-fill-1", 4))?;
    }
    let store = OmsStore::open(&path)?;
    let fills = store.fills();
    assert_eq!(fills.len(), 1, "the execution event is durable");
    assert_eq!(
        fills[0].execution_id, "exec-1",
        "execution survives restart"
    );
    assert_eq!(fills[0].charges_paise, 20, "fill event field");
    Ok(())
}

#[test]
fn malformed_typed_event_is_refused() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.journal"))?;
    let token = fence(&mut store);
    working(&mut store, &token, ClientOrderId::new(1)?)?;
    let mut bad = event("exec-1", "evt-fill-1", 4);
    bad.schema_version = 2;
    assert!(
        matches!(
            store.apply_typed_fill_fenced(&token, bad),
            Err(FencingError::Oms(OmsError::InvalidIdentifier(_)))
        ),
        "an unsupported event schema is rejected"
    );
    Ok(())
}
