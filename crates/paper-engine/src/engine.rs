//! Deterministic paper-trading order lifecycle.
//!
//! Derived from the internal Nirnay `crates/nirnay-engine/src/engine.rs` at source snapshot
//! `cc3a34c317a86a9984401efe90fa3f43d67a1787`.
//!
//! The engine is an event-sourced paper broker: orders, fills, risk decisions,
//! reconciliation and feed events are appended to the account journal and the
//! in-memory projection is rebuilt by replay. Matching consumes the *displayed*
//! top-of-book quantity, so it is a deterministic fill model rather than a
//! realistic historical execution simulator.

use std::collections::{BTreeMap, BTreeSet};

use account_journal::{
    FencingToken, //
    Journal,
    JournalError,
    StoredRecord,
};
use cost_model::{ChargeBook, FeeError};
use risk_engine::{RiskContext, RiskPolicy, ValidatedRiskPolicy, evaluate_risk};
use thiserror::Error;
use trading_core::{
    AccountId, CheckedPaise, Continuity, DataQualityFlag, DecisionRecord, DomainError,
    EventEnvelope, EventId, EventPayload, EventTimes, FeedHealth, Fill, FillId, InstrumentCatalog,
    InstrumentId, IntentId, JournalRef, LogicalNanos, Order, OrderId, OrderIntent, OrderStatus,
    Paise, PricePaise, Product, Quantity, QuoteSnapshot, ReconciliationReport,
    ReconciliationStatus, RiskAssessment, RunId, SessionPhase, Side, StoredBody, TradeDate,
    UtcNanos,
};

use crate::capsule::{CapsuleError, DecisionCapsule};
use crate::market::{MarketError, QuoteDisposition};
use crate::state::Projection;

#[derive(Clone, Debug)]
pub struct PaperProfile {
    /// Extra ticks applied beyond the displayed opposite top-of-book level.
    pub slippage_ticks: u32,
    /// Simulation consumes displayed top-of-book quantity only.
    pub consume_displayed_liquidity_only: bool,
}

