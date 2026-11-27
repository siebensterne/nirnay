#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.6 adapter contract tests. The broker is a deterministic simulator; no
//! real endpoint, SDK or live order path exists.

use authoritative_oms::{
    ClientOrderId, //
    DispatchClaim,
    FencingToken,
    OmsOrderState,
    OmsStore,
    PermitRef,
    Resolution,
};
use broker_adapter::{
    BrokerAdapter, //
    BrokerEvent,
    BrokerOrderRequest,
    CancelRequest,
    SimulatedBroker,
    SimulatorConfig,
    SubmissionOutcome,
    SubmitBehavior,
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
    store
        .fence("adapter-contract-owner")
        .expect("fence the OMS")
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
        reason: "adapter fixture".to_owned(),
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
        permit_id: 9,
        policy_version: "risk-v1".to_owned(),
        snapshot_revision: 2,
        digest: permit_digest(9, "risk-v1", 2, &canonical),
    }
}

fn dispatch(
    store: &mut OmsStore,
    token: &FencingToken,
    client: ClientOrderId,
    quantity: i64,
) -> Result<DispatchClaim, Box<dyn std::error::Error>> {
    store.create_fenced(token, client, &intent(1, quantity)?, &permit(), 1_000)?;
    store.accept_risk_approved_fenced(token, client, 1_010)?;
    let claim = store
        .claim_fenced(token, "dispatcher", 2_000, 1_000)?
        .ok_or("no claim")?;
    store.mark_dispatching_fenced(token, claim.attempt_id, 2_100)?;
    Ok(claim)
}

fn configured(behavior: SubmitBehavior) -> Result<SimulatedBroker, Box<dyn std::error::Error>> {
    let mut broker = SimulatedBroker::new(SimulatorConfig::new("sim-nse", behavior))?;
    broker.authenticate();
    Ok(broker)
}

#[test]
fn request_binds_the_complete_typed_order_request() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.journal"))?;
    let token = fence(&mut store);
    let claim = dispatch(&mut store, &token, ClientOrderId::new(1)?, 10)?;
    let request = BrokerOrderRequest::from_claim(&claim, &permit(), "sim-nse")?;

    assert_eq!(
        *request.intent(),
        intent(1, 10)?,
        "the exact typed order intent is bound"
    );
    assert_eq!(
        request.intent().venue,
        Venue::Nse,
        "venue is bound, not inferred"
    );
    assert_eq!(
        request.intent().product,
        Product::Intraday,
        "product is bound"
    );
    assert_eq!(
        request.submission_attempt_id(),
        claim.attempt_id,
        "the attempt identity is bound"
    );
    assert!(
        request.matches_claim(&claim),
        "the request still matches its OMS claim"
    );

    assert!(
        !request.matches(
            &claim,
            &PermitRef {
                permit_id: 0,
                ..permit()
            },
            "sim-nse"
        ),
        "a different permit no longer matches the request"
    );
    Ok(())
}

#[test]
fn unauthenticated_or_incomplete_requests_are_refused() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.journal"))?;
    let token = fence(&mut store);
    let claim = dispatch(&mut store, &token, ClientOrderId::new(1)?, 10)?;
    let request = BrokerOrderRequest::from_claim(&claim, &permit(), "sim-nse")?;

    let mut broker = SimulatedBroker::new(SimulatorConfig::new(
        "sim-nse",
        SubmitBehavior::ImmediateAck,
    ))?;
    assert!(
        matches!(
            broker.submit(&request),
            Err(broker_adapter::AdapterError::NotAuthenticated)
        ),
        "an unauthenticated adapter cannot submit"
    );
    assert!(
        BrokerOrderRequest::from_claim(&claim, &permit(), "  ").is_err(),
        "an empty adapter target is refused"
    );
    broker.authenticate();
    assert!(
        broker.submit(&request).is_ok(),
        "an authenticated adapter can submit"
    );
    Ok(())
}

