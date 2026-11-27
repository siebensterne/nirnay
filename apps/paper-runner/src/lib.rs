#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.8 bounded end-to-end paper/shadow certification harness.
//!
//! Wires the M2.1–M2.7 components into one deterministic pass over a synthetic
//! verified session. This is a systems integration test, not a claim of real
//! source capture, broker interaction, validated alpha or live readiness.

pub mod report;

use std::{
    collections::{
        BTreeMap, //
        BTreeSet,
    },
    io::ErrorKind,
    path::{
        Component, //
        Path,
        PathBuf,
    },
};

use authoritative_oms::{
    BrokerOrderId, //
    ClientOrderId,
    FillEvent,
    OmsOrderState,
    OmsStore,
    OrderView,
    OutboxState,
    PermitRef,
    Projector,
    ReconciliationInput,
    ReconciliationOutcome,
    reconcile,
    store::fill_content_digest,
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
    AuthorityOwnership,
    ControlStore,
    DecisionStore,
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
    RecoveryAuthenticity,
    RecoveryConfig,
    RecoveryJournalRoot,
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
    ReconciliationStatus,
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

/// Inputs to the persistent startup transaction.
///
/// STARTUP INTEGRATION BOUNDARY ONLY: this wires account ownership, the owner
/// recovery loader and authority reconstruction into one fail-closed
/// transaction. It is NOT production authorization or readiness. The
/// application pins an owner-provisioned recovery root and the code-selected
/// authenticity model; it never supplies journal records, market data,
/// provider collection, experiments, holdouts or live execution.
pub struct PersistentStartup {
    /// Owner-controlled directory provisioned outside this process. Every
    /// journal, binding sidecar, terminal anchor and high-water path is derived
    /// from this root by the library loader, never chosen by the application.
    pub recovery_root: PathBuf,
    /// Code-selected authenticity posture (never inferred from file contents).
    pub authenticity: RecoveryAuthenticity,
    /// Account ownership lock acquired before any journal load.
    pub journal_ownership_path: PathBuf,
    /// Reconstructed-state lock consumed by `RiskAuthority::recover`.
    pub authority_ownership_path: PathBuf,
    /// Durable kill-switch and reconciliation store supplied by the owner.
    pub control: Box<dyn ControlStore + Send + Sync>,
    /// Durable decision store supplied by the owner.
    pub decisions: Box<dyn DecisionStore + Send + Sync>,
    /// Effective-dated sector map supplied by the reference-data owner.
    pub sectors: BTreeMap<InstrumentId, String>,
}

/// Explicit readiness of a startup transaction.
///
/// Recovery only reconstructs durable state. It is never production
/// authorization: a recovered authority stays NOT-READY until fresh owner
/// observations and an authenticated control resolution are supplied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupReadiness {
    NotReady,
}

impl StartupReadiness {
    pub const fn label(self) -> &'static str {
        match self {
            Self::NotReady => "not-ready",
        }
    }
}

/// Outcome of the persistent startup transaction. The recovered authority is
/// returned with an explicit NOT-READY status and its reasons; no new risk is
/// enabled by this boundary.
pub struct RecoveredStartup {
    pub startup: PaperStartup,
    pub readiness: StartupReadiness,
    pub not_ready_reasons: Vec<String>,
}

/// Authority under an explicit mode, before any observation is supplied.
pub struct PaperStartup {
    pub mode: PaperMode,
    pub identity: PaperIdentity,
    pub authority: RiskAuthority,
    /// Private guard so the account ownership acquired before recovery is held
    /// for exactly the lifetime of the startup value. Callers cannot fabricate
    /// an owned startup from outside this module.
    _journal_ownership: Option<AuthorityOwnership>,
}

/// Verify that the durable stores are readable before they can influence
/// reconstruction. A non-durable or corrupt store is a NOT-READY failure.
fn verify_durable_stores(
    control: &dyn ControlStore,
    decisions: &dyn DecisionStore,
) -> Result<(), String> {
    if !control.is_durable() || !decisions.is_durable() {
        return Err(
            "NOT-READY: persistent startup requires durable control and decision stores".to_owned(),
        );
    }
    control
        .load_kill_switch()
        .map_err(|error| format!("NOT-READY: control store unreadable: {error}"))?;
    control
        .load_reconciliation()
        .map_err(|error| format!("NOT-READY: control store unreadable: {error}"))?;
    decisions
        .load_decisions()
        .map_err(|error| format!("NOT-READY: decision store unreadable: {error}"))?;
    Ok(())
}

