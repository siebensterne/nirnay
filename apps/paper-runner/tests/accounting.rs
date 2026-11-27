// SPDX-License-Identifier: Apache-2.0
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Neutral fill-accounting and strategy-state regression tests.
//!
//! These fixtures construct no market observations, quote sequences, provider
//! collection, experiments, holdouts, broker adapters or live execution. They
//! exercise the authoritative OMS and the account ledger directly.

use std::collections::BTreeMap;

use authoritative_oms::{
    BrokerOrderId, //
    ChargeStatus,
    ClientOrderId,
    FillEvent,
    OmsStore,
    OrderView,
    PermitRef,
    Projector,
    ReconciliationDifference,
    ReconciliationInput,
    reconcile,
};
use paper_runner::{
    PaperExecutionEvidence, //
    StrategySnapshot,
    apply_authoritative_fill,
    strategy_event_from_snapshot,
    strategy_snapshot,
};
use strategy_runtime::{
    Rational, //
    RuntimeIntent,
    Strategy,
    StrategyError,
    StrategyEvent,
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
    ReconciliationStatus,
    Side,
    StrategyId,
    TradeDate,
    Venue,
    permit_digest,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const ACCOUNT: u64 = 11;
const STRATEGY: u64 = 12;
const ALGO: u64 = 13;
const SCOPE: &str = "paper";
const INSTRUMENT: &str = "paper.accounting";

fn account() -> AccountId {
    AccountId::new(ACCOUNT).expect("account")
}

fn date() -> TradeDate {
    TradeDate::new(2026, 1, 1).expect("date")
}

fn instrument_id() -> InstrumentId {
    InstrumentId::new(INSTRUMENT).expect("instrument")
}

fn neutral_intent(id: u64, side: Side, quantity: i64) -> OrderIntent {
    neutral_intent_at(id, side, quantity, 1_000)
}

fn neutral_intent_at(id: u64, side: Side, quantity: i64, price: i64) -> OrderIntent {
    OrderIntent {
        intent_id: IntentId::new(id).expect("intent id"),
        account_id: account(),
        strategy_id: StrategyId::new(STRATEGY).expect("strategy"),
        strategy_version: "paper-neutral-v1".to_owned(),
        algo_id: AlgoId::new(ALGO).expect("algo"),
        instrument_id: instrument_id(),
        venue: Venue::Nse,
        side,
        product: Product::Intraday,
        quantity: Quantity::new(quantity).expect("quantity"),
        limit_price: PricePaise::new(Paise(price)).expect("price"),
        reason: "neutral fill accounting fixture".to_owned(),
        evidence: vec![JournalRef {
            sequence: id,
            hash_hex: "ab".repeat(32),
        }],
        expires_at: LogicalNanos::new(1_000),
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    store: OmsStore,
    projector: Projector,
    instruments: BTreeMap<String, InstrumentId>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("fixture directory");
        let oms_path = dir.path().join("oms.journal");
        let store = OmsStore::open(&oms_path).expect("oms store");
        let mut projector = Projector::open(dir.path().join("ledger"), account()).expect("ledger");
        projector
            .open_account(
                SCOPE,
                Paise(1_000_000),
                date(),
                OpeningBalanceEvidence {
                    statement_ref: "paper-neutral-statement".to_owned(),
                    journal_ref: JournalRef {
                        sequence: 1,
                        hash_hex: store.state_digest().to_owned(),
                    },
                    observed_at_ns: 1,
                },
            )
            .expect("open account");
        Self {
            _dir: dir,
            store,
            projector,
            instruments: BTreeMap::new(),
        }
    }

    fn working_order(&mut self, id: u64, side: Side, quantity: i64) -> ClientOrderId {
        self.working_order_at(id, side, quantity, 1_000)
    }

    fn working_order_at(
        &mut self,
        id: u64,
        side: Side,
        quantity: i64,
        price: i64,
    ) -> ClientOrderId {
        let intent = neutral_intent_at(id, side, quantity, price);
        let permit = PermitRef {
            permit_id: id,
            policy_version: "paper-neutral".to_owned(),
            snapshot_revision: 1,
            digest: permit_digest(id, "paper-neutral", 1, &intent),
        };
        let client = ClientOrderId::new(id).expect("client order id");
        self.store
            .create(client, &intent, &permit, 1)
            .expect("create order");
        self.store
            .accept_risk_approved(client, 2)
            .expect("accept order");
        let claim = self
            .store
            .claim("paper-test", 3, 1_000)
            .expect("claim")
            .expect("claim is available");
        assert_eq!(
            claim.client_order_id, client,
            "claim belongs to the requested order"
        );
        self.store
            .mark_dispatching(claim.attempt_id, 4)
            .expect("dispatch order");
        let broker = BrokerOrderId::new(format!("broker-{id}")).expect("broker id");
        self.store
            .acknowledge(claim.attempt_id, &broker, &format!("ack-{id}"), 5)
            .expect("acknowledge order");
        client
    }

    fn dispatched_order(&mut self, id: u64, side: Side, quantity: i64) -> ClientOrderId {
        let intent = neutral_intent(id, side, quantity);
        let permit = PermitRef {
            permit_id: id,
            policy_version: "paper-neutral".to_owned(),
            snapshot_revision: 1,
            digest: permit_digest(id, "paper-neutral", 1, &intent),
        };
        let client = ClientOrderId::new(id).expect("client order id");
        self.store
            .create(client, &intent, &permit, 1)
            .expect("create order");
        self.store
            .accept_risk_approved(client, 2)
            .expect("accept order");
        let claim = self
            .store
            .claim("paper-test", 3, 1_000)
            .expect("claim")
            .expect("claim is available");
        self.store
            .mark_dispatching(claim.attempt_id, 4)
            .expect("dispatch without acknowledgement");
        client
    }

    fn deliver(
        &mut self,
        client: ClientOrderId,
        execution: &str,
        side: Side,
        quantity: i64,
    ) -> Result<bool, String> {
        self.deliver_at(client, execution, side, quantity, 1_000)
    }

    fn deliver_at(
        &mut self,
        client: ClientOrderId,
        execution: &str,
        side: Side,
        quantity: i64,
        price: i64,
    ) -> Result<bool, String> {
        let evidence = PaperExecutionEvidence {
            execution_id: execution.to_owned(),
            broker_event_id: format!("event-{execution}"),
            side,
            quantity,
            price_paise: price,
            now_ns: 10,
        };
        apply_authoritative_fill(
            &mut self.store,
            &mut self.projector,
            &mut self.instruments,
            client,
            &instrument_id(),
            evidence,
        )
    }

    fn position(&self) -> i64 {
        self.projector
            .state()
            .positions
            .get(SCOPE)
            .and_then(|by_instrument| by_instrument.get(&instrument_id()))
            .copied()
            .unwrap_or(0)
    }

    fn snapshot(&self, status: ReconciliationStatus) -> Result<StrategySnapshot, String> {
        strategy_snapshot(&self.store, &self.projector, &instrument_id(), status)
    }
}

#[test]
fn buy_ten_increases_ledger_position_by_ten() -> TestResult {
    let mut fixture = Fixture::new();
    let client = fixture.working_order(1, Side::Buy, 10);
    assert!(
        fixture.deliver(client, "buy-10", Side::Buy, 10)?,
        "buy is applied"
    );
    assert_eq!(
        fixture.position(),
        10,
        "ledger position increases by bought lots"
    );
    let view: OrderView = fixture.store.order(client)?.ok_or("missing order")?;
    assert_eq!(
        view.filled_quantity, 10,
        "OMS records the authoritative fill"
    );
    Ok(())
}

#[test]
fn sell_four_then_six_flattens_the_position() -> TestResult {
    let mut fixture = Fixture::new();
    let buy = fixture.working_order(1, Side::Buy, 10);
    fixture.deliver(buy, "buy-10", Side::Buy, 10)?;
    let sell_four = fixture.working_order(2, Side::Sell, 4);
    fixture.deliver(sell_four, "sell-4", Side::Sell, 4)?;
    assert_eq!(
        fixture.position(),
        6,
        "a sell-to-close decreases the ledger position"
    );
    let stored = fixture.store.fill("sell-4").ok_or("missing sell fill")?;
    assert_eq!(
        stored.side,
        Side::Sell,
        "the execution is stored with the sell side"
    );
    let sell_six = fixture.working_order(3, Side::Sell, 6);
    fixture.deliver(sell_six, "sell-6", Side::Sell, 6)?;
    assert_eq!(
        fixture.position(),
        0,
        "the remaining six lots close the position"
    );
    Ok(())
}

#[test]
fn duplicate_execution_id_has_no_second_effect() -> TestResult {
    let mut fixture = Fixture::new();
    let client = fixture.working_order(1, Side::Buy, 10);
    assert!(
        fixture.deliver(client, "buy-10", Side::Buy, 10)?,
        "first delivery applies"
    );
    let digest = fixture.projector.state().state_digest();
    assert!(
        !fixture.deliver(client, "buy-10", Side::Buy, 10)?,
        "duplicate is a no-op"
    );
    assert_eq!(
        fixture.projector.state().state_digest(),
        digest,
        "duplicate delivery does not change ledger state"
    );
    assert_eq!(
        fixture.position(),
        10,
        "duplicate does not double the position"
    );
    Ok(())
}

#[test]
fn contradictory_fill_side_is_rejected_before_any_mutation() -> TestResult {
    let mut fixture = Fixture::new();
    let client = fixture.working_order(1, Side::Buy, 10);
    let error = fixture
        .deliver(client, "wrong-side", Side::Sell, 4)
        .expect_err("contradictory side must fail closed");
    assert!(
        error.contains("side"),
        "error names the side conflict: {error}"
    );
    assert_eq!(
        fixture.position(),
        0,
        "position is unchanged after rejection"
    );
    let view = fixture.store.order(client)?.ok_or("missing order")?;
    assert_eq!(
        view.state,
        authoritative_oms::OmsOrderState::Working,
        "order stays working"
    );
    Ok(())
}

#[test]
fn over_fill_is_rejected_and_the_order_stays_unresolved() -> TestResult {
    let mut fixture = Fixture::new();
    let client = fixture.working_order(1, Side::Buy, 10);
    let error = fixture
        .deliver(client, "over-fill", Side::Buy, 11)
        .expect_err("over-fill must fail closed");
    assert!(
        error.contains("remaining"),
        "error names the remaining quantity: {error}"
    );
    assert_eq!(
        fixture.position(),
        0,
        "no position is credited by a rejected fill"
    );
    let view = fixture.store.order(client)?.ok_or("missing order")?;
    assert!(
        !view.state.terminal(),
        "the order is not terminalised by a rejected fill"
    );
    Ok(())
}

#[test]
fn late_fill_after_cancel_is_rejected_and_flagged_by_reconciliation() -> TestResult {
    let mut fixture = Fixture::new();
    let client = fixture.working_order(1, Side::Buy, 10);
    fixture
        .store
        .request_cancel(client, 6)
        .expect("request cancel");
    fixture
        .store
        .apply_cancel(client, "event-cancel", 7)
        .expect("apply broker cancel evidence");
    let error = fixture
        .deliver(client, "late-fill", Side::Buy, 10)
        .expect_err("a fill after cancel must not move position");
    assert!(
        error.contains("terminal"),
        "error names the terminal order: {error}"
    );
    assert_eq!(
        fixture.position(),
        0,
        "a cancelled order never changes position"
    );

    let late = FillEvent {
        schema_version: 1,
        execution_id: "late-fill".to_owned(),
        broker_event_id: "event-late-fill".to_owned(),
        client_order_id: client,
        oms_order_id: client.to_string(),
        side: Side::Buy,
        quantity: 10,
        price_paise: 1_000,
        currency: "INR".to_owned(),
        broker_timestamp_ns: 20,
        received_at_ns: 20,
        source: "neutral-test".to_owned(),
        account_scope: SCOPE.to_owned(),
        charges_paise: 0,
        charge_status: ChargeStatus::Estimated,
        provenance: "late evidence delivery".to_owned(),
    };
    fixture
        .instruments
        .insert("late-fill".to_owned(), instrument_id());
    let outcome = reconcile(
        &fixture.store,
        &fixture.projector,
        &ReconciliationInput {
            deliveries: vec![late],
            instruments: fixture.instruments.clone(),
        },
    )?;
    assert!(
        outcome
            .differences
            .contains(&ReconciliationDifference::LateFillAfterCancel {
                execution_id: "late-fill".to_owned(),
                order_id: 1,
            }),
        "authoritative reconciliation flags the late execution"
    );
    Ok(())
}

#[test]
fn trade_bust_reverses_the_posting() -> TestResult {
    let mut fixture = Fixture::new();
    let client = fixture.working_order(1, Side::Buy, 10);
    fixture.deliver(client, "buy-10", Side::Buy, 10)?;
    assert_eq!(fixture.position(), 10, "fill establishes a position");
    assert!(
        fixture.projector.bust("buy-10")?,
        "bust reverses the posting"
    );
    assert_eq!(fixture.position(), 0, "the corrected ledger is flat");
    assert!(
        !fixture.projector.bust("buy-10")?,
        "a second bust is a no-op"
    );
    Ok(())
}

#[test]
fn overflow_fails_closed_before_oms_mutation() -> TestResult {
    let mut fixture = Fixture::new();
    let large = fixture.working_order_at(1, Side::Buy, i64::MAX, 1);
    fixture.deliver_at(large, "buy-max", Side::Buy, i64::MAX, 1)?;
    assert_eq!(fixture.position(), i64::MAX, "the large fill is posted");
    let small = fixture.working_order(2, Side::Buy, 1);
    let error = fixture
        .deliver(small, "overflow", Side::Buy, 1)
        .expect_err("position overflow must fail closed");
    assert!(
        error.contains("overflow"),
        "error names the overflow: {error}"
    );
    assert_eq!(fixture.position(), i64::MAX, "the ledger is unchanged");
    assert!(
        fixture.store.fill("overflow").is_none(),
        "the OMS was not mutated before the overflow check"
    );
    Ok(())
}

#[test]
fn snapshot_reports_authoritative_order_and_position_state() -> TestResult {
    let mut fixture = Fixture::new();
    let client = fixture.working_order(1, Side::Buy, 10);
    fixture.deliver(client, "buy-4", Side::Buy, 4)?;
    let snapshot = fixture.snapshot(ReconciliationStatus::Matched)?;
    assert_eq!(
        snapshot.position_quantity, 4,
        "position comes from the ledger"
    );
    assert!(
        snapshot.active_orders.contains(&client),
        "a working order is visible to the strategy"
    );
    assert!(
        snapshot.has_open_order,
        "strategy sees the real open-order state"
    );
    assert_eq!(
        snapshot.reconciliation_status,
        ReconciliationStatus::Matched,
        "reconciliation status is carried into the snapshot"
    );
    Ok(())
}

#[test]
fn snapshot_classifies_partial_unknown_and_pending_cancel() -> TestResult {
    let mut fixture = Fixture::new();
    let partial = fixture.working_order(1, Side::Buy, 10);
    fixture.deliver(partial, "partial-4", Side::Buy, 4)?;
    let unknown = fixture.dispatched_order(2, Side::Buy, 3);
    fixture
        .store
        .expire_dispatched(7, 1)
        .expect("expire dispatch");
    let pending = fixture.working_order(3, Side::Buy, 5);
    fixture
        .store
        .request_cancel(pending, 8)
        .expect("pending cancel");
    let snapshot = fixture.snapshot(ReconciliationStatus::Unavailable)?;
    assert!(
        snapshot.partial_fills.contains(&partial),
        "partial fills are indexed"
    );
    assert!(
        snapshot.pending_cancel_replace.contains(&pending),
        "pending cancels are indexed"
    );
    assert!(
        snapshot.unknown_orders.contains(&unknown),
        "unknown orders are indexed"
    );
    assert_eq!(
        snapshot.active_orders.len(),
        3,
        "all non-terminal orders are active"
    );
    assert_eq!(
        snapshot.reconciliation_status,
        ReconciliationStatus::Unavailable,
        "unavailable reconciliation is visible to the strategy"
    );
    Ok(())
}

struct RecordingStrategy {
    seen: Vec<StrategyEvent>,
}

impl Strategy for RecordingStrategy {
    type Intent = RuntimeIntent;

    fn on_event(&mut self, event: &StrategyEvent) -> Result<Option<Self::Intent>, StrategyError> {
        self.seen.push(event.clone());
        Ok(None)
    }
}

#[test]
fn strategy_decision_sees_the_authoritative_open_order() -> TestResult {
    let mut fixture = Fixture::new();
    let client = fixture.working_order(1, Side::Buy, 20);
    fixture.deliver(client, "buy-10", Side::Buy, 10)?;
    let snapshot = fixture.snapshot(ReconciliationStatus::Matched)?;
    let midpoint = Rational::integer(1);
    let event = strategy_event_from_snapshot(1, INSTRUMENT.to_owned(), midpoint, 1, 2, &snapshot);
    let mut strategy = RecordingStrategy { seen: Vec::new() };
    strategy
        .on_event(&event)
        .expect("strategy accepts the event");
    let received = strategy.seen.first().ok_or("no event received")?;
    assert!(
        received.has_open_order,
        "the strategy is told a real order is open"
    );
    assert_eq!(
        received.position_quantity, 10,
        "the strategy is told the ledger position"
    );
    assert_eq!(
        received.observation_sequence, 1,
        "the event carries the neutral observation ordinal"
    );
    assert!(
        fixture.store.order(client)?.is_some(),
        "the authoritative order still exists"
    );
    Ok(())
}

#[test]
fn snapshot_view_is_not_an_id_scan() -> TestResult {
    let source = include_str!("../src/lib.rs");
    assert!(
        source.contains("store.orders()"),
        "the snapshot uses the indexed OMS view"
    );
    assert!(
        !source.contains("for id in 1..=ordinal"),
        "the historical id scan is removed"
    );
    Ok(())
}
