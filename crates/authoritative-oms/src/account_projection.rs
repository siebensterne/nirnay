//! Deterministic account projection from durable OMS fill events.
//!
//! The projector is the only writer of the account ledger derived from OMS
//! executions. It lives in the engines layer (a legal downward dependency on
//! storage); `account-journal` itself stays a generic append-only primitive.
//! Every posting is content-addressed, so duplicate delivery is a no-op and a
//! reused execution id with different content is a hard conflict. Corrections
//! are always new reversing records; history is never rewritten.

use std::{
    collections::{
        BTreeMap, //
        BTreeSet,
    },
    path::Path,
};

use account_journal::{
    CommitFault, //
    FencingToken,
    Journal,
    JournalError,
    StoredRecord,
    VerificationReport,
};
use serde::{
    Deserialize, //
    Serialize,
};
use sha2::{
    Digest, //
    Sha256,
};
use thiserror::Error;
use trading_core::{
    AccountId, //
    ChargeComponents,
    CheckedPaise,
    EventEnvelope,
    EventId,
    EventPayload,
    EventTimes,
    Fill,
    FillId,
    InstrumentId,
    LedgerChargeStatus,
    LedgerPostingKind,
    LogicalNanos,
    OpeningBalanceEvidence,
    OrderId,
    Paise,
    PricePaise,
    Quantity,
    StoredBody,
    TradeDate,
    UtcNanos,
};

use crate::{
    error::OmsError, //
    store::{
        ChargeStatus, //
        ClientOrderId,
        FillEvent,
        OmsStore,
        fill_content_digest,
    },
};

/// Stream identity for the projected account ledger.
pub const LEDGER_STREAM_ID: &str = "account-ledger-v1";

/// Fail-closed projection failures. No partial state is observable to callers
/// after an error, because the underlying journal poisons itself on uncertain
/// appends.
#[derive(Debug, Error)]
pub enum ProjectionError {
    #[error("account journal failure: {0}")]
    Journal(#[from] JournalError),
    #[error("domain contract failure: {0}")]
    Domain(#[from] trading_core::DomainError),
    #[error("execution `{execution_id}` was delivered with different content")]
    ContentConflict { execution_id: String },
    #[error("currency `{currency}` is not supported by this INR vertical")]
    CurrencyNotInr { currency: String },
    #[error("account scope `{account_scope}` has no evidenced opening balance")]
    AccountNotOpen { account_scope: String },
    #[error("execution `{execution_id}` is not in the ledger")]
    UnknownExecution { execution_id: String },
    #[error("execution `{execution_id}` has already been reversed and is not active")]
    ExecutionNotActive { execution_id: String },
    #[error("accounting overflow in `{0}`")]
    Overflow(String),
    #[error("replay digest mismatch: journal {journal}, recomputed {recomputed}")]
    ReplayMismatch { journal: String, recomputed: String },
    #[error("OMS failure: {0}")]
    Oms(#[from] OmsError),
}

/// Weighted-average cost lot for one (scope, instrument) position.
///
/// Cost is held in i128 paise so that allocation on a partial close never
/// wraps. Integer division truncates toward zero and the residual stays with
/// the remaining position; this is the fixed lot-matching policy of v1.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostLot {
    pub quantity: i64,
    pub cost_paise: i128,
}

/// The full deterministic account state, rebuilt only from journal records.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerState {
    pub opening_cash: Paise,
    pub cash: Paise,
    pub positions: BTreeMap<String, BTreeMap<InstrumentId, i64>>,
    pub cost_lots: BTreeMap<String, BTreeMap<InstrumentId, CostLot>>,
    pub charges_estimated: Paise,
    pub charges_confirmed: Paise,
    pub realised_pnl: Paise,
    /// `None` means no fresh mark exists for every open position: unrealised
    /// P&L is unavailable, never a silently assumed zero.
    pub unrealised_pnl: Option<Paise>,
    pub gross_exposure_cost_basis: Paise,
    pub marked_gross_exposure: Option<Paise>,
    pub posted_executions: BTreeSet<String>,
}

impl Default for LedgerState {
    fn default() -> Self {
        Self {
            opening_cash: Paise(0),
            cash: Paise(0),
            positions: BTreeMap::new(),
            cost_lots: BTreeMap::new(),
            charges_estimated: Paise(0),
            charges_confirmed: Paise(0),
            realised_pnl: Paise(0),
            unrealised_pnl: None,
            gross_exposure_cost_basis: Paise(0),
            marked_gross_exposure: None,
            posted_executions: BTreeSet::new(),
        }
    }
}

impl LedgerState {
    /// SHA-256 of the canonical state serialization. This is a semantic
    /// digest, independent of the journal's ordinal BLAKE3 chain.
    #[must_use]
    pub fn state_digest(&self) -> String {
        let encoded = serde_json::to_vec(self).unwrap_or_default();
        let raw = Sha256::digest(&encoded);
        raw.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

/// Journal-derived replay cursor. It is recomputed from the journal on every
/// open, so it can never drift from the durable source of truth.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerCursor {
    pub last_oms_execution_id: Option<String>,
    pub last_content_digest: Option<String>,
    pub journal_head_hash_hex: String,
    pub journal_record_count: u64,
    pub ledger_state_digest: String,
}

/// Result of an independent replay verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayVerification {
    pub cursor: LedgerCursor,
    pub replayed_state_digest: String,
    pub matched: bool,
}

#[derive(Clone, Debug)]
struct ProjectedExecution {
    event_id: EventId,
    execution_id: String,
    content_digest: String,
    account_scope: String,
    instrument_id: InstrumentId,
    fill: Fill,
    broker_event_id: String,
    client_order_id: u64,
    oms_order_id: String,
    source: String,
    provenance: String,
    source_time_utc: i64,
    received_time_utc: i64,
    charge_status: LedgerChargeStatus,
    /// True when this posting has itself been reversed with no superseding
    /// corrected posting: the execution is no longer active.
    reversed: bool,
}

/// Projector owning one account journal writer and its replayed state.
pub struct Projector {
    journal: Journal,
    account_id: AccountId,
    state: LedgerState,
    opening_ids: BTreeMap<String, EventId>,
}

impl Projector {
    /// Open the journal and rebuild all state from its verified records.
    pub fn new(journal: Journal, account_id: AccountId) -> Result<Self, ProjectionError> {
        let mut projector = Self {
            journal,
            account_id,
            state: LedgerState::default(),
            opening_ids: BTreeMap::new(),
        };
        projector.rebuild()?;
        Ok(projector)
    }