#[test]
fn immediate_ack_moves_the_oms_order_to_working() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.journal"))?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    let claim = dispatch(&mut store, &token, client, 10)?;
    let request = BrokerOrderRequest::from_claim(&claim, &permit(), "sim-nse")?;
    let mut broker = configured(SubmitBehavior::ImmediateAck)?;

    let outcome = broker.submit(&request)?;
    let SubmissionOutcome::Acknowledged(ack) = &outcome else {
        return Err("expected acknowledgement".into());
    };
    assert!(
        store.acknowledge_fenced(
            &token,
            claim.attempt_id,
            &authoritative_oms::BrokerOrderId::new(&ack.broker_order_id)?,
            &ack.broker_event_id,
            2_300
        )?,
        "the acknowledgement is applied to the OMS"
    );
    let order = store.order(client)?.ok_or("no order")?;
    assert_eq!(
        order.state,
        OmsOrderState::Working,
        "acknowledgement moves the order to working"
    );
    assert_eq!(
        broker.adapter_id(),
        "sim-nse",
        "the adapter reports its configured identity"
    );
    Ok(())
}

/// A timeout is ambiguous. The OMS goes UNKNOWN and the broker may still hold
/// the order; reconciliation resolves it from the broker view.
#[test]
fn timeout_is_unknown_and_reconciliation_resolves_it() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.journal"))?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    let claim = dispatch(&mut store, &token, client, 10)?;
    let request = BrokerOrderRequest::from_claim(&claim, &permit(), "sim-nse")?;
    let mut broker = configured(SubmitBehavior::Timeout)?;

    let outcome = broker.submit(&request)?;
    assert!(
        matches!(outcome, SubmissionOutcome::TimedOut { .. }),
        "a timeout is reported, not an ack"
    );
    store.expire_dispatched_fenced(&token, 5_000, 1_000)?;
    assert_eq!(
        store.order(client)?.ok_or("no order")?.state,
        OmsOrderState::Unknown,
        "a timeout leaves the order UNKNOWN"
    );
    assert!(
        store.claim_fenced(&token, "other", 6_000, 1_000)?.is_none(),
        "no retry command exists"
    );

    let broker_view = broker.query_order("SIM-1")?;
    assert!(
        broker_view.is_some(),
        "the broker may hold the order after a timeout"
    );
    store.begin_reconciliation_fenced(&token, client, 5_100)?;
    store.resolve_unknown_fenced(
        &token,
        client,
        Resolution::Working {
            broker_order_id: authoritative_oms::BrokerOrderId::new("SIM-1")?,
        },
        "reconcile-timeout-1",
        5_200,
    )?;
    assert_eq!(
        store.order(client)?.ok_or("no order")?.state,
        OmsOrderState::Working,
        "the reconciled order is working"
    );
    Ok(())
}

#[test]
fn rejection_moves_the_oms_order_to_rejected() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.journal"))?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    let claim = dispatch(&mut store, &token, client, 10)?;
    let request = BrokerOrderRequest::from_claim(&claim, &permit(), "sim-nse")?;
    let mut broker = configured(SubmitBehavior::Reject {
        reason: "price collar".to_owned(),
    })?;

    let outcome = broker.submit(&request)?;
    let SubmissionOutcome::Rejected {
        reason,
        broker_event_id,
        ..
    } = outcome
    else {
        return Err("expected rejection".into());
    };
    assert!(
        store.reject_submission_fenced(
            &token,
            claim.attempt_id,
            &reason,
            &broker_event_id,
            2_300
        )?,
        "the rejection is recorded in the OMS"
    );
    assert_eq!(
        store.order(client)?.ok_or("no order")?.state,
        OmsOrderState::Rejected,
        "the order is rejected after the broker rejection"
    );
    Ok(())
}