/// The two M4 ownership sentinels derived for one owner-controlled root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnershipPaths {
    pub journal: PathBuf,
    pub authority: PathBuf,
}

/// Derive M4 sentinel paths strictly under the owner-controlled root. The name
/// binds the account identity, the canonical directory identity and the label,
/// so a sentinel cannot be reused for another account or recovery directory.
/// The root must already exist; this function never creates a directory.
pub fn derive_ownership_paths(
    root: &Path,
    account_id: AccountId,
    label: &str,
) -> Result<OwnershipPaths, String> {
    if label.trim().is_empty() {
        return Err("ownership sentinel label must not be empty".to_owned());
    }
    let root = root
        .canonicalize()
        .map_err(|error| format!("owner-controlled recovery root unavailable: {error}"))?;
    if !root.is_dir() {
        return Err("owner-controlled recovery root is not a directory".to_owned());
    }
    let material = format!(
        "paper-ownership-v1|{}|{}|{}",
        account_id.get(),
        root.display(),
        label
    );
    let digest = Sha256::digest(material.as_bytes());
    let prefix: String = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let stem = format!("{prefix}-{}", account_id.get());
    Ok(OwnershipPaths {
        journal: root.join(format!("{stem}-journal.sentinel")),
        authority: root.join(format!("{stem}-authority.sentinel")),
    })
}

/// Check the cheap structural invariants every M4 sentinel path must satisfy:
/// both live under the owner-controlled root, neither walks out of it with
/// `..`, and the two roles use distinct paths.
pub fn validate_ownership_scope(
    root: &Path,
    journal: &Path,
    authority: &Path,
) -> Result<(), String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("owner-controlled recovery root unavailable: {error}"))?;
    for path in [journal, authority] {
        if path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(format!(
                "ownership path {} leaves its directory scope",
                path.display()
            ));
        }
        let anchor = path
            .parent()
            .ok_or_else(|| format!("ownership path {} has no directory anchor", path.display()))?;
        let anchor = anchor.canonicalize().map_err(|error| {
            format!(
                "ownership directory {} unavailable: {error}",
                anchor.display()
            )
        })?;
        if !anchor.starts_with(&root) {
            return Err(format!(
                "ownership path {} is outside the owner-controlled root",
                path.display()
            ));
        }
    }
    if journal == authority {
        return Err("journal and authority sentinels must use distinct paths".to_owned());
    }
    Ok(())
}

/// Strict identity check: the supplied paths must be exactly those derived
/// from the root, account and label. Callers use this before handing paths to
/// `PersistentStartup` so a sentinel cannot be rebound to another identity.
pub fn validate_ownership_paths(
    root: &Path,
    account_id: AccountId,
    label: &str,
    paths: &OwnershipPaths,
) -> Result<(), String> {
    let derived = derive_ownership_paths(root, account_id, label)?;
    validate_ownership_scope(root, &paths.journal, &paths.authority)?;
    if paths.journal != derived.journal || paths.authority != derived.authority {
        return Err(
            "ownership paths are not bound to this account, directory and label".to_owned(),
        );
    }
    Ok(())
}

/// Explicit operator evidence for clearing a stale sentinel after a crash.
/// PIDs, file ages and process liveness are deliberately absent from this
/// contract: they are not proof that no owner is active.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnershipClearEvidence {
    pub operator_id: String,
    pub verification_ref: String,
    pub account_id: AccountId,
    pub recovery_root: PathBuf,
    pub label: String,
}