    /// Open a journal rooted at `root`.
    pub fn open(root: impl AsRef<Path>, account_id: AccountId) -> Result<Self, ProjectionError> {
        let journal = Journal::open(root, LEDGER_STREAM_ID)?;
        Self::new(journal, account_id)
    }

    #[must_use]
    pub fn state(&self) -> &LedgerState {
        &self.state
    }

    #[must_use]
    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    #[must_use]
    pub fn verification_report(&self) -> VerificationReport {
        self.journal.verification_report()
    }

    /// Inject a commit fault at an append boundary. Test-only wiring.
    pub fn force_commit_fault(&mut self, fault: Option<CommitFault>) {
        self.journal.force_commit_fault(fault);
    }

    /// Grant an owner the next durable account-journal fencing term. This is
    /// the token-issuance entry point; every projection mutation below needs
    /// the returned token.
    pub fn fence(&mut self, owner: &str) -> Result<FencingToken, JournalError> {
        self.journal.fence(owner)
    }

    /// Fenced opening-balance append. Stale or wrong-owner tokens are refused
    /// before any work, including on an idempotent duplicate.
    pub fn open_account_fenced(
        &mut self,
        token: &FencingToken,
        account_scope: &str,
        starting_cash: Paise,
        trade_date: TradeDate,
        evidence: OpeningBalanceEvidence,
    ) -> Result<bool, ProjectionError> {
        self.journal.check_fencing(token)?;
        self.open_account(token, account_scope, starting_cash, trade_date, evidence)
    }

    /// Fenced posting of one OMS execution. The internal posting primitive is
    /// crate-only; external callers cannot append to the account journal
    /// without a current token:
    ///
    /// ```compile_fail
    /// # use authoritative_oms::{FillEvent, Projector};
    /// # use trading_core::InstrumentId;
    /// # fn outside(projector: &mut Projector, event: &FillEvent, instrument: &InstrumentId) {
    /// projector.project_fill(event, instrument);
    /// # }
    /// ```
    pub fn project_fill_fenced(
        &mut self,
        token: &FencingToken,
        event: &FillEvent,
        instrument_id: &InstrumentId,
    ) -> Result<bool, ProjectionError> {
        self.journal.check_fencing(token)?;
        self.project_fill(token, event, instrument_id)
    }

    /// Fenced bulk posting of persisted OMS fills.
    pub fn project_store_fills_fenced(
        &mut self,
        token: &FencingToken,
        store: &OmsStore,
        instruments: &BTreeMap<String, InstrumentId>,
    ) -> Result<usize, ProjectionError> {
        self.journal.check_fencing(token)?;
        self.project_store_fills(token, store, instruments)
    }

