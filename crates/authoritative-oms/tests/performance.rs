#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Opt-in OMS durability and reconciliation benchmark. All commits and faults
//! are synthetic and in-process; no broker, provider or live routing exists.
//! Reported latencies and rates are measured locally and are not thresholds,
//! gates or a durability/throughput certificate.

use std::{
    collections::BTreeSet, //
    time::Instant,
};

use authoritative_oms::{
    BrokerOrderId, //
    ChargeStatus,
    ClientOrderId,
    CommitFault,
    FencingToken,
    FillEvent,
    OmsOrderState,
    OmsStore,
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
    store.fence("oms-benchmark-owner").expect("fence the OMS")
}

const COMMITS: u64 = 200;
const DUPLICATES: u64 = 2_000;
const UNKNOWN_ORDERS: u64 = 64;

fn intent(id: u64, quantity: i64) -> Result<OrderIntent, Box<dyn std::error::Error>> {
    Ok(OrderIntent {
        intent_id: IntentId::new(id)?,
        account_id: AccountId::new(1)?,
        strategy_id: StrategyId::new(2)?,
        strategy_version: "bench-strategy-v1".to_owned(),
        algo_id: AlgoId::new(3)?,
        instrument_id: InstrumentId::new("nse.cash.synidx")?,
        venue: Venue::Nse,
        side: Side::Buy,
        product: Product::Intraday,
        quantity: Quantity::new(quantity)?,
        limit_price: PricePaise::new(Paise(1_000))?,
        reason: "oms benchmark fixture".to_owned(),
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
        policy_version: "risk-bench-v1".to_owned(),
        snapshot_revision: 3,
        digest: permit_digest(permit_id, "risk-bench-v1", 3, intent),
    }
}

fn fill_event(execution_id: &str, broker_event_id: &str, client: u64) -> FillEvent {
    FillEvent {
        schema_version: 1,
        execution_id: execution_id.to_owned(),
        broker_event_id: broker_event_id.to_owned(),
        client_order_id: ClientOrderId::new(client).expect("client id"),
        oms_order_id: client.to_string(),
        side: Side::Buy,
        quantity: 10,
        price_paise: 1_000,
        currency: "INR".to_owned(),
        broker_timestamp_ns: 2_300,
        received_at_ns: 2_310,
        source: "synthetic-broker".to_owned(),
        account_scope: "paper-account-bench".to_owned(),
        charges_paise: 20,
        charge_status: ChargeStatus::BrokerConfirmed,
        provenance: "typed fill benchmark fixture".to_owned(),
    }
}

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
        assert_eq!(percentile(&samples, 0.5), 30, "median sample");
        assert_eq!(percentile(&samples, 0.999), 50, "upper sample");
        assert_eq!(percentile(&[], 0.5), 0, "empty input has no rank");
    }
}

#[test]
#[ignore = "opt-in performance measurement"]
fn restart_recovery_after_k_commits_with_injected_fault() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("oms.journal");
    let mut sync_failure_count = 0_u64;
    let partial_write_count = 0_u64;
    {
        let mut store = OmsStore::open(&path)?;
        let token = fence(&mut store);
        for id in 1..=COMMITS {
            store.create_fenced(
                &token,
                ClientOrderId::new(id)?,
                &intent(id, 1)?,
                &permit(id, &intent(id, 1)?),
                id as i64 * 1_000,
            )?;
        }
        store.force_commit_fault(CommitFault::SyncFailure);
        sync_failure_count += 1;
        let uncertain = COMMITS + 1;
        let failed = store.create_fenced(
            &token,
            ClientOrderId::new(uncertain)?,
            &intent(uncertain, 1)?,
            &permit(uncertain, &intent(uncertain, 1)?),
            uncertain as i64 * 1_000,
        );
        assert!(failed.is_err(), "the injected sync failure is surfaced");
        assert!(store.is_poisoned(), "the uncertain writer is poisoned");
    }
    let started = Instant::now();
    let recovered = OmsStore::recover(&path)?;
    let elapsed = started.elapsed();
    let mut replayed = 0_u64;
    for id in 1..=COMMITS + 1 {
        if recovered.order(ClientOrderId::new(id)?)?.is_some() {
            replayed += 1;
        }
    }
    eprintln!(
        "BENCH-OMS-RECOVERY commits={} replayed={replayed} seconds={:.3} replay_per_sec={:.1} fault=SyncFailure sync_failure_count={sync_failure_count} partial_write_count={partial_write_count} source=synthetic-local",
        COMMITS + 1,
        elapsed.as_secs_f64(),
        replayed as f64 / elapsed.as_secs_f64()
    );
    assert_eq!(
        replayed,
        COMMITS + 1,
        "the durable prefix is fully replayed after recovery"
    );
    Ok(())
}