/// Clear a stale M4 sentinel only after explicit operator verification.
///
/// RUNBOOK: a holder acquired this path with `create_new` and removes it on a
/// clean `Drop`. If the process is terminated while holding it, removal never
/// runs and the next startup fails closed. An operator verifies out of band
/// that no owner is active, supplies a non-trivial verification reference and
/// calls this function; it refuses PID- or age-shaped references and refuses
/// paths not derived from the verified account, directory and label. No other
/// clearance path exists.
pub fn clear_stale_ownership_sentinel(
    path: &Path,
    evidence: OwnershipClearEvidence,
) -> Result<bool, String> {
    if evidence.operator_id.trim().is_empty() {
        return Err("sentinel clearance requires an operator identity".to_owned());
    }
    let reference = evidence.verification_ref.trim();
    if reference.len() < 8
        || !reference
            .chars()
            .any(|character| !character.is_ascii_digit())
    {
        return Err(
            "sentinel clearance requires explicit verification; PID or age is not evidence"
                .to_owned(),
        );
    }
    let derived = derive_ownership_paths(
        &evidence.recovery_root,
        evidence.account_id,
        &evidence.label,
    )?;
    validate_ownership_paths(
        &evidence.recovery_root,
        evidence.account_id,
        &evidence.label,
        &derived,
    )?;
    if path != derived.journal && path != derived.authority {
        return Err("sentinel clearance requires the exact derived ownership path".to_owned());
    }
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "sentinel clearance failed for {}: {error}",
            path.display()
        )),
    }
}

/// The single cohesive persistent startup transaction.
///
/// STARTUP INTEGRATION BOUNDARY ONLY, NOT production authorization or
/// readiness. Order is deliberate and fail-closed: validate the sentinel paths,
/// acquire account ownership, load the owner recovery root, verify durable
/// stores, then reconstruct and reconcile the authority. A failure at any step
/// returns NOT-READY and never constructs an authority or enables new risk.
///
/// M3/M4 LOCK RUNBOOK: the M3 recovery lock is owned by the library loader and
/// by `RiskAuthority::recover`; it exists only for the duration of those calls
/// and is released when they return. The M4 ownership sentinel is the
/// `AuthorityOwnership` acquired below: the journal sentinel is held from
/// before any journal read until the returned startup value is dropped, and
/// the authority sentinel is acquired inside `RiskAuthority::recover` and held
/// for the same lifetime. Acquisition order is therefore M4 journal sentinel,
/// then M3 recovery lock in the loader, then durable-store verification, then
/// M4 authority sentinel. A clean drop removes each sentinel; a process
/// terminated while holding one leaves it behind and the next startup fails
/// closed until an operator clears it with explicit verification (never PID or
/// age) via `clear_stale_ownership_sentinel`.
pub fn startup_persistent(
    account_id: AccountId,
    policy: ValidatedRiskPolicy,
    instruments: Vec<Instrument>,
    trade_date: TradeDate,
    config: PersistentStartup,
) -> Result<RecoveredStartup, String> {
    // 0. M4 sentinels must stay inside the owner-controlled root before any
    // lock is taken. This check cannot itself create or modify a sentinel.
    validate_ownership_scope(
        &config.recovery_root,
        &config.journal_ownership_path,
        &config.authority_ownership_path,
    )
    .map_err(|error| format!("NOT-READY: invalid ownership path: {error}"))?;

    // 1. Account ownership BEFORE any journal is read. A competing owner or a
    // stale lock fails the whole transaction here, before the loader runs.
    let journal_ownership = AuthorityOwnership::acquire(&config.journal_ownership_path)
        .map_err(|error| format!("NOT-READY: account ownership unavailable: {error}"))?;

    // 2. Pin the owner-controlled root. The library loader derives every
    // journal, binding sidecar, terminal anchor and high-water path itself and
    // verifies all of them; no application-supplied record vector is accepted.
    let loader = RecoveryJournalRoot::new(config.recovery_root.clone(), config.authenticity);
    let (oms, account) = loader
        .load(account_id)
        .map_err(|error| format!("NOT-READY: recovery load failed: {error}"))?;

    // 3. Verify durable control and decision stores before reconstruction.
    verify_durable_stores(config.control.as_ref(), config.decisions.as_ref())?;

    // 4. Reconstruct through the library. `recover` re-derives both sealed
    // bindings, replays durable decisions over OMS permits and the account
    // journal, and fails closed on any account/OMS disagreement. Its own
    // authority ownership lock is a separate guard from the pre-load one.
    let authority = RiskAuthority::recover(RecoveryConfig {
        account_id,
        policy,
        instruments,
        trade_date,
        control: config.control,
        decisions: config.decisions,
        oms,
        account,
        sectors: config.sectors,
        ownership_path: config.authority_ownership_path,
    })
    .map_err(|error| format!("NOT-READY: authority recovery failed: {error}"))?;

    if authority.is_simulation() {
        return Err("NOT-READY: recovered authority reports a simulation identity".to_owned());
    }
    if authority.startup_error().is_some() {
        return Err("NOT-READY: account and OMS reconciliation did not complete".to_owned());
    }

    Ok(RecoveredStartup {
        startup: PaperStartup {
            mode: PaperMode::PersistentPaper,
            identity: PaperIdentity::recovered(),
            authority,
            _journal_ownership: Some(journal_ownership),
        },
        readiness: StartupReadiness::NotReady,
        not_ready_reasons: vec![
            "recovery reconstructed durable state; owner observations and an authenticated control resolution are still required".to_owned(),
            "startup never enables new risk; readiness is never inferred from recovered state".to_owned(),
        ],
    })
}