    /// Fenced reversal of a prior posting.
    pub fn bust_fenced(
        &mut self,
        token: &FencingToken,
        execution_id: &str,
    ) -> Result<bool, ProjectionError> {
        self.journal.check_fencing(token)?;
        self.bust(token, execution_id)
    }

    /// Fenced charge-confirmation correction.
    pub fn confirm_charges_fenced(
        &mut self,
        token: &FencingToken,
        execution_id: &str,
        confirmed_charges_paise: i64,
    ) -> Result<bool, ProjectionError> {
        self.journal.check_fencing(token)?;
        self.confirm_charges(token, execution_id, confirmed_charges_paise)
    }

    /// Rebuild state from every journal record in sequence order.
    fn rebuild(&mut self) -> Result<(), ProjectionError> {
        let mut state = LedgerState::default();
        let mut opening_ids = BTreeMap::new();
        for record in self.journal.records() {
            match &record.body.payload {
                EventPayload::AccountOpenedEvidenced {
                    account_scope,
                    starting_cash,
                    ..
                } => {
                    if state.opening_cash.0 != 0 && state.opening_cash != *starting_cash {
                        return Err(ProjectionError::Overflow(
                            "a second opening balance changes cash".to_owned(),
                        ));
                    }
                    if state.opening_cash.0 == 0 {
                        state.opening_cash = *starting_cash;
                        state.cash = *starting_cash;
                    }
                    opening_ids
                        .insert(account_scope.clone(), record.body.envelope.event_id.clone());
                }
                EventPayload::ExecutionProjected {
                    kind,
                    account_scope,
                    instrument_id,
                    charge_status,
                    fill,
                    cash_delta,
                    execution_id,
                    ..
                } => {
                    apply_posting(
                        &mut state,
                        *kind,
                        account_scope,
                        instrument_id,
                        fill,
                        *cash_delta,
                        *charge_status,
                    )?;
                    if *kind == LedgerPostingKind::Posting {
                        state.posted_executions.insert(execution_id.clone());
                    }
                }
                _ => {}
            }
        }
        self.state = state;
        self.opening_ids = opening_ids;
        Ok(())
    }

    /// Append an evidenced opening balance. A second opening for the same
    /// scope with a different amount is refused rather than silently applied.
    /// Internal-only opening-balance append. External callers must use
    /// [`Self::open_account_fenced`] with a current token.
    pub(crate) fn open_account(
        &mut self,
        token: &FencingToken,
        account_scope: &str,
        starting_cash: Paise,
        trade_date: TradeDate,
        evidence: OpeningBalanceEvidence,
    ) -> Result<bool, ProjectionError> {
        evidence.validate()?;
        if starting_cash.0 <= 0 {
            return Err(ProjectionError::Overflow(
                "starting cash must be positive".to_owned(),
            ));
        }
        let event_id = opening_event_id(account_scope, trade_date)?;
        if let Some(existing) = self
            .journal
            .records()
            .iter()
            .find(|record| record.body.envelope.event_id == event_id)
        {
            match &existing.body.payload {
                EventPayload::AccountOpenedEvidenced {
                    starting_cash: prior,
                    ..
                } => {
                    if *prior != starting_cash {
                        return Err(ProjectionError::Overflow(
                            "an opening balance cannot be changed".to_owned(),
                        ));
                    }
                    return Ok(false);
                }
                _ => {
                    return Err(ProjectionError::ContentConflict {
                        execution_id: account_scope.to_owned(),
                    });
                }
            }
        }
        let body = StoredBody {
            envelope: new_envelope(
                event_id.clone(),
                account_scope,
                "oms-fill-event",
                Some(event_id.clone()),
                None,
                0,
                0,
            )?,
            payload: EventPayload::AccountOpenedEvidenced {
                account_id: self.account_id,
                account_scope: account_scope.to_owned(),
                starting_cash,
                trade_date,
                evidence,
            },
        };
        self.journal.append_fenced(token, body)?;
        self.rebuild()?;
        Ok(true)
    }

    /// Project one OMS execution. Returns `false` on an identical duplicate.
    /// Internal-only posting append. External callers must use
    /// [`Self::project_fill_fenced`] with a current token.
    pub(crate) fn project_fill(
        &mut self,
        token: &FencingToken,
        event: &FillEvent,
        instrument_id: &InstrumentId,
    ) -> Result<bool, ProjectionError> {
        if event.currency != "INR" {
            return Err(ProjectionError::CurrencyNotInr {
                currency: event.currency.clone(),
            });
        }
        let opening = self
            .opening_ids
            .get(&event.account_scope)
            .cloned()
            .ok_or_else(|| ProjectionError::AccountNotOpen {
                account_scope: event.account_scope.clone(),
            })?;
        self.append_posting(
            token,
            LedgerPostingKind::Posting,
            event,
            instrument_id,
            &opening,
            None,
            None,
        )
    }

