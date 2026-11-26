//! Stateful risk authority with one atomic authorization operation.
//!
//! The authority owns every fact that `evaluate_risk` consumes: account-global
//! exposure, positions, pending orders, rate state, daily P&L, the logical
//! clock, snapshot availability, kill switch and reconciliation state. A caller
//! supplies only an [`OrderIntent`] and raw observations from registered owner
//! adapters; it cannot assert a `Healthy` flag, an exposure figure, an account
//! snapshot, or forge another owner's facts.
//!
//! Evaluation and reservation happen under a single mutex lock, so two
//! concurrent callers can never both evaluate against the same stale state.
//! On full acceptance the reservation is applied before the lock is released;
//! on any rejection nothing is reserved.
//!
//! `revision` versions every fact used by risk evaluation, not just the clock
//! and exposure. Each transition validates on a cloned candidate, performs
//! every checked computation, then commits the candidate once with exactly one
//! revision bump; a defined no-op commits nothing. The mutex serializes
//! callers, and the candidate is what makes a late failure roll back the
//! whole transition instead of only the step that failed.

use std::{
    collections::{
        BTreeMap, //
        BTreeSet,
        VecDeque,
    },
    hash::{BuildHasher, RandomState},
    sync::{
        Mutex, //
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use risk_engine::{
    RiskContext, //
    ValidatedRiskPolicy,
    evaluate_risk,
};
use serde::{
    Deserialize, //
    Serialize,
};
use trading_core::{
    AccountId, //
    CheckedPaise,
    FeedHealth,
    Instrument,
    InstrumentId,
    IntentId,
    LogicalNanos,
    OrderIntent,
    Paise,
    Quantity,
    RiskAssessment,
    Side,
    StrategyId,
    TradeDate,
    Venue,
    permit_digest,
};

use crate::{
    control::{
        ControlError, //
        ControlStore,
        KillSwitchState,
        NoopControlStore,
    },
    error::{
        AuthorityError, //
        OpsThresholdKind,
    },
    observation::{
        OBSERVATION_SCHEMA_VERSION, //
        Observation,
        ObservationPolicy,
        ObservationSource,
        Observations,
        OmsObservation,
        OpsObservation,
        OwnerToken,
        OwnerTokens,
    },
};

const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// The next revision, or a fail-closed error. A saturated revision would
/// silently reuse a fence already handed to a caller, so exhaustion is never
/// allowed to succeed.
fn next_revision(revision: u64) -> Result<u64, AuthorityError> {
    revision
        .checked_add(1)
        .ok_or_else(|| AuthorityError::AccountingOverflow("revision".to_owned()))
}

static NEXT_OWNER_NONCE: AtomicU64 = AtomicU64::new(1);

/// Issue a process-local, unguessable token. The `RandomState` keys are seeded
/// by the OS per instance, so an in-process caller cannot predict a token it
/// did not receive from `register_owner`.
fn issue_owner_token() -> OwnerToken {
    let hasher = RandomState::new();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let value = hasher.hash_one((
        std::process::id(),
        NEXT_OWNER_NONCE.fetch_add(1, Ordering::Relaxed),
        nanos,
    ));
    OwnerToken::from_value(value)
}

/// A committed reservation: the only output that may proceed toward an OMS.
/// It is a permit, not an order submission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reservation {
    pub permit_id: u64,
    pub intent_id: IntentId,
    pub account_id: AccountId,
    pub strategy_id: StrategyId,
    pub instrument_id: InstrumentId,
    pub venue: Venue,
    pub side: Side,
    pub quantity: Quantity,
    pub reserved_notional_paise: Paise,
    /// Authority state revision against which this permit was granted.
    pub snapshot_revision: u64,
    pub assessment: RiskAssessment,
}

impl Reservation {
    /// A permit must be self-consistent before any downstream consumer acts
    /// on it. Deserialized or fabricated values are not authoritative.
    pub fn validate_invariants(&self) -> Result<(), AuthorityError> {
        if self.permit_id == 0 {
            return Err(AuthorityError::InvalidReservation(
                "zero permit id".to_owned(),
            ));
        }
        if !self.assessment.accepted {
            return Err(AuthorityError::InvalidReservation(
                "assessment was not accepted".to_owned(),
            ));
        }
        if self.assessment.intent_id != self.intent_id {
            return Err(AuthorityError::InvalidReservation(
                "intent id mismatch".to_owned(),
            ));
        }
        if self.reserved_notional_paise.0 <= 0 {
            return Err(AuthorityError::InvalidReservation(
                "non-positive notional".to_owned(),
            ));
        }
        if self.quantity.is_zero() {
            return Err(AuthorityError::InvalidReservation(
                "zero quantity".to_owned(),
            ));
        }
        Ok(())
    }

    /// Deterministic, unkeyed digest binding this permit to the exact order
    /// terms it authorized. The OMS recomputes the same value from its stored
    /// intent and `PermitRef` before accepting an order.
    #[must_use]
    pub fn digest(&self, intent: &OrderIntent) -> String {
        permit_digest(
            self.permit_id,
            &self.assessment.policy_version,
            self.snapshot_revision,
            intent,
        )
    }
}

/// Read-only view of authority state for operating evidence and tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoritySnapshot {
    pub revision: u64,
    pub now: LogicalNanos,
    pub gross_exposure_paise: Paise,
    pub pending_gross_exposure_paise: Paise,
    pub strategy_exposure: BTreeMap<StrategyId, Paise>,
    pub sector_exposure: BTreeMap<String, Paise>,
    pub open_orders: u32,
    pub orders_last_second: u32,
    pub positions: BTreeMap<InstrumentId, i64>,
    pub kill_switch_engaged: bool,
    pub reconciliation_blocked: bool,
    pub account_snapshot_available: bool,
    pub reservation_count: usize,
}

/// All mutable account-global facts. Never constructed from caller claims.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(Serialize))]
struct AuthorityState {
    now: LogicalNanos,
    revision: u64,
    session_open: bool,
    account_snapshot_available: bool,
    kill_switch_engaged: bool,
    reconciliation_blocked: bool,
    daily_pnl: Paise,
    positions: BTreeMap<InstrumentId, i64>,
    pending_buy: BTreeMap<InstrumentId, i64>,
    pending_sell: BTreeMap<InstrumentId, i64>,
    gross_exposure_paise: Paise,
    pending_gross_exposure_paise: Paise,
    strategy_exposure: BTreeMap<StrategyId, Paise>,
    sector_exposure: BTreeMap<String, Paise>,
    open_orders: u32,
    order_times: VecDeque<LogicalNanos>,
    reservations: BTreeMap<IntentId, Reservation>,
    decided: BTreeMap<IntentId, OrderIntent>,
    consumed: BTreeSet<IntentId>,
    next_permit: u64,
    /// Only tokens issued by this authority are accepted for observations and
    /// authenticated control actions.
    owner_tokens: BTreeMap<ObservationSource, OwnerToken>,
}