#[test]
#[ignore = "opt-in performance measurement"]
fn duplicate_event_handling_rate() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("oms.journal");
    let mut store = OmsStore::open(&path)?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    store.create_fenced(
        &token,
        client,
        &intent(1, 10)?,
        &permit(7, &intent(1, 10)?),
        1_000,
    )?;
    store.accept_risk_approved_fenced(&token, client, 1_010)?;
    let claim = store
        .claim_fenced(&token, "bench-dispatcher", 2_000, 1_000)?
        .expect("claim");
    store.mark_dispatching_fenced(&token, claim.attempt_id, 2_100)?;
    store.acknowledge_fenced(
        &token,
        claim.attempt_id,
        &BrokerOrderId::new("B-bench-1")?,
        "evt-bench-ack",
        2_200,
    )?;
    let event = fill_event("exec-bench-1", "evt-bench-fill-1", 1);
    assert!(
        store.apply_typed_fill_fenced(&token, event.clone())?,
        "first fill is accepted"
    );

    // No durable-write fault is injected in the duplicate-delivery workload.
    let sync_failure_count = 0_u64;
    let partial_write_count = 0_u64;
    let mut latencies = Vec::with_capacity(DUPLICATES as usize);
    let mut ignored = 0_u64;
    let started = Instant::now();
    for _ in 0..DUPLICATES {
        let item_started = Instant::now();
        if !store.apply_typed_fill_fenced(&token, event.clone())? {
            ignored += 1;
        }
        latencies.push(item_started.elapsed().as_nanos());
    }
    let elapsed = started.elapsed();
    latencies.sort_unstable();
    eprintln!(
        "BENCH-OMS-DUPLICATE attempts={DUPLICATES} ignored={ignored} seconds={:.3} duplicates_per_sec={:.1} p50_us={:.1} p99_us={:.1} p999_us={:.1} sync_failure_count={sync_failure_count} partial_write_count={partial_write_count} source=synthetic-local",
        elapsed.as_secs_f64(),
        ignored as f64 / elapsed.as_secs_f64(),
        percentile(&latencies, 0.50) as f64 / 1000.0,
        percentile(&latencies, 0.99) as f64 / 1000.0,
        percentile(&latencies, 0.999) as f64 / 1000.0,
    );
    assert_eq!(ignored, DUPLICATES, "every duplicate delivery is ignored");
    assert_eq!(store.fills().len(), 1, "no duplicate fill is persisted");
    Ok(())
}

#[test]
#[ignore = "opt-in performance measurement"]
fn reconciliation_completeness_against_injected_unknown_set() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("oms.journal");
    let mut store = OmsStore::open(&path)?;
    let token = fence(&mut store);
    for id in 1..=UNKNOWN_ORDERS {
        let client = ClientOrderId::new(id)?;
        store.create_fenced(
            &token,
            client,
            &intent(id, 1)?,
            &permit(id, &intent(id, 1)?),
            1_000,
        )?;
        store.accept_risk_approved_fenced(&token, client, 1_010)?;
        let claim = store
            .claim_fenced(&token, "bench-dispatcher", 2_000, 1_000)?
            .expect("claim");
        store.mark_dispatching_fenced(&token, claim.attempt_id, 2_100)?;
    }
    let expired = store.expire_dispatched_fenced(&token, 5_000, 0)?;
    assert_eq!(
        expired, UNKNOWN_ORDERS as usize,
        "every dispatched order becomes UNKNOWN"
    );
    let injected: BTreeSet<u64> = store
        .unresolved()?
        .into_iter()
        .map(|view| view.client_order_id.get())
        .collect();
    assert_eq!(
        injected.len(),
        UNKNOWN_ORDERS as usize,
        "the injected UNKNOWN set is complete"
    );

    // No durable-write fault is injected in the reconciliation workload.
    let sync_failure_count = 0_u64;
    let partial_write_count = 0_u64;
    let mut latencies = Vec::with_capacity(UNKNOWN_ORDERS as usize);
    let started = Instant::now();
    for id in 1..=UNKNOWN_ORDERS {
        let client = ClientOrderId::new(id)?;
        let item_started = Instant::now();
        store.begin_reconciliation_fenced(&token, client, 5_100)?;
        store.resolve_unknown_fenced(
            &token,
            client,
            Resolution::Working {
                broker_order_id: BrokerOrderId::new(format!("B-bench-{id}"))?,
            },
            &format!("evt-bench-reconcile-{id}"),
            5_200,
        )?;
        latencies.push(item_started.elapsed().as_nanos());
    }
    let elapsed = started.elapsed();
    latencies.sort_unstable();
    let mut resolved = BTreeSet::new();
    for id in &injected {
        if let Some(view) = store.order(ClientOrderId::new(*id)?)?
            && view.state == OmsOrderState::Working
        {
            resolved.insert(*id);
        }
    }
    assert_eq!(resolved, injected, "every injected UNKNOWN order resolved");
    assert!(
        store.unresolved()?.is_empty(),
        "no injected UNKNOWN order remains unresolved"
    );
    eprintln!(
        "BENCH-OMS-RECON unknown_injected={} resolved={} complete=true seconds={:.3} reconciliations_per_sec={:.1} p50_us={:.1} p99_us={:.1} p999_us={:.1} sync_failure_count={sync_failure_count} partial_write_count={partial_write_count} source=synthetic-local",
        injected.len(),
        resolved.len(),
        elapsed.as_secs_f64(),
        resolved.len() as f64 / elapsed.as_secs_f64(),
        percentile(&latencies, 0.50) as f64 / 1000.0,
        percentile(&latencies, 0.99) as f64 / 1000.0,
        percentile(&latencies, 0.999) as f64 / 1000.0,
    );
    Ok(())
}