    /// Project every persisted OMS fill using a caller-supplied execution to
    /// instrument mapping. Delivery order does not affect the final state.
    /// Internal-only bulk posting append. External callers must use
    /// [`Self::project_store_fills_fenced`] with a current token.
    pub(crate) fn project_store_fills(
        &mut self,
        token: &FencingToken,
        store: &OmsStore,
        instruments: &BTreeMap<String, InstrumentId>,
    ) -> Result<usize, ProjectionError> {
        let mut projected = 0;
        for event in store.fills() {
            let instrument = instruments.get(&event.execution_id).ok_or_else(|| {
                ProjectionError::UnknownExecution {
                    execution_id: event.execution_id.clone(),
                }
            })?;
            if self.project_fill(token, &event, instrument)? {
                projected += 1;
            }
        }
        Ok(projected)
    }

    /// Append a reversing record for a prior posting. The original record is
    /// never mutated; the net state change is the exact negative delta.
    /// Internal-only reversal append. External callers must use
    /// [`Self::bust_fenced`] with a current token.
    pub(crate) fn bust(
        &mut self,
        token: &FencingToken,
        execution_id: &str,
    ) -> Result<bool, ProjectionError> {
        let prior = self.find_execution(execution_id)?;
        if prior.reversed {
            return Ok(false);
        }
        let original = prior.to_fill_event(prior.charge_status, prior.execution_id.clone())?;
        prior.check_digest(&original)?;
        self.append_posting(
            token,
            LedgerPostingKind::Reversal,
            &original,
            &prior.instrument_id,
            &prior.event_id,
            None,
            Some(bust_event_id(&prior.account_scope, execution_id)?),
        )
    }

    /// Rebook estimated charges as broker-confirmed values by appending a
    /// reversal of the original posting followed by a new positive posting.
    /// Internal-only correction append. External callers must use
    /// [`Self::confirm_charges_fenced`] with a current token.
    pub(crate) fn confirm_charges(
        &mut self,
        token: &FencingToken,
        execution_id: &str,
        confirmed_charges_paise: i64,
    ) -> Result<bool, ProjectionError> {
        let prior = self.find_execution(execution_id)?;
        if prior.reversed {
            return Err(ProjectionError::ExecutionNotActive {
                execution_id: execution_id.to_owned(),
            });
        }
        let original = prior.to_fill_event(prior.charge_status, prior.execution_id.clone())?;
        prior.check_digest(&original)?;
        let reversal_id =
            charge_reversal_event_id(&prior.account_scope, execution_id, confirmed_charges_paise)?;
        let appended = self.append_posting(
            token,
            LedgerPostingKind::Reversal,
            &original,
            &prior.instrument_id,
            &prior.event_id,
            None,
            Some(reversal_id.clone()),
        )?;
        if !appended {
            return Ok(false);
        }
        let corrected_id = format!("{execution_id}:confirmed:{confirmed_charges_paise}");
        let corrected = prior.to_fill_event(LedgerChargeStatus::BrokerConfirmed, corrected_id)?;
        let corrected = FillEvent {
            charges_paise: confirmed_charges_paise,
            charge_status: ChargeStatus::BrokerConfirmed,
            provenance: "broker-confirmed charge correction".to_owned(),
            ..corrected
        };
        let opening = self
            .opening_ids
            .get(&prior.account_scope)
            .cloned()
            .ok_or_else(|| ProjectionError::AccountNotOpen {
                account_scope: prior.account_scope.clone(),
            })?;
        self.append_posting(
            token,
            LedgerPostingKind::Posting,
            &corrected,
            &prior.instrument_id,
            &opening,
            Some(reversal_id),
            Some(execution_event_id(
                &prior.account_scope,
                &corrected.execution_id,
            )?),
        )?;
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)] // internal posting primitive carries the mandatory token
    fn append_posting(
        &mut self,
        token: &FencingToken,
        kind: LedgerPostingKind,
        event: &FillEvent,
        instrument_id: &InstrumentId,
        causation: &EventId,
        correlation: Option<EventId>,
        event_id_override: Option<EventId>,
    ) -> Result<bool, ProjectionError> {
        let digest = fill_content_digest(event);
        let event_id = match event_id_override {
            Some(id) => id,
            None if kind == LedgerPostingKind::Posting => {
                execution_event_id(&event.account_scope, &event.execution_id)?
            }
            None => bust_event_id(&event.account_scope, &event.execution_id)?,
        };
        if let Some(existing) = self
            .journal
            .records()
            .iter()
            .find(|record| record.body.envelope.event_id == event_id)
        {
            match &existing.body.payload {
                EventPayload::ExecutionProjected { content_digest, .. } => {
                    if *content_digest != digest {
                        return Err(ProjectionError::ContentConflict {
                            execution_id: event.execution_id.clone(),
                        });
                    }
                    return Ok(false);
                }
                _ => {
                    return Err(ProjectionError::ContentConflict {
                        execution_id: event.execution_id.clone(),
                    });
                }
            }
        }
        let fill = to_fill(event)?;
        let expected = fill.expected_cash_delta()?;
        let cash_delta = if kind == LedgerPostingKind::Reversal {
            expected.checked_neg()?
        } else {
            expected
        };
        let charge_status = match event.charge_status {
            ChargeStatus::Estimated => LedgerChargeStatus::Estimated,
            ChargeStatus::BrokerConfirmed => LedgerChargeStatus::BrokerConfirmed,
        };
        let body = StoredBody {
            envelope: new_envelope(
                event_id,
                &event.account_scope,
                &event.source,
                Some(causation.clone()),
                correlation,
                event.broker_timestamp_ns,
                event.received_at_ns,
            )?,
            payload: EventPayload::ExecutionProjected {
                kind,
                execution_id: event.execution_id.clone(),
                content_digest: digest,
                broker_event_id: event.broker_event_id.clone(),
                client_order_id: event.client_order_id.get(),
                oms_order_id: event.oms_order_id.clone(),
                charge_status,
                account_scope: event.account_scope.clone(),
                instrument_id: instrument_id.clone(),
                provenance: event.provenance.clone(),
                fill,
                cash_delta,
            },
        };
        self.journal.append_fenced(token, body)?;
        self.rebuild()?;
        Ok(true)
    }

