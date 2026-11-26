#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.8 bounded end-to-end paper/shadow certification harness.
//!
//! Wires the M2.1–M2.7 components into one deterministic pass over a synthetic
//! verified session. This is a systems integration test, not a claim of real
//! source capture, broker interaction, validated alpha or live readiness.

pub mod report;

use std::{
    collections::BTreeMap, //
    path::Path,
};

use authoritative_oms::{
    BrokerOrderId, //
    ClientOrderId,
    FillEvent,
    OmsStore,
    PermitRef,
    Projector,
    ReconciliationInput,
    ReconciliationOutcome,
    reconcile,
};
use broker_adapter::{
    BrokerAdapter, //
    BrokerEvent,
    BrokerOrderRequest, //
    SimulatedBroker,
    SimulatorConfig, //
    SubmissionOutcome,
    SubmitBehavior,
};
use market_event::{
    KernelEvent, //
    MarketByPriceSnapshot,
    MarketDataEvent,
    MbpLevel,
    OriginClass, //
    SequenceScope,
    StreamState,
    capture_frame_id,
};
use market_kernel::{
    KernelSession, //
    SourceObservation,
    project_session,
};
use ops_runtime::{
    HealthSnapshot, //
    RuntimeThresholds,
};
use report::{
    DecisionRecord, //
    PaperReport,
};
use risk_authority::{
    AccountJournalObservation, //
    FeedObservation,
    MarketObservation,
    Observation,
    ObservationSource,
    Observations,
    OmsObservation,
    OperatorObservation,
    OpsObservation,
    OwnerTokens,
    ReconciliationObservation,
    RecoveryConfig,
    RiskAuthority,
    SequenceGap,
};
use risk_engine::{
    RiskPolicy, //
    ValidatedRiskPolicy,
};
use serde::{
    Deserialize, //
    Serialize,
};
use sha2::{
    Digest, //
    Sha256,
};
use strategy_runtime::{
    FrozenArtifact, //
    Rational,
    RationalEmaCross, //
    RuntimeIntent,
    SUPPORTED_RULE,
    Strategy, //
    StrategyEvent,
};
use trading_core::{
    AccountId, //
    AlgoId,
    Continuity,
    Instrument,
    InstrumentId,
    InstrumentType,
    IntentId,
    JournalRef,
    LiquidityDepth,
    Listing,
    ListingStatus,
    ListingTerms,
    LogicalNanos,
    OpeningBalanceEvidence,
    OrderIntent,
    Paise,
    PricePaise,
    Product,
    Quantity,
    QuoteSnapshot,
    ReconciliationReport,
    Segment,
    SettlementRegime,
    Side,
    StrategyId,
    TickAssignment,
    TradeDate,
    Venue,
};

/// Serializable input for one paper run. All market data is synthetic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperInput {
    pub schema_version: u32,
    pub instrument_id: String,
    pub account_id: u64,
    pub strategy_id: u64,
    pub algo_id: u64,
    pub trade_date: TradeDate,
    pub policy: RiskPolicy,
    pub fast: i64,
    pub slow: i64,
    pub quantity: i64,
    pub min_observations: u32,
    pub aggressive_limit_bps: i64,
    /// Each entry is a synthetic (bid, ask) in paise and a receive age in ns.
    pub quotes: Vec<(i64, i64, u64)>,
    pub feed_age_ns: u64,
    pub disk_available_bytes: u64,
    /// Scripted broker behaviour: `ack`, `timeout` or `fill`.
    pub broker_behavior: String,
}

/// How the harness was started. This is chosen by the caller, never inferred
/// from the presence of a control file or from synthetic state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperMode {
    /// In-memory fixture: no durable state, no recovery claim, no live routing.
    EphemeralSimulation,
    /// Durable paper/shadow pass built exclusively from recovered owner state.
    PersistentPaper,
}

impl PaperMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::EphemeralSimulation => "ephemeral-simulation",
            Self::PersistentPaper => "persistent-paper",
        }
    }

    pub const fn is_simulation(self) -> bool {
        matches!(self, Self::EphemeralSimulation)
    }
}

/// Explicit, non-production identity attached to a simulation startup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperIdentity {
    pub mode: PaperMode,
    pub label: String,
}

impl PaperIdentity {
    fn simulation() -> Self {
        Self {
            mode: PaperMode::EphemeralSimulation,
            label: format!("simulation:{}", PaperMode::EphemeralSimulation.label()),
        }
    }

    fn recovered() -> Self {
        Self {
            mode: PaperMode::PersistentPaper,
            label: format!("recovered:{}", PaperMode::PersistentPaper.label()),
        }
    }
}

/// The recovery adapters persistent startup requires. `None` is never a
/// fallback: it means the caller has no verified OMS/account journals and no
/// durable decision store, so persistent startup must fail closed.
pub struct PersistentStartup {
    pub recovery: RecoveryConfig,
}

/// Authority under an explicit mode, before any observation is supplied.
pub struct PaperStartup {
    pub mode: PaperMode,
    pub identity: PaperIdentity,
    pub authority: RiskAuthority,
}

