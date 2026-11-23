//! Trading events, orders, fills and account contracts.
//!
//! Derived from the internal Nirnay `crates/nirnay-core/src/model.rs` at source snapshot
//! `cc3a34c317a86a9984401efe90fa3f43d67a1787`.
//!
//! The events, order lifecycle and reconciliation shapes are preserved. The
//! identity and value types are not: [`InstrumentId`] is the canonical
//! `india_domain::InstrumentId`, [`Paise`] is canonical cash, [`Quantity`]
//! is non-negative lots, and [`Side`] is `market_types::Side`. Nirnay's
//! instrument catalogue is supplied by [`crate::InstrumentCatalog`].

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    AccountId, AlgoId, CheckedPaise, DomainError, DoubledPaise, EventId, FillId, InstrumentId,
    IntentId, LogicalNanos, OrderId, Paise, PricePaise, Quantity, Side, StrategyId, TradeDate,
    Venue,
};

pub use crate::time::EventTimes;

/// Continuity of a quote update.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Continuity {
    Snapshot,
    Delta,
}

/// Displayed top of book for one instrument.
///
/// This is *displayed* liquidity, not order-book depth. A paper matcher that
/// consumes it is a deterministic fill model, not a historical execution
/// simulator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteSnapshot {
    pub instrument_id: InstrumentId,
    /// Venue the quote was observed on. Feed health, the price collar and
    /// market matching are venue-specific: an NSE quote is not evidence for
    /// a BSE order on the same instrument.
    pub venue: Venue,
    pub bid_price: Option<PricePaise>,
    pub bid_quantity: Quantity,
    pub ask_price: Option<PricePaise>,
    pub ask_quantity: Quantity,
    pub continuity: Continuity,
}