    /// Resolve the active (unreversed) posting for a logical execution.
    ///
    /// A charge confirmation appends a reversal of the previous posting and a
    /// new corrected posting linked by `correlation_id`. Following that chain
    /// prevents a later bust from reversing the already-reversed original,
    /// which would double-reverse cash and charges.
    fn find_execution(&self, execution_id: &str) -> Result<ProjectedExecution, ProjectionError> {
        let mut current = self.find_posting_by_id(execution_id)?;
        while let Some(reversal_id) = self
            .journal
            .records()
            .iter()
            .find(|record| {
                matches!(
                    record.body.payload,
                    EventPayload::ExecutionProjected {
                        kind: LedgerPostingKind::Reversal,
                        ..
                    }
                ) && record.body.envelope.causation_id.as_ref() == Some(&current.event_id)
            })
            .map(|record| record.body.envelope.event_id.clone())
        {
            if let Some(superseding) = self.journal.records().iter().find(|record| {
                matches!(
                    record.body.payload,
                    EventPayload::ExecutionProjected {
                        kind: LedgerPostingKind::Posting,
                        ..
                    }
                ) && record.body.envelope.correlation_id.as_ref() == Some(&reversal_id)
            }) {
                current = self.to_projected_execution(superseding)?;
            } else {
                current.reversed = true;
                break;
            }
        }
        Ok(current)
    }

    fn find_posting_by_id(
        &self,
        execution_id: &str,
    ) -> Result<ProjectedExecution, ProjectionError> {
        let record = self
            .journal
            .records()
            .iter()
            .find(|record| {
                matches!(
                    &record.body.payload,
                    EventPayload::ExecutionProjected {
                        kind: LedgerPostingKind::Posting,
                        execution_id: recorded_id,
                        ..
                    } if recorded_id == execution_id
                )
            })
            .ok_or_else(|| ProjectionError::UnknownExecution {
                execution_id: execution_id.to_owned(),
            })?;
        self.to_projected_execution(record)
    }