/// The single startup boundary. Ephemeral work gets only the explicit
/// simulation constructor; persistent work goes through the full transaction
/// and still returns a NOT-READY startup.
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
            _journal_ownership: None,
        }),
        PaperMode::PersistentPaper => {
            let config = persistent.ok_or_else(|| {
                "persistent paper mode requires recovery adapters: verified OMS and account journals, a durable decision store and exclusive state ownership"
                    .to_owned()
            })?;
            Ok(startup_persistent(account_id, policy, instruments, trade_date, config)?.startup)
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
    /// Authoritative snapshots supplied to each strategy decision, in order.
    pub strategy_snapshots: Vec<StrategySnapshot>,
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

/// The paper account scope opened by `run_paper`. It is a ledger identity, not
/// a provider, broker or venue identity.
pub const PAPER_ACCOUNT_SCOPE: &str = "paper";

/// Neutral broker execution evidence. The side here is delivered evidence and
/// is *checked*, never trusted: the authoritative OMS order decides the side
/// that is recorded in the ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperExecutionEvidence {
    pub execution_id: String,
    pub broker_event_id: String,
    pub side: Side,
    pub quantity: i64,
    pub price_paise: i64,
    pub now_ns: i64,
}

/// Build the durable execution event from broker evidence and the side of the
/// authoritative OMS order. Callers must pass the OMS side, never a locally
/// inferred or hardcoded side.
#[must_use]
pub fn paper_fill_event(
    client: ClientOrderId,
    evidence: &PaperExecutionEvidence,
    authoritative_side: Side,
) -> FillEvent {
    FillEvent {
        schema_version: 1,
        execution_id: evidence.execution_id.clone(),
        broker_event_id: evidence.broker_event_id.clone(),
        client_order_id: client,
        oms_order_id: client.to_string(),
        side: authoritative_side,
        quantity: evidence.quantity,
        price_paise: evidence.price_paise,
        currency: "INR".to_owned(),
        broker_timestamp_ns: evidence.now_ns,
        received_at_ns: evidence.now_ns,
        source: "paper-simulated-broker".to_owned(),
        account_scope: PAPER_ACCOUNT_SCOPE.to_owned(),
        charges_paise: 0,
        charge_status: authoritative_oms::ChargeStatus::Estimated,
        provenance: "paper-runner synthetic fill".to_owned(),
    }
}

/// Read the position from the account ledger, the only authority for position.
/// There is no parallel local position counter anywhere in the harness.
pub fn ledger_position(projector: &Projector, instrument_id: &InstrumentId) -> Result<i64, String> {
    Ok(projector
        .state()
        .positions
        .get(PAPER_ACCOUNT_SCOPE)
        .and_then(|by_instrument| by_instrument.get(instrument_id))
        .copied()
        .unwrap_or(0))
}