/// Duplicate and out-of-order broker events must not double-fill the OMS.
#[test]
fn duplicate_and_out_of_order_events_are_safe() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.journal"))?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    let claim = dispatch(&mut store, &token, client, 10)?;
    let request = BrokerOrderRequest::from_claim(&claim, &permit(), "sim-nse")?;
    let mut config = SimulatorConfig::new(
        "sim-nse",
        SubmitBehavior::AckThenFills {
            fills: vec![(4, 1_000), (6, 1_000)],
        },
    );
    config.duplicate_events = true;
    config.reverse_events = true;
    let mut broker = SimulatedBroker::new(config)?;
    broker.authenticate();
    broker.submit(&request)?;

    let mut now = 3_000;
    for event in broker.poll_events()? {
        now += 1;
        match event {
            BrokerEvent::Acknowledged {
                broker_order_id,
                broker_event_id,
                ..
            } => {
                store.acknowledge_fenced(
                    &token,
                    claim.attempt_id,
                    &authoritative_oms::BrokerOrderId::new(broker_order_id)?,
                    &broker_event_id,
                    now,
                )?;
            }
            BrokerEvent::Filled {
                execution_id,
                broker_event_id,
                quantity,
                price_paise,
                ..
            } => {
                store.apply_fill_fenced(
                    &token,
                    client,
                    &execution_id,
                    &broker_event_id,
                    quantity,
                    price_paise,
                    now,
                )?;
            }
            other => return Err(format!("unexpected event: {other:?}").into()),
        }
    }
    let order = store.order(client)?.ok_or("no order")?;
    assert_eq!(
        order.state,
        OmsOrderState::Filled,
        "the order is filled exactly once"
    );
    assert_eq!(
        order.filled_quantity, 10,
        "duplicate and reversed events did not double-fill"
    );
    Ok(())
}

#[test]
fn cancel_request_uses_the_bound_broker_identity() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.journal"))?;
    let token = fence(&mut store);
    let client = ClientOrderId::new(1)?;
    let claim = dispatch(&mut store, &token, client, 10)?;
    let request = BrokerOrderRequest::from_claim(&claim, &permit(), "sim-nse")?;
    let mut broker = configured(SubmitBehavior::ImmediateAck)?;
    let SubmissionOutcome::Acknowledged(ack) = broker.submit(&request)? else {
        return Err("expected ack".into());
    };
    store.acknowledge_fenced(
        &token,
        claim.attempt_id,
        &authoritative_oms::BrokerOrderId::new(&ack.broker_order_id)?,
        &ack.broker_event_id,
        2_300,
    )?;

    let cancel_attempt = store.request_cancel_fenced(&token, client, 3_000)?;
    assert!(cancel_attempt > 0, "a cancel attempt identity was created");
    let cancel = CancelRequest {
        client_order_id: client,
        broker_order_id: ack.broker_order_id.clone(),
        broker_event_id: "cancel-evt-1".to_owned(),
    };
    assert!(
        broker.cancel(&cancel).is_ok(),
        "the broker order is identified exactly"
    );
    assert!(
        store.apply_cancel_fenced(&token, client, "cancel-evt-1", 3_100)?,
        "the cancellation is applied to the OMS"
    );
    assert_eq!(
        store.order(client)?.ok_or("no order")?.state,
        OmsOrderState::Cancelled,
        "the order is cancelled"
    );

    let wrong = CancelRequest {
        client_order_id: client,
        broker_order_id: "OTHER".to_owned(),
        broker_event_id: "cancel-evt-2".to_owned(),
    };
    assert!(
        broker.cancel(&wrong).is_err(),
        "an unbound broker id cannot cancel"
    );
    Ok(())
}

#[test]
fn no_strategy_risk_sdk_or_live_coupling() -> TestResult {
    let manifest = include_str!("../Cargo.toml").to_ascii_lowercase();
    for forbidden in [
        "strategy-runtime",
        "risk-authority",
        "upstox",
        "fyers",
        "kite",
        "reqwest",
        "tokio",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "manifest must not depend on {forbidden}"
        );
    }
    Ok(())
}