    fn to_projected_execution(
        &self,
        record: &StoredRecord,
    ) -> Result<ProjectedExecution, ProjectionError> {
        let EventPayload::ExecutionProjected {
            execution_id,
            content_digest,
            account_scope,
            instrument_id,
            broker_event_id,
            client_order_id,
            oms_order_id,
            charge_status,
            provenance,
            fill,
            ..
        } = &record.body.payload
        else {
            return Err(ProjectionError::UnknownExecution {
                execution_id: "<non-posting record>".to_owned(),
            });
        };
        Ok(ProjectedExecution {
            event_id: record.body.envelope.event_id.clone(),
            execution_id: execution_id.clone(),
            content_digest: content_digest.clone(),
            account_scope: account_scope.clone(),
            instrument_id: instrument_id.clone(),
            fill: fill.clone(),
            broker_event_id: broker_event_id.clone(),
            client_order_id: *client_order_id,
            oms_order_id: oms_order_id.clone(),
            source: record.body.envelope.source_id.clone().unwrap_or_default(),
            provenance: provenance.clone(),
            source_time_utc: record
                .body
                .envelope
                .times
                .source_time_utc
                .map_or(0, UtcNanos::get),
            received_time_utc: record.body.envelope.times.received_time_utc.get(),
            charge_status: *charge_status,
            reversed: false,
        })
    }

    /// Recompute all state from the journal and compare it with the live state
    /// and the journal verification report.
    pub fn verify_replay(&self) -> Result<ReplayVerification, ProjectionError> {
        let mut replayed = LedgerState::default();
        for record in self.journal.records() {
            match &record.body.payload {
                EventPayload::AccountOpenedEvidenced {
                    account_scope,
                    starting_cash,
                    ..
                } => {
                    if replayed.opening_cash.0 == 0 {
                        replayed.opening_cash = *starting_cash;
                        replayed.cash = *starting_cash;
                    }
                    let _ = account_scope;
                }
                EventPayload::ExecutionProjected {
                    kind,
                    account_scope,
                    instrument_id,
                    charge_status,
                    fill,
                    cash_delta,
                    execution_id,
                    ..
                } => {
                    apply_posting(
                        &mut replayed,
                        *kind,
                        account_scope,
                        instrument_id,
                        fill,
                        *cash_delta,
                        *charge_status,
                    )?;
                    if *kind == LedgerPostingKind::Posting {
                        replayed.posted_executions.insert(execution_id.clone());
                    }
                }
                _ => {}
            }
        }
        let report = self.journal.verification_report();
        let (last_id, last_digest) = self
            .journal
            .records()
            .iter()
            .rev()
            .find_map(|record| match &record.body.payload {
                EventPayload::ExecutionProjected {
                    execution_id,
                    content_digest,
                    ..
                } => Some((Some(execution_id.clone()), Some(content_digest.clone()))),
                _ => None,
            })
            .unwrap_or((None, None));
        let cursor = LedgerCursor {
            last_oms_execution_id: last_id,
            last_content_digest: last_digest,
            journal_head_hash_hex: report.head_hash_hex,
            journal_record_count: report.record_count,
            ledger_state_digest: self.state.state_digest(),
        };
        // Marks come from an external kernel quote, not the journal, so they
        // are excluded from the replay equality comparison.
        replayed.unrealised_pnl = self.state.unrealised_pnl;
        replayed.marked_gross_exposure = self.state.marked_gross_exposure;
        let replayed_digest = replayed.state_digest();
        let matched = replayed == self.state && replayed_digest == cursor.ledger_state_digest;
        Ok(ReplayVerification {
            cursor,
            replayed_state_digest: replayed_digest,
            matched,
        })
    }

    /// Mark open positions to market. Missing marks leave unrealised P&L and
    /// marked exposure `None`, so risk sees `Unavailable` rather than zero.
    pub fn mark_to_market(
        &mut self,
        marks: &BTreeMap<String, BTreeMap<InstrumentId, PricePaise>>,
    ) -> Result<(), ProjectionError> {
        let mut unrealised = 0_i128;
        let mut gross = 0_i128;
        let mut complete = true;
        for (scope, positions) in &self.state.positions {
            for (instrument, quantity) in positions {
                if *quantity == 0 {
                    continue;
                }
                let Some(price) = marks
                    .get(scope)
                    .and_then(|by_instrument| by_instrument.get(instrument))
                else {
                    complete = false;
                    continue;
                };
                let market_value = i128::from(price.get().0) * i128::from(*quantity);
                let cost = self
                    .state
                    .cost_lots
                    .get(scope)
                    .and_then(|by_instrument| by_instrument.get(instrument))
                    .map_or(0, |lot| lot.cost_paise);
                unrealised = unrealised
                    .checked_add(market_value - cost)
                    .ok_or_else(|| ProjectionError::Overflow("unrealised pnl".to_owned()))?;
                gross = gross
                    .checked_add(market_value.abs())
                    .ok_or_else(|| ProjectionError::Overflow("marked exposure".to_owned()))?;
            }
        }
        if complete {
            self.state.unrealised_pnl = Some(to_paise(unrealised, "unrealised pnl")?);
            self.state.marked_gross_exposure = Some(to_paise(gross, "marked exposure")?);
        } else {
            self.state.unrealised_pnl = None;
            self.state.marked_gross_exposure = None;
        }
        Ok(())
    }
}