/// Apply one broker execution to the authoritative OMS and then to the account
/// ledger. Fail-closed order of checks:
/// 1. the client order must exist in the OMS;
/// 2. the delivered side must match the OMS order's side (contradictions are
///    rejected before any durable state is touched);
/// 3. an already-seen execution id must carry identical economics, otherwise
///    it is an idempotent no-op;
/// 4. a terminal order cannot accept a late fill without authoritative
///    reconciliation evidence;
/// 5. remaining quantity and the prospective ledger position use checked
///    arithmetic, so over-fill and overflow fail closed;
/// 6. only an accepted OMS fill is projected into the ledger.
///
/// Returns `true` only when a new execution was applied.
pub fn apply_authoritative_fill(
    store: &mut OmsStore,
    projector: &mut Projector,
    instruments: &mut BTreeMap<String, InstrumentId>,
    client: ClientOrderId,
    instrument_id: &InstrumentId,
    evidence: PaperExecutionEvidence,
) -> Result<bool, String> {
    let order = store
        .order(client)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("fill for unknown OMS order {client}"))?;
    let authoritative_side = order.intent.side;
    if evidence.side != authoritative_side {
        return Err(format!(
            "fill side {} contradicts authoritative OMS order side {authoritative_side}",
            evidence.side
        ));
    }
    let candidate = paper_fill_event(client, &evidence, authoritative_side);
    if let Some(stored) = store.fill(&evidence.execution_id) {
        if stored.side != authoritative_side
            || fill_content_digest(stored) != fill_content_digest(&candidate)
        {
            return Err(format!(
                "execution {} reuses an id with contradictory economics",
                evidence.execution_id
            ));
        }
        return Ok(false);
    }
    if order.state.terminal() {
        return Err(format!(
            "fill for terminal OMS order {client} in state {:?}",
            order.state
        ));
    }
    let remaining = order
        .intent
        .quantity
        .get()
        .checked_sub(order.filled_quantity)
        .ok_or("fill accounting overflow: filled quantity exceeds order quantity")?;
    if evidence.quantity <= 0 || evidence.quantity > remaining {
        return Err(format!(
            "fill quantity {} exceeds remaining quantity {remaining}",
            evidence.quantity
        ));
    }
    let signed_quantity = if authoritative_side == Side::Buy {
        evidence.quantity
    } else {
        evidence
            .quantity
            .checked_neg()
            .ok_or("fill accounting overflow: side negation")?
    };
    ledger_position(projector, instrument_id)?
        .checked_add(signed_quantity)
        .ok_or("fill accounting overflow: ledger position")?;

    let applied = store
        .apply_typed_fill(candidate)
        .map_err(|error| error.to_string())?;
    if applied {
        instruments.insert(evidence.execution_id.clone(), instrument_id.clone());
        let stored = store
            .fill(&evidence.execution_id)
            .ok_or("missing persisted fill")?;
        projector
            .project_fill(stored, instrument_id)
            .map_err(|error| error.to_string())?;
    }
    Ok(applied)
}

/// The authoritative snapshot handed to a strategy before every decision.
/// Every field is recomputed from durable OMS/ledger state and the independent
/// reconciliation report; no local flags or historical-id scans are involved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrategySnapshot {
    pub position_quantity: i64,
    pub active_orders: Vec<ClientOrderId>,
    pub partial_fills: Vec<ClientOrderId>,
    pub unknown_orders: Vec<ClientOrderId>,
    pub pending_cancel_replace: Vec<ClientOrderId>,
    pub reconciliation_status: ReconciliationStatus,
    pub has_open_order: bool,
}