impl PaperProfile {
    pub fn validate(&self) -> Result<(), EngineError> {
        if self.slippage_ticks > 100 || !self.consume_displayed_liquidity_only {
            return Err(EngineError::InvalidConfiguration(
                "paper profile must use displayed liquidity and a bounded slippage value"
                    .to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub stream_id: String,
    /// Identity of this simulated run. A replay must reuse the recorded value
    /// so deterministic event identifiers reproduce exactly.
    pub run_id: RunId,
    pub account_id: AccountId,
    pub starting_cash: Paise,
    pub trade_date: TradeDate,
    pub instruments: InstrumentCatalog,
    pub risk_policy: RiskPolicy,
    pub charge_book: ChargeBook,
    pub paper_profile: PaperProfile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconcileInput {
    /// `None` means that the source did not provide a positions snapshot.
    pub positions: Option<BTreeMap<InstrumentId, i64>>,
    /// `None` means that the source did not provide an open-order snapshot.
    pub active_orders: Option<BTreeSet<OrderId>>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SubmissionOutcome {
    pub risk: Option<RiskAssessment>,
    pub order_id: Option<OrderId>,
    pub deduplicated: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CancelOutcome {
    Pending,
    Cancelled,
    LateAfterFill,
    AlreadyTerminal,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum QuoteOutcome {
    Accepted,
    Duplicate,
    Late,
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("configuration is invalid: {0}")]
    InvalidConfiguration(String),
    #[error("stream identity or account configuration differs from the journal")]
    StreamConfigurationMismatch,
    #[error("journal replay failed: {0}")]
    Replay(String),
    #[error("market event failed: {0}")]
    Market(String),
    #[error("fee schedule failed: {0}")]
    Fees(String),
    #[error("decision capsule could not be assembled: {0}")]
    Capsule(String),
    #[error("decision evidence reference does not resolve to the recorded input event")]
    InvalidEvidenceReference,
    #[error("intent identifier conflicts with a prior request")]
    IdempotencyConflict,
    #[error("order identity does not exist")]
    UnknownOrder,
    #[error("order transition is not valid for the current state")]
    InvalidOrderTransition,
    #[error("unknown order state requires a complete authoritative broker report")]
    UnknownOutcomeNeedsResolution,
    #[error("logical availability clock moved backwards")]
    LogicalClockRegression,
    #[error("the journal committed an event but its in-memory projection failed; stop and reopen")]
    ProjectionAfterCommit,
    #[error("the engine cannot accept writes after a projection failure")]
    Poisoned,
}

#[derive(Clone, Debug)]
struct RecordMeta {
    times: EventTimes,
    source_id: Option<String>,
    source_clock_domain: Option<String>,
    source_sequence: Option<u64>,
    correlation_id: Option<EventId>,
    causation_id: Option<EventId>,
    schema_version: u16,
}

impl RecordMeta {
    fn internal(mut times: EventTimes) -> Self {
        // Internal transitions have no venue clock; drop any inherited source timestamp so the
        // envelope cannot claim a source time without a source identity.
        times.source_time_utc = None;
        Self {
            times,
            source_id: None,
            source_clock_domain: None,
            source_sequence: None,
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
        }
    }

    /// Genesis-of-run meta (first journal record): declares schema
    /// version 2, the run format that requires fill-to-quote
    /// provenance. Its causation is self-referential (the run root).
    fn genesis(mut times: EventTimes, event_id: EventId) -> Self {
        times.source_time_utc = None;
        Self {
            times,
            source_id: None,
            source_clock_domain: None,
            source_sequence: None,
            correlation_id: None,
            causation_id: Some(event_id),
            schema_version: 2,
        }
    }

    /// Provenance-bearing meta for fills committed against a specific quote
    /// event: the journal envelope (schema version 2) carries the quote's
    /// identity as causation, so replay can verify the fill against the
    /// exact event that funded it.
    fn for_fill(mut times: EventTimes, quote_event_id: EventId) -> Self {
        times.source_time_utc = None;
        Self {
            times,
            source_id: None,
            source_clock_domain: None,
            source_sequence: None,
            correlation_id: None,
            causation_id: Some(quote_event_id),
            schema_version: 2,
        }
    }
}

pub struct Engine {
    journal: Journal,
    /// Current local fencing token, issued to this writer at open. Every
    /// journal append presents it; the durable term advances on each reopen.
    fencing: FencingToken,
    config: EngineConfig,
    validated_policy: ValidatedRiskPolicy,
    projection: Projection,
    poisoned: bool,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Engine")
            .field("stream_id", &self.config.stream_id)
            .field("account_id", &self.config.account_id)
            .field("records", &self.journal.records().len())
            .field("poisoned", &self.poisoned)
            .finish_non_exhaustive()
    }
}

impl Engine {
    pub fn open(
        journal_path: impl AsRef<std::path::Path>,
        config: EngineConfig,
        startup_times: EventTimes,
    ) -> Result<Self, EngineError> {
        // The evaluator only accepts a validated policy value: invalid policies
        // cannot reach risk evaluation from this engine's construction path.
        let validated_policy = ValidatedRiskPolicy::from_policy(&config.risk_policy)?;
        config.paper_profile.validate()?;
        if config.starting_cash.0 <= 0 || config.stream_id.trim().is_empty() {
            return Err(EngineError::InvalidConfiguration(
                "stream identity and starting cash must be set".to_owned(),
            ));
        }
        for instrument_id in &config.risk_policy.allowed_instruments {
            if config
                .instruments
                .at(instrument_id, config.trade_date)
                .is_none()
            {
                return Err(EngineError::InvalidConfiguration(format!(
                    "allowed instrument {instrument_id} has no active version on {}",
                    config.trade_date
                )));
            }
        }
        for product in [Product::Delivery, Product::Intraday] {
            for side in [Side::Buy, Side::Sell] {
                config
                    .charge_book
                    .calculate(config.trade_date, product, side, Paise(0))
                    .map_err(fee_error)?;
            }
        }
        let mut journal = Journal::open(journal_path, config.stream_id.clone())?;
        // Replay reproduces the matcher against this run's own fill-price
        // rule: reopening with a different slippage profile or instrument
        // catalog fails fills instead of re-pricing history.
        let mut projection = Projection {
            slippage_ticks: config.paper_profile.slippage_ticks,
            instruments: Some(config.instruments.clone()),
            max_quote_age_ns: Some(config.risk_policy.max_quote_age_ns),
            charge_book: Some(config.charge_book.clone()),
            ..Projection::default()
        };
        for record in journal.records() {
            projection
                .apply(record)
                .map_err(|error| EngineError::Replay(error.to_string()))?;
            if let EventPayload::RiskAssessed { assessment } = &record.body.payload
                && assessment.policy_version != config.risk_policy.version
            {
                return Err(EngineError::StreamConfigurationMismatch);
            }
        }
        let owner = format!("paper-engine:{}", config.run_id);
        let fencing = journal.fence(&owner)?;
        let mut engine = Self {
            journal,
            fencing,
            config,
            validated_policy,
            projection,
            poisoned: false,
        };
        if engine.journal.records().is_empty() {
            // New journals declare the provenance run format at genesis;
            // pre-cutover journals keep their v1 declaration and replay
            // under the legacy contract.
            let genesis_event_id = engine
                .journal
                .next_event_id(engine.config.run_id)
                .map_err(|error| EngineError::Replay(error.to_string()))?;
            engine.record(
                EventPayload::AccountOpened {
                    account_id: engine.config.account_id,
                    starting_cash: engine.config.starting_cash,
                    trade_date: engine.config.trade_date,
                },
                RecordMeta::genesis(startup_times, genesis_event_id),
            )?;
        } else if engine.projection.account_id != Some(engine.config.account_id)
            || engine.projection.starting_cash != Some(engine.config.starting_cash)
            || engine.projection.trade_date != Some(engine.config.trade_date)
        {
            return Err(EngineError::StreamConfigurationMismatch);
        }
        Ok(engine)
    }

    pub fn set_session_phase(
        &mut self,
        phase: SessionPhase,
        calendar_version: impl Into<String>,
        times: EventTimes,
    ) -> Result<(), EngineError> {
        let calendar_version = calendar_version.into();
        if calendar_version.trim().is_empty() {
            return Err(EngineError::InvalidConfiguration(
                "calendar version is required".to_owned(),
            ));
        }
        self.record(
            EventPayload::SessionStateObserved {
                trade_date: self.config.trade_date,
                phase,
                calendar_version,
            },
            RecordMeta::internal(times),
        )?;
        Ok(())
    }

    pub fn set_kill_switch(
        &mut self,
        enabled: bool,
        actor_id: u64,
        reason: impl Into<String>,
        times: EventTimes,
    ) -> Result<(), EngineError> {
        let reason = reason.into();
        if actor_id == 0 || reason.trim().is_empty() {
            return Err(EngineError::InvalidConfiguration(
                "operator identity and reason are required for kill-switch changes".to_owned(),
            ));
        }
        if !enabled
            && (self.projection.reconciliation_blocked
                || !self.projection.account_snapshot_available)
        {
            return Err(EngineError::InvalidOrderTransition);
        }
        self.record(
            EventPayload::KillSwitchChanged {
                enabled,
                actor_id,
                reason,
            },
            RecordMeta::internal(times),
        )?;
        Ok(())
    }

    pub fn observe_risk_state(
        &mut self,
        daily_pnl: Paise,
        account_snapshot_available: bool,
        times: EventTimes,
    ) -> Result<(), EngineError> {
        self.record(
            EventPayload::RiskStateObserved {
                daily_pnl,
                account_snapshot_available,
            },
            RecordMeta::internal(times),
        )?;
        Ok(())
    }

    pub fn on_quote(
        &mut self,
        source_id: impl Into<String>,
        source_sequence: u64,
        quote: QuoteSnapshot,
        times: EventTimes,
    ) -> Result<QuoteOutcome, EngineError> {
        self.ensure_writable()?;
        if source_sequence == u64::MAX {
            return Err(EngineError::InvalidConfiguration(
                "source sequence exhausted".to_owned(),
            ));
        }
        quote.validate()?;
        let source_id = source_id.into();
        if source_id.trim().is_empty() {
            return Err(EngineError::InvalidConfiguration(
                "market-data source identity is empty".to_owned(),
            ));
        }
        let disposition = self
            .projection
            .market
            .classify(&source_id, source_sequence, &quote.instrument_id, &quote)
            .map_err(market_error)?;
        match disposition {
            QuoteDisposition::Duplicate => return Ok(QuoteOutcome::Duplicate),
            QuoteDisposition::Late => {
                let expected = self.projection.market.expected_sequence(&source_id);
                self.record(
                    EventPayload::FeedGapDetected {
                        expected_sequence: expected,
                        observed_sequence: source_sequence,
                    },
                    RecordMeta {
                        times: times.clone(),
                        source_id: Some(source_id.clone()),
                        source_clock_domain: Some("UTC".to_owned()),
                        source_sequence: Some(source_sequence),
                        correlation_id: None,
                        causation_id: None,
                        schema_version: 1,
                    },
                )?;
                self.record(
                    EventPayload::LateQuoteObserved {
                        quote,
                        reason: "source sequence arrived behind the active watermark".to_owned(),
                    },
                    RecordMeta {
                        times,
                        source_id: Some(source_id.clone()),
                        source_clock_domain: Some("UTC".to_owned()),
                        source_sequence: Some(source_sequence),
                        correlation_id: None,
                        causation_id: None,
                        schema_version: 1,
                    },
                )?;
                return Ok(QuoteOutcome::Late);
            }
            QuoteDisposition::New => {}
        }
        let expected = self.projection.market.expected_sequence(&source_id);
        let sequence_gap =
            quote.continuity == Continuity::Delta && expected != Some(source_sequence);
        let record_quote = quote.clone();
        if sequence_gap {
            let mut quality = BTreeSet::new();
            quality.insert(DataQualityFlag::SequenceGap);
            self.record(
                EventPayload::FeedGapDetected {
                    expected_sequence: expected,
                    observed_sequence: source_sequence,
                },
                RecordMeta {
                    times: times.clone(),
                    source_id: Some(source_id.clone()),
                    source_clock_domain: Some("UTC".to_owned()),
                    source_sequence: Some(source_sequence),
                    correlation_id: None,
                    causation_id: None,
                    schema_version: 1,
                },
            )?;
            self.record_with_quality(
                EventPayload::QuoteReceived {
                    quote: record_quote,
                },
                RecordMeta {
                    times: times.clone(),
                    source_id: Some(source_id.clone()),
                    source_clock_domain: Some("UTC".to_owned()),
                    source_sequence: Some(source_sequence),
                    correlation_id: None,
                    causation_id: None,
                    schema_version: 1,
                },
                quality,
            )?;
        } else {
            self.record(
                EventPayload::QuoteReceived {
                    quote: record_quote,
                },
                RecordMeta {
                    times: times.clone(),
                    source_id: Some(source_id.clone()),
                    source_clock_domain: Some("UTC".to_owned()),
                    source_sequence: Some(source_sequence),
                    correlation_id: None,
                    causation_id: None,
                    schema_version: 1,
                },
            )?;
        }
        self.match_quote(&quote, times)?;
        Ok(QuoteOutcome::Accepted)
    }

    pub fn submit_decision(
        &mut self,
        decision: DecisionRecord,
        times: EventTimes,
    ) -> Result<SubmissionOutcome, EngineError> {
        self.ensure_writable()?;
        if let Some(intent) = &decision.intent {
            if let Some(existing) = self.projection.intent(intent.intent_id) {
                if existing.decision != decision {
                    return Err(EngineError::IdempotencyConflict);
                }
                return Ok(SubmissionOutcome {
                    risk: existing.risk.clone(),
                    order_id: existing.order_id,
                    deduplicated: true,
                });
            }
            if decision.strategy_id != intent.strategy_id {
                return Err(EngineError::InvalidConfiguration(
                    "decision and intent strategy identities differ".to_owned(),
                ));
            }
            self.validate_evidence_refs(&decision.input_refs, times.available_time)?;
            self.validate_evidence_refs(&intent.evidence, times.available_time)?;
            if decision.input_refs != intent.evidence {
                return Err(EngineError::InvalidEvidenceReference);
            }
        }
        let event = self.record(
            EventPayload::DecisionMade {
                decision: decision.clone(),
            },
            RecordMeta::internal(times.clone()),
        )?;
        let Some(intent) = decision.intent else {
            return Ok(SubmissionOutcome {
                risk: None,
                order_id: None,
                deduplicated: false,
            });
        };
        let risk_context = self.risk_context(&intent, times.available_time)?;
        let assessment = evaluate_risk(&intent, &self.validated_policy, &risk_context);
        self.record(
            EventPayload::RiskAssessed {
                assessment: assessment.clone(),
            },
            RecordMeta {
                times: times.clone(),
                source_id: None,
                source_clock_domain: None,
                source_sequence: None,
                correlation_id: Some(event.body.envelope.event_id.clone()),
                causation_id: Some(event.body.envelope.event_id.clone()),
                schema_version: 1,
            },
        )?;
        if !assessment.accepted {
            return Ok(SubmissionOutcome {
                risk: Some(assessment),
                order_id: None,
                deduplicated: false,
            });
        }
        let order_id = OrderId::new(intent.intent_id.get())?;
        let order = Order {
            order_id,
            intent,
            status: OrderStatus::Working,
            filled_quantity: Quantity::ZERO,
            filled_notional: Paise(0),
            average_fill_price: None,
            fill_count: 0,
        };
        self.record(
            EventPayload::OrderCreated { order },
            RecordMeta {
                times,
                source_id: None,
                source_clock_domain: None,
                source_sequence: None,
                correlation_id: Some(event.body.envelope.event_id.clone()),
                causation_id: Some(event.body.envelope.event_id.clone()),
                schema_version: 1,
            },
        )?;
        Ok(SubmissionOutcome {
            risk: Some(assessment),
            order_id: Some(order_id),
            deduplicated: false,
        })
    }

    pub fn request_cancel(
        &mut self,
        order_id: OrderId,
        reason: impl Into<String>,
        times: EventTimes,
    ) -> Result<CancelOutcome, EngineError> {
        let reason = reason.into();
        let order = self
            .projection
            .order(order_id)
            .ok_or(EngineError::UnknownOrder)?
            .order
            .clone();
        match order.status {
            OrderStatus::Working | OrderStatus::PartiallyFilled => {
                self.record(
                    EventPayload::OrderTransitioned {
                        order_id,
                        from: order.status,
                        to: OrderStatus::CancelPending,
                        reason,
                    },
                    RecordMeta::internal(times),
                )?;
                Ok(CancelOutcome::Pending)
            }
            OrderStatus::CancelPending => Ok(CancelOutcome::Pending),
            OrderStatus::Filled | OrderStatus::Cancelled | OrderStatus::Rejected => {
                Ok(CancelOutcome::AlreadyTerminal)
            }
            OrderStatus::OutcomeUnknown => Err(EngineError::UnknownOutcomeNeedsResolution),
        }
    }

    pub fn acknowledge_cancel(
        &mut self,
        order_id: OrderId,
        times: EventTimes,
    ) -> Result<CancelOutcome, EngineError> {
        let order = self
            .projection
            .order(order_id)
            .ok_or(EngineError::UnknownOrder)?
            .order
            .clone();
        if order.status == OrderStatus::CancelPending {
            self.record(
                EventPayload::OrderTransitioned {
                    order_id,
                    from: OrderStatus::CancelPending,
                    to: OrderStatus::Cancelled,
                    reason: "paper exchange acknowledged cancel".to_owned(),
                },
                RecordMeta::internal(times.clone()),
            )?;
            self.record(
                EventPayload::BrokerReportObserved {
                    order_id,
                    report_kind: "cancel_acknowledgement".to_owned(),
                    applied: true,
                    current_status: OrderStatus::Cancelled,
                },
                RecordMeta::internal(times),
            )?;
            return Ok(CancelOutcome::Cancelled);
        }
        if order.status == OrderStatus::Filled {
            self.record(
                EventPayload::BrokerReportObserved {
                    order_id,
                    report_kind: "late_cancel_acknowledgement".to_owned(),
                    applied: false,
                    current_status: order.status,
                },
                RecordMeta::internal(times),
            )?;
            return Ok(CancelOutcome::LateAfterFill);
        }
        if order.status.terminal() {
            return Ok(CancelOutcome::AlreadyTerminal);
        }
        Err(EngineError::InvalidOrderTransition)
    }

    pub fn mark_outcome_unknown(
        &mut self,
        order_id: OrderId,
        reason: impl Into<String>,
        times: EventTimes,
    ) -> Result<(), EngineError> {
        let current = self
            .projection
            .order(order_id)
            .ok_or(EngineError::UnknownOrder)?
            .order
            .status;
        if !matches!(
            current,
            OrderStatus::Working | OrderStatus::PartiallyFilled | OrderStatus::CancelPending
        ) {
            return Err(EngineError::InvalidOrderTransition);
        }
        self.record(
            EventPayload::OrderTransitioned {
                order_id,
                from: current,
                to: OrderStatus::OutcomeUnknown,
                reason: reason.into(),
            },
            RecordMeta::internal(times),
        )?;
        Ok(())
    }

    /// Resolve an unknown submit only after an authoritative lookup supplies the order's present
    /// state. Fill quantities and trade reports must already be booked from the broker trade tape.
    pub fn resolve_unknown(
        &mut self,
        order_id: OrderId,
        authoritative_status: OrderStatus,
        broker_query_id: impl Into<String>,
        times: EventTimes,
    ) -> Result<(), EngineError> {
        let query_id = broker_query_id.into();
        if query_id.trim().is_empty() {
            return Err(EngineError::InvalidConfiguration(
                "authoritative query identity is required".to_owned(),
            ));
        }
        let current = self
            .projection
            .order(order_id)
            .ok_or(EngineError::UnknownOrder)?
            .order
            .clone();
        if current.status != OrderStatus::OutcomeUnknown {
            return Err(EngineError::InvalidOrderTransition);
        }
        let quantity = current
            .intent
            .quantity
            .checked_sub(current.filled_quantity)?;
        let valid = match authoritative_status {
            OrderStatus::Working => current.filled_quantity.is_zero(),
            OrderStatus::PartiallyFilled => {
                !current.filled_quantity.is_zero() && !quantity.is_zero()
            }
            OrderStatus::Filled => quantity.is_zero(),
            OrderStatus::Cancelled => !quantity.is_zero(),
            OrderStatus::Rejected => current.filled_quantity.is_zero(),
            OrderStatus::CancelPending | OrderStatus::OutcomeUnknown => false,
        };
        if !valid {
            return Err(EngineError::UnknownOutcomeNeedsResolution);
        }
        self.record(
            EventPayload::OrderTransitioned {
                order_id,
                from: OrderStatus::OutcomeUnknown,
                to: authoritative_status,
                reason: format!("authoritative broker query {query_id}"),
            },
            RecordMeta::internal(times.clone()),
        )?;
        self.record(
            EventPayload::BrokerReportObserved {
                order_id,
                report_kind: format!("authoritative_query:{query_id}"),
                applied: true,
                current_status: authoritative_status,
            },
            RecordMeta::internal(times),
        )?;
        Ok(())
    }

    pub fn reconcile(
        &mut self,
        input: ReconcileInput,
        reconciliation_id: impl Into<String>,
        times: EventTimes,
    ) -> Result<ReconciliationReport, EngineError> {
        let reconciliation_id = reconciliation_id.into();
        if reconciliation_id.trim().is_empty() {
            return Err(EngineError::InvalidConfiguration(
                "reconciliation identity is required".to_owned(),
            ));
        }
        let expected_positions = self.projection.position_snapshot();
        let expected_active_orders = self.projection.active_order_ids();
        let observed_positions = input.positions.map(normalize_positions);
        let observed_active_orders = input.active_orders;
        let unresolved_unknown = self
            .projection
            .orders
            .values()
            .any(|view| view.order.status == OrderStatus::OutcomeUnknown);
        let mut differences = Vec::new();
        let status = match (&observed_positions, &observed_active_orders) {
            (Some(positions), Some(orders)) => {
                if positions != &expected_positions {
                    differences.push(format!(
                        "positions differ: expected {expected_positions:?}, observed {positions:?}"
                    ));
                }
                if orders != &expected_active_orders {
                    differences.push(format!(
                        "active orders differ: expected {expected_active_orders:?}, observed {orders:?}"
                    ));
                }
                if unresolved_unknown {
                    differences.push("at least one order has an unresolved outcome".to_owned());
                }
                if differences.is_empty() {
                    ReconciliationStatus::Matched
                } else {
                    ReconciliationStatus::Mismatch
                }
            }
            _ => {
                if observed_positions.is_none() {
                    differences.push("positions snapshot unavailable".to_owned());
                }
                if observed_active_orders.is_none() {
                    differences.push("active-order snapshot unavailable".to_owned());
                }
                ReconciliationStatus::Unavailable
            }
        };
        let report = ReconciliationReport {
            reconciliation_id,
            status,
            expected_positions,
            observed_positions,
            expected_active_orders,
            observed_active_orders,
            differences,
        };
        self.record(
            EventPayload::ReconciliationObserved {
                report: report.clone(),
            },
            RecordMeta::internal(times),
        )?;
        Ok(report)
    }

    #[must_use]
    pub fn account_view(&self) -> Option<crate::AccountView> {
        self.projection.view()
    }

    #[must_use]
    pub fn records(&self) -> &[StoredRecord] {
        self.journal.records()
    }

    #[must_use]
    pub fn verification_report(&self) -> account_journal::VerificationReport {
        self.journal.verification_report()
    }

    #[must_use]
    pub fn stream_id(&self) -> &str {
        self.journal.stream_id()
    }

    #[must_use]
    pub fn order(&self, order_id: OrderId) -> Option<&Order> {
        self.projection.order(order_id).map(|view| &view.order)
    }

    #[must_use]
    pub fn risk_assessment(&self, intent_id: IntentId) -> Option<&RiskAssessment> {
        self.projection
            .intent(intent_id)
            .and_then(|intent| intent.risk.as_ref())
    }

    #[must_use]
    pub fn quote_reference(
        &self,
        instrument_id: &InstrumentId,
        venue: trading_core::Venue,
    ) -> Option<JournalRef> {
        let (sequence, hash_hex) = self
            .projection
            .market
            .source_quote_ref(instrument_id, venue)?;
        Some(JournalRef { sequence, hash_hex })
    }

    /// Signed lots by instrument; zero positions are omitted.
    #[must_use]
    pub fn positions(&self) -> BTreeMap<InstrumentId, i64> {
        self.projection.position_snapshot()
    }

    /// Fills booked so far, keyed by fill identity.
    #[must_use]
    pub fn fills(&self) -> BTreeMap<FillId, Fill> {
        self.projection.fills.clone()
    }

    /// Sequence of the newest journal record, or zero before the first append.
    #[must_use]
    pub fn last_sequence(&self) -> u64 {
        self.projection.last_sequence
    }

    /// Calendar version asserted by the latest session observation.
    #[must_use]
    pub fn calendar_version(&self) -> Option<&str> {
        self.projection.calendar_version.as_deref()
    }

    /// Assemble a read-only evidence capsule for one decision from the journal.
    pub fn decision_capsule(&self, intent_id: IntentId) -> Result<DecisionCapsule, EngineError> {
        DecisionCapsule::build(self.journal.records(), intent_id).map_err(capsule_error)
    }

    pub fn record_time(
        source_time_utc: Option<i64>,
        received_time_utc: i64,
        available_time: u64,
        recorded_time_utc: i64,
    ) -> EventTimes {
        EventTimes {
            source_time_utc: source_time_utc.map(UtcNanos::new),
            received_time_utc: UtcNanos::new(received_time_utc),
            available_time: LogicalNanos::new(available_time),
            recorded_time_utc: UtcNanos::new(recorded_time_utc),
        }
    }

    fn risk_context(
        &mut self,
        intent: &OrderIntent,
        now: LogicalNanos,
    ) -> Result<RiskContext, EngineError> {
        self.projection.trim_order_times(now);
        let instrument = self
            .config
            .instruments
            .at(&intent.instrument_id, self.config.trade_date)
            .cloned();
        let quote = self
            .projection
            .market
            .book(&intent.instrument_id, intent.venue)
            .map(|book| book.quote.clone());
        let feed_health = self.projection.market.health(
            &intent.instrument_id,
            intent.venue,
            now,
            self.config.risk_policy.max_quote_age_ns,
        );
        let gross_exposure_paise = self
            .projection
            .gross_exposure_paise()
            .unwrap_or(Paise(i64::MAX));
        let pending_gross_exposure_paise = self
            .projection
            .pending_order_gross_exposure_paise()
            .unwrap_or(Paise(i64::MAX));
        let (pending_buy_quantity, pending_sell_quantity) = self
            .projection
            .pending_order_quantities(&intent.instrument_id)
            .unwrap_or((Quantity::MAX, Quantity::MAX));
        let session_open = self.projection.trade_date == Some(self.config.trade_date)
            && self
                .projection
                .session_phase
                .is_some_and(SessionPhase::permits_regular_orders);
        Ok(RiskContext {
            account_id: self.config.account_id,
            strategy_id: intent.strategy_id,
            venue: intent.venue,
            instrument,
            quote,
            feed_health,
            trade_date: self.config.trade_date,
            now,
            position_quantity: self.projection.risk_context_position(&intent.instrument_id),
            pending_buy_quantity,
            pending_sell_quantity,
            gross_exposure_paise,
            pending_gross_exposure_paise,
            // The paper engine has one synthetic strategy and no sector,
            // stress, depth, Greeks or borrow master; these controls stay
            // inert here and are exercised with typed fixtures in the
            // risk-engine and risk-authority tests.
            strategy_exposure_paise: Paise(0),
            sector_label: None,
            sector_exposure_paise: Paise(0),
            positions: Default::default(),
            marks_paise: Default::default(),
            depth: None,
            greeks: None,
            is_derivative: false,
            borrow: None,
            daily_pnl: self.projection.daily_pnl,
            open_orders: self.projection.open_order_count(),
            orders_last_second: self.projection.orders_last_second(now),
            session_open,
            account_snapshot_available: self.projection.account_snapshot_available,
            kill_switch_engaged: self.projection.kill_switch_engaged,
            reconciliation_blocked: self.projection.reconciliation_blocked,
            duplicate_intent: false,
        })
    }

    fn validate_evidence_refs(
        &self,
        refs: &[JournalRef],
        decision_time: LogicalNanos,
    ) -> Result<(), EngineError> {
        for reference in refs {
            let index = reference
                .sequence
                .checked_sub(1)
                .and_then(|sequence| usize::try_from(sequence).ok())
                .ok_or(EngineError::InvalidEvidenceReference)?;
            let record = self
                .journal
                .records()
                .get(index)
                .filter(|record| record.sequence == reference.sequence)
                .ok_or(EngineError::InvalidEvidenceReference)?;
            if record.hash.hex() != reference.hash_hex
                || record.body.envelope.times.available_time > decision_time
            {
                return Err(EngineError::InvalidEvidenceReference);
            }
        }
        Ok(())
    }

    fn match_quote(&mut self, quote: &QuoteSnapshot, times: EventTimes) -> Result<(), EngineError> {
        if !matches!(
            self.projection.market.health(
                &quote.instrument_id,
                quote.venue,
                times.available_time,
                self.config.risk_policy.max_quote_age_ns
            ),
            FeedHealth::Healthy { .. }
        ) {
            return Ok(());
        }
        let instrument = self
            .config
            .instruments
            .at(&quote.instrument_id, self.config.trade_date)
            .ok_or_else(|| {
                EngineError::InvalidConfiguration("active instrument metadata is absent".to_owned())
            })?;
        let terms = instrument
            .effective_terms_on(quote.venue, self.config.trade_date)
            .ok_or_else(|| {
                EngineError::InvalidConfiguration(
                    "active instrument trading terms are absent".to_owned(),
                )
            })?;
        // Assigned tick (NSE: fixed for the month by the previous
        // month-end close; uniform across the term interval). Replay
        // reproduces exactly this value in `state.rs`.
        let assigned_tick = Paise(terms.tick_assignment.tick_paise);
        let mut buys: Vec<_> = self
            .projection
            .active_orders_for(&quote.instrument_id, quote.venue)
            .into_iter()
            .filter(|view| {
                matches!(
                    view.order.status,
                    OrderStatus::Working
                        | OrderStatus::PartiallyFilled
                        | OrderStatus::CancelPending
                ) && view.order.intent.side == Side::Buy
            })
            .collect();
        let mut sells: Vec<_> = self
            .projection
            .active_orders_for(&quote.instrument_id, quote.venue)
            .into_iter()
            .filter(|view| {
                matches!(
                    view.order.status,
                    OrderStatus::Working
                        | OrderStatus::PartiallyFilled
                        | OrderStatus::CancelPending
                ) && view.order.intent.side == Side::Sell
            })
            .collect();
        buys.sort_by(|left, right| {
            right
                .order
                .intent
                .limit_price
                .cmp(&left.order.intent.limit_price)
                .then_with(|| left.created_sequence.cmp(&right.created_sequence))
        });
        sells.sort_by(|left, right| {
            left.order
                .intent
                .limit_price
                .cmp(&right.order.intent.limit_price)
                .then_with(|| left.created_sequence.cmp(&right.created_sequence))
        });
        // Matches consume the residual liquidity budget for the displayed
        // quote line, never the snapshot quantity a second time: an
        // unchanged re-quote carries no new liquidity.
        let quote_journal_sequence = self
            .projection
            .market
            .book_journal_sequence(&quote.instrument_id, quote.venue)
            .ok_or_else(|| {
                EngineError::InvalidConfiguration("funding quote identity is absent".to_owned())
            })?;
        let quote_event_id = EventId::for_run(self.config.run_id, quote_journal_sequence);
        if let Some(ask) = quote.ask_price {
            self.match_side(
                &buys,
                self.projection
                    .market
                    .ask_remaining(&quote.instrument_id, quote.venue),
                ask,
                assigned_tick,
                times.clone(),
                quote_event_id.clone(),
            )?;
        }
        if let Some(bid) = quote.bid_price {
            self.match_side(
                &sells,
                self.projection
                    .market
                    .bid_remaining(&quote.instrument_id, quote.venue),
                bid,
                assigned_tick,
                times,
                quote_event_id,
            )?;
        }
        Ok(())
    }

    fn match_side(
        &mut self,
        orders: &[crate::OrderView],
        mut available_quantity: i64,
        displayed_price: PricePaise,
        tick: Paise,
        times: EventTimes,
        quote_event_id: EventId,
    ) -> Result<(), EngineError> {
        for view in orders {
            if available_quantity == 0 {
                break;
            }
            let order = self
                .projection
                .order(view.order.order_id)
                .ok_or(EngineError::UnknownOrder)?
                .order
                .clone();
            let remaining = order.intent.quantity.checked_sub(order.filled_quantity)?;
            if remaining.is_zero() {
                continue;
            }
            let signed_slippage = match order.intent.side {
                Side::Buy => i64::from(self.config.paper_profile.slippage_ticks),
                Side::Sell => -i64::from(self.config.paper_profile.slippage_ticks),
            };
            let fill_price = displayed_price.shifted_ticks(tick, signed_slippage)?;
            let within_limit = match order.intent.side {
                Side::Buy => fill_price <= order.intent.limit_price,
                Side::Sell => fill_price >= order.intent.limit_price,
            };
            if !within_limit {
                continue;
            }
            let fill_quantity = remaining.get().min(available_quantity);
            if fill_quantity == 0 {
                continue;
            }
            let quantity = Quantity::new(fill_quantity)?;
            let turnover = fill_price.notional(quantity)?;
            let charges = self
                .config
                .charge_book
                .calculate(
                    self.config.trade_date,
                    order.intent.product,
                    order.intent.side,
                    turnover,
                )
                .map_err(fee_error)?;
            let total_charges = charges.total()?;
            let cash_delta = match order.intent.side {
                Side::Buy => turnover.checked_add(total_charges)?.checked_neg()?,
                Side::Sell => turnover.checked_sub(total_charges)?,
            };
            let total_filled = order.filled_quantity.checked_add(quantity)?;
            let next_status = if total_filled == order.intent.quantity {
                OrderStatus::Filled
            } else if order.status == OrderStatus::CancelPending {
                OrderStatus::CancelPending
            } else {
                OrderStatus::PartiallyFilled
            };
            let fill_id = FillId::new(self.projection.next_fill_id)?;
            let fill = Fill {
                fill_id,
                order_id: order.order_id,
                side: order.intent.side,
                quantity,
                price: fill_price,
                charges,
                cash_delta,
            };
            self.record(
                EventPayload::FillBooked { fill, next_status },
                RecordMeta::for_fill(times.clone(), quote_event_id.clone()),
            )?;
            available_quantity -= fill_quantity;
        }
        Ok(())
    }

    fn record(
        &mut self,
        payload: EventPayload,
        meta: RecordMeta,
    ) -> Result<StoredRecord, EngineError> {
        self.record_with_quality(payload, meta, BTreeSet::new())
    }

    fn record_with_quality(
        &mut self,
        payload: EventPayload,
        meta: RecordMeta,
        quality: BTreeSet<DataQualityFlag>,
    ) -> Result<StoredRecord, EngineError> {
        self.ensure_writable()?;
        if self
            .projection
            .last_available_time
            .is_some_and(|previous| meta.times.available_time < previous)
        {
            return Err(EngineError::LogicalClockRegression);
        }
        let envelope = EventEnvelope {
            event_id: self.journal.next_event_id(self.config.run_id)?,
            // The journal contract stamps the minimum version the payload
            // requires; a venue-bearing decision, order or quote is written
            // as version 3 even when the transition meta is internal.
            schema_version: meta.schema_version.max(payload.required_schema_version()),
            stream_id: self.config.stream_id.clone(),
            source_id: meta.source_id,
            source_clock_domain: meta.source_clock_domain,
            session_id: Some(self.config.trade_date.to_string()),
            source_sequence: meta.source_sequence,
            correlation_id: meta.correlation_id,
            causation_id: meta.causation_id,
            times: meta.times,
            quality,
        };
        let body = StoredBody { envelope, payload };
        body.validate()?;
        let record = self.journal.append_fenced(&self.fencing, body)?;
        if self.projection.apply(&record).is_err() {
            self.poisoned = true;
            return Err(EngineError::ProjectionAfterCommit);
        }
        Ok(record)
    }

    fn ensure_writable(&self) -> Result<(), EngineError> {
        if self.poisoned {
            Err(EngineError::Poisoned)
        } else {
            Ok(())
        }
    }
}

fn normalize_positions(mut positions: BTreeMap<InstrumentId, i64>) -> BTreeMap<InstrumentId, i64> {
    positions.retain(|_, quantity| *quantity != 0);
    positions
}

fn fee_error(error: FeeError) -> EngineError {
    EngineError::Fees(error.to_string())
}

fn market_error(error: MarketError) -> EngineError {
    EngineError::Market(error.to_string())
}

fn capsule_error(error: CapsuleError) -> EngineError {
    EngineError::Capsule(error.to_string())
}
