//! Account projection materialized by replaying the account journal.
//!
//! Derived from the internal Nirnay `crates/nirnay-engine/src/state.rs` at source snapshot
//! `cc3a34c317a86a9984401efe90fa3f43d67a1787`.
//!
//! The projection is the accounting authority for the paper engine: cash,
//! charges, signed positions and order state are derived only from journaled
//! events. The earlier money and quantity definitions are replaced by the
//! canonical exact types.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use account_journal::StoredRecord;
use cost_model::ChargeBook;
use thiserror::Error;
use trading_core::{
    AccountId, ChargeComponents, CheckedPaise, DecisionRecord, DomainError, DoubledPaise,
    EventEnvelope, EventPayload, FeedHealth, Fill, FillId, InstrumentCatalog, InstrumentId,
    IntentId, LogicalNanos, Order, OrderId, OrderIntent, OrderStatus, Paise, PricePaise, Product,
    Quantity, ReconciliationStatus, RiskAssessment, SessionPhase, Side, TradeDate, Venue,
};

use crate::market::{MarketError, MarketState};

/// Run format of a journal, declared once by the genesis `account_opened`
/// record's schema version and enforced for every fill that follows.
/// Legacy (1) journals were written before fill provenance existed;
/// provenance (2) journals require every fill to name its funding quote.
pub(super) const FORMAT_PROVENANCE_LEGACY: u16 = 1;
pub(super) const FORMAT_PROVENANCE_REQUIRED: u16 = 2;