impl Default for AuthorityState {
    fn default() -> Self {
        Self {
            now: LogicalNanos::ZERO,
            revision: 0,
            // No startup evidence exists: a fresh authority must deny new risk
            // until each readiness fact is supplied by its owner.
            session_open: false,
            account_snapshot_available: false,
            kill_switch_engaged: true,
            reconciliation_blocked: true,
            daily_pnl: Paise(0),
            positions: BTreeMap::new(),
            pending_buy: BTreeMap::new(),
            pending_sell: BTreeMap::new(),
            gross_exposure_paise: Paise(0),
            pending_gross_exposure_paise: Paise(0),
            strategy_exposure: BTreeMap::new(),
            sector_exposure: BTreeMap::new(),
            open_orders: 0,
            order_times: VecDeque::new(),
            reservations: BTreeMap::new(),
            decided: BTreeMap::new(),
            consumed: BTreeSet::new(),
            next_permit: 1,
            owner_tokens: BTreeMap::new(),
        }
    }
}

impl AuthorityState {
    fn orders_last_second(&self) -> u32 {
        let cutoff = self.now.get().saturating_sub(NANOS_PER_SECOND);
        let count = self
            .order_times
            .iter()
            .filter(|at| at.get() >= cutoff)
            .count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    fn pending_quantity(
        quantities: &BTreeMap<InstrumentId, i64>,
        id: &InstrumentId,
    ) -> Result<Quantity, AuthorityError> {
        let value = quantities.get(id).copied().unwrap_or(0);
        Quantity::new(value).map_err(|error| AuthorityError::InvalidIntent(error.to_string()))
    }

    /// Keep only order times inside the rate window so memory stays bounded.
    fn prune_order_times(&mut self) {
        let cutoff = self.now.get().saturating_sub(NANOS_PER_SECOND);
        while self.order_times.front().is_some_and(|at| at.get() < cutoff) {
            self.order_times.pop_front();
        }
    }

    /// Advance a proposed state by exactly one revision. Exhaustion must fail
    /// the whole transition rather than silently reusing an issued fence.
    fn bump_revision(&mut self) -> Result<(), AuthorityError> {
        self.revision = next_revision(self.revision)?;
        Ok(())
    }

    /// Deterministic serialization of every mutable fact. Tests use this to
    /// prove that a failed transition changed nothing, not just the fields
    /// exposed by `AuthoritySnapshot`.
    #[cfg(test)]
    fn digest(&self) -> String {
        serde_json::to_string(self).expect("state is serializable")
    }
}

/// The authorization boundary. Shareable across threads; all state transitions
/// are serialized internally.
pub struct RiskAuthority {
    account_id: AccountId,
    policy: ValidatedRiskPolicy,
    instruments: BTreeMap<InstrumentId, Instrument>,
    /// Synthetic instrument-to-sector mapping. Absent labels deny new risk
    /// once a sector cap is configured in the policy.
    sectors: BTreeMap<InstrumentId, String>,
    trade_date: TradeDate,
    observation_policy: ObservationPolicy,
    control: Box<dyn ControlStore + Send + Sync>,
    state: Mutex<AuthorityState>,
}

impl RiskAuthority {
    /// Build an authority with validated policy and owned effective-dated
    /// reference data. No order, OMS, broker or live dependency. Without a
    /// control store the kill switch stays in memory and defaults engaged.
    pub fn new(
        account_id: AccountId,
        policy: ValidatedRiskPolicy,
        instruments: Vec<Instrument>,
        trade_date: TradeDate,
    ) -> Self {
        Self::new_with_control_store(
            account_id,
            policy,
            instruments,
            trade_date,
            Box::new(NoopControlStore),
        )
    }

    /// Build an authority that loads and persists kill-switch state through
    /// `control`. An absent or unreadable control record fails closed engaged.
    pub fn new_with_control_store(
        account_id: AccountId,
        policy: ValidatedRiskPolicy,
        instruments: Vec<Instrument>,
        trade_date: TradeDate,
        control: Box<dyn ControlStore + Send + Sync>,
    ) -> Self {
        let persisted = control.load_kill_switch().ok().flatten();
        let mut state = AuthorityState::default();
        if let Some(record) = persisted {
            state.kill_switch_engaged = record.enabled;
        }
        Self {
            account_id,
            policy,
            instruments: instruments
                .into_iter()
                .map(|instrument| (instrument.instrument_id.clone(), instrument))
                .collect(),
            sectors: BTreeMap::new(),
            trade_date,
            observation_policy: ObservationPolicy::default(),
            control,
            state: Mutex::new(state),
        }
    }

    /// Install a synthetic instrument-to-sector map. This is a fixture-only
    /// placeholder: no real NSE sector master is imported or claimed.
    #[must_use]
    pub fn with_sectors(mut self, sectors: BTreeMap<InstrumentId, String>) -> Self {
        self.sectors = sectors;
        self
    }

    /// Override authority-owned observation limits. Immutable after build.
    #[must_use]
    pub fn with_observation_policy(mut self, policy: ObservationPolicy) -> Self {
        self.observation_policy = policy;
        self
    }

    /// Expose the deterministic permit-to-intent digest for a committed
    /// reservation. Deserialized or fabricated reservations are rejected by
    /// the same invariant check used before a permit is issued.
    pub fn permit_digest(
        &self,
        reservation: &Reservation,
        intent: &OrderIntent,
    ) -> Result<String, AuthorityError> {
        reservation.validate_invariants()?;
        Ok(reservation.digest(intent))
    }

    /// Whether the kill switch is currently engaged (including persisted
    /// state loaded at construction).
    pub fn kill_switch_engaged(&self) -> Result<bool, AuthorityError> {
        Ok(self.lock()?.kill_switch_engaged)
    }

    /// Register one producer and return its unguessable process-local token.
    /// Re-registering the same source returns the same token.
    pub fn register_owner(&self, source: ObservationSource) -> Result<OwnerToken, AuthorityError> {
        let mut state = self.lock()?;
        Ok(*state
            .owner_tokens
            .entry(source)
            .or_insert_with(issue_owner_token))
    }

    /// Register all six owners in one call for in-process wiring.
    pub fn register_all_owners(&self) -> Result<OwnerTokens, AuthorityError> {
        Ok(OwnerTokens {
            market: self.register_owner(ObservationSource::MarketDataKernel)?,
            oms: self.register_owner(ObservationSource::Oms)?,
            account_journal: self.register_owner(ObservationSource::AccountJournal)?,
            reconciliation: self.register_owner(ObservationSource::Reconciliation)?,
            ops: self.register_owner(ObservationSource::Ops)?,
            operator: self.register_owner(ObservationSource::OperatorControl)?,
        })
    }

    /// Authenticate a control action or observation against the registered
    /// owner of `expected`.
    fn check_owner_binding(
        &self,
        state: &AuthorityState,
        evidence_token: OwnerToken,
        expected: ObservationSource,
    ) -> Result<(), AuthorityError> {
        let registered = state.owner_tokens.get(&expected).copied().ok_or(
            AuthorityError::UnregisteredProducer {
                observation_source: expected,
            },
        )?;
        if evidence_token != registered {
            return Err(AuthorityError::ProducerTokenMismatch {
                observation_source: expected,
            });
        }
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, AuthorityState>, AuthorityError> {
        self.state.lock().map_err(|_| AuthorityError::StatePoisoned)
    }

    /// Current state revision, used as an optimistic-concurrency fence by
    /// callers that hold a previously observed account snapshot.
    pub fn revision(&self) -> Result<u64, AuthorityError> {
        Ok(self.lock()?.revision)
    }

    pub fn set_now(&self, now: LogicalNanos) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        let mut candidate = state.clone();
        candidate.now = now;
        candidate.prune_order_times();
        if candidate.now == state.now && candidate.order_times == state.order_times {
            return Ok(());
        }
        candidate.bump_revision()?;
        *state = candidate;
        Ok(())
    }

