#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M3.5.0 OMS journal durability tests. All faults are injected in-process;
//! no broker, provider or live routing is involved.

use std::fs;

use authoritative_oms::{
    ClientOrderId, //
    FencingError,
    FencingToken,
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
    store.fence("durability-owner").expect("fence the OMS")
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
        reason: "oms durability fixture".to_owned(),
        evidence: vec![JournalRef {
            sequence: 1,
            hash_hex: "ab".repeat(32),
        }],
        expires_at: LogicalNanos::new(100_000),
    })
}

fn permit(permit_id: u64, intent: &OrderIntent) -> PermitRef {
    PermitRef {
        permit_id,
        policy_version: "risk-v1".to_owned(),
        snapshot_revision: 3,
        digest: permit_digest(permit_id, "risk-v1", 3, intent),
    }
}

#[test]
fn a_second_writer_is_refused() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("oms.journal");
    let _first = OmsStore::open(&path)?;
    let second = OmsStore::open(&path);
    assert!(
        matches!(second, Err(OmsError::WriterLocked)),
        "two processes must not append from stale state"
    );
    Ok(())
}

#[test]
fn writer_lock_is_released_when_the_store_is_dropped() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("oms.journal");
    {
        let mut first = OmsStore::open(&path)?;
        let token = fence(&mut first);
        first.create_fenced(
            &token,
            ClientOrderId::new(1)?,
            &intent(1, 10)?,
            &permit(7, &intent(1, 10)?),
            1_000,
        )?;
    }
    let reopened = OmsStore::open(&path)?;
    assert!(
        reopened.order(ClientOrderId::new(1)?)?.is_some(),
        "the journal replays after the writer is released"
    );
    Ok(())
}

#[test]
fn partial_write_poisons_the_writer_and_recovery_fails_closed() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("oms.journal");
    let mut store = OmsStore::open(&path)?;
    let token = fence(&mut store);
    store.force_commit_fault(authoritative_oms::CommitFault::PartialWrite);
    let failed = store.create_fenced(
        &token,
        ClientOrderId::new(1)?,
        &intent(1, 10)?,
        &permit(7, &intent(1, 10)?),
        1_000,
    );
    assert!(failed.is_err(), "an incomplete append must fail");
    assert!(store.is_poisoned(), "the writer must be poisoned");

    let after = store.create_fenced(
        &token,
        ClientOrderId::new(2)?,
        &intent(1, 11)?,
        &permit(8, &intent(1, 11)?),
        1_100,
    );
    assert!(
        matches!(after, Err(FencingError::Oms(OmsError::WriterPoisoned))),
        "a poisoned writer must refuse further mutations"
    );
    assert!(
        OmsStore::recover(&path).is_err(),
        "a torn tail must not be silently accepted on recovery"
    );
    Ok(())
}

#[test]
fn sync_failure_poisons_the_writer_and_recovery_replays_the_durable_prefix() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("oms.journal");
    let mut store = OmsStore::open(&path)?;
    let token = fence(&mut store);
    store.create_fenced(
        &token,
        ClientOrderId::new(1)?,
        &intent(1, 10)?,
        &permit(7, &intent(1, 10)?),
        1_000,
    )?;
    store.force_commit_fault(authoritative_oms::CommitFault::SyncFailure);
    let failed = store.create_fenced(
        &token,
        ClientOrderId::new(2)?,
        &intent(1, 11)?,
        &permit(8, &intent(1, 11)?),
        1_100,
    );
    assert!(failed.is_err(), "an uncertain durability result must fail");
    assert!(store.is_poisoned(), "the writer must stay poisoned");
    assert!(
        matches!(
            store.create_fenced(
                &token,
                ClientOrderId::new(3)?,
                &intent(1, 12)?,
                &permit(9, &intent(1, 12)?),
                1_200
            ),
            Err(FencingError::Oms(OmsError::WriterPoisoned))
        ),
        "the poisoned instance stays unusable"
    );
    drop(store);

    let recovered = OmsStore::recover(&path)?;
    assert!(!recovered.is_poisoned(), "recovery yields a clean writer");
    assert!(
        recovered.order(ClientOrderId::new(1)?)?.is_some()
            && recovered.order(ClientOrderId::new(2)?)?.is_some(),
        "the fully written record is replayed after recovery"
    );
    Ok(())
}

#[test]
fn restart_preserves_event_order() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("oms.journal");
    {
        let mut store = OmsStore::open(&path)?;
        let token = fence(&mut store);
        store.create_fenced(
            &token,
            ClientOrderId::new(1)?,
            &intent(1, 10)?,
            &permit(7, &intent(1, 10)?),
            1_000,
        )?;
        store.accept_risk_approved_fenced(&token, ClientOrderId::new(1)?, 1_010)?;
    }
    let mut store = OmsStore::open(&path)?;
    let token = fence(&mut store);
    let order = store
        .order(ClientOrderId::new(1)?)?
        .expect("the order survives restart");
    assert_eq!(
        order.state,
        authoritative_oms::OmsOrderState::PendingSubmission,
        "the last committed state survives restart"
    );
    store.create_fenced(
        &token,
        ClientOrderId::new(2)?,
        &intent(1, 20)?,
        &permit(10, &intent(1, 20)?),
        2_000,
    )?;
    let lines = fs::read_to_string(&path)?;
    let sequences: Vec<u64> = lines
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|value| value.get("seq").and_then(|s| s.as_u64()))
        .collect();
    assert_eq!(
        sequences,
        vec![1, 2, 3, 4, 5],
        "fence records and order records share one contiguous sequence"
    );
    Ok(())
}