#[test]
fn simulator_configuration_is_validated() -> TestResult {
    assert!(
        SimulatedBroker::new(SimulatorConfig::new(" ", SubmitBehavior::ImmediateAck)).is_err(),
        "an empty adapter id is refused"
    );
    assert!(
        SimulatedBroker::new(SimulatorConfig::new(
            "sim",
            SubmitBehavior::AckThenFills { fills: Vec::new() }
        ))
        .is_err(),
        "an empty fill scenario is refused"
    );
    Ok(())
}

/// A request bound to a different adapter target is refused at submit, even
/// though request fields are private and set only by `from_claim`.
#[test]
fn wrong_target_broker_is_refused() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.journal"))?;
    let token = fence(&mut store);
    let claim = dispatch(&mut store, &token, ClientOrderId::new(1)?, 10)?;
    let request = BrokerOrderRequest::from_claim(&claim, &permit(), "other-broker")?;
    let mut broker = configured(SubmitBehavior::ImmediateAck)?;
    assert!(
        matches!(
            broker.submit(&request),
            Err(broker_adapter::AdapterError::IncompleteRequest)
        ),
        "a request addressed to another adapter is refused"
    );
    assert!(
        request.matches(&claim, &permit(), "other-broker"),
        "the request matches its own claim and target"
    );
    assert!(
        !request.matches(&claim, &permit(), "sim-nse"),
        "a different target does not match"
    );
    Ok(())
}

/// Cancellation must bind the client order as well as the broker order.
#[test]
fn cancel_requires_the_bound_client_identity() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = OmsStore::open(directory.path().join("oms.journal"))?;
    let token = fence(&mut store);
    let claim = dispatch(&mut store, &token, ClientOrderId::new(1)?, 10)?;
    let request = BrokerOrderRequest::from_claim(&claim, &permit(), "sim-nse")?;
    let mut broker = configured(SubmitBehavior::ImmediateAck)?;
    let SubmissionOutcome::Acknowledged(ack) = broker.submit(&request)? else {
        return Err("expected ack".into());
    };
    let wrong = CancelRequest {
        client_order_id: ClientOrderId::new(2)?,
        broker_order_id: ack.broker_order_id.clone(),
        broker_event_id: "cancel-wrong-client".to_owned(),
    };
    assert!(
        matches!(
            broker.cancel(&wrong),
            Err(broker_adapter::AdapterError::IncompleteRequest)
        ),
        "a cancel for a different client order is refused"
    );
    Ok(())
}

/// The simulator delivers reversed/duplicated events; the adapter's
/// poll normalizes them by declared sequence before the OMS sees them.
#[test]
fn broker_delivery_is_out_of_order_but_poll_normalizes_it() -> TestResult {
    let mut config = SimulatorConfig::new(
        "sim-nse",
        SubmitBehavior::AckThenFills {
            fills: vec![(4, 1_000), (6, 1_000)],
        },
    );
    config.duplicate_events = true;
    config.reverse_events = true;
    let mut broker = SimulatedBroker::new(config)?;
    broker.authenticate();
    let request = broker_adapter::BrokerOrderRequest::from_claim(
        &DispatchClaim {
            attempt_id: 5,
            client_order_id: ClientOrderId::new(1)?,
            command_type: "SUBMIT".to_owned(),
            intent: intent(1, 10)?,
        },
        &permit(),
        "sim-nse",
    )?;
    broker.submit(&request)?;

    let raw = broker.drain_raw_events();
    let raw_sequences: Vec<u64> = raw.iter().map(BrokerEvent::sequence).collect();
    assert!(
        raw_sequences.windows(2).any(|pair| pair[0] > pair[1]),
        "the simulator delivers at least one event out of sequence"
    );
    Ok(())
}