impl ProjectedExecution {
    fn check_digest(&self, event: &FillEvent) -> Result<(), ProjectionError> {
        if fill_content_digest(event) != self.content_digest {
            return Err(ProjectionError::ContentConflict {
                execution_id: self.execution_id.clone(),
            });
        }
        Ok(())
    }

    fn to_fill_event(
        &self,
        charge_status: LedgerChargeStatus,
        execution_id: String,
    ) -> Result<FillEvent, ProjectionError> {
        let status = match charge_status {
            LedgerChargeStatus::Estimated => ChargeStatus::Estimated,
            LedgerChargeStatus::BrokerConfirmed => ChargeStatus::BrokerConfirmed,
        };
        Ok(FillEvent {
            schema_version: 1,
            execution_id,
            broker_event_id: self.broker_event_id.clone(),
            client_order_id: ClientOrderId::new(self.client_order_id)
                .map_err(ProjectionError::Oms)?,
            oms_order_id: self.oms_order_id.clone(),
            side: self.fill.side,
            quantity: self.fill.quantity.get(),
            price_paise: self.fill.price.get().0,
            currency: "INR".to_owned(),
            broker_timestamp_ns: self.source_time_utc,
            received_at_ns: self.received_time_utc,
            source: self.source.clone(),
            account_scope: self.account_scope.clone(),
            charges_paise: self.fill.charges.total()?.0,
            charge_status: status,
            provenance: self.provenance.clone(),
        })
    }
}

fn to_fill(event: &FillEvent) -> Result<Fill, ProjectionError> {
    let quantity = Quantity::new(event.quantity)?;
    let price = PricePaise::new(Paise(event.price_paise))?;
    let cash_delta = price.notional(quantity)?;
    let charges = ChargeComponents {
        other: Paise(event.charges_paise),
        ..ChargeComponents::default()
    };
    let expected = if event.side == trading_core::Side::Buy {
        cash_delta.checked_add(charges.total()?)?.checked_neg()?
    } else {
        cash_delta.checked_sub(charges.total()?)?
    };
    Ok(Fill {
        fill_id: FillId::new(id_from_digest(&fill_content_digest(event), 0)?)?,
        order_id: OrderId::new(id_from_digest(&fill_content_digest(event), 16)?)?,
        side: event.side,
        quantity,
        price,
        charges,
        cash_delta: expected,
    })
}

fn apply_posting(
    state: &mut LedgerState,
    kind: LedgerPostingKind,
    account_scope: &str,
    instrument_id: &InstrumentId,
    fill: &Fill,
    cash_delta: Paise,
    charge_status: LedgerChargeStatus,
) -> Result<(), ProjectionError> {
    let sign: i64 = if kind == LedgerPostingKind::Posting {
        1
    } else {
        -1
    };
    // The payload carries the already-signed cash delta: a reversal stores the
    // negation of the original cash effect, so it is applied exactly once.
    state.cash = state.cash.checked_add(cash_delta)?;

    let quantity = fill.quantity.get();
    let signed_quantity = if fill.side == trading_core::Side::Buy {
        quantity
    } else {
        -quantity
    };
    let position_delta = signed_quantity
        .checked_mul(sign)
        .ok_or_else(|| ProjectionError::Overflow("position quantity".to_owned()))?;
    let position = state
        .positions
        .entry(account_scope.to_owned())
        .or_default()
        .entry(instrument_id.clone())
        .or_insert(0);
    *position = position
        .checked_add(position_delta)
        .ok_or_else(|| ProjectionError::Overflow("position quantity".to_owned()))?;

    let charges_total = fill.charges.total()?;
    let charges_delta = charges_total.checked_scale(sign)?;
    let bucket = if charge_status == LedgerChargeStatus::Estimated {
        &mut state.charges_estimated
    } else {
        &mut state.charges_confirmed
    };
    *bucket = bucket.checked_add(charges_delta)?;

    let lot = state
        .cost_lots
        .entry(account_scope.to_owned())
        .or_default()
        .entry(instrument_id.clone())
        .or_default();
    let turnover = fill.price.notional(fill.quantity)?;
    if fill.side == trading_core::Side::Buy {
        let cost = turnover.checked_add(charges_total)?.checked_scale(sign)?;
        lot.cost_paise = lot
            .cost_paise
            .checked_add(i128::from(cost.0))
            .ok_or_else(|| ProjectionError::Overflow("cost basis".to_owned()))?;
    } else {
        let available = lot.quantity.max(0);
        let allocated = if available == 0 || quantity >= available {
            lot.cost_paise
        } else {
            lot.cost_paise * i128::from(quantity) / i128::from(available)
        };
        let realised = i128::from(turnover.0) - allocated - i128::from(charges_total.0);
        let signed_realised = realised * i128::from(sign);
        state.realised_pnl = state
            .realised_pnl
            .checked_add(to_paise(signed_realised, "realised pnl")?)?;
        lot.cost_paise = lot
            .cost_paise
            .checked_sub(allocated * i128::from(sign))
            .ok_or_else(|| ProjectionError::Overflow("cost basis".to_owned()))?;
    }
    lot.quantity = lot
        .quantity
        .checked_add(position_delta)
        .ok_or_else(|| ProjectionError::Overflow("cost lot quantity".to_owned()))?;

    let mut exposure = 0_i128;
    for (scope, positions) in &state.positions {
        for (instrument, held) in positions {
            if *held == 0 {
                continue;
            }
            let cost = state
                .cost_lots
                .get(scope)
                .and_then(|lots| lots.get(instrument))
                .map_or(0, |lot| lot.cost_paise.abs());
            exposure = exposure
                .checked_add(cost)
                .ok_or_else(|| ProjectionError::Overflow("cost exposure".to_owned()))?;
        }
    }
    state.gross_exposure_cost_basis = to_paise(exposure, "cost exposure")?;
    Ok(())
}