impl QuoteSnapshot {
    /// Reject a quantity that has no matching price.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.bid_price.is_none() && !self.bid_quantity.is_zero() {
            return Err(DomainError::InvalidInstrument(
                "bid quantity without a bid".to_owned(),
            ));
        }
        if self.ask_price.is_none() && !self.ask_quantity.is_zero() {
            return Err(DomainError::InvalidInstrument(
                "ask quantity without an ask".to_owned(),
            ));
        }
        Ok(())
    }

    #[must_use]
    /// Exact mid price between the bid and ask in doubled paise (M₂ = b + a):
    /// never rounded into whole paise here. `None` until both sides exist.
    pub fn midpoint_doubled(&self) -> Option<DoubledPaise> {
        Some(self.bid_price?.midpoint_doubled(self.ask_price?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataQualityFlag {
    SourceClockUncertain,
    SequenceGap,
    LateArrival,
    RecoveredAfterDisconnect,
    IncompleteBook,
    SyntheticInput,
}

/// Event envelope. Identity is deterministic for a simulated run; the legal
/// source clock (venue time) is distinct from the receive, availability and
/// record clocks in [`EventTimes`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event_id: EventId,
    pub schema_version: u16,
    pub stream_id: String,
    pub source_id: Option<String>,
    pub source_clock_domain: Option<String>,
    pub session_id: Option<String>,
    pub source_sequence: Option<u64>,
    pub correlation_id: Option<EventId>,
    pub causation_id: Option<EventId>,
    pub times: EventTimes,
    pub quality: BTreeSet<DataQualityFlag>,
}

impl EventEnvelope {
    pub fn validate(&self) -> Result<(), DomainError> {
        if !(1..=3).contains(&self.schema_version) {
            return Err(DomainError::InvalidEventTime(format!(
                "unsupported schema version {}",
                self.schema_version
            )));
        }
        // Schema version 2 is the causally enriched envelope: fills carry
        // the identity of the quote event that funded them. Version 1 is the
        // legacy contract (fills without provenance, accepted for replay
        // compatibility until migrated). Version 3 adds venue identity to
        // order intents and quotes; v1/v2 bodies predate it and are read
        // through the migration in `StoredBody`'s deserializer.
        if self.schema_version == 2 && self.causation_id.is_none() {
            return Err(DomainError::InvalidEventTime(
                "schema version 2 envelopes must carry causation".to_owned(),
            ));
        }
        if self.stream_id.trim().is_empty() {
            return Err(DomainError::InvalidEventTime(
                "empty stream identifier".to_owned(),
            ));
        }
        if self.source_id.is_some() != self.source_clock_domain.is_some() {
            return Err(DomainError::InvalidEventTime(
                "source identity and clock domain must be supplied together".to_owned(),
            ));
        }
        if self.times.source_time_utc.is_some() && self.source_id.is_none() {
            return Err(DomainError::InvalidEventTime(
                "source timestamp requires a source identity and clock domain".to_owned(),
            ));
        }
        if self
            .source_id
            .as_ref()
            .is_some_and(|source| source.trim().is_empty())
            || self
                .source_clock_domain
                .as_ref()
                .is_some_and(|domain| domain.trim().is_empty())
            || self
                .session_id
                .as_ref()
                .is_some_and(|session| session.trim().is_empty())
        {
            return Err(DomainError::InvalidEventTime(
                "source, clock domain and session identifiers cannot be empty".to_owned(),
            ));
        }
        if self.source_sequence.is_some() && self.source_id.is_none() {
            return Err(DomainError::InvalidEventTime(
                "source sequence requires a source identifier".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Pointer into an append-only journal, carried as decision evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalRef {
    pub sequence: u64,
    pub hash_hex: String,
}

impl JournalRef {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.hash_hex.len() != 64 || !self.hash_hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(DomainError::InvalidInstrument(
                "journal reference hash must be 256-bit hexadecimal".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Durable evidence for an opening cash balance.
///
/// A ledger opening is never an invented number: it must name the broker or
/// operator statement and the journal record that carries that statement. The
/// reference is validated before the account-opening record is appended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpeningBalanceEvidence {
    pub statement_ref: String,
    pub journal_ref: JournalRef,
    pub observed_at_ns: i64,
}

impl OpeningBalanceEvidence {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.statement_ref.trim().is_empty() {
            return Err(DomainError::InvalidInstrument(
                "opening balance requires a statement reference".to_owned(),
            ));
        }
        if self.observed_at_ns <= 0 {
            return Err(DomainError::InvalidEventTime(
                "opening balance requires a positive observation time".to_owned(),
            ));
        }
        self.journal_ref.validate()
    }
}

/// Whether a ledger posting opens an economic effect or reverses a prior one.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerPostingKind {
    Posting,
    Reversal,
}

/// Domain-owned charge provenance. The OMS type is engine-local and cannot be
/// referenced from the domain layer, so the ledger carries its own typed value.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerChargeStatus {
    Estimated,
    BrokerConfirmed,
}

/// Brokerage product. `Delivery` is held to settlement; `Intraday` is
/// squared off within the session.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Product {
    Delivery,
    Intraday,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderIntent {
    pub intent_id: IntentId,
    pub account_id: AccountId,
    pub strategy_id: StrategyId,
    pub strategy_version: String,
    pub algo_id: AlgoId,
    pub instrument_id: InstrumentId,
    /// Venue the order is intended for. Execution eligibility is resolved
    /// against this venue's listing, never the first listing of the
    /// instrument: the same ISIN can carry different settlement regimes on
    /// NSE and BSE.
    ///
    /// This is part of the journaled order contract. A body that predates
    /// the field fails to decode rather than defaulting to a venue.
    pub venue: Venue,
    pub side: Side,
    pub product: Product,
    pub quantity: Quantity,
    pub limit_price: PricePaise,
    pub reason: String,
    pub evidence: Vec<JournalRef>,
    pub expires_at: LogicalNanos,
}

impl OrderIntent {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.strategy_version.trim().is_empty() {
            return Err(DomainError::InvalidInstrument(
                "order intent needs a strategy version".to_owned(),
            ));
        }
        if self.reason.trim().is_empty() {
            return Err(DomainError::InvalidInstrument(
                "order intent needs a reason".to_owned(),
            ));
        }
        if self.evidence.is_empty() {
            return Err(DomainError::InvalidInstrument(
                "order intent needs decision evidence".to_owned(),
            ));
        }
        if self.quantity.is_zero() {
            return Err(DomainError::ZeroQuantity);
        }
        for reference in &self.evidence {
            reference.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub strategy_id: StrategyId,
    pub strategy_version: String,
    pub source_build_hash: String,
    pub configuration_hash: String,
    pub dataset_hash: String,
    pub input_refs: Vec<JournalRef>,
    pub features_fixed_point: BTreeMap<String, i64>,
    pub rationale: String,
    pub intent: Option<OrderIntent>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Working,
    PartiallyFilled,
    CancelPending,
    Cancelled,
    Filled,
    Rejected,
    OutcomeUnknown,
}

impl OrderStatus {
    #[must_use]
    pub const fn terminal(self) -> bool {
        matches!(self, Self::Cancelled | Self::Filled | Self::Rejected)
    }

    #[must_use]
    pub const fn active(self) -> bool {
        matches!(
            self,
            Self::Working | Self::PartiallyFilled | Self::CancelPending | Self::OutcomeUnknown
        )
    }

    /// Whether `next` is a legal lifecycle transition.
    ///
    /// `OutcomeUnknown` is resumable so a broker report can
    /// resolve an ambiguous submission; terminal states never move again.
    #[must_use]
    pub const fn permits(self, next: Self) -> bool {
        match self {
            Self::Working => matches!(
                next,
                Self::PartiallyFilled
                    | Self::CancelPending
                    | Self::Cancelled
                    | Self::Filled
                    | Self::Rejected
                    | Self::OutcomeUnknown
            ),
            Self::PartiallyFilled => matches!(
                next,
                Self::CancelPending | Self::Cancelled | Self::Filled | Self::OutcomeUnknown
            ),
            Self::CancelPending => matches!(
                next,
                Self::PartiallyFilled | Self::Cancelled | Self::Filled | Self::OutcomeUnknown
            ),
            Self::OutcomeUnknown => matches!(
                next,
                Self::Working
                    | Self::PartiallyFilled
                    | Self::Cancelled
                    | Self::Filled
                    | Self::Rejected
            ),
            Self::Cancelled | Self::Filled | Self::Rejected => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    pub order_id: OrderId,
    pub intent: OrderIntent,
    pub status: OrderStatus,
    pub filled_quantity: Quantity,
    pub filled_notional: Paise,
    pub average_fill_price: Option<PricePaise>,
    pub fill_count: u32,
}

impl Order {
    /// Whether the filled quantity is consistent with the fill notional and
    /// the reported average price.
    pub fn validate_accounting(&self) -> Result<(), DomainError> {
        if self.filled_quantity.is_zero() {
            if self.filled_notional.0 != 0 || self.average_fill_price.is_some() {
                return Err(DomainError::InvalidInstrument(
                    "unfilled order carries fill accounting".to_owned(),
                ));
            }
            return Ok(());
        }
        let average = self.average_fill_price.ok_or_else(|| {
            DomainError::InvalidInstrument("partially filled order has no average price".to_owned())
        })?;
        if self.filled_notional != average.notional(self.filled_quantity)? {
            return Err(DomainError::InvalidInstrument(
                "filled notional differs from average price and quantity".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChargeComponents {
    pub brokerage: Paise,
    pub securities_transaction_tax: Paise,
    pub exchange_transaction: Paise,
    pub sebi_turnover: Paise,
    pub stamp_duty: Paise,
    pub gst: Paise,
    pub other: Paise,
}

impl Default for ChargeComponents {
    fn default() -> Self {
        Self {
            brokerage: Paise(0),
            securities_transaction_tax: Paise(0),
            exchange_transaction: Paise(0),
            sebi_turnover: Paise(0),
            stamp_duty: Paise(0),
            gst: Paise(0),
            other: Paise(0),
        }
    }
}

impl ChargeComponents {
    pub fn total(self) -> Result<Paise, DomainError> {
        self.brokerage
            .checked_add(self.securities_transaction_tax)?
            .checked_add(self.exchange_transaction)?
            .checked_add(self.sebi_turnover)?
            .checked_add(self.stamp_duty)?
            .checked_add(self.gst)?
            .checked_add(self.other)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fill {
    pub fill_id: FillId,
    pub order_id: OrderId,
    pub side: Side,
    pub quantity: Quantity,
    pub price: PricePaise,
    pub charges: ChargeComponents,
    pub cash_delta: Paise,
}

impl Fill {
    /// Exact cash movement implied by this fill, before any reversal sign is
    /// applied. Buys are an outflow (including charges); sells are an inflow
    /// net of charges.
    pub fn expected_cash_delta(&self) -> Result<Paise, DomainError> {
        if self.quantity.is_zero() {
            return Err(DomainError::ZeroQuantity);
        }
        let total_charges = self.charges.total()?;
        let turnover = self.price.notional(self.quantity)?;
        match self.side {
            Side::Buy => turnover.checked_add(total_charges)?.checked_neg(),
            Side::Sell => turnover.checked_sub(total_charges),
        }
    }

    /// Recompute the cash movement implied by this fill and compare it with
    /// the recorded value. Buys are a cash outflow (including charges); sells
    /// are an inflow net of charges.
    pub fn validate_accounting(&self) -> Result<(), DomainError> {
        let expected_cash = self.expected_cash_delta()?;
        if expected_cash != self.cash_delta {
            return Err(DomainError::InvalidInstrument(
                "fill cash delta differs from its accounting inputs".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedHealth {
    Healthy {
        age_ns: u64,
    },
    NoQuote,
    Stale {
        age_ns: u64,
    },
    Gapped {
        expected_sequence: Option<u64>,
        observed_sequence: u64,
    },
    Crossed,
    OneSided,
    Incomplete,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskCheckResult {
    pub check: RiskCheck,
    pub passed: bool,
    pub detail: String,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskCheck {
    AccountAllowlist,
    StrategyAllowlist,
    AlgoAllowlist,
    EvidenceReferences,
    InstrumentAllowlist,
    InstrumentReferenceData,
    /// Requested product (intraday/delivery) is compatible with the
    /// settlement regime and surveillance designation in force. A
    /// trade-for-trade or Z-category listing is tradable but not
    /// intraday-eligible.
    ProductEligibility,
    SessionOpen,
    MarketFeedHealth,
    AccountSnapshotAvailable,
    QuantityLimit,
    LotAlignment,
    TickAlignment,
    OrderNotional,
    PriceCollar,
    PositionLimit,
    GrossExposure,
    /// Projected notional for the intent's strategy, including pending and
    /// filled exposure, must fit the strategy cap.
    StrategyExposure,
    /// Synthetic sector concentration. An unlabelled instrument fails closed
    /// once a sector cap is configured.
    SectorExposure,
    /// Deterministic projected loss under the configured synthetic scenario.
    StressLoss,
    /// Order notional against synthetic displayed depth / ADV participation.
    LiquidityParticipation,
    /// Synthetic Greeks of a derivative-flagged instrument. Missing greeks on
    /// such an instrument fails closed.
    Greeks,
    /// Synthetic short-borrow availability and financing cost.
    BorrowFinancing,
    DailyLossLimit,
    OpenOrderLimit,
    OrderRateLimit,
    KillSwitch,
    Reconciliation,
    Idempotency,
    IntentExpiry,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub intent_id: IntentId,
    pub policy_version: String,
    pub accepted: bool,
    pub checks: Vec<RiskCheckResult>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationStatus {
    Matched,
    Mismatch,
    Unavailable,
}

/// Comparison of the internal projection with a broker/venue snapshot.
///
/// Positions are signed lots; `observed_positions` is `None` when the broker
/// view is unavailable, which must not be treated as a match.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconciliationReport {
    pub reconciliation_id: String,
    pub status: ReconciliationStatus,
    pub expected_positions: BTreeMap<InstrumentId, i64>,
    pub observed_positions: Option<BTreeMap<InstrumentId, i64>>,
    pub expected_active_orders: BTreeSet<OrderId>,
    pub observed_active_orders: Option<BTreeSet<OrderId>>,
    pub differences: Vec<String>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    PreOpen,
    OpeningAuction,
    Continuous,
    ClosingAuction,
    PostClose,
    SpecialSession,
    Closed,
    Holiday,
}

impl SessionPhase {
    #[must_use]
    pub const fn permits_regular_orders(self) -> bool {
        matches!(self, Self::Continuous)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventPayload {
    AccountOpened {
        account_id: AccountId,
        starting_cash: Paise,
        trade_date: TradeDate,
    },
    /// Account opening with explicit, durable evidence. Distinct from the
    /// legacy `AccountOpened` so that evidence can never be omitted silently.
    AccountOpenedEvidenced {
        account_id: AccountId,
        account_scope: String,
        starting_cash: Paise,
        trade_date: TradeDate,
        evidence: OpeningBalanceEvidence,
    },
    /// One OMS execution projected onto the account ledger. `content_digest`
    /// is the semantic identity of the source `FillEvent`, independent of the
    /// journal's ordinal hash chain.
    ExecutionProjected {
        kind: LedgerPostingKind,
        execution_id: String,
        content_digest: String,
        broker_event_id: String,
        client_order_id: u64,
        oms_order_id: String,
        charge_status: LedgerChargeStatus,
        account_scope: String,
        instrument_id: InstrumentId,
        provenance: String,
        fill: Fill,
        cash_delta: Paise,
    },
    SessionStateObserved {
        trade_date: TradeDate,
        phase: SessionPhase,
        calendar_version: String,
    },
    QuoteReceived {
        quote: QuoteSnapshot,
    },
    LateQuoteObserved {
        quote: QuoteSnapshot,
        reason: String,
    },
    FeedGapDetected {
        expected_sequence: Option<u64>,
        observed_sequence: u64,
    },
    DecisionMade {
        decision: DecisionRecord,
    },
    RiskAssessed {
        assessment: RiskAssessment,
    },
    OrderCreated {
        order: Order,
    },
    OrderTransitioned {
        order_id: OrderId,
        from: OrderStatus,
        to: OrderStatus,
        reason: String,
    },
    FillBooked {
        fill: Fill,
        next_status: OrderStatus,
    },
    ReconciliationObserved {
        report: ReconciliationReport,
    },
    KillSwitchChanged {
        enabled: bool,
        actor_id: u64,
        reason: String,
    },
    RiskStateObserved {
        daily_pnl: Paise,
        account_snapshot_available: bool,
    },
    BrokerReportObserved {
        order_id: OrderId,
        report_kind: String,
        applied: bool,
        current_status: OrderStatus,
    },
}

impl EventPayload {
    /// The minimum journal schema version this payload requires.
    ///
    /// Venue-bearing payloads need version 3; everything else may be written
    /// as version 1. `Journal::append` rejects a body that understates this,
    /// so a venue-bearing record can never be written as a legacy body. Fill
    /// provenance (version 2) is decided by the engine's record meta, not by
    /// the payload, because legacy journals are still replayable.
    #[must_use]
    pub const fn required_schema_version(&self) -> u16 {
        match self {
            Self::QuoteReceived { .. } | Self::LateQuoteObserved { .. } => 3,
            Self::OrderCreated { .. } => 3,
            Self::DecisionMade { decision } if decision.intent.is_some() => 3,
            _ => 1,
        }
    }

    /// Whether the payload serializes an order intent or a quote, i.e. the
    /// contracts that carry a venue in schema version 3.
    #[must_use]
    pub const fn carries_venue_contract(&self) -> bool {
        matches!(
            self,
            Self::QuoteReceived { .. }
                | Self::LateQuoteObserved { .. }
                | Self::OrderCreated { .. }
                | Self::DecisionMade { .. }
        )
    }

    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::AccountOpened { .. } => "account_opened",
            Self::AccountOpenedEvidenced { .. } => "account_opened_evidenced",
            Self::ExecutionProjected { .. } => "execution_projected",
            Self::SessionStateObserved { .. } => "session_state_observed",
            Self::QuoteReceived { .. } => "quote_received",
            Self::LateQuoteObserved { .. } => "late_quote_observed",
            Self::FeedGapDetected { .. } => "feed_gap_detected",
            Self::DecisionMade { .. } => "decision_made",
            Self::RiskAssessed { .. } => "risk_assessed",
            Self::OrderCreated { .. } => "order_created",
            Self::OrderTransitioned { .. } => "order_transitioned",
            Self::FillBooked { .. } => "fill_booked",
            Self::ReconciliationObserved { .. } => "reconciliation_observed",
            Self::KillSwitchChanged { .. } => "kill_switch_changed",
            Self::RiskStateObserved { .. } => "risk_state_observed",
            Self::BrokerReportObserved { .. } => "broker_report_observed",
        }
    }
}

/// A journal record body: envelope plus typed payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StoredBody {
    pub envelope: EventEnvelope,
    pub payload: EventPayload,
}

/// Legacy journal bodies (schema versions 1 and 2) predate venue identity.
///
/// They were written by the NSE-only paper engine, so the migration records
/// that fact explicitly instead of failing replay or inventing a venue for a
/// BSE order. Version 3 bodies must carry the field: a new record that omits
/// it is rejected rather than defaulted.
impl<'de> Deserialize<'de> for StoredBody {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawBody {
            envelope: EventEnvelope,
            payload: serde_json::Value,
        }

        let mut raw = RawBody::deserialize(deserializer)?;
        migrate_venue_fields(&mut raw.payload, raw.envelope.schema_version)
            .map_err(serde::de::Error::custom)?;
        let payload: EventPayload =
            serde_json::from_value(raw.payload).map_err(serde::de::Error::custom)?;
        Ok(Self {
            envelope: raw.envelope,
            payload,
        })
    }
}

/// Inject the legacy NSE venue into pre-v3 bodies, and require it in v3.
///
/// The paths are the only places an order intent or quote is serialized. For
/// `schema_version >= 3` a missing field is an error; below 3 it is filled
/// with `NSE`, which is what the pre-venue engine could produce.
fn migrate_venue_fields(
    payload: &mut serde_json::Value,
    schema_version: u16,
) -> Result<(), String> {
    const VENUE_PATHS: [&str; 4] = [
        "/QuoteReceived/quote",
        "/LateQuoteObserved/quote",
        "/DecisionMade/decision/intent",
        "/OrderCreated/order/intent",
    ];
    for path in VENUE_PATHS {
        let Some(object) = payload.pointer_mut(path) else {
            continue;
        };
        if object.is_null() {
            continue;
        }
        let Some(fields) = object.as_object_mut() else {
            return Err(format!("{path} is not an object"));
        };
        if fields.contains_key("venue") {
            continue;
        }
        if schema_version >= 3 {
            return Err(format!(
                "schema version {schema_version} body at {path} has no venue"
            ));
        }
        fields.insert(
            "venue".to_owned(),
            serde_json::Value::String("NSE".to_owned()),
        );
    }
    Ok(())
}

impl StoredBody {
    /// Validate the envelope and the payload-specific invariants before the
    /// record is appended. Fail-closed: an invalid body is never journaled.
    pub fn validate(&self) -> Result<(), DomainError> {
        self.envelope.validate()?;
        // Schema version 2 marks causally enriched records. It is reserved
        // for provenance-bearing fills; v2 lines carrying any other payload
        // do not name the market fact that caused them.
        if self.envelope.schema_version == 2
            && !matches!(
                self.payload,
                EventPayload::FillBooked { .. }
                    | EventPayload::AccountOpened { .. }
                    | EventPayload::AccountOpenedEvidenced { .. }
                    | EventPayload::ExecutionProjected { .. }
            )
        {
            return Err(DomainError::InvalidEventTime(
                "schema version 2 is reserved for fills and account openings".to_owned(),
            ));
        }
        // Version 3 exists for venue-bearing payloads. A v3 body with any
        // other payload would claim a contract it does not carry.
        if self.envelope.schema_version == 3 && !self.payload.carries_venue_contract() {
            return Err(DomainError::InvalidEventTime(
                "schema version 3 is reserved for venue-bearing payloads".to_owned(),
            ));
        }
        // A v2 genesis record is self-causative: it names the run root,
        // which is its own event identity.
        if self.envelope.schema_version == 2
            && matches!(
                self.payload,
                EventPayload::AccountOpened { .. } | EventPayload::AccountOpenedEvidenced { .. }
            )
            && self.envelope.causation_id.as_ref() != Some(&self.envelope.event_id)
        {
            return Err(DomainError::InvalidEventTime(
                "schema version 2 account opening must be self-causative".to_owned(),
            ));
        }
        match &self.payload {
            EventPayload::QuoteReceived { quote }
            | EventPayload::LateQuoteObserved { quote, .. } => {
                if self.envelope.source_id.is_none() || self.envelope.source_sequence.is_none() {
                    return Err(DomainError::InvalidEventTime(
                        "market quote needs a source identity and sequence".to_owned(),
                    ));
                }
                quote.validate()?;
            }
            EventPayload::FeedGapDetected {
                observed_sequence, ..
            } => {
                if self.envelope.source_id.is_none()
                    || self.envelope.source_sequence != Some(*observed_sequence)
                {
                    return Err(DomainError::InvalidEventTime(
                        "feed gap must match its source sequence".to_owned(),
                    ));
                }
            }
            EventPayload::AccountOpened { starting_cash, .. } if starting_cash.0 <= 0 => {
                return Err(DomainError::InvalidInstrument(
                    "starting cash must be positive".to_owned(),
                ));
            }
            EventPayload::AccountOpenedEvidenced {
                starting_cash,
                account_scope,
                evidence,
                ..
            } => {
                if starting_cash.0 <= 0 {
                    return Err(DomainError::InvalidInstrument(
                        "evidenced starting cash must be positive".to_owned(),
                    ));
                }
                if account_scope.trim().is_empty() {
                    return Err(DomainError::InvalidInstrument(
                        "account scope is required".to_owned(),
                    ));
                }
                evidence.validate()?;
            }
            EventPayload::ExecutionProjected {
                kind,
                execution_id,
                content_digest,
                broker_event_id,
                account_scope,
                provenance,
                fill,
                cash_delta,
                ..
            } => {
                if execution_id.trim().is_empty()
                    || broker_event_id.trim().is_empty()
                    || provenance.trim().is_empty()
                    || account_scope.trim().is_empty()
                {
                    return Err(DomainError::InvalidInstrument(
                        "projected execution requires execution, broker, scope and provenance identity".to_owned(),
                    ));
                }
                if content_digest.len() != 64
                    || !content_digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return Err(DomainError::InvalidInstrument(
                        "projected execution content digest must be 256-bit hexadecimal".to_owned(),
                    ));
                }
                let expected = fill.expected_cash_delta()?;
                let recorded = match kind {
                    LedgerPostingKind::Posting => expected,
                    LedgerPostingKind::Reversal => expected.checked_neg()?,
                };
                if recorded != *cash_delta {
                    return Err(DomainError::InvalidInstrument(
                        "projected execution cash delta differs from its accounting inputs"
                            .to_owned(),
                    ));
                }
            }
            EventPayload::DecisionMade { decision } => {
                for hash in [
                    &decision.source_build_hash,
                    &decision.configuration_hash,
                    &decision.dataset_hash,
                ] {
                    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                        return Err(DomainError::InvalidInstrument(
                            "decision hashes must be 256-bit hexadecimal values".to_owned(),
                        ));
                    }
                }
                if decision.strategy_version.trim().is_empty()
                    || decision.rationale.trim().is_empty()
                {
                    return Err(DomainError::InvalidInstrument(
                        "decision version and rationale are required".to_owned(),
                    ));
                }
                if let Some(intent) = &decision.intent {
                    intent.validate()?;
                    if decision.input_refs != intent.evidence {
                        return Err(DomainError::InvalidInstrument(
                            "decision inputs must match the order intent evidence".to_owned(),
                        ));
                    }
                }
            }
            EventPayload::OrderCreated { order } => {
                order.intent.validate()?;
                order.validate_accounting()?;
            }
            EventPayload::OrderTransitioned { from, to, .. } => {
                if !from.permits(*to) {
                    return Err(DomainError::InvalidInstrument(format!(
                        "illegal order transition {from:?} -> {to:?}"
                    )));
                }
            }
            EventPayload::FillBooked { fill, next_status } => {
                fill.validate_accounting()?;
                if next_status.terminal() && *next_status == OrderStatus::Rejected {
                    return Err(DomainError::InvalidInstrument(
                        "a booked fill cannot reject its order".to_owned(),
                    ));
                }
            }
            EventPayload::RiskAssessed { assessment } => {
                let failed = assessment.checks.iter().any(|check| !check.passed);
                if assessment.accepted == failed && !assessment.checks.is_empty() {
                    return Err(DomainError::InvalidInstrument(
                        "risk assessment decision contradicts its checks".to_owned(),
                    ));
                }
            }
            EventPayload::BrokerReportObserved { .. }
            | EventPayload::ReconciliationObserved { .. }
            | EventPayload::SessionStateObserved { .. }
            | EventPayload::KillSwitchChanged { .. }
            | EventPayload::RiskStateObserved { .. }
            | EventPayload::AccountOpened { .. } => {}
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "market-shaped-tests"))]
mod tests {
    use super::{
        ChargeComponents, EventEnvelope, EventPayload, EventTimes, Fill, Order, OrderIntent,
        OrderStatus, QuoteSnapshot,
    };
    use crate::{
        AccountId, AlgoId, DecisionRecord, DomainError, EventId, FillId, InstrumentId, IntentId,
        JournalRef, LogicalNanos, OrderId, Paise, PricePaise, Product, Quantity, RunId, Side,
        StoredBody, StrategyId, UtcNanos, Venue,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn envelope() -> EventEnvelope {
        EventEnvelope {
            event_id: EventId::for_run(RunId::from_bytes([7; 16]), 1),
            schema_version: 1,
            stream_id: "test-stream".to_owned(),
            source_id: None,
            source_clock_domain: None,
            session_id: None,
            source_sequence: None,
            correlation_id: None,
            causation_id: None,
            times: EventTimes {
                source_time_utc: None,
                received_time_utc: UtcNanos::new(1),
                available_time: LogicalNanos::new(1),
                recorded_time_utc: UtcNanos::new(1),
            },
            quality: BTreeSet::new(),
        }
    }

    fn fill(cash_delta: i64) -> Result<Fill, DomainError> {
        Ok(Fill {
            fill_id: FillId::new(1)?,
            order_id: OrderId::new(1)?,
            side: Side::Buy,
            quantity: Quantity::new(3)?,
            price: PricePaise::new(Paise(1_500))?,
            charges: ChargeComponents {
                brokerage: Paise(10),
                ..ChargeComponents::default()
            },
            cash_delta: Paise(cash_delta),
        })
    }

    #[test]
    fn order_status_never_leaves_a_terminal_state() {
        assert!(!OrderStatus::Filled.permits(OrderStatus::Cancelled));
        assert!(!OrderStatus::Cancelled.permits(OrderStatus::Working));
        assert!(OrderStatus::OutcomeUnknown.permits(OrderStatus::Filled));
    }

    #[test]
    fn schema_version_two_scopes_fill_provenance() -> Result<(), DomainError> {
        let body =
            |schema_version: u16, causation: Option<EventId>| -> Result<StoredBody, DomainError> {
                Ok(StoredBody {
                    envelope: EventEnvelope {
                        causation_id: causation,
                        schema_version,
                        ..envelope()
                    },
                    payload: EventPayload::FillBooked {
                        fill: fill(-4_510)?,
                        next_status: OrderStatus::Filled,
                    },
                })
            };
        // Legacy v1 fills: no causation identity (pre-provenance contract).
        assert!(body(1, None)?.validate().is_ok());
        // v2 must carry causation; v2 without causation is rejected.
        let quote_ref = EventId::for_run(RunId::from_bytes([9u8; 16]), 42);
        assert!(body(2, Some(quote_ref.clone()))?.validate().is_ok());
        assert!(body(2, None)?.validate().is_err());
        // v2 is reserved for provenance-bearing fills and account
        // openings (the genesis run-format declaration).
        let opened = StoredBody {
            envelope: EventEnvelope {
                causation_id: Some(quote_ref.clone()),
                schema_version: 2,
                ..envelope()
            },
            payload: EventPayload::AccountOpened {
                account_id: AccountId::new(1)?,
                starting_cash: Paise(1_000_000),
                trade_date: jiff::civil::Date::new(2026, 10, 28)
                    .ok()
                    .ok_or_else(|| DomainError::InvalidEventTime("invalid date".to_owned()))?,
            },
        };
        assert!(opened.validate().is_err());
        let self_causative_opened = StoredBody {
            envelope: EventEnvelope {
                causation_id: Some(EventId::for_run(RunId::from_bytes([7u8; 16]), 1)),
                schema_version: 2,
                ..envelope()
            },
            payload: EventPayload::AccountOpened {
                account_id: AccountId::new(1)?,
                starting_cash: Paise(1_000_000),
                trade_date: jiff::civil::Date::new(2026, 10, 28)
                    .ok()
                    .ok_or_else(|| DomainError::InvalidEventTime("invalid date".to_owned()))?,
            },
        };
        assert!(self_causative_opened.validate().is_ok());
        let other = StoredBody {
            envelope: EventEnvelope {
                causation_id: Some(quote_ref.clone()),
                schema_version: 2,
                ..envelope()
            },
            payload: EventPayload::DecisionMade {
                decision: DecisionRecord {
                    strategy_id: StrategyId::new(2)?,
                    strategy_version: "strategy-v1".to_owned(),
                    source_build_hash: "a".repeat(64),
                    configuration_hash: "b".repeat(64),
                    dataset_hash: "c".repeat(64),
                    input_refs: Vec::new(),
                    features_fixed_point: BTreeMap::new(),
                    rationale: "v2 decision".to_owned(),
                    intent: None,
                },
            },
        };
        assert!(other.validate().is_err());
        Ok(())
    }

    #[test]
    fn fill_cash_delta_is_verified() -> Result<(), DomainError> {
        // 1500 paise * 3 lots = 4500, plus 10 brokerage = 4510 outflow.
        assert_eq!(fill(-4_510)?.validate_accounting(), Ok(()));
        assert!(fill(-4_500)?.validate_accounting().is_err());
        Ok(())
    }

    #[test]
    fn zero_quantity_fill_is_rejected() -> Result<(), DomainError> {
        let mut value = fill(-10)?;
        value.quantity = Quantity::ZERO;
        assert_eq!(value.validate_accounting(), Err(DomainError::ZeroQuantity));
        Ok(())
    }

    #[test]
    fn account_opening_rejects_non_positive_cash() -> Result<(), DomainError> {
        let body = StoredBody {
            envelope: envelope(),
            payload: EventPayload::AccountOpened {
                account_id: AccountId::new(1)?,
                starting_cash: Paise(0),
                trade_date: jiff::civil::Date::new(2026, 10, 28)
                    .ok()
                    .ok_or(DomainError::InvalidInstrument("bad date".to_owned()))?,
            },
        };
        assert!(body.validate().is_err());
        Ok(())
    }

    fn intent_order_body(schema_version: u16, venue: Venue) -> Result<StoredBody, DomainError> {
        Ok(StoredBody {
            envelope: EventEnvelope {
                schema_version,
                ..envelope()
            },
            payload: EventPayload::OrderCreated {
                order: Order {
                    order_id: OrderId::new(1)?,
                    intent: OrderIntent {
                        intent_id: IntentId::new(1)?,
                        account_id: AccountId::new(1)?,
                        strategy_id: StrategyId::new(2)?,
                        strategy_version: "strategy-v1".to_owned(),
                        algo_id: AlgoId::new(3)?,
                        instrument_id: InstrumentId::new("nse.cash.synidx")
                            .map_err(|error| DomainError::InvalidInstrument(error.to_string()))?,
                        venue,
                        side: Side::Buy,
                        product: Product::Intraday,
                        quantity: Quantity::new(1)?,
                        limit_price: PricePaise::new(Paise(1_000))?,
                        reason: "test intent".to_owned(),
                        evidence: vec![JournalRef {
                            sequence: 1,
                            hash_hex: "ab".repeat(32),
                        }],
                        expires_at: LogicalNanos::new(10),
                    },
                    status: OrderStatus::Working,
                    filled_quantity: Quantity::ZERO,
                    filled_notional: Paise(0),
                    average_fill_price: None,
                    fill_count: 0,
                },
            },
        })
    }

    /// The serialized shape a pre-venue engine wrote: no `venue` field.
    fn strip_venue(body: &StoredBody) -> serde_json::Value {
        let mut value = serde_json::to_value(body).expect("body serializes");
        let intent = value["payload"]["OrderCreated"]["order"]["intent"]
            .as_object_mut()
            .expect("intent object");
        assert!(intent.remove("venue").is_some(), "fixture must have venue");
        value
    }

    #[test]
    fn legacy_intent_body_migrates_to_nse_on_read() -> Result<(), DomainError> {
        let body = intent_order_body(3, Venue::Bse)?;
        let mut value = strip_venue(&body);
        value["envelope"]["schema_version"] = serde_json::json!(1);
        let decoded: StoredBody = serde_json::from_value(value).expect("legacy body decodes");
        match decoded.payload {
            EventPayload::OrderCreated { order } => {
                assert_eq!(
                    order.intent.venue,
                    Venue::Nse,
                    "pre-venue records were produced by the NSE-only engine"
                );
            }
            other => panic!("unexpected payload {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn venue_bearing_body_round_trips() -> Result<(), DomainError> {
        let body = intent_order_body(3, Venue::Bse)?;
        let text = serde_json::to_string(&body).expect("body serializes");
        let decoded: StoredBody = serde_json::from_str(&text).expect("v3 body decodes");
        assert_eq!(decoded, body);
        Ok(())
    }

    #[test]
    fn version_three_body_without_venue_is_rejected() -> Result<(), DomainError> {
        let body = intent_order_body(3, Venue::Bse)?;
        let value = strip_venue(&body);
        let error =
            serde_json::from_value::<StoredBody>(value).expect_err("a v3 body must name its venue");
        assert!(
            error.to_string().contains("venue"),
            "the rejection must name the missing venue: {error}"
        );
        Ok(())
    }

    #[test]
    fn schema_version_three_is_reserved_for_venue_payloads() -> Result<(), DomainError> {
        let body = StoredBody {
            envelope: EventEnvelope {
                schema_version: 3,
                ..envelope()
            },
            payload: EventPayload::KillSwitchChanged {
                enabled: true,
                actor_id: 7,
                reason: "not a venue contract".to_owned(),
            },
        };
        assert!(body.validate().is_err());
        Ok(())
    }

    #[test]
    fn venue_payloads_require_schema_version_three() -> Result<(), DomainError> {
        let quote = QuoteSnapshot {
            instrument_id: InstrumentId::new("nse.cash.synidx")
                .map_err(|error| DomainError::InvalidInstrument(error.to_string()))?,
            venue: Venue::Nse,
            bid_price: Some(PricePaise::new(Paise(995))?),
            bid_quantity: Quantity::new(1)?,
            ask_price: Some(PricePaise::new(Paise(1_005))?),
            ask_quantity: Quantity::new(1)?,
            continuity: super::Continuity::Snapshot,
        };
        let body = StoredBody {
            envelope: EventEnvelope {
                schema_version: 1,
                source_id: Some("feed".to_owned()),
                source_clock_domain: Some("utc".to_owned()),
                source_sequence: Some(1),
                ..envelope()
            },
            payload: EventPayload::QuoteReceived { quote },
        };
        assert_eq!(body.payload.required_schema_version(), 3);
        assert_eq!(
            intent_order_body(1, Venue::Nse)?
                .payload
                .required_schema_version(),
            3
        );
        Ok(())
    }
}