#[derive(Debug, Error)]
pub(super) enum ProjectionError {
    #[error("event stream does not begin with account_opened")]
    MissingAccountOpening,
    #[error("event at sequence {sequence} is inconsistent: {reason}")]
    Inconsistent { sequence: u64, reason: String },
    #[error("domain arithmetic failed: {0}")]
    Domain(#[from] DomainError),
    #[error("market state failed: {0}")]
    Market(#[from] MarketError),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OrderView {
    pub order: Order,
    pub created_sequence: u64,
    pub last_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PositionView {
    pub instrument_id: InstrumentId,
    /// Signed lots: positive is long, negative is short.
    pub quantity: i64,
    /// Exact mark in doubled paise (M₂ = bid + ask): no rounding has happened
    /// here; gross values round once, after multiplication by quantity.
    pub mark_price_doubled: Option<DoubledPaise>,
    pub gross_paise: Option<Paise>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AccountView {
    pub account_id: AccountId,
    pub cash: Paise,
    pub charges_paid: Paise,
    pub daily_pnl: Paise,
    pub account_snapshot_available: bool,
    pub reconciliation_blocked: bool,
    pub kill_switch_engaged: bool,
    pub session_phase: Option<SessionPhase>,
    pub calendar_version: Option<String>,
    pub positions: Vec<PositionView>,
    pub open_orders: Vec<OrderView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct IntentProjection {
    pub decision: DecisionRecord,
    pub risk: Option<RiskAssessment>,
    pub order_id: Option<OrderId>,
}

/// Gross value of a position against an exact doubled mark, rounded once
/// to integer paise at the accounting boundary: `roundHalfEven(|q| × M₂ / 2)`
/// with checked arithmetic. The half is never dropped before the
/// multiplication, so odd marks and odd quantities stay exact.
fn rounded_gross_paise(quantity: i64, mark: DoubledPaise) -> Option<Paise> {
    let scaled = i128::from(quantity.unsigned_abs()).checked_mul(mark.value())?;
    let quotient = scaled.div_euclid(2);
    let rounded = if scaled.rem_euclid(2) == 1 && quotient % 2 != 0 {
        quotient + 1
    } else {
        quotient
    };
    i64::try_from(rounded).ok().map(Paise)
}

#[derive(Clone, Debug)]
pub(super) struct Projection {
    pub market: MarketState,
    pub account_id: Option<AccountId>,
    pub trade_date: Option<TradeDate>,
    pub starting_cash: Option<Paise>,
    pub session_phase: Option<SessionPhase>,
    pub calendar_version: Option<String>,
    pub cash: Paise,
    pub charges_paid: Paise,
    pub daily_pnl: Paise,
    pub account_snapshot_available: bool,
    pub reconciliation_blocked: bool,
    pub kill_switch_engaged: bool,
    pub positions: BTreeMap<InstrumentId, i64>,
    pub orders: BTreeMap<OrderId, OrderView>,
    pub intents: BTreeMap<IntentId, IntentProjection>,
    pub fills: BTreeMap<FillId, Fill>,
    pub order_times: VecDeque<LogicalNanos>,
    pub last_sequence: u64,
    pub last_available_time: Option<LogicalNanos>,
    pub next_fill_id: u64,
    /// Run format declared by the genesis record; unknown (legacy value)
    /// until `account_opened` is applied.
    pub format_provenance: u16,
    /// Fill-price rule of the configuration that opened this run: the
    /// matcher shifts the displayed price by these many ticks, so replay
    /// must reproduce the shift exactly. Reopening with different values
    /// fails the next fill, never silently re-prices history.
    pub slippage_ticks: u32,
    /// Instrument catalog of the opening configuration, used to resolve
    /// the tick that priced each fill.
    pub instruments: Option<InstrumentCatalog>,
    /// Quote-age threshold used by the live matcher to determine whether a
    /// book is executable. Replay of a provenance journal must apply the
    /// same feed-health gate to the fill's availability time.
    pub max_quote_age_ns: Option<u64>,
    /// Fee schedule of the opening configuration. Replay recalculates every
    /// charge component for each fill from this schedule; recorded fees are
    /// not trusted from cash-balanced totals alone.
    pub charge_book: Option<ChargeBook>,
}

impl Default for Projection {
    fn default() -> Self {
        Self {
            market: MarketState::default(),
            account_id: None,
            trade_date: None,
            starting_cash: None,
            session_phase: None,
            calendar_version: None,
            cash: Paise(0),
            charges_paid: Paise(0),
            daily_pnl: Paise(0),
            account_snapshot_available: false,
            reconciliation_blocked: true,
            kill_switch_engaged: true,
            positions: BTreeMap::new(),
            orders: BTreeMap::new(),
            intents: BTreeMap::new(),
            fills: BTreeMap::new(),
            order_times: VecDeque::new(),
            last_sequence: 0,
            last_available_time: None,
            next_fill_id: 1,
            format_provenance: FORMAT_PROVENANCE_LEGACY,
            slippage_ticks: 0,
            instruments: None,
            max_quote_age_ns: None,
            charge_book: None,
        }
    }
}

impl Projection {
    pub fn apply(&mut self, record: &StoredRecord) -> Result<(), ProjectionError> {
        let sequence = record.sequence;
        let envelope = &record.body.envelope;
        if let Some(previous) = self.last_available_time
            && envelope.times.available_time < previous
        {
            return Err(inconsistent(
                sequence,
                "logical availability time moved backwards",
            ));
        }
        if self.last_sequence == 0
            && !matches!(&record.body.payload, EventPayload::AccountOpened { .. })
        {
            return Err(ProjectionError::MissingAccountOpening);
        }
        match &record.body.payload {
            EventPayload::AccountOpened {
                account_id,
                starting_cash,
                trade_date,
            } => {
                if starting_cash.0 <= 0 || self.account_id.is_some() || sequence != 1 {
                    return Err(inconsistent(
                        sequence,
                        "account_opened must occur once at sequence one",
                    ));
                }
                // A v2 genesis is self-causative: its causation names the
                // run root, i.e. this record's own event identity.
                if envelope.schema_version == FORMAT_PROVENANCE_REQUIRED
                    && envelope.causation_id.as_ref() != Some(&envelope.event_id)
                {
                    return Err(inconsistent(
                        sequence,
                        "schema version 2 account_opened must be self-causative",
                    ));
                }
                // The genesis record declares the run format for the whole
                // journal: legacy (1) or provenance-required (2). The
                // declaration is journal-level — provenance acceptance is
                // never decided per fill.
                self.format_provenance = envelope.schema_version;
                self.account_id = Some(*account_id);
                self.trade_date = Some(*trade_date);
                self.starting_cash = Some(*starting_cash);
                self.cash = *starting_cash;
                self.kill_switch_engaged = true;
                self.reconciliation_blocked = true;
                self.account_snapshot_available = false;
            }
            EventPayload::SessionStateObserved {
                trade_date,
                phase,
                calendar_version,
            } => {
                if Some(*trade_date) != self.trade_date || calendar_version.trim().is_empty() {
                    return Err(inconsistent(
                        sequence,
                        "session state does not match the stream trading date or calendar",
                    ));
                }
                self.session_phase = Some(*phase);
                self.calendar_version = Some(calendar_version.clone());
            }
            EventPayload::QuoteReceived { .. }
            | EventPayload::LateQuoteObserved { .. }
            | EventPayload::FeedGapDetected { .. } => self.market.apply_record(record)?,
            EventPayload::DecisionMade { decision } => {
                if let Some(intent) = &decision.intent {
                    if decision.strategy_id != intent.strategy_id
                        || decision.strategy_version != intent.strategy_version
                        || decision.input_refs != intent.evidence
                    {
                        return Err(inconsistent(
                            sequence,
                            "decision and intent identities or evidence differ",
                        ));
                    }
                    if self.intents.contains_key(&intent.intent_id) {
                        return Err(inconsistent(sequence, "intent identity was reused"));
                    }
                    self.intents.insert(
                        intent.intent_id,
                        IntentProjection {
                            decision: decision.clone(),
                            risk: None,
                            order_id: None,
                        },
                    );
                }
            }
            EventPayload::RiskAssessed { assessment } => {
                let intent = self.intents.get_mut(&assessment.intent_id).ok_or_else(|| {
                    inconsistent(sequence, "risk assessment has no prior decision")
                })?;
                if intent.risk.is_some() {
                    return Err(inconsistent(
                        sequence,
                        "intent has multiple risk assessments",
                    ));
                }
                intent.risk = Some(assessment.clone());
            }
            EventPayload::OrderCreated { order } => {
                if order.status != OrderStatus::Working
                    || order.filled_quantity != Quantity::ZERO
                    || order.average_fill_price.is_some()
                    || order.fill_count != 0
                    || order.filled_notional.0 != 0
                {
                    return Err(inconsistent(sequence, "new order has non-initial state"));
                }
                let intent = self
                    .intents
                    .get_mut(&order.intent.intent_id)
                    .ok_or_else(|| inconsistent(sequence, "order has no prior decision"))?;
                if !intent
                    .risk
                    .as_ref()
                    .is_some_and(|assessment| assessment.accepted)
                    || intent.order_id.is_some()
                    || intent.decision.intent.as_ref() != Some(&order.intent)
                {
                    return Err(inconsistent(
                        sequence,
                        "order lacks a unique accepted risk decision",
                    ));
                }
                intent.order_id = Some(order.order_id);
                if self
                    .orders
                    .insert(
                        order.order_id,
                        OrderView {
                            order: order.clone(),
                            created_sequence: sequence,
                            last_sequence: sequence,
                        },
                    )
                    .is_some()
                {
                    return Err(inconsistent(sequence, "order identity was reused"));
                }
                self.order_times.push_back(envelope.times.available_time);
            }
            EventPayload::OrderTransitioned {
                order_id, from, to, ..
            } => {
                let view = self.orders.get_mut(order_id).ok_or_else(|| {
                    inconsistent(sequence, "transition references an unknown order")
                })?;
                if view.order.status != *from || !from.permits(*to) {
                    return Err(inconsistent(
                        sequence,
                        "order state transition is not permitted",
                    ));
                }
                let total = view.order.intent.quantity;
                let filled = view.order.filled_quantity;
                let state_matches_fills = match to {
                    OrderStatus::Working | OrderStatus::Rejected => filled.is_zero(),
                    OrderStatus::PartiallyFilled => !filled.is_zero() && filled < total,
                    OrderStatus::Filled => filled == total,
                    OrderStatus::CancelPending | OrderStatus::OutcomeUnknown => filled <= total,
                    OrderStatus::Cancelled => filled < total,
                };
                if !state_matches_fills {
                    return Err(inconsistent(
                        sequence,
                        "order status contradicts cumulative fill quantity",
                    ));
                }
                view.order.status = *to;
                view.last_sequence = sequence;
                if *to == OrderStatus::OutcomeUnknown {
                    self.reconciliation_blocked = true;
                    self.account_snapshot_available = false;
                }
            }
            EventPayload::FillBooked { fill, next_status } => {
                if self.fills.contains_key(&fill.fill_id) || fill.fill_id.get() != self.next_fill_id
                {
                    return Err(inconsistent(
                        sequence,
                        "fill identity is repeated or out of sequence",
                    ));
                }
                let view = self
                    .orders
                    .get(&fill.order_id)
                    .ok_or_else(|| inconsistent(sequence, "fill references an unknown order"))?;
                if fill.side != view.order.intent.side || fill.quantity.is_zero() {
                    return Err(inconsistent(
                        sequence,
                        "fill side or quantity differs from its order",
                    ));
                }
                let instrument_id = view.order.intent.instrument_id.clone();
                let venue = view.order.intent.venue;
                // ---- validation phase: no mutations until every check passes ----
                if self.format_provenance == FORMAT_PROVENANCE_LEGACY {
                    // Legacy journals: pre-v2 fills have no provenance by
                    // contract; only budget and accounting still apply.
                    if record.body.envelope.schema_version == 2 {
                        return Err(inconsistent(
                            sequence,
                            "provenance event in a legacy journal",
                        ));
                    }
                } else if self.format_provenance != FORMAT_PROVENANCE_REQUIRED {
                    return Err(inconsistent(sequence, "unknown journal run format"));
                } else {
                    // Provenance-format journal: every fill must be a v2
                    // causally enriched record authored by the engine.
                    if record.body.envelope.schema_version == 1 {
                        return Err(inconsistent(
                            sequence,
                            "legacy fill in a provenance journal",
                        ));
                    }
                    validate_fill_provenance(
                        &self.market,
                        &record.body.envelope,
                        &view.order.intent,
                        fill,
                        sequence,
                        FillProvenanceRule {
                            slippage_ticks: self.slippage_ticks,
                            tick: self.fill_price_tick(&instrument_id, venue),
                            max_quote_age_ns: self.max_quote_age_ns,
                        },
                    )?;
                }
                // Budget sufficiency pre-check (no mutation here).
                let market_remaining = match fill.side {
                    Side::Buy => self.market.ask_remaining(&instrument_id, venue),
                    Side::Sell => self.market.bid_remaining(&instrument_id, venue),
                };
                if fill.quantity.get() > market_remaining {
                    return Err(inconsistent(
                        sequence,
                        "fill exceeds the remaining liquidity budget",
                    ));
                }
                let remaining = view
                    .order
                    .intent
                    .quantity
                    .checked_sub(view.order.filled_quantity)?;
                if fill.quantity > remaining {
                    return Err(inconsistent(
                        sequence,
                        "fill exceeds remaining order quantity",
                    ));
                }
                if !view.order.status.permits(*next_status) && view.order.status != *next_status {
                    return Err(inconsistent(
                        sequence,
                        "fill produces an invalid order state transition",
                    ));
                }
                let new_filled = view.order.filled_quantity.checked_add(fill.quantity)?;
                let expected_status = if new_filled == view.order.intent.quantity {
                    OrderStatus::Filled
                } else if view.order.status == OrderStatus::CancelPending {
                    OrderStatus::CancelPending
                } else {
                    OrderStatus::PartiallyFilled
                };
                if *next_status != expected_status {
                    return Err(inconsistent(
                        sequence,
                        "fill state does not match cumulative filled quantity",
                    ));
                }
                let turnover = fill.price.notional(fill.quantity)?;
                let new_notional = view.order.filled_notional.checked_add(turnover)?;
                let weighted_price =
                    i64::try_from(i128::from(new_notional.0) / i128::from(new_filled.get()))
                        .map_err(|_| {
                            inconsistent(sequence, "average fill price arithmetic failed")
                        })?;
                let average_fill_price = PricePaise::new(Paise(weighted_price))?;
                let charge_total = fill.charges.total()?;
                // Replay does not trust the recorded fee total. The opening
                // ChargeBook must reproduce every component for this date,
                // product, side, and turnover; a total that balances only
                // because cash was adjusted is still a forged fill.
                let expected_charges = self.replay_fill_charges(
                    view.order.intent.product,
                    fill.side,
                    turnover,
                    sequence,
                )?;
                if expected_charges != fill.charges {
                    return Err(inconsistent(
                        sequence,
                        "fill fees do not match the charge schedule for its trade date",
                    ));
                }
                let expected_cash_delta = match fill.side {
                    Side::Buy => turnover.checked_add(charge_total)?.checked_neg()?,
                    Side::Sell => turnover.checked_sub(charge_total)?,
                };
                if expected_cash_delta != fill.cash_delta {
                    return Err(inconsistent(
                        sequence,
                        "fill cash delta differs from turnover and charges",
                    ));
                }
                // Every derived value is computed before mutation. An
                // arithmetic overflow here therefore rejects the fill
                // without changing any projection field.
                let position_delta = signed_quantity(fill.side, fill.quantity.get());
                let current_position = self.positions.get(&instrument_id).copied().unwrap_or(0);
                let next_position =
                    i64::try_from(i128::from(current_position) + i128::from(position_delta))
                        .map_err(|_| inconsistent(sequence, "position arithmetic overflow"))?;
                let next_cash = self.cash.checked_add(fill.cash_delta)?;
                let next_charges = self.charges_paid.checked_add(charge_total)?;
                let next_fill_count = view
                    .order
                    .fill_count
                    .checked_add(1)
                    .ok_or_else(|| inconsistent(sequence, "fill counter overflow"))?;
                let next_fill_id = fill
                    .fill_id
                    .get()
                    .checked_add(1)
                    .ok_or_else(|| inconsistent(sequence, "fill identifier space exhausted"))?;
                // ---- mutation phase: no fallible arithmetic or validation ----
                self.market.debit_validated_liquidity(
                    &instrument_id,
                    venue,
                    fill.side,
                    fill.quantity.get(),
                );
                if let Some(view) = self.orders.get_mut(&fill.order_id) {
                    view.order.filled_quantity = new_filled;
                    view.order.filled_notional = new_notional;
                    view.order.average_fill_price = Some(average_fill_price);
                    view.order.fill_count = next_fill_count;
                    view.order.status = *next_status;
                    view.last_sequence = sequence;
                }
                self.positions.insert(instrument_id, next_position);
                self.cash = next_cash;
                self.charges_paid = next_charges;
                self.fills.insert(fill.fill_id, fill.clone());
                self.next_fill_id = next_fill_id;
            }
            EventPayload::ReconciliationObserved { report } => {
                let unresolved_unknown = self
                    .orders
                    .values()
                    .any(|view| view.order.status == OrderStatus::OutcomeUnknown);
                let actual_status =
                    match (&report.observed_positions, &report.observed_active_orders) {
                        (Some(positions), Some(orders)) => {
                            if positions == &report.expected_positions
                                && orders == &report.expected_active_orders
                                && !unresolved_unknown
                            {
                                ReconciliationStatus::Matched
                            } else {
                                ReconciliationStatus::Mismatch
                            }
                        }
                        _ => ReconciliationStatus::Unavailable,
                    };
                if report.status != actual_status {
                    return Err(inconsistent(
                        sequence,
                        "reconciliation status contradicts its snapshots",
                    ));
                }
                if report.expected_positions != self.position_snapshot()
                    || report.expected_active_orders != self.active_order_ids()
                {
                    return Err(inconsistent(
                        sequence,
                        "reconciliation expected state differs from the ledger",
                    ));
                }
                self.reconciliation_blocked = report.status != ReconciliationStatus::Matched;
                self.account_snapshot_available =
                    report.status != ReconciliationStatus::Unavailable;
            }
            EventPayload::KillSwitchChanged { enabled, .. } => {
                if !enabled && (self.reconciliation_blocked || !self.account_snapshot_available) {
                    return Err(inconsistent(
                        sequence,
                        "kill switch release requires a matched account snapshot",
                    ));
                }
                self.kill_switch_engaged = *enabled;
            }
            EventPayload::RiskStateObserved {
                daily_pnl,
                account_snapshot_available,
            } => {
                self.daily_pnl = *daily_pnl;
                self.account_snapshot_available = *account_snapshot_available;
                if !account_snapshot_available {
                    self.reconciliation_blocked = true;
                }
            }
            EventPayload::BrokerReportObserved { applied, .. } => {
                if !applied {
                    self.reconciliation_blocked = true;
                }
            }
            // The M3.5.2 ledger payloads belong to the account-ledger stream,
            // not to the paper-engine stream. Refuse them rather than silently
            // projecting economic facts into the wrong state machine.
            EventPayload::AccountOpenedEvidenced { .. }
            | EventPayload::ExecutionProjected { .. } => {
                return Err(inconsistent(
                    sequence,
                    "ledger projection payload is not valid in the paper engine stream",
                ));
            }
        }
        self.last_sequence = sequence;
        self.last_available_time = Some(envelope.times.available_time);
        Ok(())
    }

    #[must_use]
    pub fn position_snapshot(&self) -> BTreeMap<InstrumentId, i64> {
        self.positions
            .iter()
            .filter_map(|(id, quantity)| (*quantity != 0).then_some((id.clone(), *quantity)))
            .collect()
    }

    #[must_use]
    pub fn active_order_ids(&self) -> BTreeSet<OrderId> {
        self.orders
            .iter()
            .filter_map(|(id, view)| view.order.status.active().then_some(*id))
            .collect()
    }

    #[must_use]
    pub fn open_order_count(&self) -> u32 {
        self.orders
            .values()
            .filter(|view| view.order.status.active())
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }

    #[must_use]
    pub fn orders_last_second(&self, now: LogicalNanos) -> u32 {
        self.order_times
            .iter()
            .filter(|time| {
                now.get()
                    .checked_sub(time.get())
                    .is_some_and(|age| age < 1_000_000_000)
            })
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }

    pub fn trim_order_times(&mut self, now: LogicalNanos) {
        while self.order_times.front().is_some_and(|time| {
            now.get()
                .checked_sub(time.get())
                .is_some_and(|age| age >= 1_000_000_000)
        }) {
            self.order_times.pop_front();
        }
    }

    /// Recalculate every charge component for a fill from the opening
    /// configuration's fee schedule. This is the replay-side analogue of
    /// the charge calculation performed when a fill is first booked.
    fn replay_fill_charges(
        &self,
        product: Product,
        side: Side,
        turnover: Paise,
        sequence: u64,
    ) -> Result<ChargeComponents, ProjectionError> {
        let charge_book = self.charge_book.as_ref().ok_or_else(|| {
            inconsistent(
                sequence,
                "fill fee schedule cannot be reproduced: charge book is absent",
            )
        })?;
        let trade_date = self.trade_date.ok_or_else(|| {
            inconsistent(
                sequence,
                "fill fee schedule cannot be reproduced: trade date is absent",
            )
        })?;
        charge_book
            .calculate(trade_date, product, side, turnover)
            .map_err(|error| {
                inconsistent(
                    sequence,
                    &format!("fill fee schedule cannot be reproduced: {error}"),
                )
            })
    }

    /// Tick that priced fills for this instrument on the stream trade
    /// date, resolved from the opening configuration's catalog (matching
    /// the engine). `None` when the configuration cannot reproduce the
    /// fill-price rule.
    fn fill_price_tick(&self, instrument_id: &InstrumentId, venue: Venue) -> Option<Paise> {
        let instruments = self.instruments.as_ref()?;
        let trade_date = self.trade_date?;
        let terms = instruments
            .at(instrument_id, trade_date)?
            .effective_terms_on(venue, trade_date)?;
        Some(Paise(terms.tick_assignment.tick_paise))
    }

    pub fn view(&self) -> Option<AccountView> {
        let account_id = self.account_id?;
        let positions = self
            .positions
            .iter()
            .filter_map(|(instrument_id, quantity)| {
                if *quantity == 0 {
                    return None;
                }
                // Gross value: multiply by the exact doubled mark BEFORE
                // rounding (round-once at the accounting boundary).
                let mark_price_doubled = self.market.mark_price_doubled(instrument_id);
                let gross_paise =
                    mark_price_doubled.and_then(|exact| rounded_gross_paise(*quantity, exact));
                Some(PositionView {
                    instrument_id: instrument_id.clone(),
                    quantity: *quantity,
                    mark_price_doubled,
                    gross_paise,
                })
            })
            .collect();
        let open_orders = self
            .orders
            .values()
            .filter(|view| view.order.status.active())
            .cloned()
            .collect();
        Some(AccountView {
            account_id,
            cash: self.cash,
            charges_paid: self.charges_paid,
            daily_pnl: self.daily_pnl,
            account_snapshot_available: self.account_snapshot_available,
            reconciliation_blocked: self.reconciliation_blocked,
            kill_switch_engaged: self.kill_switch_engaged,
            session_phase: self.session_phase,
            calendar_version: self.calendar_version.clone(),
            positions,
            open_orders,
        })
    }

    #[must_use]
    pub fn risk_context_position(&self, instrument_id: &InstrumentId) -> i64 {
        self.positions.get(instrument_id).copied().unwrap_or(0)
    }

    #[must_use]
    pub fn gross_exposure_paise(&self) -> Option<Paise> {
        let mut total = 0_i128;
        for (instrument_id, quantity) in &self.positions {
            if *quantity == 0 {
                continue;
            }
            // Identical multiply-then-round boundary as PositionView.
            let mark = self.market.mark_price_doubled(instrument_id)?;
            let exposure = i128::from(rounded_gross_paise(*quantity, mark)?.0);
            total = total.checked_add(exposure)?;
        }
        i64::try_from(total).ok().map(Paise)
    }

    #[must_use]
    pub fn pending_order_quantities(
        &self,
        instrument_id: &InstrumentId,
    ) -> Option<(Quantity, Quantity)> {
        let mut buys = 0_i64;
        let mut sells = 0_i64;
        for view in self.orders.values().filter(|view| {
            view.order.intent.instrument_id == *instrument_id && view.order.status.active()
        }) {
            let remaining = view
                .order
                .intent
                .quantity
                .checked_sub(view.order.filled_quantity)
                .ok()?
                .get();
            match view.order.intent.side {
                Side::Buy => buys = buys.checked_add(remaining)?,
                Side::Sell => sells = sells.checked_add(remaining)?,
            }
        }
        Some((Quantity::new(buys).ok()?, Quantity::new(sells).ok()?))
    }

    #[must_use]
    pub fn pending_order_gross_exposure_paise(&self) -> Option<Paise> {
        let mut total = 0_i128;
        for view in self
            .orders
            .values()
            .filter(|view| view.order.status.active())
        {
            let remaining = view
                .order
                .intent
                .quantity
                .checked_sub(view.order.filled_quantity)
                .ok()?;
            let value = i128::from(view.order.intent.limit_price.get().0)
                .checked_mul(i128::from(remaining.get()))?;
            total = total.checked_add(value)?;
        }
        i64::try_from(total).ok().map(Paise)
    }

    #[must_use]
    pub fn intent(&self, id: IntentId) -> Option<&IntentProjection> {
        self.intents.get(&id)
    }

    #[must_use]
    pub fn order(&self, id: OrderId) -> Option<&OrderView> {
        self.orders.get(&id)
    }

    #[must_use]
    pub fn active_orders_for(&self, instrument_id: &InstrumentId, venue: Venue) -> Vec<OrderView> {
        self.orders
            .values()
            .filter(|view| {
                view.order.intent.instrument_id == *instrument_id
                    && view.order.intent.venue == venue
                    && view.order.status.active()
            })
            .cloned()
            .collect()
    }
}

pub(super) fn signed_quantity(side: Side, quantity: i64) -> i64 {
    match side {
        Side::Buy => quantity,
        Side::Sell => -quantity,
    }
}

fn inconsistent(sequence: u64, reason: &str) -> ProjectionError {
    ProjectionError::Inconsistent {
        sequence,
        reason: reason.to_owned(),
    }
}

/// Prove that a (schema version 2) fill names the quote event that funded
/// it, and that the fill respects the market-intent execution rules: the
/// referenced quote must be the active book entry at the time of the fill
/// (not an earlier quote for the same instrument), the book must be
/// executable under the same feed-health gate as live matching, carry the
/// displayed side (ask liquidity for buys, bid liquidity for sells), price
/// exactly at the configured slippage rule applied to that displayed
/// price, and respect the order limit.
/// Inputs needed to reproduce the matcher against the opening
/// configuration: the configured slippage, the instrument's assigned
/// tick, and the quote-age threshold used by feed-health eligibility.
struct FillProvenanceRule {
    slippage_ticks: u32,
    tick: Option<Paise>,
    max_quote_age_ns: Option<u64>,
}

fn validate_fill_provenance(
    market: &MarketState,
    envelope: &EventEnvelope,
    intent: &OrderIntent,
    fill: &Fill,
    sequence: u64,
    rule: FillProvenanceRule,
) -> Result<(), ProjectionError> {
    let causation = envelope.causation_id.as_ref().ok_or_else(|| {
        inconsistent(
            sequence,
            "schema version 2 fill carries no quote provenance",
        )
    })?;
    let book = market
        .book(&intent.instrument_id, intent.venue)
        .ok_or_else(|| {
            inconsistent(
                sequence,
                "fill references an instrument with no active book",
            )
        })?;
    // The cited event must be the currently active book's own funding
    // event: citations to any earlier quote for this instrument are
    // rejected, so a forged fill cannot revive a stale quote level.
    if book.journal_event_id != *causation {
        return Err(inconsistent(
            sequence,
            "fill quote provenance is not the active book quote",
        ));
    }
    if book.quote.instrument_id != intent.instrument_id {
        return Err(inconsistent(
            sequence,
            "fill quote provenance names another instrument",
        ));
    }
    let maximum_age_ns = rule.max_quote_age_ns.ok_or_else(|| {
        inconsistent(
            sequence,
            "fill feed-health rule cannot be reproduced: quote-age threshold is absent",
        )
    })?;
    if !matches!(
        market.health(
            &intent.instrument_id,
            intent.venue,
            envelope.times.available_time,
            maximum_age_ns
        ),
        FeedHealth::Healthy { .. }
    ) {
        return Err(inconsistent(
            sequence,
            "fill cites a book that was not healthy at fill time",
        ));
    }
    let displayed = match fill.side {
        Side::Buy => book.quote.ask_price,
        Side::Sell => book.quote.bid_price,
    }
    .ok_or_else(|| {
        inconsistent(
            sequence,
            "fill is funded by a side the referenced quote does not display",
        )
    })?;
    // The matcher prices a fill by shifting the displayed price by the
    // configured slippage in ticks, where the tick is the instrument's
    // assigned tick for the term interval. Replay must reproduce that
    // exact price: a fill at any other price — even one inside the order
    // limit — is a price the engine would never have generated.
    let tick = rule.tick.ok_or_else(|| {
        inconsistent(
            sequence,
            "fill price rule cannot be reproduced: instrument tick terms are absent",
        )
    })?;
    let signed_slippage = match fill.side {
        Side::Buy => i64::from(rule.slippage_ticks),
        Side::Sell => -i64::from(rule.slippage_ticks),
    };
    let expected_price = displayed
        .shifted_ticks(tick, signed_slippage)
        .map_err(|_| inconsistent(sequence, "fill price rule arithmetic failed"))?;
    if fill.price != expected_price {
        return Err(inconsistent(
            sequence,
            "fill price does not match the configured slippage rule",
        ));
    }
    let within_limit = match fill.side {
        Side::Buy => fill.price <= intent.limit_price,
        Side::Sell => fill.price >= intent.limit_price,
    };
    if !within_limit {
        return Err(inconsistent(
            sequence,
            "fill price violates the order limit",
        ));
    }
    Ok(())
}

#[cfg(all(test, feature = "market-shaped-tests"))]
mod tests {
    use account_journal::{Hash32, StoredRecord};
    use cost_model::{ChargeBook, ChargeRate};
    use trading_core::{
        AccountId, AlgoId, ChargeComponents, Continuity, DoubledPaise, EventEnvelope, EventId,
        EventPayload, EventTimes, Fill, FillId, InstrumentId, IntentId, LogicalNanos, Order,
        OrderId, OrderIntent, OrderStatus, Paise, PricePaise, Product, Quantity, QuoteSnapshot,
        RunId, Side, StoredBody, StrategyId, TradeDate, UtcNanos,
    };

    use super::{OrderView, Projection, rounded_gross_paise};

    fn instrument(value: &str) -> InstrumentId {
        InstrumentId::new(value).unwrap()
    }

    fn quote_record(id: &InstrumentId, bid: i64, ask: i64, sequence: u64) -> StoredRecord {
        StoredRecord {
            sequence,
            previous_hash: Hash32::ZERO,
            hash: Hash32::ZERO,
            body: StoredBody {
                envelope: EventEnvelope {
                    event_id: EventId::for_run(RunId::from_bytes([7u8; 16]), sequence),
                    schema_version: 1,
                    stream_id: "test".to_owned(),
                    source_id: Some("feed".to_owned()),
                    source_clock_domain: None,
                    session_id: None,
                    source_sequence: Some(sequence),
                    correlation_id: None,
                    causation_id: None,
                    times: EventTimes {
                        source_time_utc: None,
                        received_time_utc: UtcNanos::new(0),
                        available_time: LogicalNanos::new(0),
                        recorded_time_utc: UtcNanos::new(0),
                    },
                    quality: std::collections::BTreeSet::new(),
                },
                payload: EventPayload::QuoteReceived {
                    quote: QuoteSnapshot {
                        instrument_id: id.clone(),
                        venue: trading_core::Venue::Nse,
                        bid_price: Some(PricePaise::new(Paise(bid)).unwrap()),
                        bid_quantity: Quantity::new(1).unwrap(),
                        ask_price: Some(PricePaise::new(Paise(ask)).unwrap()),
                        ask_quantity: Quantity::new(1).unwrap(),
                        continuity: Continuity::Snapshot,
                    },
                },
            },
        }
    }

    fn opened_record(schema_version: u16) -> StoredRecord {
        StoredRecord {
            sequence: 1,
            previous_hash: Hash32::ZERO,
            hash: Hash32::ZERO,
            body: StoredBody {
                envelope: EventEnvelope {
                    event_id: EventId::for_run(RunId::from_bytes([7u8; 16]), 1),
                    schema_version,
                    stream_id: "test".to_owned(),
                    source_id: None,
                    source_clock_domain: None,
                    session_id: None,
                    source_sequence: None,
                    correlation_id: None,
                    causation_id: (schema_version == 2)
                        .then_some(EventId::for_run(RunId::from_bytes([7u8; 16]), 1)),
                    times: EventTimes {
                        source_time_utc: None,
                        received_time_utc: UtcNanos::new(0),
                        available_time: LogicalNanos::new(0),
                        recorded_time_utc: UtcNanos::new(0),
                    },
                    quality: std::collections::BTreeSet::new(),
                },
                payload: EventPayload::AccountOpened {
                    account_id: AccountId::new(1).unwrap(),
                    starting_cash: Paise(1_000_000),
                    trade_date: TradeDate::new(2026, 10, 28).unwrap(),
                },
            },
        }
    }

    fn fee_schedule_with_brokerage_bpb(brokerage_ppb: u64) -> ChargeBook {
        let mut rates = Vec::new();
        for product in [Product::Delivery, Product::Intraday] {
            for side in [Side::Buy, Side::Sell] {
                rates.push(ChargeRate {
                    effective_from: TradeDate::new(2026, 1, 1).unwrap(),
                    product,
                    side,
                    brokerage_ppb,
                    stt_ppb: 0,
                    exchange_ppb: 0,
                    sebi_ppb: 0,
                    stamp_ppb: 0,
                    gst_ppb: 0,
                });
            }
        }
        ChargeBook::new(rates).unwrap()
    }

    fn base_projection(side: Side) -> (Projection, InstrumentId, OrderId) {
        let id = instrument("nse.cash.synidx");
        let mut projection = Projection {
            charge_book: Some(fee_schedule_with_brokerage_bpb(0)),
            ..Projection::default()
        };
        projection.apply(&opened_record(1)).unwrap();
        projection
            .market
            .apply_record(&quote_record(&id, 995, 1_005, 2))
            .unwrap();
        let intent = OrderIntent {
            intent_id: IntentId::new(1).unwrap(),
            account_id: AccountId::new(1).unwrap(),
            strategy_id: StrategyId::new(2).unwrap(),
            strategy_version: "strategy-v1".to_owned(),
            algo_id: AlgoId::new(3).unwrap(),
            instrument_id: id.clone(),
            venue: trading_core::Venue::Nse,
            side,
            product: Product::Intraday,
            quantity: Quantity::new(1).unwrap(),
            limit_price: PricePaise::new(Paise(match side {
                Side::Buy => 1_005,
                Side::Sell => 995,
            }))
            .unwrap(),
            reason: "atomicity fixture".to_owned(),
            evidence: Vec::new(),
            expires_at: LogicalNanos::new(10),
        };
        let order_id = OrderId::new(1).unwrap();
        projection.orders.insert(
            order_id,
            OrderView {
                order: Order {
                    order_id,
                    intent,
                    status: OrderStatus::Working,
                    filled_quantity: Quantity::ZERO,
                    filled_notional: Paise(0),
                    average_fill_price: None,
                    fill_count: 0,
                },
                created_sequence: 2,
                last_sequence: 2,
            },
        );
        (projection, id, order_id)
    }

    fn fill_record(order_id: OrderId, side: Side, fill_id: u64) -> StoredRecord {
        let (price, cash_delta, charges) = match side {
            Side::Buy => (Paise(1_005), Paise(-1_005), ChargeComponents::default()),
            Side::Sell => (Paise(995), Paise(995), ChargeComponents::default()),
        };
        StoredRecord {
            sequence: 3,
            previous_hash: Hash32::ZERO,
            hash: Hash32::ZERO,
            body: StoredBody {
                envelope: EventEnvelope {
                    event_id: EventId::for_run(RunId::from_bytes([7u8; 16]), 3),
                    schema_version: 1,
                    stream_id: "test".to_owned(),
                    source_id: None,
                    source_clock_domain: None,
                    session_id: None,
                    source_sequence: None,
                    correlation_id: None,
                    causation_id: None,
                    times: EventTimes {
                        source_time_utc: None,
                        received_time_utc: UtcNanos::new(0),
                        available_time: LogicalNanos::new(0),
                        recorded_time_utc: UtcNanos::new(0),
                    },
                    quality: std::collections::BTreeSet::new(),
                },
                payload: EventPayload::FillBooked {
                    fill: Fill {
                        fill_id: FillId::new(fill_id).unwrap(),
                        order_id,
                        side,
                        quantity: Quantity::new(1).unwrap(),
                        price: PricePaise::new(price).unwrap(),
                        charges,
                        cash_delta,
                    },
                    next_status: OrderStatus::Filled,
                },
            },
        }
    }

    fn rejected_fill_leaves_projection_unchanged(
        mut projection: Projection,
        record: StoredRecord,
        expected_reason: &str,
    ) {
        let before = format!("{projection:?}");
        let error = projection
            .apply(&record)
            .expect_err("fill must be rejected");
        assert!(error.to_string().contains(expected_reason), "{error}");
        assert_eq!(before, format!("{projection:?}"));
    }

    #[test]
    fn rounded_gross_rounds_after_multiplication() {
        let odd = DoubledPaise::from_paise_pair(Paise(100), Paise(101));
        assert_eq!(rounded_gross_paise(1, odd), Some(Paise(100)));
        assert_eq!(rounded_gross_paise(2, odd), Some(Paise(201)));
        assert_eq!(rounded_gross_paise(3, odd), Some(Paise(302)));
        assert_eq!(rounded_gross_paise(7, odd), Some(Paise(704)));
        assert_eq!(rounded_gross_paise(0, odd), Some(Paise(0)));
        assert_eq!(rounded_gross_paise(-3, odd), Some(Paise(302)));
        let half = DoubledPaise::from_paise_pair(Paise(1), Paise(1));
        assert_eq!(rounded_gross_paise(1, half), Some(Paise(1)));
        let huge = DoubledPaise::from_paise_pair(Paise(i64::MAX), Paise(i64::MAX));
        assert_eq!(rounded_gross_paise(i64::MAX, huge), None);
    }

    #[test]
    fn valuation_keeps_exact_mark_until_the_multiplication() {
        let id = instrument("nse.cash.synidx");
        let mut projection = Projection::default();
        projection
            .market
            .apply_record(&quote_record(&id, 100, 101, 1))
            .unwrap();
        projection.account_id = Some(AccountId::new(1).unwrap());
        projection.positions.insert(id.clone(), 3);

        let view = projection.view().expect("account opened");
        let position = view
            .positions
            .iter()
            .find(|view| view.instrument_id == id)
            .expect("position exists");
        assert_eq!(
            position.mark_price_doubled,
            Some(DoubledPaise::from_paise_pair(Paise(100), Paise(101)))
        );
        assert_eq!(position.gross_paise, Some(Paise(302)));
        assert_eq!(projection.gross_exposure_paise(), Some(Paise(302)));
    }

    #[test]
    fn multiple_positions_round_independently_before_summing() {
        let id_a = instrument("nse.cash.synidx");
        let id_b = instrument("nse.cash.synidx2");
        let mut projection = Projection::default();
        for (id, sequence) in [(&id_a, 1u64), (&id_b, 2u64)] {
            projection
                .market
                .apply_record(&quote_record(id, 100, 101, sequence))
                .unwrap();
        }
        projection.account_id = Some(AccountId::new(1).unwrap());
        projection.positions.insert(id_a.clone(), 1);
        projection.positions.insert(id_b.clone(), 1);

        // Each position rounds alone: 1 x 100.5 -> 100 (twice) = 200 total,
        // rather than the 201 that rounding only at the grand total gives.
        assert_eq!(projection.gross_exposure_paise(), Some(Paise(200)));

        let view = projection.view().expect("account opened");
        assert_eq!(view.positions.len(), 2);
        for position in &view.positions {
            assert_eq!(position.gross_paise, Some(Paise(100)));
            assert_eq!(
                position.mark_price_doubled,
                Some(DoubledPaise::from_paise_pair(Paise(100), Paise(101)))
            );
        }
    }

    #[test]
    fn overflow_yields_no_gross_value_not_a_wrong_value() {
        let id = instrument("nse.cash.synidx");
        let mut projection = Projection::default();
        projection
            .market
            .apply_record(&quote_record(&id, i64::MAX, i64::MAX, 1))
            .unwrap();
        projection.account_id = Some(AccountId::new(1).unwrap());
        // i64::MAX lots against a near-i64::MAX doubled mark: even the i128
        // bridge is exhausted, so the gross value must fail closed as None,
        // never land on a wrong number.
        projection.positions.insert(id, i64::MAX);
        let view = projection.view().expect("account opened");
        assert_eq!(view.positions[0].gross_paise, None);
        assert_eq!(projection.gross_exposure_paise(), None);
    }

    #[test]
    fn position_overflow_rejects_fill_without_projection_mutation() {
        let (mut projection, id, order_id) = base_projection(Side::Buy);
        projection.positions.insert(id, i64::MAX);
        rejected_fill_leaves_projection_unchanged(
            projection,
            fill_record(order_id, Side::Buy, 1),
            "position arithmetic",
        );
    }

    #[test]
    fn cash_overflow_rejects_fill_without_projection_mutation() {
        let (mut projection, _, order_id) = base_projection(Side::Sell);
        projection.cash = Paise(i64::MAX);
        rejected_fill_leaves_projection_unchanged(
            projection,
            fill_record(order_id, Side::Sell, 1),
            "domain arithmetic",
        );
    }

    #[test]
    fn charge_overflow_rejects_fill_without_projection_mutation() {
        let (mut projection, _, order_id) = base_projection(Side::Buy);
        projection.charges_paid = Paise(i64::MAX);
        projection.charge_book = Some(fee_schedule_with_brokerage_bpb(2_000_000));
        let mut record = fill_record(order_id, Side::Buy, 1);
        if let EventPayload::FillBooked { fill, .. } = &mut record.body.payload {
            fill.charges = ChargeComponents {
                brokerage: Paise(2),
                ..ChargeComponents::default()
            };
            fill.cash_delta = Paise(-1_007);
        }
        rejected_fill_leaves_projection_unchanged(projection, record, "domain arithmetic");
    }

    #[test]
    fn fill_counter_overflow_rejects_fill_without_projection_mutation() {
        let (mut projection, _, order_id) = base_projection(Side::Buy);
        projection
            .orders
            .get_mut(&order_id)
            .expect("fixture order")
            .order
            .fill_count = u32::MAX;
        rejected_fill_leaves_projection_unchanged(
            projection,
            fill_record(order_id, Side::Buy, 1),
            "fill counter",
        );
    }

    #[test]
    fn fee_components_must_match_the_opening_charge_book() {
        let (projection, _, order_id) = base_projection(Side::Buy);
        // Zero-rate schedule, but the fill records a 2-paise brokerage and
        // adjusts cash accordingly: arithmetic balances, replay must not.
        let mut record = fill_record(order_id, Side::Buy, 1);
        if let EventPayload::FillBooked { fill, .. } = &mut record.body.payload {
            fill.charges = ChargeComponents {
                brokerage: Paise(2),
                ..ChargeComponents::default()
            };
            fill.cash_delta = Paise(-1_007);
        }
        rejected_fill_leaves_projection_unchanged(
            projection,
            record,
            "fill fees do not match the charge schedule",
        );
    }

    #[test]
    fn fill_identity_overflow_rejects_fill_without_projection_mutation() {
        let (mut projection, _, order_id) = base_projection(Side::Buy);
        projection.next_fill_id = u64::MAX;
        rejected_fill_leaves_projection_unchanged(
            projection,
            fill_record(order_id, Side::Buy, u64::MAX),
            "fill identifier",
        );
    }
}