fn to_paise(value: i128, label: &str) -> Result<Paise, ProjectionError> {
    i64::try_from(value)
        .map(Paise)
        .map_err(|_| ProjectionError::Overflow(label.to_owned()))
}

fn new_envelope(
    event_id: EventId,
    account_scope: &str,
    source_id: &str,
    causation: Option<EventId>,
    correlation: Option<EventId>,
    source_time_ns: i64,
    received_time_ns: i64,
) -> Result<EventEnvelope, ProjectionError> {
    Ok(EventEnvelope {
        event_id,
        schema_version: 2,
        stream_id: LEDGER_STREAM_ID.to_owned(),
        source_id: Some(source_id.to_owned()),
        source_clock_domain: Some("broker-event-clock".to_owned()),
        session_id: Some(account_scope.to_owned()),
        source_sequence: None,
        correlation_id: correlation,
        causation_id: causation,
        times: EventTimes {
            source_time_utc: Some(UtcNanos::new(source_time_ns)),
            received_time_utc: UtcNanos::new(received_time_ns),
            available_time: LogicalNanos::new(received_time_ns.max(0) as u64),
            recorded_time_utc: UtcNanos::new(received_time_ns),
        },
        quality: BTreeSet::new(),
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn execution_event_id(account_scope: &str, execution_id: &str) -> Result<EventId, ProjectionError> {
    let raw = Sha256::digest(format!("nirnay.exec:{account_scope}:{execution_id}").as_bytes());
    Ok(EventId::parse(hex(&raw[..24]))?)
}

fn bust_event_id(account_scope: &str, execution_id: &str) -> Result<EventId, ProjectionError> {
    let raw = Sha256::digest(format!("nirnay.bust:{account_scope}:{execution_id}").as_bytes());
    Ok(EventId::parse(hex(&raw[..24]))?)
}

fn charge_reversal_event_id(
    account_scope: &str,
    execution_id: &str,
    confirmed_charges_paise: i64,
) -> Result<EventId, ProjectionError> {
    // Including the amount makes each confirmation a distinct reversal, so a
    // second confirmation with a different amount rebooks the delta instead of
    // silently no-opping on a reused event id.
    let raw = Sha256::digest(
        format!("nirnay.charge-reversal:{account_scope}:{execution_id}:{confirmed_charges_paise}")
            .as_bytes(),
    );
    Ok(EventId::parse(hex(&raw[..24]))?)
}

fn opening_event_id(
    account_scope: &str,
    trade_date: TradeDate,
) -> Result<EventId, ProjectionError> {
    let raw = Sha256::digest(format!("nirnay.ledger.open:{account_scope}:{trade_date}").as_bytes());
    Ok(EventId::parse(hex(&raw[..24]))?)
}

fn id_from_digest(digest: &str, offset: usize) -> Result<u64, ProjectionError> {
    let slice = digest
        .get(offset..offset + 16)
        .ok_or_else(|| ProjectionError::Overflow("digest slice".to_owned()))?;
    let value = u64::from_str_radix(slice, 16)
        .map_err(|_| ProjectionError::Overflow("digest identifier".to_owned()))?;
    Ok(if value == 0 { 1 } else { value })
}