/// The single startup boundary. Ephemeral work gets only the explicit
/// simulation constructor; persistent work gets only a recovered authority
/// with real OMS/account adapters and a durable decision store.
pub fn startup(
    mode: PaperMode,
    account_id: AccountId,
    policy: ValidatedRiskPolicy,
    instruments: Vec<Instrument>,
    trade_date: TradeDate,
    persistent: Option<PersistentStartup>,
) -> Result<PaperStartup, String> {
    match mode {
        PaperMode::EphemeralSimulation => Ok(PaperStartup {
            mode,
            identity: PaperIdentity::simulation(),
            authority: RiskAuthority::new_simulation(account_id, policy, instruments, trade_date),
        }),
        PaperMode::PersistentPaper => {
            let adapters = persistent.ok_or_else(|| {
                "persistent paper mode requires recovery adapters: verified OMS and account journals, a durable decision store and exclusive state ownership"
                    .to_owned()
            })?;
            if !adapters.recovery.control.is_durable() || !adapters.recovery.decisions.is_durable()
            {
                return Err(
                    "persistent paper mode requires durable control and decision stores".to_owned(),
                );
            }
            let authority = RiskAuthority::recover(adapters.recovery).map_err(|e| e.to_string())?;
            if authority.is_simulation() {
                return Err("recovered authority reported a simulation identity".to_owned());
            }
            Ok(PaperStartup {
                mode,
                identity: PaperIdentity::recovered(),
                authority,
            })
        }
    }
}

/// Translate an authority verdict into report fields. A simulation verdict is
/// fixture evidence only: it must never be reported as production-style
/// "safe for new risk".
fn readiness_for_mode(mode: PaperMode, verdict: Result<(), String>) -> (bool, Vec<String>) {
    let mut reasons = Vec::new();
    if mode.is_simulation() {
        reasons.push(format!(
            "{}: simulation only; no production readiness or recovery claim",
            mode.label()
        ));
    }
    reasons.extend(verdict.err());
    (!mode.is_simulation() && reasons.is_empty(), reasons)
}

