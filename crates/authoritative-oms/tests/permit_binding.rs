#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Permit-to-intent binding tests. All permits are synthetic local digests;
//! no signature, KMS key or signed-permit capability is claimed here.

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
    store.fence("permit-binding-owner").expect("fence the OMS")
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
        reason: "permit binding fixture".to_owned(),
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
fn tampered_intent_after_issuance_is_denied() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("oms.journal");
    let mut store = OmsStore::open(&path)?;
    let token = fence(&mut store);
    let authorized = intent(1, 10)?;
    let issued = permit(7, &authorized);

    // A permit issued for 10 lots is presented with a tampered 11-lot intent.
    // The OMS recomputes the digest from the submitted terms and denies it.
    let mut tampered = authorized;
    tampered.quantity = Quantity::new(11)?;
    let result = store.create_fenced(&token, ClientOrderId::new(1)?, &tampered, &issued, 1_100);
    assert!(
        matches!(
            result,
            Err(FencingError::Oms(OmsError::PermitDigestMismatch {
                permit_id: 7
            }))
        ),
        "a tampered intent must not open an order: {result:?}"
    );
    Ok(())
}

#[test]
fn tampered_permit_identity_is_denied() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.journal"))?;
    let token = fence(&mut store);
    let authorized = intent(1, 10)?;
    let issued = permit(7, &authorized);
    let wrong_version = PermitRef {
        policy_version: "risk-v2".to_owned(),
        ..issued.clone()
    };
    let result = store.create_fenced(
        &token,
        ClientOrderId::new(1)?,
        &authorized,
        &wrong_version,
        1_000,
    );
    assert!(
        matches!(
            result,
            Err(FencingError::Oms(OmsError::PermitDigestMismatch {
                permit_id: 7
            }))
        ),
        "a permit with a rewritten policy version must be denied: {result:?}"
    );

    let wrong_revision = PermitRef {
        snapshot_revision: 4,
        ..issued.clone()
    };
    let result = store.create_fenced(
        &token,
        ClientOrderId::new(2)?,
        &authorized,
        &wrong_revision,
        1_000,
    );
    assert!(
        matches!(
            result,
            Err(FencingError::Oms(OmsError::PermitDigestMismatch {
                permit_id: 7
            }))
        ),
        "a permit with a rewritten snapshot revision must be denied: {result:?}"
    );
    Ok(())
}

#[test]
fn identical_replay_is_idempotent_and_a_new_client_id_is_denied() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.journal"))?;
    let token = fence(&mut store);
    let order = intent(1, 10)?;
    let issued = permit(7, &order);
    store.create_fenced(&token, ClientOrderId::new(1)?, &order, &issued, 1_000)?;

    store.create_fenced(&token, ClientOrderId::new(1)?, &order, &issued, 2_000)?;
    assert_eq!(
        store.orders()?.len(),
        1,
        "an identical replay is an idempotent no-op"
    );

    let reused = store.create_fenced(&token, ClientOrderId::new(2)?, &order, &issued, 3_000);
    assert!(
        matches!(
            reused,
            Err(FencingError::Oms(OmsError::PermitAlreadyConsumed {
                permit_id: 7
            }))
        ),
        "a spent permit cannot open a second client order: {reused:?}"
    );
    assert_eq!(store.orders()?.len(), 1, "the denied replay added no order");
    Ok(())
}

#[test]
fn consumed_permits_survive_recovery() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("oms.journal");
    let order = intent(1, 10)?;
    let issued = permit(7, &order);
    {
        let mut store = OmsStore::open(&path)?;
        let token = fence(&mut store);
        store.create_fenced(&token, ClientOrderId::new(1)?, &order, &issued, 1_000)?;
    }
    let mut recovered = OmsStore::recover(&path)?;
    let token = fence(&mut recovered);
    let replay = recovered.create_fenced(&token, ClientOrderId::new(2)?, &order, &issued, 2_000);
    assert!(
        matches!(
            replay,
            Err(FencingError::Oms(OmsError::PermitAlreadyConsumed {
                permit_id: 7
            }))
        ),
        "the consumed set must replay after restart: {replay:?}"
    );
    assert_eq!(
        recovered.orders()?.len(),
        1,
        "recovery preserves exactly the committed order"
    );
    Ok(())
}