    pub fn set_session_open(&self, open: bool) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        if state.session_open == open {
            return Ok(());
        }
        let mut candidate = state.clone();
        candidate.session_open = open;
        candidate.bump_revision()?;
        *state = candidate;
        Ok(())
    }

    pub fn set_snapshot_available(&self, available: bool) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        if state.account_snapshot_available == available {
            return Ok(());
        }
        let mut candidate = state.clone();
        candidate.account_snapshot_available = available;
        candidate.bump_revision()?;
        *state = candidate;
        Ok(())
    }

    /// Authored kill-switch decision from the registered operator owner. The
    /// control store is fsynced before the in-memory flag changes, so a crash
    /// between the two resolves to engaged on replay. Releasing an engaged
    /// switch additionally requires a non-empty `release_evidence_ref`.
    pub fn set_kill_switch(
        &self,
        token: OwnerToken,
        engaged: bool,
        actor_id: String,
        reason: String,
        release_evidence_ref: Option<String>,
    ) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        self.check_owner_binding(&state, token, ObservationSource::OperatorControl)?;
        if !engaged
            && release_evidence_ref
                .as_ref()
                .is_none_or(|reference| reference.trim().is_empty())
        {
            return Err(AuthorityError::MissingEvidence {
                action: "kill_switch_release".to_owned(),
            });
        }
        // The durable record and the in-memory flag share one version. The
        // version is computed with checked arithmetic before the store write,
        // and the state is committed only after the write succeeds, so a
        // control failure changes neither the record nor the authority.
        let revision = next_revision(state.revision)?;
        let record = KillSwitchState {
            enabled: engaged,
            actor_id,
            reason,
            revision,
            at_ns: i64::try_from(state.now.get()).unwrap_or(i64::MAX),
        };
        self.control
            .append_kill_switch(&record)
            .map_err(|error: ControlError| AuthorityError::ControlFailure(error.to_string()))?;
        let mut candidate = state.clone();
        candidate.kill_switch_engaged = engaged;
        candidate.revision = revision;
        *state = candidate;
        Ok(())
    }

    /// Clear the sticky reconciliation latch. Only the registered
    /// reconciliation owner may do so, with a documented reason and evidence.
    pub fn set_reconciliation_blocked(
        &self,
        token: OwnerToken,
        blocked: bool,
        reason: String,
        evidence_ref: Option<String>,
    ) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        self.check_owner_binding(&state, token, ObservationSource::Reconciliation)?;
        if !blocked
            && evidence_ref
                .as_ref()
                .is_none_or(|reference| reference.trim().is_empty())
        {
            return Err(AuthorityError::MissingEvidence {
                action: "reconciliation_resolution".to_owned(),
            });
        }
        // `reason` is the reconciliation owner's authored justification. The
        // authority has no durable field for it: adding one would change the
        // persisted recovery record shape, which is explicitly out of scope
        // for this transition-only change. It is acknowledged here rather
        // than silently discarded; the evidenced, versioned latch is what is
        // committed. A repeated value is a defined no-op with no revision use.
        let _ = reason;
        if state.reconciliation_blocked == blocked {
            return Ok(());
        }
        let mut candidate = state.clone();
        candidate.reconciliation_blocked = blocked;
        candidate.bump_revision()?;
        *state = candidate;
        Ok(())
    }

    pub fn set_daily_pnl(&self, pnl: Paise) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        if state.daily_pnl == pnl {
            return Ok(());
        }
        let mut candidate = state.clone();
        candidate.daily_pnl = pnl;
        candidate.bump_revision()?;
        *state = candidate;
        Ok(())
    }

    /// Validate one observation's identity, schema and freshness against the
    /// authority's own clock. Ownership and all parsing live inside the lock.
    fn validate_observation<T>(
        &self,
        state: &AuthorityState,
        observation: &Observation<T>,
        expected: ObservationSource,
        now: LogicalNanos,
        max_age_ns: u64,
    ) -> Result<(), AuthorityError> {
        self.check_owner_binding(state, observation.producer_token, expected)?;
        if observation.source != expected {
            return Err(AuthorityError::ObservationSourceMismatch {
                observation_source: expected,
            });
        }
        if observation.schema_version != OBSERVATION_SCHEMA_VERSION {
            return Err(AuthorityError::UnsupportedObservationVersion {
                observation_source: expected,
                version: observation.schema_version,
            });
        }
        if observation.observed_at > observation.received_at {
            return Err(AuthorityError::InvalidObservationTime {
                observation_source: expected,
                detail: "observed time is after receipt time".to_owned(),
            });
        }
        if observation.received_at > now {
            return Err(AuthorityError::FutureObservation);
        }
        let age_ns = now.get().saturating_sub(observation.observed_at.get());
        if age_ns > max_age_ns {
            return Err(AuthorityError::StaleObservation {
                observation_source: expected,
                age_ns,
                max_age_ns,
            });
        }
        Ok(())
    }

    /// Public health check for evidence only. It runs exactly the same
    /// in-lock aggregation as a reservation, but never reserves exposure.
    pub fn check_observations(&self, observations: &Observations) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        self.observe(&mut state, observations)
    }

    /// Aggregate observations into a candidate and commit only the four
    /// authority-owned readiness latches. A fail-closed latch persists even
    /// when the health decision denies new risk, so it is committed with its
    /// own revision before the decision is returned. No exposure, permit or
    /// decision fact is written here: those belong to atomic transitions.
    fn observe(
        &self,
        state: &mut AuthorityState,
        observations: &Observations,
    ) -> Result<(), AuthorityError> {
        let mut candidate = state.clone();
        let decision = self.check_observations_locked(&mut candidate, observations);
        let latched = state.session_open != candidate.session_open
            || state.account_snapshot_available != candidate.account_snapshot_available
            || state.kill_switch_engaged != candidate.kill_switch_engaged
            || state.reconciliation_blocked != candidate.reconciliation_blocked;
        if latched {
            candidate.bump_revision()?;
            *state = candidate;
        }
        decision
    }

    /// Validate and aggregate all six owner observations while holding the
    /// authority lock. A missing, stale, wrong-version or mismatched report
    /// denies new risk before any policy evaluation.
    fn check_observations_locked(
        &self,
        state: &mut AuthorityState,
        observations: &Observations,
    ) -> Result<(), AuthorityError> {
        let policy = self.observation_policy;
        let now = state.now;

        let market = observations
            .market
            .as_ref()
            .ok_or(AuthorityError::MissingObservation {
                observation_source: ObservationSource::MarketDataKernel,
            })?;
        self.validate_observation(
            state,
            market,
            ObservationSource::MarketDataKernel,
            now,
            policy.max_market_age_ns,
        )?;
        let oms = observations
            .oms
            .as_ref()
            .ok_or(AuthorityError::MissingObservation {
                observation_source: ObservationSource::Oms,
            })?;
        self.validate_observation(
            state,
            oms,
            ObservationSource::Oms,
            now,
            policy.max_oms_age_ns,
        )?;
        let journal =
            observations
                .account_journal
                .as_ref()
                .ok_or(AuthorityError::MissingObservation {
                    observation_source: ObservationSource::AccountJournal,
                })?;
        self.validate_observation(
            state,
            journal,
            ObservationSource::AccountJournal,
            now,
            policy.max_account_journal_age_ns,
        )?;
        let reconciliation =
            observations
                .reconciliation
                .as_ref()
                .ok_or(AuthorityError::MissingObservation {
                    observation_source: ObservationSource::Reconciliation,
                })?;
        self.validate_observation(
            state,
            reconciliation,
            ObservationSource::Reconciliation,
            now,
            policy.max_reconciliation_age_ns,
        )?;
        let ops = observations
            .ops
            .as_ref()
            .ok_or(AuthorityError::MissingObservation {
                observation_source: ObservationSource::Ops,
            })?;
        self.validate_observation(
            state,
            ops,
            ObservationSource::Ops,
            now,
            policy.max_ops_age_ns,
        )?;
        let operator =
            observations
                .operator
                .as_ref()
                .ok_or(AuthorityError::MissingObservation {
                    observation_source: ObservationSource::OperatorControl,
                })?;
        self.validate_observation(
            state,
            operator,
            ObservationSource::OperatorControl,
            now,
            policy.max_operator_age_ns,
        )?;

        // Aggregate owner facts into authority-owned state, all under the same
        // mutex as the reservation that follows.
        state.session_open = market.payload.session_open;
        // An observation can only engage. Releasing an engaged switch is a
        // separate, authenticated operator action (`set_kill_switch`) so
        // a snapshot replay cannot silently clear persisted control state.
        if operator.payload.kill_switch_engaged {
            state.kill_switch_engaged = true;
        }
        state.account_snapshot_available = journal.payload.record_count > 0
            && journal.payload.opening_balance_evidence_ref.is_some();
        if journal.payload.head_hash_hex.len() != 64 {
            return Err(AuthorityError::ObservationDigestMismatch {
                observation_source: ObservationSource::AccountJournal,
            });
        }
        Self::check_oms_observation(oms)?;
        if journal.payload.ledger_state_digest.len() != 64 {
            return Err(AuthorityError::ObservationDigestMismatch {
                observation_source: ObservationSource::AccountJournal,
            });
        }
        if !reconciliation.payload.replay_digest_matched {
            return Err(AuthorityError::ObservationDigestMismatch {
                observation_source: ObservationSource::Reconciliation,
            });
        }
        let recon_status = reconciliation.payload.report.status;
        // A mismatch or unavailable view latches the block. A later Matched
        // observation never clears it: only an explicit, evidenced operator
        // authenticated reconciliation-owner resolution (`set_reconciliation_blocked`) can do that.
        if recon_status != trading_core::ReconciliationStatus::Matched {
            state.reconciliation_blocked = true;
            return Err(AuthorityError::ReconciliationMismatch {
                report_id: reconciliation.payload.report.reconciliation_id.clone(),
                differences: reconciliation.payload.report.differences.clone(),
            });
        }
        if state.kill_switch_engaged {
            return Err(AuthorityError::KillSwitchEngaged {
                reason: operator.payload.reason.clone(),
                actor_id: operator.payload.actor_id.clone(),
            });
        }
        Self::check_ops_observation(ops, &policy)?;
        Ok(())
    }

    fn check_oms_observation(oms: &Observation<OmsObservation>) -> Result<(), AuthorityError> {
        if oms.payload.is_poisoned
            || oms.payload.fills_digest.is_empty()
            || oms.payload.last_journal_digest.len() != 64
        {
            return Err(AuthorityError::ObservationDigestMismatch {
                observation_source: ObservationSource::Oms,
            });
        }
        Ok(())
    }

    fn check_ops_observation(
        ops: &Observation<OpsObservation>,
        policy: &ObservationPolicy,
    ) -> Result<(), AuthorityError> {
        let checks = [
            (
                OpsThresholdKind::SequenceGaps,
                u64::from(ops.payload.sequence_gaps),
                u64::from(policy.max_sequence_gaps),
            ),
            (
                OpsThresholdKind::JournalAppendLatency,
                ops.payload.journal_append_latency_ns,
                policy.max_journal_append_latency_ns,
            ),
            (
                OpsThresholdKind::JournalQueueDepth,
                ops.payload.journal_queue_depth,
                policy.max_journal_queue_depth,
            ),
            (
                OpsThresholdKind::DiskCapacity,
                ops.payload.disk_available_bytes,
                policy.min_disk_available_bytes,
            ),
            (
                OpsThresholdKind::UnknownOrders,
                u64::from(ops.payload.unknown_orders),
                u64::from(policy.max_unknown_orders),
            ),
            (
                OpsThresholdKind::ReconciliationDifferences,
                u64::from(ops.payload.reconciliation_differences),
                u64::from(policy.max_reconciliation_differences),
            ),
        ];
        for (kind, observed, limit) in checks {
            let exceeded = match kind {
                OpsThresholdKind::DiskCapacity => observed < limit,
                _ => observed > limit,
            };
            if exceeded {
                return Err(AuthorityError::OpsThresholdExceeded {
                    kind,
                    observed,
                    limit,
                });
            }
        }
        Ok(())
    }

    /// The single atomic gate: validate typed owner observations, evaluate the
    /// intent, then reserve exposure only on full acceptance.
    pub fn evaluate_and_reserve(
        &self,
        intent: OrderIntent,
        observations: Observations,
        expected_revision: Option<u64>,
    ) -> Result<Reservation, AuthorityError> {
        let mut state = self.lock()?;
        intent
            .validate()
            .map_err(|error| AuthorityError::InvalidIntent(error.to_string()))?;

        // Idempotency: an accepted replay returns its original permit without
        // reserving again; a reused id with different terms is conflicting.
        if let Some(existing) = state.reservations.get(&intent.intent_id) {
            let prior = state
                .decided
                .get(&intent.intent_id)
                .ok_or(AuthorityError::StatePoisoned)?;
            if *prior != intent {
                return Err(AuthorityError::ConflictingIntent(
                    intent.intent_id.to_string(),
                ));
            }
            return Ok(existing.clone());
        }
        if let Some(prior) = state.decided.get(&intent.intent_id) {
            if *prior != intent {
                return Err(AuthorityError::ConflictingIntent(
                    intent.intent_id.to_string(),
                ));
            }
            if state.consumed.contains(&intent.intent_id) {
                return Err(AuthorityError::AlreadyConsumed(
                    intent.intent_id.to_string(),
                ));
            }
            return Err(AuthorityError::PreviouslyRejected(
                intent.intent_id.to_string(),
            ));
        }

        if let Some(expected) = expected_revision
            && expected != state.revision
        {
            return Err(AuthorityError::StaleSnapshot {
                expected,
                current: state.revision,
            });
        }

        // The fence was checked against the state as observed on entry.
        // Aggregation may itself latch readiness facts; that is a real
        // transition and is committed with its own revision before the health
        // decision is returned. No caller-owned boolean or reason is accepted.
        self.observe(&mut state, &observations)?;

        let context = self.build_context(&state, &intent, &observations)?;
        let assessment = evaluate_risk(&intent, &self.policy, &context);
        if !assessment.accepted {
            // Remembering a rejection is a state change in its own right and
            // must invalidate any fence taken before it.
            let mut candidate = state.clone();
            candidate.decided.insert(intent.intent_id, intent);
            candidate.bump_revision()?;
            *state = candidate;
            return Err(AuthorityError::Rejected(assessment));
        }

        self.commit_reservation(&mut state, &intent, &assessment)
    }

    fn build_context(
        &self,
        state: &AuthorityState,
        intent: &OrderIntent,
        observations: &Observations,
    ) -> Result<RiskContext, AuthorityError> {
        let market = observations
            .market
            .as_ref()
            .ok_or(AuthorityError::MissingObservation {
                observation_source: ObservationSource::MarketDataKernel,
            })?;
        let feed = &market.payload.feed;
        let instrument = self.instruments.get(&intent.instrument_id).cloned();
        let quote = feed.quote.clone().filter(|value| {
            value.instrument_id == intent.instrument_id && value.venue == intent.venue
        });
        let feed_health = if quote.is_some() {
            feed.health(state.now, self.policy.max_quote_age_ns)
        } else {
            FeedHealth::NoQuote
        };
        let position_quantity = state
            .positions
            .get(&intent.instrument_id)
            .copied()
            .unwrap_or(0);
        let pending_buy_quantity =
            AuthorityState::pending_quantity(&state.pending_buy, &intent.instrument_id)?;
        let pending_sell_quantity =
            AuthorityState::pending_quantity(&state.pending_sell, &intent.instrument_id)?;
        // The context carries the *current* grouped exposures; `evaluate_risk`
        // adds the order notional once to project them.
        let strategy_exposure = state
            .strategy_exposure
            .get(&intent.strategy_id)
            .copied()
            .unwrap_or(Paise(0));
        let sector_label = self.sectors.get(&intent.instrument_id).cloned();
        let sector_exposure = sector_label
            .as_ref()
            .and_then(|label| state.sector_exposure.get(label))
            .copied()
            .unwrap_or(Paise(0));
        let is_derivative = instrument.as_ref().is_some_and(|value| {
            value.expiry_date.is_some()
                || value.strike_price_paise.is_some()
                || value.option_kind.is_some()
        });

        Ok(RiskContext {
            account_id: self.account_id,
            strategy_id: intent.strategy_id,
            venue: intent.venue,
            instrument,
            quote,
            feed_health,
            trade_date: self.trade_date,
            now: state.now,
            position_quantity,
            pending_buy_quantity,
            pending_sell_quantity,
            gross_exposure_paise: state.gross_exposure_paise,
            pending_gross_exposure_paise: state.pending_gross_exposure_paise,
            strategy_exposure_paise: strategy_exposure,
            sector_label,
            sector_exposure_paise: sector_exposure,
            positions: state.positions.clone(),
            marks_paise: market.payload.marks_paise.clone(),
            depth: market.payload.depth,
            greeks: market.payload.greeks,
            is_derivative,
            borrow: market.payload.borrow,
            daily_pnl: state.daily_pnl,
            open_orders: state.open_orders,
            orders_last_second: state.orders_last_second(),
            session_open: state.session_open,
            account_snapshot_available: state.account_snapshot_available,
            kill_switch_engaged: state.kill_switch_engaged,
            reconciliation_blocked: state.reconciliation_blocked,
            duplicate_intent: false,
        })
    }

    fn commit_reservation(
        &self,
        state: &mut AuthorityState,
        intent: &OrderIntent,
        assessment: &RiskAssessment,
    ) -> Result<Reservation, AuthorityError> {
        // Plan every affected quantity with checked arithmetic before any of
        // it is written back, so a late overflow cannot leave a partial
        // exposure, permit or revision behind.
        let notional = i128::from(intent.limit_price.get().0) * i128::from(intent.quantity.get());
        let notional_paise =
            Paise(i64::try_from(notional).map_err(|_| AuthorityError::NotionalOverflow)?);
        let pending_gross = state
            .pending_gross_exposure_paise
            .checked_add(notional_paise)
            .map_err(|_| AuthorityError::NotionalOverflow)?;
        let strategy_exposure = state
            .strategy_exposure
            .get(&intent.strategy_id)
            .copied()
            .unwrap_or(Paise(0))
            .checked_add(notional_paise)
            .map_err(|_| AuthorityError::NotionalOverflow)?;
        let sector_exposure = if let Some(label) = self.sectors.get(&intent.instrument_id) {
            let current = state
                .sector_exposure
                .get(label)
                .copied()
                .unwrap_or(Paise(0));
            Some(
                current
                    .checked_add(notional_paise)
                    .map_err(|_| AuthorityError::NotionalOverflow)?,
            )
        } else {
            None
        };
        let pending_quantity = match intent.side {
            Side::Buy => state
                .pending_buy
                .get(&intent.instrument_id)
                .copied()
                .unwrap_or(0),
            Side::Sell => state
                .pending_sell
                .get(&intent.instrument_id)
                .copied()
                .unwrap_or(0),
        }
        .checked_add(intent.quantity.get())
        .ok_or(AuthorityError::AccountingOverflow(
            "pending quantity".to_owned(),
        ))?;
        let open_orders = state
            .open_orders
            .checked_add(1)
            .ok_or(AuthorityError::AccountingOverflow("open orders".to_owned()))?;
        let revision = next_revision(state.revision)?;
        // A saturated permit counter would reuse an issued permit id, so it
        // fails the entire reservation closed instead of silently wrapping.
        let permit_id = state.next_permit;
        let next_permit = state
            .next_permit
            .checked_add(1)
            .ok_or(AuthorityError::AccountingOverflow("permit id".to_owned()))?;

        let reservation = Reservation {
            permit_id,
            intent_id: intent.intent_id,
            account_id: self.account_id,
            strategy_id: intent.strategy_id,
            instrument_id: intent.instrument_id.clone(),
            venue: intent.venue,
            side: intent.side,
            quantity: intent.quantity,
            reserved_notional_paise: notional_paise,
            snapshot_revision: revision,
            assessment: assessment.clone(),
        };
        reservation.validate_invariants()?;

        // Commit once: every field below was derived from the pre-transition
        // state and none of the remaining operations can fail.
        let mut candidate = state.clone();
        candidate.pending_gross_exposure_paise = pending_gross;
        candidate
            .strategy_exposure
            .insert(intent.strategy_id, strategy_exposure);
        if let (Some(label), Some(exposure)) =
            (self.sectors.get(&intent.instrument_id), sector_exposure)
        {
            candidate.sector_exposure.insert(label.clone(), exposure);
        }
        let pending = match intent.side {
            Side::Buy => &mut candidate.pending_buy,
            Side::Sell => &mut candidate.pending_sell,
        };
        pending.insert(intent.instrument_id.clone(), pending_quantity);
        candidate.open_orders = open_orders;
        candidate.order_times.push_back(candidate.now);
        candidate.prune_order_times();
        candidate.revision = revision;
        candidate.next_permit = next_permit;
        candidate
            .reservations
            .insert(intent.intent_id, reservation.clone());
        candidate.decided.insert(intent.intent_id, intent.clone());
        *state = candidate;
        Ok(reservation)
    }

    /// Idempotent replay of an accepted intent returns its original permit.
    pub fn reservation(&self, intent_id: IntentId) -> Result<Reservation, AuthorityError> {
        let state = self.lock()?;
        let reservation = state
            .reservations
            .get(&intent_id)
            .cloned()
            .ok_or(AuthorityError::UnknownIntent(intent_id.to_string()))?;
        reservation.validate_invariants()?;
        Ok(reservation)
    }

    /// Risk-reducing path. It is observation-free by design so positions
    /// can be flattened when health is degraded; accounting invariants remain
    /// fail-closed. See invariant L5.
    pub fn release(&self, intent_id: IntentId) -> Result<Reservation, AuthorityError> {
        let mut state = self.lock()?;
        let mut candidate = state.clone();
        let reservation = candidate
            .reservations
            .get(&intent_id)
            .cloned()
            .ok_or(AuthorityError::UnknownIntent(intent_id.to_string()))?;
        // All fallible accounting runs on the candidate. The reservation is
        // only removed at the commit point below, so a failed subtraction
        // leaves it available for a later, successful release.
        candidate.pending_gross_exposure_paise = Paise(
            candidate
                .pending_gross_exposure_paise
                .0
                .checked_sub(reservation.reserved_notional_paise.0)
                .ok_or(AuthorityError::AccountingOverflow(
                    "pending exposure".to_owned(),
                ))?,
        );
        self.remove_pending(&mut candidate, &reservation)?;
        self.release_grouped_exposure(&mut candidate, &reservation)?;
        candidate.open_orders = candidate
            .open_orders
            .checked_sub(1)
            .ok_or(AuthorityError::AccountingOverflow("open orders".to_owned()))?;
        candidate.bump_revision()?;
        candidate.reservations.remove(&intent_id);
        candidate.consumed.insert(intent_id);
        *state = candidate;
        Ok(reservation)
    }

    /// Risk-reducing path: convert a reservation into a filled position.
    /// Observation-free by design so fills can always be accounted for, even
    /// when the feed is stale or the kill switch is engaged (L5).
    pub fn apply_fill(&self, intent_id: IntentId) -> Result<Reservation, AuthorityError> {
        let mut state = self.lock()?;
        let mut candidate = state.clone();
        let reservation = candidate
            .reservations
            .get(&intent_id)
            .cloned()
            .ok_or(AuthorityError::UnknownIntent(intent_id.to_string()))?;
        // Nothing is consumed from state until every checked step succeeds.
        // The reservation, pending buckets, gross, grouped exposure, position,
        // open-order count, permit counter and revision therefore all roll
        // back together on any failure.
        candidate.pending_gross_exposure_paise = Paise(
            candidate
                .pending_gross_exposure_paise
                .0
                .checked_sub(reservation.reserved_notional_paise.0)
                .ok_or(AuthorityError::AccountingOverflow(
                    "pending exposure".to_owned(),
                ))?,
        );
        candidate.gross_exposure_paise = Paise(
            candidate
                .gross_exposure_paise
                .0
                .checked_add(reservation.reserved_notional_paise.0)
                .ok_or(AuthorityError::AccountingOverflow(
                    "gross exposure".to_owned(),
                ))?,
        );
        self.remove_pending(&mut candidate, &reservation)?;
        // The strategy/sector buckets already include this reservation from
        // commit time; a fill only moves pending exposure into a position.
        // Verify the checked cover before treating the fill as accounted.
        let strategy_cover = candidate
            .strategy_exposure
            .get(&reservation.strategy_id)
            .copied()
            .unwrap_or(Paise(0));
        if strategy_cover.0 < reservation.reserved_notional_paise.0 {
            return Err(AuthorityError::AccountingOverflow(
                "strategy exposure".to_owned(),
            ));
        }
        if let Some(label) = self.sectors.get(&reservation.instrument_id) {
            let sector_cover = candidate
                .sector_exposure
                .get(label)
                .copied()
                .unwrap_or(Paise(0));
            if sector_cover.0 < reservation.reserved_notional_paise.0 {
                return Err(AuthorityError::AccountingOverflow(
                    "sector exposure".to_owned(),
                ));
            }
        }
        let signed = match reservation.side {
            Side::Buy => reservation.quantity.get(),
            Side::Sell => -reservation.quantity.get(),
        };
        let position = candidate
            .positions
            .entry(reservation.instrument_id.clone())
            .or_insert(0);
        *position = position
            .checked_add(signed)
            .ok_or(AuthorityError::AccountingOverflow("position".to_owned()))?;
        candidate.open_orders = candidate
            .open_orders
            .checked_sub(1)
            .ok_or(AuthorityError::AccountingOverflow("open orders".to_owned()))?;
        candidate.bump_revision()?;
        candidate.reservations.remove(&intent_id);
        candidate.consumed.insert(intent_id);
        *state = candidate;
        Ok(reservation)
    }

    /// Subtract a released reservation from its strategy and (when mapped)
    /// sector buckets. A missing or insufficient bucket is a hard accounting
    /// failure, never a silent clamp.
    fn release_grouped_exposure(
        &self,
        state: &mut AuthorityState,
        reservation: &Reservation,
    ) -> Result<(), AuthorityError> {
        decrement_exposure(
            &mut state.strategy_exposure,
            &reservation.strategy_id,
            reservation.reserved_notional_paise,
        )?;
        if let Some(label) = self.sectors.get(&reservation.instrument_id) {
            decrement_exposure(
                &mut state.sector_exposure,
                label,
                reservation.reserved_notional_paise,
            )?;
        }
        Ok(())
    }

    fn remove_pending(
        &self,
        state: &mut AuthorityState,
        reservation: &Reservation,
    ) -> Result<(), AuthorityError> {
        let pending = match reservation.side {
            Side::Buy => &mut state.pending_buy,
            Side::Sell => &mut state.pending_sell,
        };
        if let std::collections::btree_map::Entry::Occupied(mut entry) =
            pending.entry(reservation.instrument_id.clone())
        {
            let remaining = entry.get().checked_sub(reservation.quantity.get()).ok_or(
                AuthorityError::AccountingOverflow("pending quantity".to_owned()),
            )?;
            if remaining <= 0 {
                entry.remove();
            } else {
                *entry.get_mut() = remaining;
            }
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Result<AuthoritySnapshot, AuthorityError> {
        let state = self.lock()?;
        Ok(AuthoritySnapshot {
            revision: state.revision,
            now: state.now,
            gross_exposure_paise: state.gross_exposure_paise,
            pending_gross_exposure_paise: state.pending_gross_exposure_paise,
            strategy_exposure: state.strategy_exposure.clone(),
            sector_exposure: state.sector_exposure.clone(),
            open_orders: state.open_orders,
            orders_last_second: state.orders_last_second(),
            positions: state.positions.clone(),
            kill_switch_engaged: state.kill_switch_engaged,
            reconciliation_blocked: state.reconciliation_blocked,
            account_snapshot_available: state.account_snapshot_available,
            reservation_count: state.reservations.len(),
        })
    }

    pub fn policy_version(&self) -> &str {
        &self.policy.version
    }
}

fn decrement_exposure<K: Ord + Clone>(
    bucket: &mut BTreeMap<K, Paise>,
    key: &K,
    amount: Paise,
) -> Result<(), AuthorityError> {
    match bucket.entry(key.clone()) {
        std::collections::btree_map::Entry::Vacant(_) => Err(AuthorityError::AccountingOverflow(
            "grouped exposure".to_owned(),
        )),
        std::collections::btree_map::Entry::Occupied(mut entry) => {
            let remaining = Paise(entry.get().0.checked_sub(amount.0).ok_or_else(|| {
                AuthorityError::AccountingOverflow("grouped exposure".to_owned())
            })?);
            if remaining.0 == 0 {
                entry.remove();
            } else {
                *entry.get_mut() = remaining;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use risk_engine::{
        RiskPolicy, //
        ValidatedRiskPolicy,
    };
    use trading_core::{
        AlgoId, //
        JournalRef,
        PricePaise,
        Product,
    };

    fn sectors() -> BTreeMap<InstrumentId, String> {
        BTreeMap::from([(
            InstrumentId::new("nse.cash.test").expect("instrument"),
            "synthetic-sector".to_owned(),
        )])
    }

    fn test_authority() -> RiskAuthority {
        let raw = RiskPolicy {
            version: "risk-atomic-test".to_owned(),
            allowed_accounts: BTreeSet::from([AccountId::new(1).expect("account")]),
            allowed_strategies: BTreeSet::from([StrategyId::new(2).expect("strategy")]),
            allowed_algos: BTreeSet::from([AlgoId::new(3).expect("algo")]),
            allowed_instruments: BTreeSet::from([
                InstrumentId::new("nse.cash.test").expect("instrument")
            ]),
            max_order_quantity: Quantity::new(100).expect("quantity"),
            max_order_notional_paise: Paise(1_000_000),
            max_position_quantity: Quantity::new(100).expect("quantity"),
            max_gross_exposure_paise: Paise(1_000_000),
            max_daily_loss_paise: Paise(5_000),
            max_open_orders: 100,
            max_orders_per_second: 100,
            max_price_deviation_ppm: 100_000,
            max_quote_age_ns: 1_000,
            allow_short: true,
            max_strategy_exposure_paise: Paise(1_000_000),
            max_sector_exposure_paise: None,
            stress_scenario: None,
            max_stress_loss_paise: None,
            max_participation_ppm: None,
            greek_limits: None,
            max_borrow_quantity: None,
            max_financing_cost_paise: None,
        };
        RiskAuthority::new(
            AccountId::new(1).expect("account"),
            ValidatedRiskPolicy::from_policy(&raw).expect("policy"),
            Vec::new(),
            TradeDate::new(2026, 1, 1).expect("date"),
        )
        .with_sectors(sectors())
    }

    fn intent(id: u64, quantity: i64, price: i64, side: Side) -> OrderIntent {
        OrderIntent {
            intent_id: IntentId::new(id).expect("intent id"),
            account_id: AccountId::new(1).expect("account"),
            strategy_id: StrategyId::new(2).expect("strategy"),
            strategy_version: "test".to_owned(),
            algo_id: AlgoId::new(3).expect("algo"),
            instrument_id: InstrumentId::new("nse.cash.test").expect("instrument"),
            venue: Venue::Nse,
            side,
            product: Product::Intraday,
            quantity: Quantity::new(quantity).expect("quantity"),
            limit_price: PricePaise::new(Paise(price)).expect("price"),
            reason: "test".to_owned(),
            evidence: vec![JournalRef {
                sequence: 1,
                hash_hex: "ab".repeat(32),
            }],
            expires_at: LogicalNanos::new(20_000),
        }
    }

    fn assessment(target: &OrderIntent) -> RiskAssessment {
        RiskAssessment {
            intent_id: target.intent_id,
            policy_version: "risk-atomic-test".to_owned(),
            accepted: true,
            checks: Vec::new(),
        }
    }

    fn reserved_id() -> IntentId {
        IntentId::new(1).expect("intent id")
    }

    /// A fully consistent committed reservation of two lots at 1_000 paise.
    fn seed_reservation(state: &mut AuthorityState, side: Side) {
        let target = intent(1, 2, 1_000, side);
        state.revision = 7;
        state.reservations.insert(
            target.intent_id,
            Reservation {
                permit_id: 1,
                intent_id: target.intent_id,
                account_id: AccountId::new(1).expect("account"),
                strategy_id: StrategyId::new(2).expect("strategy"),
                instrument_id: target.instrument_id.clone(),
                venue: Venue::Nse,
                side,
                quantity: Quantity::new(2).expect("quantity"),
                reserved_notional_paise: Paise(2_000),
                snapshot_revision: 7,
                assessment: assessment(&target),
            },
        );
        state.pending_gross_exposure_paise = Paise(2_000);
        state
            .strategy_exposure
            .insert(StrategyId::new(2).expect("strategy"), Paise(2_000));
        state
            .sector_exposure
            .insert("synthetic-sector".to_owned(), Paise(2_000));
        let pending = match side {
            Side::Buy => &mut state.pending_buy,
            Side::Sell => &mut state.pending_sell,
        };
        pending.insert(target.instrument_id, 2);
        state.open_orders = 1;
    }

    fn assert_atomic_failure<R: std::fmt::Debug>(
        authority: &RiskAuthority,
        label: &str,
        edit: impl FnOnce(&mut AuthorityState),
        operation: impl FnOnce(&RiskAuthority) -> Result<R, AuthorityError>,
    ) {
        {
            let mut state = authority.state.lock().expect("lock");
            edit(&mut state);
        }
        let before = authority.state.lock().expect("lock").digest();
        let message = format!("{label}: overflow must fail closed");
        let error = operation(authority).expect_err(&message);
        assert!(
            matches!(
                error,
                AuthorityError::NotionalOverflow | AuthorityError::AccountingOverflow(_)
            ),
            "{label}: the failure is a typed accounting error"
        );
        assert_eq!(
            authority.state.lock().expect("lock").digest(),
            before,
            "{label}: every authority fact is unchanged after the failure"
        );
    }

    fn assert_reservation_failure(label: &str, price: i64, edit: impl FnOnce(&mut AuthorityState)) {
        let authority = test_authority();
        let target = intent(1, 2, price, Side::Buy);
        let decision = assessment(&target);
        assert_atomic_failure(&authority, label, edit, |authority| {
            let mut state = authority.state.lock().expect("lock");
            authority.commit_reservation(&mut state, &target, &decision)
        });
    }

    fn assert_release_failure(label: &str, edit: impl FnOnce(&mut AuthorityState)) {
        let authority = test_authority();
        assert_atomic_failure(
            &authority,
            label,
            |state| {
                seed_reservation(state, Side::Buy);
                edit(state);
            },
            |authority| authority.release(reserved_id()),
        );
    }

    fn assert_fill_failure(label: &str, edit: impl FnOnce(&mut AuthorityState)) {
        let authority = test_authority();
        assert_atomic_failure(
            &authority,
            label,
            |state| {
                seed_reservation(state, Side::Buy);
                edit(state);
            },
            |authority| authority.apply_fill(reserved_id()),
        );
    }

    #[test]
    fn each_reservation_accounting_overflow_is_atomic() {
        assert_reservation_failure("notional conversion", i64::MAX, |_| {});
        assert_reservation_failure("pending gross", 1_000, |state| {
            state.pending_gross_exposure_paise = Paise(i64::MAX);
        });
        assert_reservation_failure("strategy exposure", 1_000, |state| {
            state
                .strategy_exposure
                .insert(StrategyId::new(2).expect("strategy"), Paise(i64::MAX));
        });
        assert_reservation_failure("sector exposure", 1_000, |state| {
            state
                .sector_exposure
                .insert("synthetic-sector".to_owned(), Paise(i64::MAX));
        });
        assert_reservation_failure("pending quantity", 1_000, |state| {
            state.pending_buy.insert(
                InstrumentId::new("nse.cash.test").expect("instrument"),
                i64::MAX,
            );
        });
        assert_reservation_failure("open orders", 1_000, |state| {
            state.open_orders = u32::MAX;
        });
    }

    #[test]
    fn each_release_accounting_overflow_is_atomic() {
        assert_release_failure("pending exposure", |state| {
            state.pending_gross_exposure_paise = Paise(i64::MIN);
        });
        assert_release_failure("pending quantity", |state| {
            state.pending_buy.insert(
                InstrumentId::new("nse.cash.test").expect("instrument"),
                i64::MIN,
            );
        });
        assert_release_failure("strategy exposure", |state| {
            state
                .strategy_exposure
                .insert(StrategyId::new(2).expect("strategy"), Paise(i64::MIN));
        });
        assert_release_failure("sector exposure", |state| {
            state
                .sector_exposure
                .insert("synthetic-sector".to_owned(), Paise(i64::MIN));
        });
        assert_release_failure("open orders", |state| state.open_orders = 0);
    }

    #[test]
    fn each_fill_accounting_overflow_is_atomic() {
        assert_fill_failure("pending exposure", |state| {
            state.pending_gross_exposure_paise = Paise(i64::MIN);
        });
        assert_fill_failure("gross exposure", |state| {
            state.gross_exposure_paise = Paise(i64::MAX);
        });
        assert_fill_failure("pending quantity", |state| {
            state.pending_buy.insert(
                InstrumentId::new("nse.cash.test").expect("instrument"),
                i64::MIN,
            );
        });
        assert_fill_failure("strategy cover", |state| {
            state
                .strategy_exposure
                .insert(StrategyId::new(2).expect("strategy"), Paise(0));
        });
        assert_fill_failure("sector cover", |state| {
            state
                .sector_exposure
                .insert("synthetic-sector".to_owned(), Paise(0));
        });
        assert_fill_failure("position", |state| {
            state.positions.insert(
                InstrumentId::new("nse.cash.test").expect("instrument"),
                i64::MAX,
            );
        });
        assert_fill_failure("open orders", |state| state.open_orders = 0);
    }

    #[test]
    fn failed_release_and_fill_retain_the_reservation() {
        let authority = test_authority();
        {
            let mut state = authority.state.lock().expect("lock");
            seed_reservation(&mut state, Side::Buy);
            state.pending_gross_exposure_paise = Paise(i64::MIN);
        }
        assert!(
            authority.release(reserved_id()).is_err(),
            "underflow denies the release"
        );
        assert_eq!(
            authority.snapshot().expect("snapshot").reservation_count,
            1,
            "the failed release did not consume the reservation"
        );
        assert_eq!(
            authority
                .reservation(reserved_id())
                .expect("retained permit")
                .permit_id,
            1,
            "the permit is still available to downstream consumers"
        );

        assert!(
            authority.apply_fill(reserved_id()).is_err(),
            "underflow denies the fill"
        );
        let snapshot = authority.snapshot().expect("snapshot");
        assert_eq!(
            snapshot.reservation_count, 1,
            "the failed fill kept the reservation"
        );
        assert_eq!(
            snapshot.pending_gross_exposure_paise,
            Paise(i64::MIN),
            "the failed fill kept the prior pending exposure"
        );
        assert_eq!(
            snapshot.gross_exposure_paise,
            Paise(0),
            "the failed fill did not move exposure into gross"
        );
    }

    #[test]
    fn permit_and_revision_exhaustion_fail_closed() {
        let authority = test_authority();
        let target = intent(1, 2, 1_000, Side::Buy);
        let decision = assessment(&target);

        let mut state = authority.state.lock().expect("lock");
        state.next_permit = u64::MAX;
        let before = state.digest();
        let error = authority
            .commit_reservation(&mut state, &target, &decision)
            .expect_err("a permit counter at its maximum must not saturate");
        assert!(
            matches!(error, AuthorityError::AccountingOverflow(field) if field == "permit id"),
            "permit exhaustion is a typed fail-closed error"
        );
        assert_eq!(
            state.digest(),
            before,
            "no partial reservation is committed"
        );

        state.next_permit = 1;
        state.revision = u64::MAX;
        let before = state.digest();
        let error = authority
            .commit_reservation(&mut state, &target, &decision)
            .expect_err("a revision at its maximum must not saturate");
        assert!(
            matches!(error, AuthorityError::AccountingOverflow(field) if field == "revision"),
            "revision exhaustion is a typed fail-closed error"
        );
        assert_eq!(
            state.digest(),
            before,
            "no partial reservation is committed"
        );
    }

    #[test]
    fn public_setters_fail_closed_at_revision_exhaustion() {
        let authority = test_authority();
        let tokens = authority.register_all_owners().expect("owners");
        {
            let mut state = authority.state.lock().expect("lock");
            state.revision = u64::MAX;
        }
        let before = authority.state.lock().expect("lock").digest();

        let failures = [
            authority.set_now(LogicalNanos::new(1)).err(),
            authority.set_session_open(true).err(),
            authority.set_snapshot_available(true).err(),
            authority.set_daily_pnl(Paise(1)).err(),
            authority
                .set_kill_switch(
                    tokens.operator,
                    true,
                    "op".to_owned(),
                    "stop".to_owned(),
                    None,
                )
                .err(),
            authority
                .set_reconciliation_blocked(
                    tokens.reconciliation,
                    false,
                    "resolved".to_owned(),
                    Some("evidence".to_owned()),
                )
                .err(),
        ];
        for failure in failures {
            assert!(
                matches!(
                    failure,
                    Some(AuthorityError::AccountingOverflow(field)) if field == "revision"
                ),
                "every setter fails closed when revision is exhausted"
            );
        }
        assert_eq!(
            authority.state.lock().expect("lock").digest(),
            before,
            "exhaustion leaves the entire state unchanged"
        );
    }
}