/// Fixture-only control release for an ephemeral simulation. Never used to
/// authorize a persistent authority: the tokens are process-local and this
/// release is labelled as synthetic, not as an operator decision.
fn release_simulation_controls(
    authority: &RiskAuthority,
    tokens: OwnerTokens,
    identity: &PaperIdentity,
) -> Result<(), String> {
    if !authority.is_simulation() {
        return Err("synthetic control release requires simulation mode".to_owned());
    }
    let fixture = format!("{}:fixture-only", identity.label);
    authority
        .set_kill_switch(
            tokens.operator,
            false,
            fixture.clone(),
            format!("{fixture}: synthetic control release, not owner authorization"),
            Some(format!("{fixture}:synthetic-release")),
        )
        .map_err(|e| e.to_string())?;
    authority
        .set_reconciliation_blocked(
            tokens.reconciliation,
            false,
            format!("{fixture}: synthetic reconciliation resolution"),
            Some(format!("{fixture}:synthetic-resolution")),
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Outcome of one pipeline pass, including the report and the OMS store so
/// tests can inspect persisted lifecycle state.
pub struct PaperRun {
    pub report: PaperReport,
    pub oms: OmsStore,
    pub ledger: Projector,
    pub reconciliation: ReconciliationOutcome,
    pub mode: PaperMode,
    pub identity: PaperIdentity,
}

fn instrument(input: &PaperInput) -> Result<Instrument, String> {
    let from = TradeDate::new(2026, 1, 1).map_err(|e| e.to_string())?;
    let to = TradeDate::new(2026, 12, 31).map_err(|e| e.to_string())?;
    Ok(Instrument {
        instrument_id: InstrumentId::new(input.instrument_id.clone()).map_err(|e| e.to_string())?,
        instrument_type: InstrumentType::Index,
        isin: None,
        name: "Synthetic paper index".to_owned(),
        underlying_instrument_id: None,
        expiry_date: None,
        strike_price_paise: None,
        option_kind: None,
        listings: vec![Listing {
            venue: Venue::Nse,
            segment: Segment::Cash,
            venue_symbol: "PAPER".to_owned(),
            token: None,
            effective_from: from,
            effective_to: Some(to),
            status: ListingStatus::Active,
            settlement_regime: SettlementRegime::Standard,
            z_category: false,
            settlement_group: None,
            terms: vec![ListingTerms {
                effective_from: from,
                effective_to: Some(to),
                tick_assignment: TickAssignment {
                    tick_paise: 1,
                    reference_close_paise: None,
                    reference_close_date: None,
                    reference_source: None,
                    reference_sha256_hex: None,
                    effective_month: None,
                    schedule_revision: "paper-fixture".to_owned(),
                    schedule_effective_from: None,
                    schedule_sha256_hex: None,
                    calendar_revision: None,
                    calendar_sha256_hex: None,
                },
                market_lot: 1,
                quantity_increment: 1,
            }],
        }],
        corporate_actions: Vec::new(),
    })
}

fn synthetic_session(input: &PaperInput) -> Result<KernelSession, String> {
    let mut observations = Vec::new();
    for (index, (bid, ask, age_ns)) in input.quotes.iter().enumerate() {
        let payload = format!("paper:{bid}:{ask}:{index}");
        let bytes = payload.as_bytes();
        let frame = capture_frame_id(bytes).map_err(|e| e.to_string())?;
        let sequence = (index as u64) + 1;
        let receive_utc_ns = 1_700_000_000_000_000_000_i64 + sequence as i64;
        let level = |price: i64| MbpLevel {
            price_paise: price,
            quantity: 1,
            order_count: Some(1),
        };
        let book = MarketByPriceSnapshot {
            instrument_id: input.instrument_id.clone(),
            venue: "NSE".to_owned(),
            source_id: "upstox".to_owned(),
            product: "full".to_owned(),
            depth_levels: 1,
            bids: vec![level(*bid)],
            asks: vec![level(*ask)],
            source_time_ns: Some(1_699_999_999_000_000_000 + sequence as i64),
            last_trade_time_ns: None,
            received_utc_ns: receive_utc_ns,
        };
        book.validate().map_err(|error| error.to_string())?;
        observations.push(SourceObservation {
            raw_frame_hash: frame.clone(),
            source_id: "upstox".to_owned(),
            source_session_id: "paper-session".to_owned(),
            provider_instrument_id: input.instrument_id.clone(),
            canonical_payload: bytes.to_vec(),
            origin_class: OriginClass::SdkCallback,
            receive_utc_ns,
            receive_monotonic_ns: sequence + *age_ns,
            source_time_ns: Some(1_699_999_999_000_000_000 + sequence as i64),
            source_clock_domain: Some("provider_feed_clock".to_owned()),
            channel_id: Some("paper-channel".to_owned()),
            connection_epoch: 1,
            receive_sequence: sequence,
            sequence: Some(sequence),
            sequence_scope: SequenceScope::Connection,
            reference_revision: Some("paper-revision".to_owned()),
            instrument_id: Some(input.instrument_id.clone()),
            evidence_token: None,
            payload: Some(MarketDataEvent::MarketByPriceSnapshot(book)),
        });
    }
    Ok(KernelSession {
        schema_version: market_kernel::KERNEL_SESSION_SCHEMA_VERSION,
        source_scope: "NSE".to_owned(),
        session_reference_revision: Some("paper-revision".to_owned()),
        observations,
    })
}

fn frozen_artifact(input: &PaperInput) -> FrozenArtifact {
    FrozenArtifact {
        candidate_id: "paper.rational-ema.v1".to_owned(),
        hypothesis: "Synthetic systems fixture for end-to-end paper wiring".to_owned(),
        return_source: "momentum".to_owned(),
        universe: vec![input.instrument_id.clone()],
        entry_rule: SUPPORTED_RULE.to_owned(),
        exit_rule: "opposite_cross".to_owned(),
        rebalance_frequency: Some("intraday".to_owned()),
        experiment_ids: vec!["paper-exp-0001".to_owned()],
        validation: strategy_runtime::ArtifactValidation {
            walk_forward_windows: 2,
            holdout_locked: true,
            multiple_testing_correction: "bonferroni".to_owned(),
            deflated_sharpe: "0.000000".to_owned(),
            cost_model: "paper-fixture-no-costs".to_owned(),
            probability_of_backtest_overfit: Some("1.000000".to_owned()),
        },
        status: strategy_runtime::ArtifactStatus::ApprovedForPaper,
        artifact_sha256: String::new(),
    }
}

fn order_intent(
    input: &PaperInput,
    intent: &RuntimeIntent,
    observation_id_text: &str,
    ordinal: u64,
) -> Result<OrderIntent, String> {
    let side = match intent.side.as_str() {
        "buy" => Side::Buy,
        "sell" => Side::Sell,
        other => return Err(format!("unsupported side `{other}`")),
    };
    Ok(OrderIntent {
        intent_id: IntentId::new(ordinal).map_err(|e| e.to_string())?,
        account_id: AccountId::new(input.account_id).map_err(|e| e.to_string())?,
        strategy_id: StrategyId::new(input.strategy_id).map_err(|e| e.to_string())?,
        strategy_version: "paper-strategy-v1".to_owned(),
        algo_id: AlgoId::new(input.algo_id).map_err(|e| e.to_string())?,
        instrument_id: InstrumentId::new(input.instrument_id.clone()).map_err(|e| e.to_string())?,
        venue: Venue::Nse,
        side,
        product: Product::Intraday,
        quantity: Quantity::new(intent.quantity).map_err(|e| e.to_string())?,
        limit_price: PricePaise::new(Paise(intent.limit_price_paise)).map_err(|e| e.to_string())?,
        reason: format!(
            "paper intent {} from {observation_id_text}",
            intent.intent_id
        ),
        evidence: vec![JournalRef {
            sequence: ordinal,
            hash_hex: observation_id_text.to_owned(),
        }],
        expires_at: LogicalNanos::new(1_000_000),
    })
}

fn thresholds() -> RuntimeThresholds {
    RuntimeThresholds {
        max_feed_age_ns: 1_500_000_000,
        max_journal_append_latency_ns: 10_000_000,
        max_journal_queue_depth: 100,
        min_disk_available_bytes: 1_000_000,
        max_unknown_orders: 0,
        max_reconciliation_differences: 0,
        max_sequence_gaps: 0,
    }
}

fn observation_policy() -> risk_authority::ObservationPolicy {
    let limits = thresholds();
    risk_authority::ObservationPolicy {
        max_market_age_ns: limits.max_feed_age_ns,
        max_oms_age_ns: limits.max_feed_age_ns,
        max_account_journal_age_ns: limits.max_feed_age_ns,
        max_reconciliation_age_ns: limits.max_feed_age_ns,
        max_ops_age_ns: limits.max_feed_age_ns,
        max_operator_age_ns: limits.max_feed_age_ns,
        max_sequence_gaps: limits.max_sequence_gaps,
        max_journal_append_latency_ns: limits.max_journal_append_latency_ns,
        max_journal_queue_depth: limits.max_journal_queue_depth,
        min_disk_available_bytes: limits.min_disk_available_bytes,
        max_unknown_orders: limits.max_unknown_orders,
        max_reconciliation_differences: limits.max_reconciliation_differences,
    }
}

fn paper_fill_event(
    client: ClientOrderId,
    execution_id: &str,
    broker_event_id: &str,
    quantity: i64,
    price_paise: i64,
    now_ns: i64,
) -> FillEvent {
    FillEvent {
        schema_version: 1,
        execution_id: execution_id.to_owned(),
        broker_event_id: broker_event_id.to_owned(),
        client_order_id: client,
        oms_order_id: client.to_string(),
        side: Side::Buy,
        quantity,
        price_paise,
        currency: "INR".to_owned(),
        broker_timestamp_ns: now_ns,
        received_at_ns: now_ns,
        source: "paper-simulated-broker".to_owned(),
        account_scope: "paper".to_owned(),
        charges_paise: 0,
        charge_status: authoritative_oms::ChargeStatus::Estimated,
        provenance: "paper-runner synthetic fill".to_owned(),
    }
}

fn fills_digest(store: &OmsStore) -> String {
    let encoded = serde_json::to_vec(&store.fills()).unwrap_or_default();
    Sha256::digest(&encoded)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Build all six owner observations from the synthetic paper sources. Every
/// market and operator field is explicitly labelled synthetic; no verdict is
/// supplied by the caller.
#[allow(clippy::too_many_arguments)] // six owners plus synthetic fixture inputs
fn build_observations(
    feed: FeedObservation,
    store: &OmsStore,
    projector: &Projector,
    health: &HealthSnapshot,
    report: ReconciliationReport,
    replay_matched: bool,
    kill_switch_engaged: bool,
    source_sequence: Option<u64>,
    tokens: OwnerTokens,
) -> Observations {
    // Observation envelopes use the origin of the logical clock; the authority
    // clock is advanced by the measured feed age, so freshness is a real check.
    let at = LogicalNanos::ZERO;
    let verification = projector.verification_report();
    let synthetic_depth = feed.quote.as_ref().map(|quote| LiquidityDepth {
        displayed_quantity: quote.ask_quantity,
        adv_quantity: quote.ask_quantity,
    });
    Observations {
        market: Some(Observation::new(
            tokens.market,
            ObservationSource::MarketDataKernel,
            at,
            at,
            1,
            MarketObservation {
                feed,
                session_open: true,
                kernel_schema_version: market_kernel::KERNEL_SESSION_SCHEMA_VERSION,
                source_sequence,
                connection_epoch: Some(1),
                // Synthetic placeholders only: no real depth, marks, greeks or
                // borrow master is available in the paper harness.
                depth: synthetic_depth,
                marks_paise: BTreeMap::new(),
                greeks: None,
                borrow: None,
                synthetic: true,
            },
        )),
        oms: Some(Observation::new(
            tokens.oms,
            ObservationSource::Oms,
            at,
            at,
            1,
            OmsObservation {
                fills_digest: fills_digest(store),
                unresolved_count: u32::try_from(
                    store.unresolved().map_or(0, |orders| orders.len()),
                )
                .unwrap_or(u32::MAX),
                last_journal_digest: store.state_digest().to_owned(),
                is_poisoned: store.is_poisoned(),
            },
        )),
        account_journal: Some(Observation::new(
            tokens.account_journal,
            ObservationSource::AccountJournal,
            at,
            at,
            1,
            AccountJournalObservation {
                head_hash_hex: verification.head_hash_hex,
                record_count: verification.record_count,
                ledger_state_digest: projector.state().state_digest(),
                opening_balance_evidence_ref: Some("paper-synthetic-statement".to_owned()),
            },
        )),
        reconciliation: Some(Observation::new(
            tokens.reconciliation,
            ObservationSource::Reconciliation,
            at,
            at,
            1,
            ReconciliationObservation {
                report,
                replay_digest_matched: replay_matched,
            },
        )),
        ops: Some(Observation::new(
            tokens.ops,
            ObservationSource::Ops,
            at,
            at,
            1,
            OpsObservation {
                feed_age_ns: health.feed_age_ns,
                sequence_gaps: health.sequence_gaps,
                journal_append_latency_ns: health.journal_append_latency_ns,
                journal_queue_depth: health.journal_queue_depth,
                disk_available_bytes: health.disk_available_bytes,
                unknown_orders: health.unknown_orders,
                reconciliation_differences: health.reconciliation_differences,
            },
        )),
        operator: Some(Observation::new(
            tokens.operator,
            ObservationSource::OperatorControl,
            at,
            at,
            1,
            OperatorObservation {
                kill_switch_engaged,
                actor_id: "paper-synthetic-operator".to_owned(),
                reason: "synthetic bounded paper run fixture".to_owned(),
                release_evidence_ref: Some("paper-synthetic-release".to_owned()),
            },
        )),
    }
}

/// Derive the top of book from the kernel's typed payload. No parallel quote
/// collection is used by the strategy or the risk authority.
fn kernel_top_of_book(event: &KernelEvent) -> Result<(i64, i64), String> {
    let Some(MarketDataEvent::MarketByPriceSnapshot(book)) = &event.payload else {
        return Err("kernel event carries no market-by-price snapshot".to_owned());
    };
    let bid = book.best_bid_paise().ok_or("kernel book has no bid")?;
    let ask = book.best_ask_paise().ok_or("kernel book has no ask")?;
    Ok((bid, ask))
}

fn kernel_quote(bid: i64, ask: i64, instrument_id: &str) -> Result<QuoteSnapshot, String> {
    Ok(QuoteSnapshot {
        instrument_id: InstrumentId::new(instrument_id).map_err(|e| e.to_string())?,
        venue: Venue::Nse,
        bid_price: Some(PricePaise::new(Paise(bid)).map_err(|e| e.to_string())?),
        bid_quantity: Quantity::new(1).map_err(|e| e.to_string())?,
        ask_price: Some(PricePaise::new(Paise(ask)).map_err(|e| e.to_string())?),
        ask_quantity: Quantity::new(1).map_err(|e| e.to_string())?,
        continuity: Continuity::Snapshot,
    })
}

/// Execute one bounded paper pass over a synthetic session. This is always an
/// ephemeral simulation: the authority has no durable control store, decision
/// store or recovery ownership, and the report makes no production claim.
pub fn run_paper(input: &PaperInput, oms_path: &Path) -> Result<PaperRun, String> {
    let session = synthetic_session(input)?;
    let events = project_session(&session).map_err(|e| e.to_string())?;
    let instrument = instrument(input)?;
    let instrument_id =
        InstrumentId::new(input.instrument_id.clone()).map_err(|e| e.to_string())?;
    let account_id = AccountId::new(input.account_id).map_err(|e| e.to_string())?;
    let policy = ValidatedRiskPolicy::from_policy(&input.policy).map_err(|e| e.to_string())?;

    let boot = startup(
        PaperMode::EphemeralSimulation,
        account_id,
        policy,
        vec![instrument],
        input.trade_date,
        None,
    )?;
    let mode = boot.mode;
    let identity = boot.identity;
    let authority = boot.authority.with_observation_policy(observation_policy());
    authority
        .set_now(LogicalNanos::new(input.feed_age_ns))
        .map_err(|e| e.to_string())?;
    let tokens = authority.register_all_owners().map_err(|e| e.to_string())?;
    // Fixture-only release under the explicit simulation identity. This is not
    // an operator authorization and never persists beyond this process.
    release_simulation_controls(&authority, tokens, &identity)?;

    let mut store = OmsStore::open(oms_path).map_err(|e| e.to_string())?;
    let mut projector = Projector::open(oms_path.with_extension("ledger"), account_id)
        .map_err(|e| e.to_string())?;
    projector
        .open_account(
            "paper",
            Paise(1_000_000),
            input.trade_date,
            OpeningBalanceEvidence {
                statement_ref: "paper-synthetic-statement".to_owned(),
                journal_ref: JournalRef {
                    sequence: 1,
                    hash_hex: store.state_digest().to_owned(),
                },
                observed_at_ns: i64::try_from(input.feed_age_ns).unwrap_or(i64::MAX),
            },
        )
        .map_err(|e| e.to_string())?;
    let mut instruments = BTreeMap::new();

    let mut strategy = RationalEmaCross::from_artifact(
        &frozen_artifact(input),
        input.fast,
        input.slow,
        input.quantity,
        input.min_observations,
        input.aggressive_limit_bps,
    )
    .map_err(|e| e.to_string())?;

    let behavior = match input.broker_behavior.as_str() {
        "ack" => SubmitBehavior::ImmediateAck,
        "timeout" => SubmitBehavior::Timeout,
        "fill" => SubmitBehavior::AckThenFills {
            fills: vec![(input.quantity, 1_000)],
        },
        other => return Err(format!("unsupported broker behavior `{other}`")),
    };
    let mut config = SimulatorConfig::new("paper-sim", behavior);
    config.duplicate_events = true;
    let mut broker = SimulatedBroker::new(config).map_err(|e| e.to_string())?;
    broker.authenticate();

    let mut health = HealthSnapshot {
        feed_age_ns: input.feed_age_ns,
        sequence_gaps: events
            .iter()
            .filter(|event| matches!(event.quality, StreamState::Gap | StreamState::Recovering))
            .count() as u32,
        reconnect_count: 0,
        journal_append_latency_ns: 1_000,
        journal_queue_depth: 0,
        disk_available_bytes: input.disk_available_bytes,
        open_orders: 0,
        unknown_orders: 0,
        reconciliation_differences: 0,
        kill_switch_engaged: authority.kill_switch_engaged().map_err(|e| e.to_string())?,
        account_snapshot_available: true,
    };

    let mut decisions = Vec::new();
    let mut deliveries: Vec<FillEvent> = Vec::new();
    let mut intents_emitted = 0_usize;
    let mut reservations = 0_usize;
    let mut ordinal = 0_u64;
    let mut position = 0_i64;

    for (index, event) in events.iter().enumerate() {
        let (bid, ask) = kernel_top_of_book(event)?;
        let midpoint = Rational::fraction(bid + ask, 2).map_err(|e| e.to_string())?;
        let strategy_event = StrategyEvent {
            observation_sequence: event.sequence.unwrap_or((index as u64) + 1),
            instrument_id: input.instrument_id.clone(),
            midpoint,
            best_bid: bid,
            best_ask: ask,
            position_quantity: position,
            has_open_order: false,
        };
        let Some(intent) = strategy
            .on_event(&strategy_event)
            .map_err(|e| e.to_string())?
        else {
            continue;
        };
        intents_emitted += 1;
        ordinal += 1;
        let order = order_intent(input, &intent, event.observation_id.as_str(), ordinal)?;
        let mut record = DecisionRecord {
            intent_id: intent.intent_id.clone(),
            observation_id: event.observation_id.to_string(),
            risk_accepted: false,
            permit_id: None,
            oms_client_id: ordinal,
            oms_final_state: "not_created".to_owned(),
            broker_order_id: None,
            quantity: intent.quantity,
            rejection_reason: None,
        };

        let mut open_orders = 0_u32;
        for id in 1..=ordinal {
            if let Some(order) = store
                .order(ClientOrderId::new(id).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
                && !order.state.terminal()
            {
                open_orders = open_orders.saturating_add(1);
            }
        }
        health.open_orders = open_orders;
        health.unknown_orders =
            u32::try_from(store.unresolved().map_err(|e| e.to_string())?.len()).unwrap_or(u32::MAX);

        let gap = if matches!(
            event.quality,
            StreamState::Gap | StreamState::Recovering | StreamState::Unverified
        ) {
            Some(SequenceGap {
                expected: None,
                observed: event.sequence.unwrap_or(0),
            })
        } else {
            None
        };
        let feed_observation = FeedObservation {
            quote: Some(kernel_quote(bid, ask, input.instrument_id.as_str())?),
            received_at: LogicalNanos::ZERO,
            gap,
            book_complete: true,
        };
        let reconciliation = reconcile(
            &store,
            &projector,
            &ReconciliationInput {
                deliveries: deliveries.clone(),
                instruments: instruments.clone(),
            },
        )
        .map_err(|e| e.to_string())?;
        health.reconciliation_differences = u32::try_from(
            reconciliation
                .differences
                .iter()
                .filter(|difference| difference.is_blocking())
                .count(),
        )
        .unwrap_or(u32::MAX);
        let observations = build_observations(
            feed_observation,
            &store,
            &projector,
            &health,
            reconciliation.report.clone(),
            projector
                .verify_replay()
                .map_err(|e| e.to_string())?
                .matched,
            authority.kill_switch_engaged().map_err(|e| e.to_string())?,
            event.sequence,
            tokens,
        );
        let reservation = match authority.evaluate_and_reserve(order.clone(), observations, None) {
            Ok(value) => value,
            Err(error) => {
                record.rejection_reason = Some(error.to_string());
                decisions.push(record);
                continue;
            }
        };
        record.risk_accepted = true;
        record.permit_id = Some(reservation.permit_id);
        reservations += 1;

        let client = ClientOrderId::new(ordinal).map_err(|e| e.to_string())?;
        let permit = PermitRef {
            permit_id: reservation.permit_id,
            policy_version: reservation.assessment.policy_version.clone(),
            snapshot_revision: reservation.snapshot_revision,
            digest: authority
                .permit_digest(&reservation, &order)
                .map_err(|e| e.to_string())?,
        };
        store
            .create(client, &order, &permit, ordinal as i64)
            .map_err(|e| e.to_string())?;
        store
            .accept_risk_approved(client, ordinal as i64 + 1)
            .map_err(|e| e.to_string())?;
        let claim = store
            .claim("paper-dispatcher", ordinal as i64 + 2, 1_000)
            .map_err(|e| e.to_string())?
            .ok_or("no submission claim")?;
        store
            .mark_dispatching(claim.attempt_id, ordinal as i64 + 3)
            .map_err(|e| e.to_string())?;
        let request = BrokerOrderRequest::from_claim(&claim, &permit, broker.adapter_id())
            .map_err(|e| e.to_string())?;
        match broker.submit(&request).map_err(|e| e.to_string())? {
            SubmissionOutcome::Acknowledged(ack) => {
                let broker_id =
                    BrokerOrderId::new(ack.broker_order_id.clone()).map_err(|e| e.to_string())?;
                store
                    .acknowledge(
                        claim.attempt_id,
                        &broker_id,
                        &ack.broker_event_id,
                        ordinal as i64 + 4,
                    )
                    .map_err(|e| e.to_string())?;
                record.broker_order_id = Some(ack.broker_order_id);
                for broker_event in broker.poll_events().map_err(|e| e.to_string())? {
                    if let BrokerEvent::Filled {
                        execution_id,
                        broker_event_id,
                        quantity,
                        price_paise,
                        ..
                    } = broker_event
                    {
                        let fill = paper_fill_event(
                            client,
                            &execution_id,
                            &broker_event_id,
                            quantity,
                            price_paise,
                            ordinal as i64 + 5,
                        );
                        let applied = store
                            .apply_typed_fill(fill.clone())
                            .map_err(|e| e.to_string())?;
                        deliveries.push(fill);
                        instruments.insert(execution_id.clone(), instrument_id.clone());
                        if applied {
                            position = position.saturating_add(quantity);
                            projector
                                .project_fill(
                                    store.fill(&execution_id).ok_or("missing persisted fill")?,
                                    &instrument_id,
                                )
                                .map_err(|e| e.to_string())?;
                            // Paper-only mark from the same synthetic kernel
                            // quote the strategy consumed; explicit rounding
                            // of the exact doubled-paise midpoint.
                            let midpoint = PricePaise::new(Paise(bid))
                                .and_then(|bid| {
                                    PricePaise::new(Paise(ask)).map(|ask| bid.midpoint_doubled(ask))
                                })
                                .and_then(|doubled| doubled.rounded_paise())
                                .map_err(|e| e.to_string())?;
                            let mark = PricePaise::new(midpoint).map_err(|e| e.to_string())?;
                            let mut marks = BTreeMap::new();
                            marks.insert(
                                "paper".to_owned(),
                                BTreeMap::from([(instrument_id.clone(), mark)]),
                            );
                            projector
                                .mark_to_market(&marks)
                                .map_err(|e| e.to_string())?;
                            authority
                                .apply_fill(order.intent_id)
                                .map_err(|e| e.to_string())?;
                        }
                    }
                }
            }
            SubmissionOutcome::TimedOut { .. } => {
                store
                    .expire_dispatched(ordinal as i64 + 10, 1)
                    .map_err(|e| e.to_string())?;
            }
            SubmissionOutcome::Rejected {
                reason,
                broker_event_id,
                ..
            } => {
                store
                    .reject_submission(
                        claim.attempt_id,
                        &reason,
                        &broker_event_id,
                        ordinal as i64 + 4,
                    )
                    .map_err(|e| e.to_string())?;
                record.rejection_reason = Some(reason);
            }
        }
        let final_state = store
            .order(client)
            .map_err(|e| e.to_string())?
            .map(|order| order.state);
        record.oms_final_state = final_state
            .map(|state| format!("{state:?}"))
            .unwrap_or_else(|| "unknown_order".to_owned());
        decisions.push(record);
    }

    health.unknown_orders =
        u32::try_from(store.unresolved().map_err(|e| e.to_string())?.len()).unwrap_or(u32::MAX);
    let reconciliation = reconcile(
        &store,
        &projector,
        &ReconciliationInput {
            deliveries: deliveries.clone(),
            instruments: instruments.clone(),
        },
    )
    .map_err(|e| e.to_string())?;
    health.reconciliation_differences = u32::try_from(
        reconciliation
            .differences
            .iter()
            .filter(|difference| difference.is_blocking())
            .count(),
    )
    .unwrap_or(u32::MAX);
    let final_observations = build_observations(
        FeedObservation {
            quote: None,
            received_at: LogicalNanos::ZERO,
            gap: None,
            book_complete: true,
        },
        &store,
        &projector,
        &health,
        reconciliation.report.clone(),
        projector
            .verify_replay()
            .map_err(|e| e.to_string())?
            .matched,
        authority.kill_switch_engaged().map_err(|e| e.to_string())?,
        None,
        tokens,
    );
    let final_verdict = authority
        .check_observations(&final_observations)
        .map_err(|e| e.to_string());
    let (final_safe, final_reasons) = readiness_for_mode(mode, final_verdict);
    let input_bytes = serde_json::to_vec(input).map_err(|e| e.to_string())?;
    let input_digest = Sha256::digest(&input_bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let report = PaperReport::new(
        events.len(),
        intents_emitted,
        reservations,
        final_safe,
        final_reasons,
        decisions,
        input_digest,
    );
    Ok(PaperRun {
        report,
        oms: store,
        ledger: projector,
        reconciliation,
        mode,
        identity,
    })
}

#[cfg(all(test, feature = "market-shaped-tests"))]
mod kernel_payload_tests {
    use super::*;

    #[test]
    fn top_of_book_comes_from_the_kernel_payload() {
        let level = |price: i64| MbpLevel {
            price_paise: price,
            quantity: 1,
            order_count: None,
        };
        let book = MarketByPriceSnapshot {
            instrument_id: "nse.cash.paperidx".to_owned(),
            venue: "NSE".to_owned(),
            source_id: "upstox".to_owned(),
            product: "full".to_owned(),
            depth_levels: 1,
            bids: vec![level(9_995)],
            asks: vec![level(10_005)],
            source_time_ns: None,
            last_trade_time_ns: None,
            received_utc_ns: 1,
        };
        let base = KernelEvent::observation_only(
            capture_frame_id(b"frame").expect("frame"),
            market_event::observation_id("upstox", "NSE", "nse.cash.paperidx", b"frame")
                .expect("id"),
            capture_frame_id(b"frame").expect("hash"),
            OriginClass::SdkCallback,
            "upstox",
            "session",
            "nse.cash.paperidx",
            1,
            1,
        )
        .expect("event");
        let mut with_book = base.clone();
        with_book.payload = Some(MarketDataEvent::MarketByPriceSnapshot(book));
        assert_eq!(
            kernel_top_of_book(&with_book).expect("top"),
            (9_995, 10_005),
            "prices come from the kernel payload"
        );
        assert!(
            kernel_top_of_book(&base).is_err(),
            "an event without a typed book fails closed"
        );
    }
}

#[cfg(test)]
mod startup_mode_tests {
    use std::collections::BTreeSet;

    use super::*;

    fn neutral_policy() -> ValidatedRiskPolicy {
        let raw = RiskPolicy {
            version: "paper-startup-neutral".to_owned(),
            allowed_accounts: BTreeSet::from([AccountId::new(1).expect("account")]),
            allowed_strategies: BTreeSet::from([StrategyId::new(2).expect("strategy")]),
            allowed_algos: BTreeSet::from([AlgoId::new(3).expect("algo")]),
            allowed_instruments: BTreeSet::from([InstrumentId::new("paper.neutral").expect("id")]),
            max_order_quantity: Quantity::new(100).expect("quantity"),
            max_order_notional_paise: Paise(1_000_000),
            max_position_quantity: Quantity::new(100).expect("quantity"),
            max_gross_exposure_paise: Paise(1_000_000),
            max_daily_loss_paise: Paise(5_000),
            max_open_orders: 100,
            max_orders_per_second: 100,
            max_price_deviation_ppm: 100_000,
            max_quote_age_ns: 1_000,
            allow_short: false,
            max_strategy_exposure_paise: Paise(1_000_000),
            max_sector_exposure_paise: None,
            stress_scenario: None,
            max_stress_loss_paise: None,
            max_participation_ppm: None,
            greek_limits: None,
            max_borrow_quantity: None,
            max_financing_cost_paise: None,
        };
        ValidatedRiskPolicy::from_policy(&raw).expect("neutral policy")
    }

    fn neutral_inputs() -> (AccountId, ValidatedRiskPolicy, Vec<Instrument>, TradeDate) {
        (
            AccountId::new(1).expect("account"),
            neutral_policy(),
            Vec::new(),
            TradeDate::new(2026, 1, 1).expect("date"),
        )
    }

    #[test]
    fn simulation_startup_has_explicit_simulation_identity() {
        let (account, policy, instruments, date) = neutral_inputs();
        let startup = startup(
            PaperMode::EphemeralSimulation,
            account,
            policy,
            instruments,
            date,
            None,
        )
        .expect("simulation startup");
        assert_eq!(
            startup.mode,
            PaperMode::EphemeralSimulation,
            "startup selects the explicit simulation mode"
        );
        assert!(
            startup.mode.is_simulation(),
            "the mode reports itself as simulation"
        );
        assert!(
            startup.identity.label.contains("simulation"),
            "identity is visibly non-production"
        );
        assert!(
            startup.authority.is_simulation(),
            "only the simulation constructor builds the ephemeral authority"
        );

        let source = include_str!("lib.rs");
        let production_constructor = concat!("RiskAuthority::", "new_with_control_store(");
        assert!(
            !source.contains(production_constructor),
            "the ephemeral path must not use the control-store constructor"
        );
    }

    #[test]
    fn persistent_startup_refuses_missing_recovery_adapters() {
        let (account, policy, instruments, date) = neutral_inputs();
        let error = match startup(
            PaperMode::PersistentPaper,
            account,
            policy,
            instruments,
            date,
            None,
        ) {
            Ok(_) => panic!("persistent mode must fail closed without adapters"),
            Err(error) => error,
        };
        assert!(
            error.contains("recovery") && error.contains("adapter"),
            "error names the missing recovery adapters: {error}"
        );
    }

    #[test]
    fn simulation_verdict_never_becomes_production_readiness() {
        let (safe, reasons) = readiness_for_mode(PaperMode::EphemeralSimulation, Ok(()));
        assert!(
            !safe,
            "even a clean simulation verdict cannot claim safe-for-new-risk"
        );
        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("simulation") && reason.contains("readiness")),
            "reasons carry the simulation-only caveat: {reasons:?}"
        );
    }

    #[test]
    fn simulation_has_no_external_execution_adapter() {
        let source = include_str!("lib.rs");
        for forbidden in [
            concat!("dyn ", "BrokerAdapter"),
            concat!("Live", "Broker"),
            concat!("Upstox", "Broker"),
            concat!("Fyers", "Broker"),
            concat!("Kite", "Broker"),
            concat!("req", "west"),
        ] {
            assert!(
                !source.contains(forbidden),
                "simulation source must not carry {forbidden}"
            );
        }
        assert!(
            source.contains("SimulatedBroker"),
            "only the in-process simulator may be constructed"
        );
    }

    #[test]
    fn synthetic_control_release_is_not_another_authoritys_authorization() {
        let (account, policy, instruments, date) = neutral_inputs();
        let first = startup(
            PaperMode::EphemeralSimulation,
            account,
            policy.clone(),
            instruments.clone(),
            date,
            None,
        )
        .expect("first simulation");
        let second = startup(
            PaperMode::EphemeralSimulation,
            account,
            policy,
            instruments,
            date,
            None,
        )
        .expect("second simulation");
        let first_tokens = first.authority.register_all_owners().expect("owner tokens");
        let foreign_release = second.authority.set_kill_switch(
            first_tokens.operator,
            false,
            "paper-synthetic-operator".to_owned(),
            "fixture".to_owned(),
            Some("paper-synthetic-release".to_owned()),
        );
        assert!(
            foreign_release.is_err(),
            "a self-issued token from one simulation cannot authorize another authority"
        );

        release_simulation_controls(&first.authority, first_tokens, &first.identity)
            .expect("labelled fixture release");
        let decision = first
            .authority
            .reconciliation_decision()
            .expect("fixture reconciliation decision");
        assert!(
            decision.reason.contains("simulation"),
            "the fixture resolution is labelled, not an owner decision"
        );
        assert!(
            decision
                .evidence_ref
                .as_deref()
                .is_some_and(|reference| reference.contains("simulation")),
            "the fixture evidence reference is labelled as simulation-only"
        );
    }
}