/// Build an indexed authoritative view from the OMS's own order/outbox views.
/// The OMS already keeps these records keyed by identity, so this is a single
/// pass over live rows and never a scan of historical client-order ids.
pub fn strategy_snapshot(
    store: &OmsStore,
    projector: &Projector,
    instrument_id: &InstrumentId,
    reconciliation_status: ReconciliationStatus,
) -> Result<StrategySnapshot, String> {
    let position_quantity = ledger_position(projector, instrument_id)?;
    let mut active_orders = BTreeSet::new();
    let mut partial_fills = BTreeSet::new();
    let mut unknown_orders = BTreeSet::new();
    let mut pending_cancel_replace = BTreeSet::new();

    for order in store.orders().map_err(|error| error.to_string())? {
        let OrderView {
            client_order_id,
            state,
            ..
        } = order;
        match state {
            OmsOrderState::PartiallyFilled => {
                active_orders.insert(client_order_id);
                partial_fills.insert(client_order_id);
            }
            OmsOrderState::Unknown | OmsOrderState::Reconciling => {
                active_orders.insert(client_order_id);
                unknown_orders.insert(client_order_id);
            }
            OmsOrderState::CancelPending => {
                active_orders.insert(client_order_id);
                pending_cancel_replace.insert(client_order_id);
            }
            OmsOrderState::Created
            | OmsOrderState::RiskAccepted
            | OmsOrderState::PendingSubmission
            | OmsOrderState::Working
            | OmsOrderState::Resolved => {
                active_orders.insert(client_order_id);
            }
            OmsOrderState::Filled | OmsOrderState::Cancelled | OmsOrderState::Rejected => {}
        }
    }
    for attempt in store.outbox().map_err(|error| error.to_string())? {
        if matches!(
            attempt.state,
            OutboxState::Pending | OutboxState::Claimed | OutboxState::Dispatching
        ) && matches!(attempt.command_type.as_str(), "CANCEL" | "REPLACE")
        {
            active_orders.insert(attempt.client_order_id);
            pending_cancel_replace.insert(attempt.client_order_id);
        }
    }

    let has_open_order = !active_orders.is_empty();
    Ok(StrategySnapshot {
        position_quantity,
        active_orders: active_orders.into_iter().collect(),
        partial_fills: partial_fills.into_iter().collect(),
        unknown_orders: unknown_orders.into_iter().collect(),
        pending_cancel_replace: pending_cancel_replace.into_iter().collect(),
        reconciliation_status,
        has_open_order,
    })
}

/// Project the authoritative snapshot into the narrow strategy input. The
/// strategy can only see these fields, so both must come from the snapshot.
#[must_use]
pub fn strategy_event_from_snapshot(
    observation_sequence: u64,
    instrument_id: String,
    midpoint: Rational,
    best_bid: i64,
    best_ask: i64,
    snapshot: &StrategySnapshot,
) -> StrategyEvent {
    StrategyEvent {
        observation_sequence,
        instrument_id,
        midpoint,
        best_bid,
        best_ask,
        position_quantity: snapshot.position_quantity,
        has_open_order: snapshot.has_open_order,
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
    let mut strategy_snapshots: Vec<StrategySnapshot> = Vec::new();

    for (index, event) in events.iter().enumerate() {
        let (bid, ask) = kernel_top_of_book(event)?;
        let midpoint = Rational::fraction(bid + ask, 2).map_err(|e| e.to_string())?;
        // Reconcile and build the authoritative snapshot before the strategy
        // decides. No local open-order flag or position counter is consulted.
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
        let snapshot = strategy_snapshot(
            &store,
            &projector,
            &instrument_id,
            reconciliation.report.status,
        )
        .map_err(|e| e.to_string())?;
        health.open_orders = u32::try_from(snapshot.active_orders.len()).unwrap_or(u32::MAX);
        health.unknown_orders = u32::try_from(snapshot.unknown_orders.len()).unwrap_or(u32::MAX);
        strategy_snapshots.push(snapshot.clone());
        let strategy_event = strategy_event_from_snapshot(
            event.sequence.unwrap_or((index as u64) + 1),
            input.instrument_id.clone(),
            midpoint,
            bid,
            ask,
            &snapshot,
        );
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
                        let authoritative = store
                            .order(client)
                            .map_err(|e| e.to_string())?
                            .ok_or("filled order disappeared from the OMS")?;
                        let evidence = PaperExecutionEvidence {
                            execution_id,
                            broker_event_id,
                            side: authoritative.intent.side,
                            quantity,
                            price_paise,
                            now_ns: ordinal as i64 + 5,
                        };
                        // The fill side is the OMS order's side, never a
                        // hardcoded default. The fill helper re-checks it and
                        // lets the ledger remain the sole position authority.
                        let fill = paper_fill_event(client, &evidence, authoritative.intent.side);
                        deliveries.push(fill);
                        let applied = apply_authoritative_fill(
                            &mut store,
                            &mut projector,
                            &mut instruments,
                            client,
                            &instrument_id,
                            evidence,
                        )
                        .map_err(|e| e.to_string())?;
                        if applied {
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
        strategy_snapshots,
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
