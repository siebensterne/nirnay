//! Stateful risk authority with one atomic authorization operation.
//!
//! The authority owns every fact that `evaluate_risk` consumes: account-global
//! exposure, positions, pending orders, rate state, daily P&L, the logical
//! clock, snapshot availability, kill switch and reconciliation state. A caller
//! supplies only an [`OrderIntent`] and raw observations from registered owner
//! adapters; it cannot assert a `Healthy` flag, an exposure figure, an account
//! snapshot, a P&L figure, or forge another owner's facts.
//!
//! Daily P&L is an immutable [`DailyPnlSnapshot`] derived only from sealed
//! account-journal and valuation records. A missing, stale, contradictory or
//! foreign snapshot denies new risk; missing P&L is never interpreted as zero.
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
    fs::{
        File, //
        OpenOptions,
    },
    hash::{BuildHasher, RandomState},
    io::{BufRead, Write},
    path::{
        Path, //
        PathBuf,
    },
    sync::{
        Arc, //
        Mutex,
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
use thiserror::Error;
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
    ReconciliationStatus,
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
        ReconciliationState,
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

/// Which signed money effects the daily-loss limit consumes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DailyPnlBasis {
    /// Account-journal realized effects only; open mark-to-market losses do not
    /// consume the daily-loss budget.
    RealizedOnly,
    /// Realized effects plus valuation-derived unrealized P&L. This is the
    /// production basis: an open mark-to-market loss is a real account loss
    /// and must consume the daily-loss budget as a booked loss does. Fees,
    /// funding and FX effects are signed account-journal facts included in
    /// both bases.
    RealizedPlusUnrealized,
}

/// The only accepted daily P&L calculation version. A record carrying any
/// other version is rejected rather than interpreted with unknown semantics.
pub const DAILY_PNL_CALCULATION_VERSION: &str = "daily-pnl-v1";

/// Explicit business-day boundary supplied by the calendar owner. No time
/// zone, exchange or venue is defaulted by the authority: the IANA time-zone
/// name, the business date and the exact half-open instant window are all
/// owner facts, verified against the immutable authority trade date.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DailyPnlCalendar {
    pub time_zone: String,
    pub business_date: TradeDate,
    /// Inclusive logical nanosecond at which this business day starts.
    pub day_start_ns: i64,
    /// Exclusive logical nanosecond at which the next business day starts.
    pub next_day_start_ns: i64,
}

impl DailyPnlCalendar {
    #[must_use]
    pub fn new(
        time_zone: String,
        business_date: TradeDate,
        day_start_ns: i64,
        next_day_start_ns: i64,
    ) -> Self {
        Self {
            time_zone,
            business_date,
            day_start_ns,
            next_day_start_ns,
        }
    }
}

/// Fail-closed daily P&L errors. Every variant denies new risk; none resolves
/// to a zero P&L.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DailyPnlError {
    #[error("daily pnl snapshot is missing")]
    Missing,
    #[error("daily pnl publication is not authenticated by the account journal owner")]
    Unauthorised,
    #[error("authority state mutex was poisoned while publishing daily pnl")]
    StatePoisoned,
    #[error("unsupported daily pnl schema version {0}")]
    UnsupportedSchema(u32),
    #[error("unsupported daily pnl calculation version")]
    UnsupportedCalculationVersion,
    #[error("daily pnl record belongs to another account")]
    WrongAccount,
    #[error("daily pnl record belongs to another business date")]
    WrongBusinessDate,
    #[error("invalid daily pnl calendar: {0}")]
    InvalidCalendar(String),
    #[error("the authority clock is outside the declared business-day boundary")]
    OutsideBusinessDay,
    #[error("daily pnl journal binding is wrong: {0}")]
    WrongJournal(String),
    #[error("daily pnl valuation inputs are wrong: {0}")]
    WrongValuation(String),
    #[error("daily pnl positions digest disagrees with authoritative positions")]
    WrongPositions,
    #[error("daily pnl reconciliation is not matched and evidenced")]
    WrongReconciliation,
    #[error("daily pnl component disagrees with the sealed records: {0}")]
    Contradictory(String),
    #[error("daily pnl is stale: age {age_ns} ns exceeds {max_age_ns} ns")]
    Stale { age_ns: u64, max_age_ns: u64 },
    #[error("daily pnl evidence is dated in the future relative to the authority clock")]
    Future,
    #[error("daily pnl money arithmetic overflowed")]
    Overflow,
}

/// Immutable P&L evidence. It is built only by the authority from sealed
/// records; no caller supplies a P&L figure or mutates a snapshot in place.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DailyPnlSnapshot {
    pub schema_version: u32,
    pub account_id: AccountId,
    pub business_date: TradeDate,
    pub base_currency: String,
    pub account_journal_sequence: u64,
    pub account_journal_digest: String,
    pub positions_digest: String,
    pub valuation_inputs_digest: String,
    pub calculation_version: String,
    pub calendar_digest: String,
    pub basis: DailyPnlBasis,
    pub calendar: DailyPnlCalendar,
    pub realized_paise: i64,
    pub unrealized_paise: i64,
    pub fees_paise: i64,
    pub funding_paise: i64,
    pub fx_paise: i64,
    pub reconciliation_status: ReconciliationStatus,
    pub as_of_ns: i64,
    /// Evidence value for the newest mark only. Freshness at evaluation is
    /// enforced per instrument through `valuation_as_of_ns_by_instrument`.
    pub valuations_as_of_ns: i64,
    /// Timestamp of every valuation mark used to derive this snapshot, keyed by
    /// instrument. Every entry is re-checked for freshness before the snapshot
    /// may feed the daily-loss limit; no single newest mark can mask a stale
    /// one.
    pub valuation_as_of_ns_by_instrument: BTreeMap<InstrumentId, i64>,
}

impl DailyPnlSnapshot {
    /// Exact signed P&L consumed by the daily-loss limit. Every addition and
    /// subtraction is checked; an overflow denies risk instead of wrapping.
    pub fn daily_limit_pnl_paise(&self) -> Result<Paise, DailyPnlError> {
        let mut total = Paise(self.realized_paise);
        if self.basis == DailyPnlBasis::RealizedPlusUnrealized {
            total = total
                .checked_add(Paise(self.unrealized_paise))
                .map_err(|_| DailyPnlError::Overflow)?;
        }
        total = total
            .checked_sub(Paise(self.fees_paise))
            .map_err(|_| DailyPnlError::Overflow)?;
        total = total
            .checked_add(Paise(self.funding_paise))
            .map_err(|_| DailyPnlError::Overflow)?;
        total = total
            .checked_add(Paise(self.fx_paise))
            .map_err(|_| DailyPnlError::Overflow)?;
        Ok(total)
    }
}

/// Owner-sealed P&L attestation. Its money fields are re-derived from the
/// account and valuation records before a snapshot is built; a mismatching
/// claim is rejected, not averaged or trusted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DailyPnlRecord {
    pub schema_version: u32,
    pub sequence: u64,
    pub prev_digest: String,
    pub digest: String,
    pub account_id: AccountId,
    pub business_date: TradeDate,
    pub base_currency: String,
    pub account_journal_sequence: u64,
    pub account_journal_digest: String,
    pub positions_digest: String,
    pub valuation_inputs_digest: String,
    pub calculation_version: String,
    pub calendar_digest: String,
    pub basis: DailyPnlBasis,
    pub realized_paise: i64,
    pub unrealized_paise: i64,
    pub fees_paise: i64,
    pub funding_paise: i64,
    pub fx_paise: i64,
    pub reconciliation_status: ReconciliationStatus,
    pub as_of_ns: i64,
    /// Digest of the verified account-journal prefix that precedes this
    /// business date. It binds the opening state so the daily result cannot
    /// silently absorb an earlier business date's effects.
    pub opening_checkpoint_digest: String,
}

impl DailyPnlRecord {
    /// Start a record with its identity and basis. Journal, valuation and
    /// money facts are attached with the explicit builder methods below.
    #[must_use]
    pub fn new(
        account_id: AccountId,
        business_date: TradeDate,
        base_currency: String,
        basis: DailyPnlBasis,
        calendar: &DailyPnlCalendar,
        as_of_ns: i64,
    ) -> Self {
        Self {
            schema_version: 1,
            sequence: 0,
            prev_digest: GENESIS_DIGEST.to_owned(),
            digest: String::new(),
            account_id,
            business_date,
            base_currency,
            account_journal_sequence: 0,
            account_journal_digest: String::new(),
            positions_digest: String::new(),
            valuation_inputs_digest: String::new(),
            calculation_version: DAILY_PNL_CALCULATION_VERSION.to_owned(),
            calendar_digest: calendar_digest(calendar),
            basis,
            realized_paise: 0,
            unrealized_paise: 0,
            fees_paise: 0,
            funding_paise: 0,
            fx_paise: 0,
            reconciliation_status: ReconciliationStatus::Matched,
            as_of_ns,
            opening_checkpoint_digest: GENESIS_DIGEST.to_owned(),
        }
    }

    /// Bind this record to the verified journal prefix that opens its business
    /// date. A genesis checkpoint means the period is the first in its journal.
    #[must_use]
    pub fn with_opening_checkpoint(mut self, digest: String) -> Self {
        self.opening_checkpoint_digest = digest;
        self
    }

    #[must_use]
    pub fn with_journal(mut self, sequence: u64, digest: String) -> Self {
        self.account_journal_sequence = sequence;
        self.account_journal_digest = digest;
        self
    }

    #[must_use]
    pub fn with_digests(mut self, positions: String, valuations: String) -> Self {
        self.positions_digest = positions;
        self.valuation_inputs_digest = valuations;
        self
    }

    #[must_use]
    pub fn with_components(
        mut self,
        realized: i64,
        unrealized: i64,
        fees: i64,
        funding: i64,
        fx: i64,
    ) -> Self {
        self.realized_paise = realized;
        self.unrealized_paise = unrealized;
        self.fees_paise = fees;
        self.funding_paise = funding;
        self.fx_paise = fx;
        self
    }

    #[must_use]
    pub fn with_reconciliation_status(mut self, status: ReconciliationStatus) -> Self {
        self.reconciliation_status = status;
        self
    }

    /// Assign the exact next sequence and seal the record to its predecessor.
    pub fn seal(&mut self, sequence: u64, prev_digest: &str) {
        let (sequence, previous, digest) = seal_fields(sequence, prev_digest, &self.preimage());
        self.sequence = sequence;
        self.prev_digest = previous;
        self.digest = digest;
    }
}

impl ChainedRecord for DailyPnlRecord {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }

    fn sequence(&self) -> u64 {
        self.sequence
    }

    fn prev_digest(&self) -> &str {
        &self.prev_digest
    }

    fn digest(&self) -> &str {
        &self.digest
    }

    fn preimage(&self) -> String {
        serde_json::json!({
            "schema_version": self.schema_version,
            "account_id": self.account_id,
            "business_date": self.business_date,
            "base_currency": self.base_currency,
            "account_journal_sequence": self.account_journal_sequence,
            "account_journal_digest": self.account_journal_digest,
            "positions_digest": self.positions_digest,
            "valuation_inputs_digest": self.valuation_inputs_digest,
            "calculation_version": self.calculation_version,
            "calendar_digest": self.calendar_digest,
            "basis": self.basis,
            "realized_paise": self.realized_paise,
            "unrealized_paise": self.unrealized_paise,
            "fees_paise": self.fees_paise,
            "funding_paise": self.funding_paise,
            "fx_paise": self.fx_paise,
            "reconciliation_status": self.reconciliation_status,
            "as_of_ns": self.as_of_ns,
            "opening_checkpoint_digest": self.opening_checkpoint_digest,
        })
        .to_string()
    }
}

/// One owner-sealed valuation input: an exact mark and per-unit cost basis for
/// the authoritative position quantity. Unrealized P&L is recomputed from
/// these facts, never copied from an observation claim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValuationRecord {
    pub schema_version: u32,
    pub sequence: u64,
    pub prev_digest: String,
    pub digest: String,
    pub account_id: AccountId,
    pub instrument_id: InstrumentId,
    pub quantity: i64,
    pub mark_paise: i64,
    pub cost_basis_paise: i64,
    pub as_of_ns: i64,
}

impl ValuationRecord {
    #[must_use]
    pub fn new(
        account_id: AccountId,
        instrument_id: InstrumentId,
        quantity: i64,
        mark_paise: i64,
        cost_basis_paise: i64,
        as_of_ns: i64,
    ) -> Self {
        Self {
            schema_version: 1,
            sequence: 0,
            prev_digest: GENESIS_DIGEST.to_owned(),
            digest: String::new(),
            account_id,
            instrument_id,
            quantity,
            mark_paise,
            cost_basis_paise,
            as_of_ns,
        }
    }

    /// Assign the exact next sequence and seal the record to its predecessor.
    pub fn seal(&mut self, sequence: u64, prev_digest: &str) {
        let (sequence, previous, digest) = seal_fields(sequence, prev_digest, &self.preimage());
        self.sequence = sequence;
        self.prev_digest = previous;
        self.digest = digest;
    }
}

impl ChainedRecord for ValuationRecord {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }

    fn sequence(&self) -> u64 {
        self.sequence
    }

    fn prev_digest(&self) -> &str {
        &self.prev_digest
    }

    fn digest(&self) -> &str {
        &self.digest
    }

    fn preimage(&self) -> String {
        serde_json::json!({
            "schema_version": self.schema_version,
            "account_id": self.account_id,
            "instrument_id": self.instrument_id,
            "quantity": self.quantity,
            "mark_paise": self.mark_paise,
            "cost_basis_paise": self.cost_basis_paise,
            "as_of_ns": self.as_of_ns,
        })
        .to_string()
    }
}

/// The sealed P&L evidence bundle required at startup. All three fields are
/// mandatory: recovery fails closed when any of them is absent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyPnlRecovery {
    pub record: Option<DailyPnlRecord>,
    pub valuations: Vec<ValuationRecord>,
    pub calendar: Option<DailyPnlCalendar>,
}

fn calendar_digest(calendar: &DailyPnlCalendar) -> String {
    serde_json::to_string(calendar)
        .map(|text| digest::sha256_hex(text.as_bytes()))
        .unwrap_or_default()
}

/// Deterministic digest over authoritative positions, exposed so owner-backed
/// fixtures can bind a P&L record to those positions.
#[must_use]
pub fn daily_pnl_positions_digest(positions: &BTreeMap<InstrumentId, i64>) -> String {
    serde_json::to_string(positions)
        .map(|text| digest::sha256_hex(text.as_bytes()))
        .unwrap_or_default()
}

fn chain_head<T: ChainedRecord>(records: &[T]) -> Result<String, DailyPnlError> {
    verify_recovery_chain(records).map_err(|error| {
        DailyPnlError::WrongJournal(format!("chain verification failed: {error}"))
    })?;
    Ok(records.last().map_or_else(
        || GENESIS_DIGEST.to_owned(),
        |record| record.digest().to_owned(),
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AccountPnl {
    realized: Paise,
    fees: Paise,
    funding: Paise,
    fx: Paise,
}

/// Sum only the declared business period's signed money effects from the
/// continuous, chain-verified account journal, and return the digest of the
/// verified opening prefix that precedes that period. No figure is read from
/// an owner summary: every component and every event timestamp is covered by
/// the journal's sequence and digest chain. Business-date assignment uses the
/// owner's half-open calendar window, never the authority's wall clock.
fn derive_account_pnl(
    records: &[AccountRecoveryRecord],
    expected_account: AccountId,
    calendar: &DailyPnlCalendar,
) -> Result<(AccountPnl, String), DailyPnlError> {
    let mut pnl = AccountPnl {
        realized: Paise(0),
        fees: Paise(0),
        funding: Paise(0),
        fx: Paise(0),
    };
    let mut opening_checkpoint = GENESIS_DIGEST.to_owned();
    // 0 = before the period, 1 = inside it, 2 = after it. A later record may
    // never be dated into an earlier phase, so the checkpoint is a true prefix.
    let mut phase = 0_u8;
    for record in records {
        if record.schema_version != 1 {
            return Err(DailyPnlError::UnsupportedSchema(record.schema_version));
        }
        if record.account_id != expected_account {
            return Err(DailyPnlError::WrongAccount);
        }
        if record.fees_paise < 0 {
            return Err(DailyPnlError::Contradictory(
                "account journal fees must be non-negative".to_owned(),
            ));
        }
        let record_phase = if record.event_at_ns < calendar.day_start_ns {
            0
        } else if record.event_at_ns < calendar.next_day_start_ns {
            1
        } else {
            2
        };
        if record_phase < phase {
            return Err(DailyPnlError::Contradictory(
                "account journal events are not ordered by business date".to_owned(),
            ));
        }
        phase = record_phase;
        if record_phase == 0 {
            opening_checkpoint = record.digest.clone();
            continue;
        }
        if record_phase == 2 {
            continue;
        }
        pnl.realized = pnl
            .realized
            .checked_add(Paise(record.realized_paise))
            .map_err(|_| DailyPnlError::Overflow)?;
        pnl.fees = pnl
            .fees
            .checked_add(Paise(record.fees_paise))
            .map_err(|_| DailyPnlError::Overflow)?;
        pnl.funding = pnl
            .funding
            .checked_add(Paise(record.funding_paise))
            .map_err(|_| DailyPnlError::Overflow)?;
        pnl.fx = pnl
            .fx
            .checked_add(Paise(record.fx_paise))
            .map_err(|_| DailyPnlError::Overflow)?;
    }
    Ok((pnl, opening_checkpoint))
}

fn check_pnl_freshness(
    as_of_ns: i64,
    now: LogicalNanos,
    max_age_ns: u64,
) -> Result<(), DailyPnlError> {
    let as_of = u64::try_from(as_of_ns).map_err(|_| DailyPnlError::Future)?;
    if as_of > now.get() {
        return Err(DailyPnlError::Future);
    }
    let age_ns = now.get() - as_of;
    if age_ns > max_age_ns {
        return Err(DailyPnlError::Stale { age_ns, max_age_ns });
    }
    Ok(())
}

/// Recompute unrealized P&L from sealed marks and per-unit cost bases. The
/// valuation set must cover exactly the non-zero authoritative positions:
/// missing, duplicate, foreign and contradictory marks all fail closed. Every
/// mark's timestamp is retained so later evaluation can recheck each one.
fn derive_valuations(
    records: &[ValuationRecord],
    expected_account: AccountId,
    positions: &BTreeMap<InstrumentId, i64>,
    now: LogicalNanos,
    max_age_ns: u64,
) -> Result<(Paise, String, i64, BTreeMap<InstrumentId, i64>), DailyPnlError> {
    let head = chain_head(records)?;
    let expected: BTreeSet<InstrumentId> = positions
        .iter()
        .filter(|(_, quantity)| **quantity != 0)
        .map(|(instrument, _)| instrument.clone())
        .collect();
    let mut actual = BTreeSet::new();
    let mut unrealized = Paise(0);
    let mut latest_as_of = 0_i64;
    let mut as_of_by_instrument = BTreeMap::new();
    for record in records {
        if record.schema_version != 1 {
            return Err(DailyPnlError::UnsupportedSchema(record.schema_version));
        }
        if record.account_id != expected_account {
            return Err(DailyPnlError::WrongAccount);
        }
        if !actual.insert(record.instrument_id.clone()) {
            return Err(DailyPnlError::WrongValuation(
                "duplicate valuation instrument".to_owned(),
            ));
        }
        let position = positions.get(&record.instrument_id).copied().unwrap_or(0);
        if record.quantity != position {
            return Err(DailyPnlError::WrongValuation(format!(
                "valuation quantity for {} disagrees with the account position",
                record.instrument_id
            )));
        }
        if record.mark_paise < 0 || record.cost_basis_paise < 0 {
            return Err(DailyPnlError::WrongValuation(
                "negative mark or cost basis".to_owned(),
            ));
        }
        check_pnl_freshness(record.as_of_ns, now, max_age_ns)?;
        as_of_by_instrument.insert(record.instrument_id.clone(), record.as_of_ns);
        latest_as_of = latest_as_of.max(record.as_of_ns);
        let per_unit = Paise(record.mark_paise)
            .checked_sub(Paise(record.cost_basis_paise))
            .map_err(|_| DailyPnlError::Overflow)?;
        unrealized = unrealized
            .checked_add(
                per_unit
                    .checked_scale(record.quantity)
                    .map_err(|_| DailyPnlError::Overflow)?,
            )
            .map_err(|_| DailyPnlError::Overflow)?;
    }
    if actual != expected {
        let missing: Vec<String> = expected
            .difference(&actual)
            .map(ToString::to_string)
            .collect();
        let foreign: Vec<String> = actual
            .difference(&expected)
            .map(ToString::to_string)
            .collect();
        return Err(DailyPnlError::WrongValuation(format!(
            "valuation coverage is not exact: missing [{}], foreign [{}]",
            missing.join(", "),
            foreign.join(", ")
        )));
    }
    Ok((unrealized, head, latest_as_of, as_of_by_instrument))
}

/// Build the only authoritative daily P&L snapshot: every money component and
/// every binding digest is either recomputed here or checked against the
/// sealed records and calendar before the snapshot may exist.
#[allow(clippy::too_many_arguments)] // every argument is a required binding or checked record input
fn derive_daily_pnl_snapshot(
    record: &DailyPnlRecord,
    accounts: &[AccountRecoveryRecord],
    valuations: &[ValuationRecord],
    calendar: &DailyPnlCalendar,
    positions: &BTreeMap<InstrumentId, i64>,
    expected_account: AccountId,
    expected_date: TradeDate,
    now: LogicalNanos,
    max_age_ns: u64,
) -> Result<DailyPnlSnapshot, DailyPnlError> {
    verify_recovery_chain(std::slice::from_ref(record))
        .map_err(|error| DailyPnlError::Contradictory(format!("P&L record: {error}")))?;
    if record.schema_version != 1 {
        return Err(DailyPnlError::UnsupportedSchema(record.schema_version));
    }
    if record.account_id != expected_account {
        return Err(DailyPnlError::WrongAccount);
    }
    if record.calculation_version != DAILY_PNL_CALCULATION_VERSION {
        return Err(DailyPnlError::UnsupportedCalculationVersion);
    }
    // Money in this authority is Paise. Until a neutral financial unit exists,
    // any currency other than INR is uninterpretable and denies risk.
    if record.base_currency != "INR" {
        return Err(DailyPnlError::Contradictory(format!(
            "unsupported base currency {} (only INR Paise is admitted)",
            record.base_currency
        )));
    }
    if calendar.time_zone.trim().is_empty() {
        return Err(DailyPnlError::InvalidCalendar(
            "time zone is not declared".to_owned(),
        ));
    }
    if calendar.day_start_ns >= calendar.next_day_start_ns {
        return Err(DailyPnlError::InvalidCalendar(
            "business-day window is empty".to_owned(),
        ));
    }
    if calendar.business_date != expected_date || record.business_date != expected_date {
        return Err(DailyPnlError::WrongBusinessDate);
    }
    if record.calendar_digest != calendar_digest(calendar) {
        return Err(DailyPnlError::InvalidCalendar(
            "calendar digest does not bind this record".to_owned(),
        ));
    }
    let at = i64::try_from(now.get()).map_err(|_| DailyPnlError::OutsideBusinessDay)?;
    if at < calendar.day_start_ns || at >= calendar.next_day_start_ns {
        return Err(DailyPnlError::OutsideBusinessDay);
    }
    check_pnl_freshness(record.as_of_ns, now, max_age_ns)?;

    let account_head = chain_head(accounts)?;
    let account_sequence = u64::try_from(accounts.len())
        .map_err(|_| DailyPnlError::WrongJournal("length".to_owned()))?;
    if record.account_journal_sequence != account_sequence
        || record.account_journal_digest != account_head
    {
        return Err(DailyPnlError::WrongJournal(
            "account journal sequence or digest disagrees".to_owned(),
        ));
    }
    let (account_pnl, opening_checkpoint) =
        derive_account_pnl(accounts, expected_account, calendar)?;
    if record.opening_checkpoint_digest != opening_checkpoint {
        return Err(DailyPnlError::WrongJournal(
            "opening checkpoint digest disagrees with the verified journal prefix".to_owned(),
        ));
    }
    let (unrealized, valuation_head, valuations_as_of, as_of_by_instrument) =
        derive_valuations(valuations, expected_account, positions, now, max_age_ns)?;
    if record.positions_digest != daily_pnl_positions_digest(positions) {
        return Err(DailyPnlError::WrongPositions);
    }
    if record.valuation_inputs_digest != valuation_head {
        return Err(DailyPnlError::WrongValuation(
            "valuation inputs digest disagrees".to_owned(),
        ));
    }
    let components = [
        ("realized", record.realized_paise, account_pnl.realized.0),
        ("unrealized", record.unrealized_paise, unrealized.0),
        ("fees", record.fees_paise, account_pnl.fees.0),
        ("funding", record.funding_paise, account_pnl.funding.0),
        ("fx", record.fx_paise, account_pnl.fx.0),
    ];
    for (name, claimed, derived) in components {
        if claimed != derived {
            return Err(DailyPnlError::Contradictory(format!(
                "{name} claim does not match sealed records"
            )));
        }
    }
    if record.reconciliation_status != ReconciliationStatus::Matched {
        return Err(DailyPnlError::WrongReconciliation);
    }

    Ok(DailyPnlSnapshot {
        schema_version: record.schema_version,
        account_id: record.account_id,
        business_date: record.business_date,
        base_currency: record.base_currency.clone(),
        account_journal_sequence: record.account_journal_sequence,
        account_journal_digest: record.account_journal_digest.clone(),
        positions_digest: record.positions_digest.clone(),
        valuation_inputs_digest: record.valuation_inputs_digest.clone(),
        calculation_version: record.calculation_version.clone(),
        calendar_digest: record.calendar_digest.clone(),
        basis: record.basis,
        calendar: calendar.clone(),
        realized_paise: record.realized_paise,
        unrealized_paise: record.unrealized_paise,
        fees_paise: record.fees_paise,
        funding_paise: record.funding_paise,
        fx_paise: record.fx_paise,
        reconciliation_status: record.reconciliation_status,
        as_of_ns: record.as_of_ns,
        valuations_as_of_ns: valuations_as_of,
        valuation_as_of_ns_by_instrument: as_of_by_instrument,
    })
}

/// The reconciled execution state of one partially filled permit. It keeps the
/// original authorization, the cumulative execution and the remaining leaves
/// quantity as separate facts; none of them is inferred from the other by
/// treating a fill as zero or as a whole order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionExposure {
    pub original_quantity: i64,
    pub filled_quantity: i64,
    pub leaves_quantity: i64,
    pub filled_notional_paise: Paise,
    pub average_fill_price_paise: Option<Paise>,
    pub fees_paise: Paise,
    pub costs_paise: Paise,
    pub execution_ids: Vec<String>,
}

impl ExecutionExposure {
    fn new(
        original_quantity: i64,
        filled_quantity: i64,
        leaves_quantity: i64,
        filled_notional_paise: Paise,
        fees_paise: Paise,
        costs_paise: Paise,
        execution_ids: Vec<String>,
    ) -> Result<Self, RecoveryError> {
        let average_fill_price_paise = if filled_quantity > 0 && filled_notional_paise.0 > 0 {
            Some(Paise(filled_notional_paise.0 / filled_quantity))
        } else if filled_notional_paise.0 < 0 {
            return Err(RecoveryError::Inconsistent(
                "a fill has negative notional".to_owned(),
            ));
        } else {
            // A reported fill without a price stays explicitly unpriced.
            None
        };
        Ok(Self {
            original_quantity,
            filled_quantity,
            leaves_quantity,
            filled_notional_paise,
            average_fill_price_paise,
            fees_paise,
            costs_paise,
            execution_ids,
        })
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
    /// Set only by the explicitly named simulation constructor.
    pub simulation: bool,
    /// Number of consumed permits whose terminal OMS outcome is still unknown.
    pub unresolved_permits: usize,
    /// Durable authored reason for the current reconciliation latch.
    pub reconciliation_reason: Option<String>,
    /// Durable evidence reference for the current reconciliation latch.
    pub reconciliation_evidence_ref: Option<String>,
    /// Whether an authoritative daily P&L snapshot has been established.
    pub daily_pnl_available: bool,
    /// The explicit basis used by the daily-loss limit, if a snapshot exists.
    pub daily_pnl_basis: Option<DailyPnlBasis>,
    /// The exact signed P&L the daily-loss limit evaluates, if established.
    pub daily_pnl_limit_paise: Option<Paise>,
    /// Account-journal head bound to the established P&L snapshot.
    pub daily_pnl_journal_digest: Option<String>,
    /// Account-journal sequence bound to the established P&L snapshot.
    pub daily_pnl_journal_sequence: Option<u64>,
    /// Reconciled execution state of permits that still have leaves quantity.
    pub executions: BTreeMap<IntentId, ExecutionExposure>,
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
    /// Immutable, owner-derived P&L evidence. `None` denies new risk.
    daily_pnl: Option<DailyPnlSnapshot>,
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
    /// Permits consumed by the OMS whose terminal outcome is not yet known.
    unresolved: BTreeSet<IntentId>,
    /// Reconciled partial execution state of still-open permits.
    executions: BTreeMap<IntentId, ExecutionExposure>,
    next_permit: u64,
    /// The last authored, durably persisted reconciliation decision.
    reconciliation_decision: Option<ReconciliationState>,
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
            daily_pnl: None,
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
            unresolved: BTreeSet::new(),
            executions: BTreeMap::new(),
            next_permit: 1,
            reconciliation_decision: None,
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
    decision_store: Option<Box<dyn DecisionStore + Send + Sync>>,
    /// Exclusive ownership of the reconstructed state, held for this value's
    /// lifetime. A second startup cannot acquire it while this one is live.
    _ownership: Option<AuthorityOwnership>,
    /// An explicit, non-silent startup failure. New risk is denied until the
    /// owning operator repairs the store and restarts.
    startup_error: Option<RecoveryError>,
    simulation: bool,
    state: Mutex<AuthorityState>,
}

struct BuildOptions {
    control: Box<dyn ControlStore + Send + Sync>,
    decision_store: Option<Box<dyn DecisionStore + Send + Sync>>,
    ownership: Option<AuthorityOwnership>,
    startup_error: Option<RecoveryError>,
    simulation: bool,
    state: AuthorityState,
}

impl RiskAuthority {
    fn build(
        account_id: AccountId,
        policy: ValidatedRiskPolicy,
        instruments: Vec<Instrument>,
        trade_date: TradeDate,
        options: BuildOptions,
    ) -> Self {
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
            control: options.control,
            decision_store: options.decision_store,
            _ownership: options.ownership,
            startup_error: options.startup_error,
            simulation: options.simulation,
            state: Mutex::new(options.state),
        }
    }

    /// Explicit simulation/test constructor. It holds no durable state and must
    /// never be used as production startup wiring.
    pub fn new_simulation(
        account_id: AccountId,
        policy: ValidatedRiskPolicy,
        instruments: Vec<Instrument>,
        trade_date: TradeDate,
    ) -> Self {
        Self::build(
            account_id,
            policy,
            instruments,
            trade_date,
            BuildOptions {
                control: Box::new(NoopControlStore),
                decision_store: None,
                ownership: None,
                startup_error: None,
                simulation: true,
                state: AuthorityState::default(),
            },
        )
    }

    /// Alias for [`Self::new_simulation`], retained for existing callers. It is
    /// not a durable startup path.
    pub fn new(
        account_id: AccountId,
        policy: ValidatedRiskPolicy,
        instruments: Vec<Instrument>,
        trade_date: TradeDate,
    ) -> Self {
        Self::new_simulation(account_id, policy, instruments, trade_date)
    }

    /// Build an authority that loads and persists control state through
    /// `control`. An absent control record fails closed; an unreadable record
    /// is an explicit startup error that denies new risk. This constructor does
    /// not reconstruct risk state: production startup must use
    /// [`Self::recover`].
    pub fn new_with_control_store(
        account_id: AccountId,
        policy: ValidatedRiskPolicy,
        instruments: Vec<Instrument>,
        trade_date: TradeDate,
        control: Box<dyn ControlStore + Send + Sync>,
    ) -> Self {
        let mut state = AuthorityState::default();
        let mut startup_error = None;
        match control.load_kill_switch() {
            Ok(Some(record)) => state.kill_switch_engaged = record.enabled,
            Ok(None) => {}
            Err(error) => {
                startup_error = Some(RecoveryError::Control(error.to_string()));
            }
        }
        match control.load_reconciliation() {
            Ok(Some(record)) => {
                state.reconciliation_decision = Some(record.clone());
                state.reconciliation_blocked = record.blocked;
            }
            Ok(None) => {}
            Err(error) => {
                if startup_error.is_none() {
                    startup_error = Some(RecoveryError::Control(error.to_string()));
                }
            }
        }
        Self::build(
            account_id,
            policy,
            instruments,
            trade_date,
            BuildOptions {
                control,
                decision_store: None,
                ownership: None,
                startup_error,
                simulation: false,
                state,
            },
        )
    }

    /// Whether this authority was built by the explicit simulation constructor.
    #[must_use]
    pub fn is_simulation(&self) -> bool {
        self.simulation
    }

    /// The explicit startup failure, if any. `None` means the control store was
    /// readable and startup reconstruction succeeded.
    #[must_use]
    pub fn startup_error(&self) -> Option<RecoveryError> {
        self.startup_error.clone()
    }

    /// The last durably authored reconciliation decision, including its reason
    /// and evidence reference.
    #[must_use]
    pub fn reconciliation_decision(&self) -> Option<ReconciliationState> {
        self.lock()
            .ok()
            .and_then(|state| state.reconciliation_decision.clone())
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
        // The control stream owns its own exact revision sequence. Deriving it
        // from the stream head prevents an unrelated authority mutation from
        // leaving a gap that a strict load would reject on restart.
        let store_revision = match self
            .control
            .load_kill_switch()
            .map_err(|error| AuthorityError::ControlFailure(error.to_string()))?
        {
            Some(head) => next_revision(head.revision)?,
            None => 1,
        };
        let record = KillSwitchState {
            enabled: engaged,
            actor_id,
            reason,
            revision: store_revision,
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
        if reason.trim().is_empty() {
            return Err(AuthorityError::MissingEvidence {
                action: "reconciliation_decision".to_owned(),
            });
        }
        let unchanged = state.reconciliation_blocked == blocked
            && state
                .reconciliation_decision
                .as_ref()
                .is_some_and(|prior| prior.reason == reason && prior.evidence_ref == evidence_ref);
        if unchanged {
            return Ok(());
        }
        let revision = next_revision(state.revision)?;
        let store_revision = match self
            .control
            .load_reconciliation()
            .map_err(|error| AuthorityError::ControlFailure(error.to_string()))?
        {
            Some(head) => next_revision(head.revision)?,
            None => 1,
        };
        let record = ReconciliationState::new(
            blocked,
            reason,
            evidence_ref,
            store_revision,
            i64::try_from(state.now.get()).unwrap_or(i64::MAX),
        );
        self.control
            .append_reconciliation(&record)
            .map_err(|error| AuthorityError::ControlFailure(error.to_string()))?;
        let mut candidate = state.clone();
        candidate.reconciliation_blocked = blocked;
        candidate.reconciliation_decision = Some(record);
        candidate.revision = revision;
        *state = candidate;
        Ok(())
    }

    /// Replace the daily P&L snapshot from sealed owner records. There is no
    /// unauthenticated P&L setter: the caller must hold the registered account
    /// journal owner's token, and every figure is re-derived from the supplied
    /// sealed records before anything is committed.
    pub fn publish_daily_pnl(
        &self,
        token: OwnerToken,
        record: DailyPnlRecord,
        accounts: &[AccountRecoveryRecord],
        valuations: &[ValuationRecord],
        calendar: &DailyPnlCalendar,
    ) -> Result<DailyPnlSnapshot, DailyPnlError> {
        let mut state = self.lock().map_err(|_| DailyPnlError::StatePoisoned)?;
        self.check_owner_binding(&state, token, ObservationSource::AccountJournal)
            .map_err(|_| DailyPnlError::Unauthorised)?;
        let snapshot = derive_daily_pnl_snapshot(
            &record,
            accounts,
            valuations,
            calendar,
            &state.positions,
            self.account_id,
            self.trade_date,
            state.now,
            self.observation_policy.max_account_journal_age_ns,
        )?;
        let mut candidate = state.clone();
        candidate.daily_pnl = Some(snapshot.clone());
        candidate
            .bump_revision()
            .map_err(|_| DailyPnlError::Contradictory("daily pnl revision exhausted".to_owned()))?;
        *state = candidate;
        Ok(snapshot)
    }

    /// Re-validate the established snapshot against the same observations and
    /// clock used by the reservation. Any contradiction denies new risk; no
    /// missing or stale value ever resolves to zero.
    fn validate_daily_pnl_for_evaluation(
        &self,
        state: &AuthorityState,
        observations: &Observations,
    ) -> Result<Paise, DailyPnlError> {
        let snapshot = state.daily_pnl.as_ref().ok_or(DailyPnlError::Missing)?;
        if snapshot.account_id != self.account_id {
            return Err(DailyPnlError::WrongAccount);
        }
        let journal = observations
            .account_journal
            .as_ref()
            .ok_or_else(|| DailyPnlError::WrongJournal("observation missing".to_owned()))?;
        if journal.payload.head_hash_hex != snapshot.account_journal_digest
            || journal.payload.record_count != snapshot.account_journal_sequence
        {
            return Err(DailyPnlError::WrongJournal(
                "journal head or sequence disagrees with the snapshot".to_owned(),
            ));
        }
        let reconciliation = observations
            .reconciliation
            .as_ref()
            .ok_or(DailyPnlError::WrongReconciliation)?;
        if !reconciliation.payload.replay_digest_matched
            || reconciliation.payload.report.status != ReconciliationStatus::Matched
            || snapshot.reconciliation_status != reconciliation.payload.report.status
        {
            return Err(DailyPnlError::WrongReconciliation);
        }
        if snapshot.business_date != self.trade_date
            || snapshot.calendar.business_date != self.trade_date
        {
            return Err(DailyPnlError::WrongBusinessDate);
        }
        let at = i64::try_from(state.now.get()).map_err(|_| DailyPnlError::OutsideBusinessDay)?;
        if at < snapshot.calendar.day_start_ns || at >= snapshot.calendar.next_day_start_ns {
            return Err(DailyPnlError::OutsideBusinessDay);
        }
        let max_age_ns = self.observation_policy.max_account_journal_age_ns;
        check_pnl_freshness(snapshot.as_of_ns, state.now, max_age_ns)?;
        // The newest mark is only an evidence value. Every retained per-
        // instrument timestamp must be current, so a stale older mark cannot
        // hide behind a fresh one.
        for (instrument, &as_of_ns) in &snapshot.valuation_as_of_ns_by_instrument {
            check_pnl_freshness(as_of_ns, state.now, max_age_ns)
                .map_err(|error| DailyPnlError::WrongValuation(format!("{instrument}: {error}")))?;
        }
        check_pnl_freshness(snapshot.valuations_as_of_ns, state.now, max_age_ns)?;
        snapshot.daily_limit_pnl_paise()
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
        if let Some(error) = &self.startup_error {
            return Err(AuthorityError::ControlFailure(format!(
                "startup failed: {error}"
            )));
        }
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

        let daily_pnl = self
            .validate_daily_pnl_for_evaluation(&state, &observations)
            .map_err(|error| AuthorityError::ControlFailure(format!("daily pnl: {error}")))?;
        let context = self.build_context(&state, &intent, &observations, daily_pnl)?;
        let assessment = evaluate_risk(&intent, &self.policy, &context);
        if !assessment.accepted {
            // Remembering a rejection is a state change in its own right and
            // must invalidate any fence taken before it.
            let mut candidate = state.clone();
            candidate.decided.insert(intent.intent_id, intent.clone());
            candidate.bump_revision()?;
            self.persist_decision(&candidate, &intent, DecisionOutcome::Rejected, 0, 0)?;
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
        daily_pnl: Paise,
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
            daily_pnl,
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
        self.persist_decision(
            &candidate,
            intent,
            DecisionOutcome::Accepted,
            permit_id,
            notional_paise.0,
        )?;
        *state = candidate;
        Ok(reservation)
    }

    /// Append the authority-owned decision record before committing the state.
    /// A store failure therefore leaves both the durable stream and the in-memory
    /// state unchanged.
    fn persist_decision(
        &self,
        state: &AuthorityState,
        intent: &OrderIntent,
        outcome: DecisionOutcome,
        permit_id: u64,
        notional_paise: i64,
    ) -> Result<(), AuthorityError> {
        let Some(store) = self.decision_store.as_ref() else {
            return Ok(());
        };
        let at_ns = i64::try_from(state.now.get()).unwrap_or(i64::MAX);
        let mut record = RiskDecisionRecord::new(
            intent.clone(),
            outcome,
            permit_id,
            notional_paise,
            state.revision,
            at_ns,
        );
        record.policy_version = self.policy.version.clone();
        store
            .append_decision(&record)
            .map_err(|error| AuthorityError::ControlFailure(format!("decision store: {error}")))?;
        Ok(())
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
        candidate.executions.remove(&intent_id);
        let intent = candidate
            .decided
            .get(&intent_id)
            .cloned()
            .ok_or(AuthorityError::StatePoisoned)?;
        self.persist_decision(
            &candidate,
            &intent,
            DecisionOutcome::Released,
            reservation.permit_id,
            reservation.reserved_notional_paise.0,
        )?;
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
        candidate.executions.remove(&intent_id);
        let intent = candidate
            .decided
            .get(&intent_id)
            .cloned()
            .ok_or(AuthorityError::StatePoisoned)?;
        self.persist_decision(
            &candidate,
            &intent,
            DecisionOutcome::Filled,
            reservation.permit_id,
            reservation.reserved_notional_paise.0,
        )?;
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
            simulation: self.simulation,
            unresolved_permits: state.unresolved.len(),
            reconciliation_reason: state
                .reconciliation_decision
                .as_ref()
                .map(|decision| decision.reason.clone()),
            reconciliation_evidence_ref: state
                .reconciliation_decision
                .as_ref()
                .and_then(|decision| decision.evidence_ref.clone()),
            daily_pnl_available: state.daily_pnl.is_some(),
            daily_pnl_basis: state.daily_pnl.as_ref().map(|pnl| pnl.basis),
            daily_pnl_limit_paise: state
                .daily_pnl
                .as_ref()
                .and_then(|pnl| pnl.daily_limit_pnl_paise().ok()),
            daily_pnl_journal_digest: state
                .daily_pnl
                .as_ref()
                .map(|pnl| pnl.account_journal_digest.clone()),
            daily_pnl_journal_sequence: state
                .daily_pnl
                .as_ref()
                .map(|pnl| pnl.account_journal_sequence),
            executions: state.executions.clone(),
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

// ---------------------------------------------------------------------------
// Startup reconstruction
// ---------------------------------------------------------------------------

/// Digest used for continuity of authority-owned and owner-identified recovery
/// records. It is a dependency-free SHA-256 over canonical JSON; it is an
/// integrity chain, not a signature or a source of authority by itself.
mod digest {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    fn sha256(bytes: &[u8]) -> [u8; 32] {
        let mut message = bytes.to_vec();
        let bit_len = (message.len() as u64).wrapping_mul(8);
        message.push(0x80);
        while message.len() % 64 != 56 {
            message.push(0);
        }
        message.extend_from_slice(&bit_len.to_be_bytes());
        let mut state = [
            0x6a09e667_u32,
            0xbb67ae85,
            0x3c6ef372,
            0xa54ff53a,
            0x510e527f,
            0x9b05688c,
            0x1f83d9ab,
            0x5be0cd19,
        ];
        for block in message.as_chunks::<64>().0 {
            let mut words = [0_u32; 64];
            for (index, word) in words.iter_mut().take(16).enumerate() {
                let offset = index * 4;
                *word = u32::from_be_bytes([
                    block[offset],
                    block[offset + 1],
                    block[offset + 2],
                    block[offset + 3],
                ]);
            }
            for index in 16..64 {
                let left = words[index - 15];
                let right = words[index - 2];
                let sigma0 = left.rotate_right(7) ^ left.rotate_right(18) ^ (left >> 3);
                let sigma1 = right.rotate_right(17) ^ right.rotate_right(19) ^ (right >> 10);
                words[index] = words[index - 16]
                    .wrapping_add(sigma0)
                    .wrapping_add(words[index - 7])
                    .wrapping_add(sigma1);
            }
            let (mut a, mut b, mut c, mut d) = (state[0], state[1], state[2], state[3]);
            let (mut e, mut f, mut g, mut h) = (state[4], state[5], state[6], state[7]);
            for index in 0..64 {
                let sigma1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let choice = (e & f) ^ ((!e) & g);
                let temp1 = h
                    .wrapping_add(sigma1)
                    .wrapping_add(choice)
                    .wrapping_add(K[index])
                    .wrapping_add(words[index]);
                let sigma0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let majority = (a & b) ^ (a & c) ^ (b & c);
                let temp2 = sigma0.wrapping_add(majority);
                h = g;
                g = f;
                f = e;
                e = d.wrapping_add(temp1);
                d = c;
                c = b;
                b = a;
                a = temp1.wrapping_add(temp2);
            }
            state[0] = state[0].wrapping_add(a);
            state[1] = state[1].wrapping_add(b);
            state[2] = state[2].wrapping_add(c);
            state[3] = state[3].wrapping_add(d);
            state[4] = state[4].wrapping_add(e);
            state[5] = state[5].wrapping_add(f);
            state[6] = state[6].wrapping_add(g);
            state[7] = state[7].wrapping_add(h);
        }
        let mut output = [0_u8; 32];
        for (chunk, word) in output.as_chunks_mut::<4>().0.iter_mut().zip(state) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        output
    }

    pub fn sha256_hex(bytes: &[u8]) -> String {
        let mut output = String::with_capacity(64);
        for byte in sha256(bytes) {
            output.push_str(&format!("{byte:02x}"));
        }
        output
    }

    /// Domain-separated HMAC-SHA-256. An unkeyed chain digest cannot stand in
    /// for an owner signature: without the key an attacker cannot forge a
    /// binding over a different identity, head or schema.
    pub fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
        const BLOCK: usize = 64;
        let mut ipad = [0x36_u8; BLOCK];
        let mut opad = [0x5c_u8; BLOCK];
        if key.len() > BLOCK {
            let digest = sha256(key);
            for (slot, byte) in ipad.iter_mut().zip(digest.iter()) {
                *slot ^= byte;
            }
            for (slot, byte) in opad.iter_mut().zip(digest.iter()) {
                *slot ^= byte;
            }
        } else {
            for (index, byte) in key.iter().enumerate() {
                ipad[index] ^= byte;
                opad[index] ^= byte;
            }
        }
        let mut inner = Vec::with_capacity(BLOCK.saturating_add(message.len()));
        inner.extend_from_slice(&ipad);
        inner.extend_from_slice(message);
        let inner_digest = sha256(&inner);
        let mut outer = Vec::with_capacity(BLOCK.saturating_add(inner_digest.len()));
        outer.extend_from_slice(&opad);
        outer.extend_from_slice(&inner_digest);
        sha256_hex(&outer)
    }

    /// Compare two fixed-width hex signatures without an early exit.
    pub fn constant_time_eq(left: &str, right: &str) -> bool {
        let left = left.as_bytes();
        let right = right.as_bytes();
        if left.len() != right.len() {
            return false;
        }
        let mut difference = 0_u8;
        for (a, b) in left.iter().zip(right) {
            difference |= a ^ b;
        }
        difference == 0
    }
}

/// Digest of an empty recovery chain.
pub const GENESIS_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Digest of an empty recovery chain.
#[must_use]
pub fn genesis_digest() -> &'static str {
    GENESIS_DIGEST
}

/// A recovery record whose position in its stream and link to its predecessor
/// can be independently verified.
pub trait ChainedRecord {
    fn schema_version(&self) -> u32;
    fn sequence(&self) -> u64;
    fn prev_digest(&self) -> &str;
    fn digest(&self) -> &str;
    fn preimage(&self) -> String;
}

fn seal_fields(sequence: u64, prev_digest: &str, preimage: &str) -> (u64, String, String) {
    let digest = digest::sha256_hex(format!("{preimage}\0{prev_digest}").as_bytes());
    (sequence, prev_digest.to_owned(), digest)
}

/// Verify exact sequence, predecessor link and self digest of every record.
pub fn verify_recovery_chain<T: ChainedRecord>(records: &[T]) -> Result<(), RecoveryError> {
    let mut previous = GENESIS_DIGEST.to_owned();
    for (index, record) in records.iter().enumerate() {
        let expected = u64::try_from(index)
            .map_err(|_| RecoveryError::Sequence("record index overflow".to_owned()))?
            + 1;
        if record.sequence() != expected {
            return Err(RecoveryError::Sequence(format!(
                "expected sequence {expected}, found {}",
                record.sequence()
            )));
        }
        if record.prev_digest() != previous {
            return Err(RecoveryError::Digest(format!(
                "record {expected} links to {} instead of {previous}",
                record.prev_digest()
            )));
        }
        let computed = digest::sha256_hex(format!("{}\0{previous}", record.preimage()).as_bytes());
        if computed != record.digest() {
            return Err(RecoveryError::Digest(format!(
                "record {expected} digest does not match its content"
            )));
        }
        previous = record.digest().to_owned();
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RecoveryError {
    #[error("control store unavailable during startup: {0}")]
    Control(String),
    #[error("risk decision store unavailable during startup: {0}")]
    DecisionStore(String),
    #[error("exclusive authority state ownership unavailable: {0}")]
    StateOwnership(String),
    #[error("required recovery journal is missing: {0}")]
    JournalMissing(PathBuf),
    #[error("recovery journal sequence is not contiguous: {0}")]
    Sequence(String),
    #[error("recovery journal digest continuity failed: {0}")]
    Digest(String),
    #[error("recovery journal authenticity check failed: {0}")]
    Authenticity(String),
    #[error("derived risk state disagrees with identified records: {0}")]
    Inconsistent(String),
    #[error("verified recovery snapshot was rejected: {0}")]
    Snapshot(String),
    #[error("startup recovery requires a durable control and decision store")]
    NonDurableStore,
    #[error("startup recovery cannot establish authoritative daily pnl: {0}")]
    DailyPnl(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DecisionStoreError {
    #[error("risk decision store io failure: {0}")]
    Io(String),
    #[error("risk decision store serialization failure: {0}")]
    Serialize(String),
    #[error("risk decision store record is corrupt: {0}")]
    Corrupt(String),
}

/// How the authority resolved one order intent. This is the durable fact that
/// makes permit issuance, rejection, release and consumption determinable after
/// restart; the OMS journal independently supplies the terminal order outcome.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcome {
    Accepted,
    Rejected,
    Released,
    Filled,
}

/// Terminal outcome identified from the verified OMS journal for a consumed
/// permit. `Open` still holds a pending order; `Unknown` latches reconciliation.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OmsPermitState {
    Open,
    Filled,
    Cancelled,
    Unknown,
}

/// One authority-authored decision. It is appended durably before the in-memory
/// transition is committed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskDecisionRecord {
    pub schema_version: u32,
    pub sequence: u64,
    pub prev_digest: String,
    pub digest: String,
    pub revision: u64,
    pub permit_id: u64,
    pub policy_version: String,
    pub intent_id: IntentId,
    pub intent: OrderIntent,
    pub outcome: DecisionOutcome,
    pub notional_paise: i64,
    pub at_ns: i64,
}

impl RiskDecisionRecord {
    #[must_use]
    pub fn new(
        intent: OrderIntent,
        outcome: DecisionOutcome,
        permit_id: u64,
        notional_paise: i64,
        revision: u64,
        at_ns: i64,
    ) -> Self {
        Self {
            schema_version: 1,
            sequence: 0,
            prev_digest: GENESIS_DIGEST.to_owned(),
            digest: String::new(),
            revision,
            permit_id,
            policy_version: String::new(),
            intent_id: intent.intent_id,
            intent,
            outcome,
            notional_paise,
            at_ns,
        }
    }

    /// Assign the exact next sequence and seal the record to its predecessor.
    pub fn seal(&mut self, sequence: u64, prev_digest: &str) {
        let (sequence, previous, digest) = seal_fields(sequence, prev_digest, &self.preimage());
        self.sequence = sequence;
        self.prev_digest = previous;
        self.digest = digest;
    }
}

impl ChainedRecord for RiskDecisionRecord {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }

    fn sequence(&self) -> u64 {
        self.sequence
    }

    fn prev_digest(&self) -> &str {
        &self.prev_digest
    }

    fn digest(&self) -> &str {
        &self.digest
    }

    fn preimage(&self) -> String {
        serde_json::json!({
            "schema_version": self.schema_version,
            "revision": self.revision,
            "permit_id": self.permit_id,
            "policy_version": self.policy_version,
            "intent_id": self.intent_id,
            "intent": self.intent,
            "outcome": self.outcome,
            "notional_paise": self.notional_paise,
            "at_ns": self.at_ns,
        })
        .to_string()
    }
}

/// An identified OMS-journal fact about one consumed permit. The integration
/// owner derives it from the verified OMS journal; the authority re-verifies its
/// chain and the permit binding before using it for reconstruction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OmsPermitRecord {
    pub schema_version: u32,
    pub sequence: u64,
    pub prev_digest: String,
    pub digest: String,
    pub permit_id: u64,
    pub intent_id: IntentId,
    pub permit_digest: String,
    pub state: OmsPermitState,
    /// FIX `CumQty`: cumulative executed quantity, never a per-report delta.
    pub filled_quantity: i64,
    /// Cumulative executed notional (paise), including price improvement.
    pub filled_notional: i64,
    /// Reported FIX `LeavesQty`. Absent means the owner did not report it, in
    /// which case recovery derives it from `OrderQty - CumQty`. Terminal states
    /// are exempt from the `OrderQty = CumQty + LeavesQty` requirement.
    #[serde(default)]
    pub leaves_quantity: Option<i64>,
}

impl OmsPermitRecord {
    #[must_use]
    pub fn new(
        permit_id: u64,
        intent_id: IntentId,
        permit_digest: String,
        state: OmsPermitState,
        filled_quantity: i64,
        filled_notional: i64,
    ) -> Self {
        Self {
            schema_version: 1,
            sequence: 0,
            prev_digest: GENESIS_DIGEST.to_owned(),
            digest: String::new(),
            permit_id,
            intent_id,
            permit_digest,
            state,
            filled_quantity,
            filled_notional,
            leaves_quantity: None,
        }
    }

    /// Bind the reported FIX `LeavesQty` to the cumulative execution state.
    #[must_use]
    pub fn with_leaves_quantity(mut self, leaves_quantity: i64) -> Self {
        self.leaves_quantity = Some(leaves_quantity);
        self
    }

    /// Assign the exact next sequence and seal the record to its predecessor.
    pub fn seal(&mut self, sequence: u64, prev_digest: &str) {
        let (sequence, previous, digest) = seal_fields(sequence, prev_digest, &self.preimage());
        self.sequence = sequence;
        self.prev_digest = previous;
        self.digest = digest;
    }
}

impl ChainedRecord for OmsPermitRecord {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }

    fn sequence(&self) -> u64 {
        self.sequence
    }

    fn prev_digest(&self) -> &str {
        &self.prev_digest
    }

    fn digest(&self) -> &str {
        &self.digest
    }

    fn preimage(&self) -> String {
        serde_json::json!({
            "schema_version": self.schema_version,
            "permit_id": self.permit_id,
            "intent_id": self.intent_id,
            "permit_digest": self.permit_digest,
            "state": self.state,
            "filled_quantity": self.filled_quantity,
            "filled_notional": self.filled_notional,
            "leaves_quantity": self.leaves_quantity,
        })
        .to_string()
    }
}

/// How one OMS execution event changed an order. Corrections and trade cancels
/// reference the execution they supersede; they never invent a new fill.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionEventKind {
    /// A new cumulative execution (trade) report.
    Fill,
    /// A correction that replaces the terms of a previously reported execution.
    Correct,
    /// A trade cancel that removes a previously reported execution.
    TradeCancel,
}

/// One owner-sealed execution-level fact for a consumed permit. The authority
/// reconciles the cumulative FIX fields of these reports into a single order
/// state: duplicate execution identities are idempotent, reports may arrive in
/// any order, and corrections/trade cancels refer to the execution they change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRecoveryRecord {
    pub schema_version: u32,
    pub sequence: u64,
    pub prev_digest: String,
    pub digest: String,
    pub permit_id: u64,
    pub intent_id: IntentId,
    /// Stable execution identity used for deduplication and correction binding.
    pub execution_id: String,
    /// Owner-supplied report order; recovery sorts by it, never by input order.
    pub report_sequence: u64,
    pub kind: ExecutionEventKind,
    /// Execution identity superseded by a correction or trade cancel.
    pub corrects: Option<String>,
    /// FIX `OrderQty`: authorized quantity after any order correction.
    pub order_quantity: i64,
    /// FIX `CumQty`: cumulative executed quantity after this report.
    pub cumulative_filled_quantity: i64,
    /// FIX `LeavesQty`: quantity still working after this report.
    pub leaves_quantity: i64,
    /// Cumulative executed notional after this report (paise).
    pub cumulative_notional_paise: i64,
    /// Cumulative fees after this report (non-negative paise).
    pub fees_paise: i64,
    /// Cumulative other transaction costs after this report (non-negative paise).
    pub costs_paise: i64,
}

impl ExecutionRecoveryRecord {
    #[must_use]
    #[allow(clippy::too_many_arguments)] // every argument is a required execution fact
    pub fn new(
        permit_id: u64,
        intent_id: IntentId,
        execution_id: String,
        report_sequence: u64,
        kind: ExecutionEventKind,
        corrects: Option<String>,
        order_quantity: i64,
        cumulative_filled_quantity: i64,
        leaves_quantity: i64,
        cumulative_notional_paise: i64,
        fees_paise: i64,
        costs_paise: i64,
    ) -> Self {
        Self {
            schema_version: 1,
            sequence: 0,
            prev_digest: GENESIS_DIGEST.to_owned(),
            digest: String::new(),
            permit_id,
            intent_id,
            execution_id,
            report_sequence,
            kind,
            corrects,
            order_quantity,
            cumulative_filled_quantity,
            leaves_quantity,
            cumulative_notional_paise,
            fees_paise,
            costs_paise,
        }
    }

    /// Assign the exact next sequence and seal the record to its predecessor.
    pub fn seal(&mut self, sequence: u64, prev_digest: &str) {
        let (sequence, previous, digest) = seal_fields(sequence, prev_digest, &self.preimage());
        self.sequence = sequence;
        self.prev_digest = previous;
        self.digest = digest;
    }
}

impl ChainedRecord for ExecutionRecoveryRecord {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }

    fn sequence(&self) -> u64 {
        self.sequence
    }

    fn prev_digest(&self) -> &str {
        &self.prev_digest
    }

    fn digest(&self) -> &str {
        &self.digest
    }

    fn preimage(&self) -> String {
        serde_json::json!({
            "schema_version": self.schema_version,
            "permit_id": self.permit_id,
            "intent_id": self.intent_id,
            "execution_id": self.execution_id,
            "report_sequence": self.report_sequence,
            "kind": self.kind,
            "corrects": self.corrects,
            "order_quantity": self.order_quantity,
            "cumulative_filled_quantity": self.cumulative_filled_quantity,
            "leaves_quantity": self.leaves_quantity,
            "cumulative_notional_paise": self.cumulative_notional_paise,
            "fees_paise": self.fees_paise,
            "costs_paise": self.costs_paise,
        })
        .to_string()
    }
}

/// An identified account-journal position delta used to cross-check the
/// positions derived from risk decisions and the OMS journal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountRecoveryRecord {
    pub schema_version: u32,
    pub sequence: u64,
    pub prev_digest: String,
    pub digest: String,
    pub event_id: String,
    pub account_id: AccountId,
    pub instrument_id: InstrumentId,
    pub quantity_delta: i64,
    /// Realized P&L booked by this journal event (signed paise).
    pub realized_paise: i64,
    /// Fees charged by this event (non-negative paise, subtracted from P&L).
    pub fees_paise: i64,
    /// Funding effect of this event (signed paise).
    pub funding_paise: i64,
    /// FX effect of this event (signed paise).
    pub fx_paise: i64,
    /// Logical instant at which this event was booked. The authority assigns
    /// it to a business date with the owner's declared calendar and sums only
    /// that period's effects.
    pub event_at_ns: i64,
}

impl AccountRecoveryRecord {
    #[must_use]
    pub fn new(
        event_id: String,
        account_id: AccountId,
        instrument_id: InstrumentId,
        quantity_delta: i64,
    ) -> Self {
        Self {
            schema_version: 1,
            sequence: 0,
            prev_digest: GENESIS_DIGEST.to_owned(),
            digest: String::new(),
            event_id,
            account_id,
            instrument_id,
            quantity_delta,
            realized_paise: 0,
            fees_paise: 0,
            funding_paise: 0,
            fx_paise: 0,
            event_at_ns: 0,
        }
    }

    /// Bind the event to the logical instant used for business-day assignment.
    #[must_use]
    pub fn with_event_at_ns(mut self, event_at_ns: i64) -> Self {
        self.event_at_ns = event_at_ns;
        self
    }

    /// Attach the account-journal money effects that the daily P&L snapshot
    /// is derived from. Fees are costs and must be non-negative.
    #[must_use]
    pub fn with_pnl_effects(
        mut self,
        realized_paise: i64,
        fees_paise: i64,
        funding_paise: i64,
        fx_paise: i64,
    ) -> Self {
        self.realized_paise = realized_paise;
        self.fees_paise = fees_paise;
        self.funding_paise = funding_paise;
        self.fx_paise = fx_paise;
        self
    }

    /// Assign the exact next sequence and seal the record to its predecessor.
    pub fn seal(&mut self, sequence: u64, prev_digest: &str) {
        let (sequence, previous, digest) = seal_fields(sequence, prev_digest, &self.preimage());
        self.sequence = sequence;
        self.prev_digest = previous;
        self.digest = digest;
    }
}

impl ChainedRecord for AccountRecoveryRecord {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }

    fn sequence(&self) -> u64 {
        self.sequence
    }

    fn prev_digest(&self) -> &str {
        &self.prev_digest
    }

    fn digest(&self) -> &str {
        &self.digest
    }

    fn preimage(&self) -> String {
        serde_json::json!({
            "schema_version": self.schema_version,
            "event_id": self.event_id,
            "account_id": self.account_id,
            "instrument_id": self.instrument_id,
            "quantity_delta": self.quantity_delta,
            "realized_paise": self.realized_paise,
            "fees_paise": self.fees_paise,
            "funding_paise": self.funding_paise,
            "fx_paise": self.fx_paise,
            "event_at_ns": self.event_at_ns,
        })
        .to_string()
    }
}

/// Verification version of a sealed recovery snapshot. A snapshot carrying any
/// other version is rejected before any state is reconstructed.
pub const RECOVERY_SNAPSHOT_VERSION: u32 = 1;

/// Stable identity of the authoritative OMS journal.
pub const OMS_JOURNAL_ID: &str = "authoritative-oms";
/// Stable identity of the authoritative account journal.
pub const ACCOUNT_JOURNAL_ID: &str = "account-journal";

/// The identity a snapshot is bound to. A caller supplies the *expected*
/// identity when wiring startup; it is metadata, never proof on its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalIdentity {
    pub source: ObservationSource,
    pub account_id: AccountId,
    pub journal_id: String,
}

impl JournalIdentity {
    #[must_use]
    pub fn new(
        source: ObservationSource,
        account_id: AccountId,
        journal_id: impl Into<String>,
    ) -> Self {
        Self {
            source,
            account_id,
            journal_id: journal_id.into(),
        }
    }
}

// --- Owner authenticity -----------------------------------------------------
//
// An unkeyed SHA-256 chain proves only that a sequence of records is internally
// consistent: anyone who can write the journal can rewrite the whole chain.
// Startup therefore requires an owner authenticity model. It is selected in
// code by the owner of the startup process, never inferred from journal content
// and never taken from a record the journal itself carries.

/// Fixed journal names under an owner-controlled root. The startup loader joins
/// these itself; application code never passes a journal path to it.
pub const OMS_PERMITS_FILE: &str = "oms_permits.jsonl";
pub const OMS_EXECUTIONS_FILE: &str = "oms_executions.jsonl";
pub const ACCOUNT_JOURNAL_FILE: &str = "account_journal.jsonl";
pub const DAILY_PNL_FILE: &str = "daily_pnl.json";
pub const OMS_ANCHOR_FILE: &str = "oms_anchor.json";
pub const ACCOUNT_ANCHOR_FILE: &str = "account_anchor.json";
/// Independently persisted trust counters, one per journal identity. These are
/// separate files from both the journal and its terminal anchor so replacing
/// the journal and anchor together cannot replace the counter that outlives
/// them.
pub const OMS_HIGH_WATER_FILE: &str = "oms_high_water.json";
pub const ACCOUNT_HIGH_WATER_FILE: &str = "account_high_water.json";
/// Stable kernel-lock file for one serialized recovery transaction. The lock
/// is held on its descriptor for the whole load, not inferred from metadata.
/// This is the single lock identity shared by recovery loaders and by writers
/// that publish a new journal/anchor state (see [`AuthorityOwnership::recovery_writer`]).
pub const RECOVERY_LOCK_FILE: &str = "recovery.lock";
pub const BINDING_SUFFIX: &str = ".auth";

const BINDING_DOMAIN: &str = "nirnay-recovery-binding-v1";
const ANCHOR_DOMAIN: &str = "nirnay-recovery-anchor-v1";
const HIGH_WATER_DOMAIN: &str = "nirnay-recovery-high-water-v1";
const AUTHENTICITY_SCHEMA: u32 = 1;

/// Owner-held secret used to sign recovery bindings. A symmetric key lets the
/// dependency-free verifier compute an HMAC-SHA-256; the secret never appears
/// in a record and is never derived from journal content.
#[derive(Clone, PartialEq, Eq)]
pub struct OwnerSigningKey {
    secret: Vec<u8>,
}

impl std::fmt::Debug for OwnerSigningKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OwnerSigningKey { redacted }")
    }
}

impl OwnerSigningKey {
    /// An empty secret is rejected so a binding can never be signed with zero
    /// effective key material.
    pub fn new(secret: impl Into<Vec<u8>>) -> Result<Self, RecoveryError> {
        let secret = secret.into();
        if secret.is_empty() {
            return Err(RecoveryError::Authenticity(
                "owner signing key is empty".to_owned(),
            ));
        }
        Ok(Self { secret })
    }

    #[must_use]
    pub fn sign(&self, domain: &str, message: &str) -> String {
        let mut preimage =
            Vec::with_capacity(domain.len().saturating_add(message.len()).saturating_add(1));
        preimage.extend_from_slice(domain.as_bytes());
        preimage.push(0);
        preimage.extend_from_slice(message.as_bytes());
        digest::hmac_sha256_hex(&self.secret, &preimage)
    }

    fn matches(&self, domain: &str, message: &str, signature: &str) -> bool {
        digest::constant_time_eq(&self.sign(domain, message), signature)
    }
}

/// How startup establishes that a journal is owner-authentic rather than merely
/// self-consistent. The model is code-selected; a journal cannot choose it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryAuthenticity {
    /// Owner HMAC signatures over identity, sequence, head digest and schema.
    OwnerSignature(OwnerSigningKey),
    /// OS owner identity, owner-only write permissions, real files and a real
    /// owner-controlled directory; symlinks and foreign directories fail.
    OwnerDirectory,
    /// Both mechanisms must pass. This is the production posture.
    Strict(OwnerSigningKey),
}

impl RecoveryAuthenticity {
    #[must_use]
    fn key(&self) -> Option<&OwnerSigningKey> {
        match self {
            Self::OwnerSignature(key) | Self::Strict(key) => Some(key),
            Self::OwnerDirectory => None,
        }
    }

    #[must_use]
    fn requires_os_owner(&self) -> bool {
        matches!(self, Self::OwnerDirectory | Self::Strict(_))
    }
}

fn source_tag(source: ObservationSource) -> String {
    serde_json::to_value(source)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn sidecar_path(path: &Path) -> PathBuf {
    let mut sidecar = path.to_path_buf();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("journal");
    sidecar.set_file_name(format!("{name}{BINDING_SUFFIX}"));
    sidecar
}

fn read_auth_json<T: serde::de::DeserializeOwned>(
    path: &Path,
    what: &str,
) -> Result<T, RecoveryError> {
    let file = File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            RecoveryError::Authenticity(format!("{what} is missing: {}", path.display()))
        } else {
            RecoveryError::Authenticity(format!("{what} is unreadable: {error}"))
        }
    })?;
    serde_json::from_reader(std::io::BufReader::new(file))
        .map_err(|error| RecoveryError::Authenticity(format!("{what} is corrupt: {error}")))
}

fn read_required_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, RecoveryError> {
    let file = File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            RecoveryError::JournalMissing(path.to_path_buf())
        } else {
            RecoveryError::Snapshot(format!("record {}: {error}", path.display()))
        }
    })?;
    serde_json::from_reader(std::io::BufReader::new(file))
        .map_err(|error| RecoveryError::Snapshot(format!("record {}: {error}", path.display())))
}

/// Owner-side check: the directory and every journal file must be real objects
/// owned by one uid, with no group/other write bit and no symlink component at
/// the file itself. Only Unix exposes owner identity without a dependency.
#[cfg(unix)]
fn check_owner_journal_paths(paths: &[&Path]) -> Result<(), RecoveryError> {
    use std::os::unix::fs::MetadataExt;

    let mut owner_uid: Option<u32> = None;
    for path in paths {
        let directory = path.parent().unwrap_or_else(|| Path::new("."));
        let directory_meta = std::fs::symlink_metadata(directory).map_err(|error| {
            RecoveryError::Authenticity(format!("journal directory is unreadable: {error}"))
        })?;
        if !directory_meta.file_type().is_dir() || directory_meta.file_type().is_symlink() {
            return Err(RecoveryError::Authenticity(
                "journal directory is not an owner-controlled real directory".to_owned(),
            ));
        }
        if directory_meta.mode() & 0o022 != 0 {
            return Err(RecoveryError::Authenticity(
                "journal directory is group- or world-writable".to_owned(),
            ));
        }
        let meta = match std::fs::symlink_metadata(path) {
            Ok(meta) => meta,
            // Let the reader report a missing required journal with its own
            // typed failure instead of masking it as an authenticity error.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(RecoveryError::Authenticity(format!(
                    "journal object is unreadable: {error}"
                )));
            }
        };
        if meta.file_type().is_symlink() {
            return Err(RecoveryError::Authenticity(format!(
                "journal path is a symlink: {}",
                path.display()
            )));
        }
        if !meta.file_type().is_file() {
            return Err(RecoveryError::Authenticity(format!(
                "journal path is not a regular file: {}",
                path.display()
            )));
        }
        if meta.mode() & 0o022 != 0 {
            return Err(RecoveryError::Authenticity(format!(
                "journal file is group- or world-writable: {}",
                path.display()
            )));
        }
        if directory_meta.uid() != meta.uid() {
            return Err(RecoveryError::Authenticity(format!(
                "journal file is not owned by the directory owner: {}",
                path.display()
            )));
        }
        match owner_uid {
            None => owner_uid = Some(meta.uid()),
            Some(uid) if uid == meta.uid() => {}
            Some(_) => {
                return Err(RecoveryError::Authenticity(
                    "journal files have mixed owners".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_owner_journal_paths(_paths: &[&Path]) -> Result<(), RecoveryError> {
    Err(RecoveryError::Authenticity(
        "OS owner authenticity requires a Unix platform".to_owned(),
    ))
}

/// Keyed binding sidecar for one journal. It is written by the owner, not by
/// the authority, and covers the identity, sequence, head digest, schema and
/// the full state digest of the replayed journal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryBindingRecord {
    pub schema_version: u32,
    pub source: String,
    pub account_id: AccountId,
    pub journal_id: String,
    pub record_count: u64,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub head_digest: String,
    pub state_digest: String,
    pub signature: String,
}

impl RecoveryBindingRecord {
    #[allow(clippy::too_many_arguments)] // every argument is a signed binding fact
    fn preimage(
        schema_version: u32,
        source: &str,
        account_id: AccountId,
        journal_id: &str,
        record_count: u64,
        first_sequence: u64,
        last_sequence: u64,
        head_digest: &str,
        state_digest: &str,
    ) -> String {
        serde_json::json!({
            "schema_version": schema_version,
            "source": source,
            "account_id": account_id,
            "journal_id": journal_id,
            "record_count": record_count,
            "first_sequence": first_sequence,
            "last_sequence": last_sequence,
            "head_digest": head_digest,
            "state_digest": state_digest,
        })
        .to_string()
    }

    /// Owner-side seal. The caller must hold the owner key; no authority state
    /// or private derivation is exposed to create this record.
    #[allow(clippy::too_many_arguments)] // every argument is a required binding fact
    pub fn seal(
        key: &OwnerSigningKey,
        identity: &JournalIdentity,
        record_count: u64,
        first_sequence: u64,
        last_sequence: u64,
        head_digest: &str,
        state_digest: &str,
    ) -> Result<Self, RecoveryError> {
        let source = source_tag(identity.source);
        let message = Self::preimage(
            AUTHENTICITY_SCHEMA,
            &source,
            identity.account_id,
            &identity.journal_id,
            record_count,
            first_sequence,
            last_sequence,
            head_digest,
            state_digest,
        );
        Ok(Self {
            schema_version: AUTHENTICITY_SCHEMA,
            source,
            account_id: identity.account_id,
            journal_id: identity.journal_id.clone(),
            record_count,
            first_sequence,
            last_sequence,
            head_digest: head_digest.to_owned(),
            state_digest: state_digest.to_owned(),
            signature: key.sign(BINDING_DOMAIN, &message),
        })
    }

    fn verify(
        &self,
        key: &OwnerSigningKey,
        identity: &JournalIdentity,
        derived: &DerivedBinding,
    ) -> Result<(), RecoveryError> {
        if self.schema_version != AUTHENTICITY_SCHEMA {
            return Err(RecoveryError::Authenticity(
                "unsupported owner binding schema".to_owned(),
            ));
        }
        if self.source != source_tag(identity.source)
            || self.account_id != identity.account_id
            || self.journal_id != identity.journal_id
        {
            return Err(RecoveryError::Authenticity(
                "owner signature is bound to another journal identity".to_owned(),
            ));
        }
        if self.record_count != derived.record_count
            || self.first_sequence != derived.first_sequence
            || self.last_sequence != derived.last_sequence
            || self.head_digest != derived.head_digest
            || self.state_digest != derived.state_digest
        {
            return Err(RecoveryError::Authenticity(
                "owner signature does not bind the replayed journal".to_owned(),
            ));
        }
        let message = Self::preimage(
            self.schema_version,
            &self.source,
            self.account_id,
            &self.journal_id,
            self.record_count,
            self.first_sequence,
            self.last_sequence,
            &self.head_digest,
            &self.state_digest,
        );
        if !key.matches(BINDING_DOMAIN, &message, &self.signature) {
            return Err(RecoveryError::Authenticity(
                "owner signature is forged or altered".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Independently protected terminal-head anchor. It is stored outside the
/// journal, so replacing a journal with an older but internally valid prefix
/// cannot hide a rollback. `counter` is only advanced by the owner when the
/// terminal head is intentionally moved forward.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryAnchor {
    pub schema_version: u32,
    pub counter: u64,
    pub source: String,
    pub account_id: AccountId,
    pub journal_id: String,
    pub sequence: u64,
    pub head_digest: String,
    pub signature: String,
}

impl RecoveryAnchor {
    fn preimage(
        schema_version: u32,
        counter: u64,
        source: &str,
        account_id: AccountId,
        journal_id: &str,
        sequence: u64,
        head_digest: &str,
    ) -> String {
        serde_json::json!({
            "schema_version": schema_version,
            "counter": counter,
            "source": source,
            "account_id": account_id,
            "journal_id": journal_id,
            "sequence": sequence,
            "head_digest": head_digest,
        })
        .to_string()
    }

    /// Owner-side anchor for an OS-owner-controlled directory. It carries no
    /// keyed signature; protection comes from the directory's owner-only write
    /// permission, which the loader verifies before trusting it.
    #[must_use]
    pub fn for_owner_directory(
        identity: &JournalIdentity,
        counter: u64,
        sequence: u64,
        head_digest: &str,
    ) -> Self {
        Self {
            schema_version: AUTHENTICITY_SCHEMA,
            counter,
            source: source_tag(identity.source),
            account_id: identity.account_id,
            journal_id: identity.journal_id.clone(),
            sequence,
            head_digest: head_digest.to_owned(),
            signature: String::new(),
        }
    }

    /// Owner-side seal of a terminal head. `counter` must be non-zero and is
    /// advanced monotonically by the owner; the authority checks that the
    /// anchor matches the exact head it re-derived from the journal.
    pub fn seal(
        key: &OwnerSigningKey,
        identity: &JournalIdentity,
        counter: u64,
        sequence: u64,
        head_digest: &str,
    ) -> Result<Self, RecoveryError> {
        if counter == 0 {
            return Err(RecoveryError::Authenticity(
                "anchor counter must be non-zero".to_owned(),
            ));
        }
        let source = source_tag(identity.source);
        let message = Self::preimage(
            AUTHENTICITY_SCHEMA,
            counter,
            &source,
            identity.account_id,
            &identity.journal_id,
            sequence,
            head_digest,
        );
        Ok(Self {
            schema_version: AUTHENTICITY_SCHEMA,
            counter,
            source,
            account_id: identity.account_id,
            journal_id: identity.journal_id.clone(),
            sequence,
            head_digest: head_digest.to_owned(),
            signature: key.sign(ANCHOR_DOMAIN, &message),
        })
    }

    fn check(
        &self,
        identity: &JournalIdentity,
        checkpoint: &RecoveryCheckpoint,
        authenticity: &RecoveryAuthenticity,
    ) -> Result<(), RecoveryError> {
        if self.schema_version != AUTHENTICITY_SCHEMA {
            return Err(RecoveryError::Authenticity(
                "unsupported terminal anchor schema".to_owned(),
            ));
        }
        if self.counter == 0 {
            return Err(RecoveryError::Authenticity(
                "terminal anchor counter is not monotonic".to_owned(),
            ));
        }
        if self.source != source_tag(identity.source)
            || self.account_id != identity.account_id
            || self.journal_id != identity.journal_id
        {
            return Err(RecoveryError::Authenticity(
                "terminal anchor belongs to another journal identity".to_owned(),
            ));
        }
        if self.counter < checkpoint.sequence {
            return Err(RecoveryError::Authenticity(
                "terminal anchor counter regressed behind the journal head".to_owned(),
            ));
        }
        if self.sequence != checkpoint.sequence || self.head_digest != checkpoint.digest {
            return Err(RecoveryError::Authenticity(
                "journal rolled back behind its independently protected terminal head".to_owned(),
            ));
        }
        if let Some(key) = authenticity.key() {
            let message = Self::preimage(
                self.schema_version,
                self.counter,
                &self.source,
                self.account_id,
                &self.journal_id,
                self.sequence,
                &self.head_digest,
            );
            if !key.matches(ANCHOR_DOMAIN, &message, &self.signature) {
                return Err(RecoveryError::Authenticity(
                    "terminal anchor signature is forged or altered".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

/// Independently persisted trust counter for one journal identity. It lives in
/// its own file, is advanced only after a load has verified the journal, its
/// binding and its anchor, and is never rewritten when a journal prefix is
/// replaced. An attacker who rolls back the journal and its anchor together
/// still faces the counter that was trusted before the rollback.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryHighWater {
    pub schema_version: u32,
    pub counter: u64,
    pub source: String,
    pub account_id: AccountId,
    pub journal_id: String,
    pub signature: String,
}

impl RecoveryHighWater {
    fn preimage(
        schema_version: u32,
        counter: u64,
        source: &str,
        account_id: AccountId,
        journal_id: &str,
    ) -> String {
        serde_json::json!({
            "schema_version": schema_version,
            "counter": counter,
            "source": source,
            "account_id": account_id,
            "journal_id": journal_id,
        })
        .to_string()
    }

    /// Owner-side counter for an OS-owner-controlled directory. It carries no
    /// keyed signature; protection comes from the directory's owner-only write
    /// permission, which the loader verifies before trusting it.
    #[must_use]
    pub fn for_owner_directory(identity: &JournalIdentity, counter: u64) -> Self {
        Self {
            schema_version: AUTHENTICITY_SCHEMA,
            counter,
            source: source_tag(identity.source),
            account_id: identity.account_id,
            journal_id: identity.journal_id.clone(),
            signature: String::new(),
        }
    }

    /// Owner-side seal of the highest trusted counter. `counter` must be
    /// non-zero and may only move forward; the authority re-verifies the
    /// signature and the identity before consulting the value.
    pub fn seal(
        key: &OwnerSigningKey,
        identity: &JournalIdentity,
        counter: u64,
    ) -> Result<Self, RecoveryError> {
        if counter == 0 {
            return Err(RecoveryError::Authenticity(
                "trusted high-water counter must be non-zero".to_owned(),
            ));
        }
        let source = source_tag(identity.source);
        let message = Self::preimage(
            AUTHENTICITY_SCHEMA,
            counter,
            &source,
            identity.account_id,
            &identity.journal_id,
        );
        Ok(Self {
            schema_version: AUTHENTICITY_SCHEMA,
            counter,
            source,
            account_id: identity.account_id,
            journal_id: identity.journal_id.clone(),
            signature: key.sign(HIGH_WATER_DOMAIN, &message),
        })
    }

    fn check(
        &self,
        identity: &JournalIdentity,
        anchor_counter: u64,
        authenticity: &RecoveryAuthenticity,
    ) -> Result<(), RecoveryError> {
        if self.schema_version != AUTHENTICITY_SCHEMA {
            return Err(RecoveryError::Authenticity(
                "unsupported trusted high-water schema".to_owned(),
            ));
        }
        if self.counter == 0 {
            return Err(RecoveryError::Authenticity(
                "trusted high-water counter is not monotonic".to_owned(),
            ));
        }
        if self.source != source_tag(identity.source)
            || self.account_id != identity.account_id
            || self.journal_id != identity.journal_id
        {
            return Err(RecoveryError::Authenticity(
                "trusted high-water belongs to another journal identity".to_owned(),
            ));
        }
        if let Some(key) = authenticity.key() {
            let message = Self::preimage(
                self.schema_version,
                self.counter,
                &self.source,
                self.account_id,
                &self.journal_id,
            );
            if !key.matches(HIGH_WATER_DOMAIN, &message, &self.signature) {
                return Err(RecoveryError::Authenticity(
                    "trusted high-water signature is forged or altered".to_owned(),
                ));
            }
        }
        if anchor_counter < self.counter {
            return Err(RecoveryError::Authenticity(
                "terminal anchor counter regressed below the trusted high-water counter".to_owned(),
            ));
        }
        Ok(())
    }
}

/// A recovery transaction is serialized by a kernel-held lock on a stable file
/// in the pinned recovery root, and every required record is opened through the
/// pinned directory descriptor. `std` has no wrappers for `flock`, `openat` or
/// `renameat`, so this is the narrow FFI boundary. Platforms without these
/// primitives fail closed instead of falling back to path-based checks.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
))]
#[allow(unsafe_code)] // std exposes no kernel-lock or relative-open wrapper
mod recovery_io {
    use super::*;
    use std::{
        ffi::CString,
        io,
        os::unix::{
            fs::{MetadataExt, OpenOptionsExt},
            io::{AsRawFd, FromRawFd},
        },
    };

    #[cfg(any(target_os = "linux", target_os = "android"))]
    const O_DIRECTORY: i32 = 0o200000;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const O_NOFOLLOW: i32 = 0o400000;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const O_CLOEXEC: i32 = 0o2000000;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const O_DIRECTORY: i32 = 0o4000000;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const O_NOFOLLOW: i32 = 0o000400;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const O_CLOEXEC: i32 = 0o100000000;

    #[cfg(any(target_os = "linux", target_os = "android"))]
    const O_CREAT: i32 = 0o100;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const O_EXCL: i32 = 0o200;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const O_CREAT: i32 = 0o1000;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const O_EXCL: i32 = 0o4000;

    const O_RDONLY: i32 = 0;
    const O_WRONLY: i32 = 1;
    const O_RDWR: i32 = 2;
    const LOCK_EX: i32 = 2;

    unsafe extern "C" {
        fn openat(dirfd: i32, path: *const std::ffi::c_char, flags: i32, ...) -> i32;
        fn renameat(
            olddirfd: i32,
            oldpath: *const std::ffi::c_char,
            newdirfd: i32,
            newpath: *const std::ffi::c_char,
        ) -> i32;
        fn unlinkat(dirfd: i32, path: *const std::ffi::c_char, flags: i32) -> i32;
        fn flock(fd: i32, operation: i32) -> i32;
    }

    fn c_name(name: &str) -> Result<CString, RecoveryError> {
        CString::new(name).map_err(|_| {
            RecoveryError::Authenticity("recovery record name contains a NUL byte".to_owned())
        })
    }

    fn sidecar_name(file: &str) -> String {
        format!("{file}{BINDING_SUFFIX}")
    }

    /// A pinned directory descriptor plus the identity it had at pin time.
    /// Every guarded operation re-checks both the descriptor identity and the
    /// root path identity before touching an object.
    pub(super) struct PinnedRecoveryDir {
        dir: File,
        root: PathBuf,
        dev: u64,
        ino: u64,
        uid: u32,
        mode: u32,
    }

    impl PinnedRecoveryDir {
        pub(super) fn open(root: &Path) -> Result<Self, RecoveryError> {
            let dir = OpenOptions::new()
                .read(true)
                .custom_flags(O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
                .open(root)
                .map_err(|error| {
                    RecoveryError::Authenticity(format!("recovery directory pin failed: {error}"))
                })?;
            let meta = dir.metadata().map_err(|error| {
                RecoveryError::Authenticity(format!("pinned directory is unreadable: {error}"))
            })?;
            if !meta.is_dir() {
                return Err(RecoveryError::Authenticity(
                    "pinned recovery root is not a directory".to_owned(),
                ));
            }
            let path_meta = std::fs::symlink_metadata(root).map_err(|error| {
                RecoveryError::Authenticity(format!("recovery root is unreadable: {error}"))
            })?;
            if !path_meta.is_dir() || path_meta.file_type().is_symlink() {
                return Err(RecoveryError::Authenticity(
                    "recovery root is not a real directory".to_owned(),
                ));
            }
            if meta.dev() != path_meta.dev() || meta.ino() != path_meta.ino() {
                return Err(RecoveryError::Authenticity(
                    "recovery root changed while it was being pinned".to_owned(),
                ));
            }
            Ok(Self {
                dir,
                root: root.to_path_buf(),
                dev: meta.dev(),
                ino: meta.ino(),
                uid: meta.uid(),
                mode: meta.mode(),
            })
        }

        pub(super) fn path(&self) -> &Path {
            &self.root
        }

        fn check_identity(&self) -> Result<(), RecoveryError> {
            let meta = self.dir.metadata().map_err(|error| {
                RecoveryError::Authenticity(format!("pinned directory is unreadable: {error}"))
            })?;
            if meta.dev() != self.dev || meta.ino() != self.ino {
                return Err(RecoveryError::Authenticity(
                    "pinned recovery directory descriptor changed".to_owned(),
                ));
            }
            let path_meta = std::fs::symlink_metadata(&self.root).map_err(|error| {
                RecoveryError::Authenticity(format!("recovery root is unreadable: {error}"))
            })?;
            if !path_meta.is_dir()
                || path_meta.file_type().is_symlink()
                || path_meta.dev() != self.dev
                || path_meta.ino() != self.ino
            {
                return Err(RecoveryError::Authenticity(
                    "recovery root identity changed after pinning".to_owned(),
                ));
            }
            Ok(())
        }

        pub(super) fn check_owner_directory(&self) -> Result<(), RecoveryError> {
            self.check_identity()?;
            if self.mode & 0o022 != 0 {
                return Err(RecoveryError::Authenticity(
                    "journal root is group- or world-writable".to_owned(),
                ));
            }
            Ok(())
        }

        fn open_at(
            &self,
            name: &str,
            flags: i32,
            mode: i32,
        ) -> Result<Option<File>, RecoveryError> {
            self.check_identity()?;
            let name_c = c_name(name)?;
            // SAFETY: the descriptor is a live, pinned O_DIRECTORY file and the
            // name is a NUL-terminated fixed record name.
            let fd = unsafe {
                openat(
                    self.dir.as_raw_fd(),
                    name_c.as_ptr(),
                    flags | O_NOFOLLOW | O_CLOEXEC,
                    mode,
                )
            };
            if fd < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::NotFound {
                    return Ok(None);
                }
                return Err(RecoveryError::Authenticity(format!(
                    "relative open of {name} failed: {error}"
                )));
            }
            // SAFETY: `openat` returned a fresh, owned descriptor.
            let file = unsafe { File::from_raw_fd(fd) };
            let meta = file.metadata().map_err(|error| {
                RecoveryError::Authenticity(format!(
                    "recovery object {name} is unreadable: {error}"
                ))
            })?;
            if !meta.file_type().is_file() {
                return Err(RecoveryError::Authenticity(format!(
                    "recovery object {name} is not a regular file"
                )));
            }
            Ok(Some(file))
        }

        /// Open a required record relative to the pinned descriptor. None means
        /// the directory entry is absent; callers map it to their typed error.
        pub(super) fn open_read(&self, name: &str) -> Result<Option<File>, RecoveryError> {
            self.open_at(name, O_RDONLY, 0)
        }

        pub(super) fn open_lock(&self, name: &str) -> Result<File, RecoveryError> {
            self.open_at(name, O_RDWR | O_CREAT, 0o600)?
                .ok_or_else(|| RecoveryError::Authenticity("recovery lock vanished".to_owned()))
        }

        pub(super) fn check_owner(&self, names: &[&str]) -> Result<(), RecoveryError> {
            self.check_owner_directory()?;
            for name in names {
                if let Some(file) = self.open_read(name)? {
                    let meta = file.metadata().map_err(|error| {
                        RecoveryError::Authenticity(format!(
                            "recovery object {name} is unreadable: {error}"
                        ))
                    })?;
                    if meta.mode() & 0o022 != 0 {
                        return Err(RecoveryError::Authenticity(format!(
                            "recovery object {name} is group- or world-writable"
                        )));
                    }
                    if meta.uid() != self.uid {
                        return Err(RecoveryError::Authenticity(format!(
                            "recovery object {name} is not owned by the directory owner"
                        )));
                    }
                }
            }
            Ok(())
        }

        pub(super) fn check_owner_optional(&self, name: &str) -> Result<(), RecoveryError> {
            if self.open_read(name)?.is_some() {
                self.check_owner(&[name])?;
            }
            Ok(())
        }

        /// Stage a fresh record and rename it into place entirely through the
        /// pinned descriptor. Any failure removes the staged file and fails the
        /// caller closed.
        pub(super) fn replace(&self, name: &str, contents: &str) -> Result<(), RecoveryError> {
            self.check_identity()?;
            let temp_name = format!(
                "{name}.tmp-{}-{}",
                std::process::id(),
                NEXT_HIGH_WATER_TEMP.fetch_add(1, Ordering::Relaxed)
            );
            let temp = c_name(&temp_name)?;
            let target = c_name(name)?;
            // SAFETY: both names are NUL-terminated fixed names and both
            // directories are the same live pinned descriptor.
            let fd = unsafe {
                openat(
                    self.dir.as_raw_fd(),
                    temp.as_ptr(),
                    O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC,
                    0o600,
                )
            };
            if fd < 0 {
                return Err(RecoveryError::Authenticity(format!(
                    "high-water advance could not be staged: {}",
                    io::Error::last_os_error()
                )));
            }
            // SAFETY: `openat` returned a fresh, owned descriptor.
            let mut file = unsafe { File::from_raw_fd(fd) };
            let write_result = file
                .write_all(contents.as_bytes())
                .and_then(|()| file.sync_all());
            if let Err(error) = write_result {
                let _ = unsafe { unlinkat(self.dir.as_raw_fd(), temp.as_ptr(), 0) };
                return Err(RecoveryError::Authenticity(format!(
                    "high-water advance could not be staged: {error}"
                )));
            }
            drop(file);
            self.check_identity()?;
            // SAFETY: the descriptor and both fixed names remain live.
            let renamed = unsafe {
                renameat(
                    self.dir.as_raw_fd(),
                    temp.as_ptr(),
                    self.dir.as_raw_fd(),
                    target.as_ptr(),
                )
            };
            if renamed != 0 {
                let error = io::Error::last_os_error();
                let _ = unsafe { unlinkat(self.dir.as_raw_fd(), temp.as_ptr(), 0) };
                return Err(RecoveryError::Authenticity(format!(
                    "high-water advance could not be committed: {error}"
                )));
            }
            Ok(())
        }
    }

    /// Kernel-held cross-process lock. It is owned by the descriptor, so a
    /// crashed process releases it and no stale-file age check is involved.
    pub(super) struct RecoveryLock {
        _file: File,
    }

    impl RecoveryLock {
        pub(super) fn acquire(dir: &PinnedRecoveryDir) -> Result<Self, RecoveryError> {
            let file = dir.open_lock(RECOVERY_LOCK_FILE)?;
            // SAFETY: `file` is a live descriptor owned by this guard.
            let locked = unsafe { flock(file.as_raw_fd(), LOCK_EX) };
            if locked != 0 {
                return Err(RecoveryError::Authenticity(format!(
                    "recovery lock acquisition failed: {}",
                    io::Error::last_os_error()
                )));
            }
            Ok(Self { _file: file })
        }
    }

    fn read_journal_in<T: serde::de::DeserializeOwned>(
        dir: &PinnedRecoveryDir,
        name: &str,
    ) -> Result<Vec<T>, RecoveryError> {
        let file = dir
            .open_read(name)?
            .ok_or_else(|| RecoveryError::JournalMissing(dir.path().join(name)))?;
        let mut records = Vec::new();
        for (index, line) in io::BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|error| {
                RecoveryError::Snapshot(format!("journal read failure: {error}"))
            })?;
            if line.trim().is_empty() {
                continue;
            }
            let record: T = serde_json::from_str(&line).map_err(|error| {
                RecoveryError::Snapshot(format!("journal line {}: {error}", index + 1))
            })?;
            records.push(record);
        }
        Ok(records)
    }

    fn read_json_in<T: serde::de::DeserializeOwned>(
        dir: &PinnedRecoveryDir,
        name: &str,
        what: &str,
    ) -> Result<T, RecoveryError> {
        let file = dir.open_read(name)?.ok_or_else(|| {
            RecoveryError::Authenticity(format!(
                "{what} is missing: {}",
                dir.path().join(name).display()
            ))
        })?;
        serde_json::from_reader(io::BufReader::new(file))
            .map_err(|error| RecoveryError::Authenticity(format!("{what} is corrupt: {error}")))
    }

    fn read_required_json_in<T: serde::de::DeserializeOwned>(
        dir: &PinnedRecoveryDir,
        name: &str,
    ) -> Result<T, RecoveryError> {
        let file = dir
            .open_read(name)?
            .ok_or_else(|| RecoveryError::JournalMissing(dir.path().join(name)))?;
        serde_json::from_reader(io::BufReader::new(file))
            .map_err(|error| RecoveryError::Snapshot(format!("record {name}: {error}")))
    }

    fn verify_oms_in_dir(
        dir: &PinnedRecoveryDir,
        expected: &JournalIdentity,
        authenticity: &RecoveryAuthenticity,
    ) -> Result<VerifiedOmsSnapshot, RecoveryError> {
        if expected.source != ObservationSource::Oms || expected.journal_id.trim().is_empty() {
            return Err(RecoveryError::Snapshot(
                "invalid OMS journal identity".to_owned(),
            ));
        }
        if authenticity.requires_os_owner() {
            dir.check_owner(&[OMS_PERMITS_FILE, OMS_EXECUTIONS_FILE])?;
            dir.check_owner_optional(&sidecar_name(OMS_PERMITS_FILE))?;
        }
        let permits = read_journal_in::<OmsPermitRecord>(dir, OMS_PERMITS_FILE)?;
        let executions = read_journal_in::<ExecutionRecoveryRecord>(dir, OMS_EXECUTIONS_FILE)?;
        verify_recovery_chain(&executions)?;
        let derived = bind_recovery_journal(&permits, &executions)?;
        if let Some(key) = authenticity.key() {
            let record: RecoveryBindingRecord = read_json_in(
                dir,
                &sidecar_name(OMS_PERMITS_FILE),
                "owner binding sidecar",
            )?;
            record.verify(key, expected, &derived)?;
        }
        Ok(VerifiedOmsSnapshot {
            binding: SnapshotBinding {
                identity: expected.clone(),
                schema_version: 1,
                verification_version: RECOVERY_SNAPSHOT_VERSION,
                record_count: derived.record_count,
                first_sequence: derived.first_sequence,
                last_sequence: derived.last_sequence,
                head_digest: derived.head_digest,
                state_digest: derived.state_digest,
                replay_checkpoint: derived.replay_checkpoint,
            },
            permits,
            executions,
        })
    }

    fn verify_account_in_dir(
        dir: &PinnedRecoveryDir,
        expected: &JournalIdentity,
        authenticity: &RecoveryAuthenticity,
    ) -> Result<VerifiedAccountSnapshot, RecoveryError> {
        if expected.source != ObservationSource::AccountJournal
            || expected.journal_id.trim().is_empty()
        {
            return Err(RecoveryError::Snapshot(
                "invalid account journal identity".to_owned(),
            ));
        }
        if authenticity.requires_os_owner() {
            dir.check_owner(&[ACCOUNT_JOURNAL_FILE, DAILY_PNL_FILE])?;
            dir.check_owner_optional(&sidecar_name(ACCOUNT_JOURNAL_FILE))?;
        }
        let records = read_journal_in::<AccountRecoveryRecord>(dir, ACCOUNT_JOURNAL_FILE)?;
        let daily_pnl = read_required_json_in::<DailyPnlRecovery>(dir, DAILY_PNL_FILE)?;
        let derived = bind_recovery_journal(&records, &daily_pnl)?;
        if let Some(key) = authenticity.key() {
            let record: RecoveryBindingRecord = read_json_in(
                dir,
                &sidecar_name(ACCOUNT_JOURNAL_FILE),
                "owner binding sidecar",
            )?;
            record.verify(key, expected, &derived)?;
        }
        Ok(VerifiedAccountSnapshot {
            binding: SnapshotBinding {
                identity: expected.clone(),
                schema_version: 1,
                verification_version: RECOVERY_SNAPSHOT_VERSION,
                record_count: derived.record_count,
                first_sequence: derived.first_sequence,
                last_sequence: derived.last_sequence,
                head_digest: derived.head_digest,
                state_digest: derived.state_digest,
                replay_checkpoint: derived.replay_checkpoint,
            },
            records,
            daily_pnl,
        })
    }

    fn advance_high_water_in_dir(
        dir: &PinnedRecoveryDir,
        authenticity: &RecoveryAuthenticity,
        identity: &JournalIdentity,
        counter: u64,
        name: &str,
    ) -> Result<(), RecoveryError> {
        let record = match authenticity.key() {
            Some(key) => RecoveryHighWater::seal(key, identity, counter)?,
            None => RecoveryHighWater::for_owner_directory(identity, counter),
        };
        let text = serde_json::to_string(&record).map_err(|error| {
            RecoveryError::Authenticity(format!("high-water advance serialization: {error}"))
        })?;
        dir.replace(name, &text)
    }

    pub(super) fn load_pinned(
        root: &RecoveryJournalRoot,
        account_id: AccountId,
    ) -> Result<(VerifiedOmsSnapshot, VerifiedAccountSnapshot), RecoveryError> {
        let dir = PinnedRecoveryDir::open(&root.root)?;
        // The lock is acquired before any record is read and released only
        // when this value drops after the high-water advance has committed.
        let _lock = RecoveryLock::acquire(&dir)?;
        if root.authenticity.requires_os_owner() {
            dir.check_owner_directory()?;
        }
        let oms_expected = JournalIdentity::new(ObservationSource::Oms, account_id, OMS_JOURNAL_ID);
        let account_expected = JournalIdentity::new(
            ObservationSource::AccountJournal,
            account_id,
            ACCOUNT_JOURNAL_ID,
        );
        let oms = verify_oms_in_dir(&dir, &oms_expected, &root.authenticity)?;
        let account = verify_account_in_dir(&dir, &account_expected, &root.authenticity)?;

        if root.authenticity.requires_os_owner() {
            dir.check_owner(&[
                OMS_ANCHOR_FILE,
                ACCOUNT_ANCHOR_FILE,
                OMS_HIGH_WATER_FILE,
                ACCOUNT_HIGH_WATER_FILE,
            ])?;
        }
        let oms_anchor_record: RecoveryAnchor =
            read_json_in(&dir, OMS_ANCHOR_FILE, "terminal anchor")?;
        let account_anchor_record: RecoveryAnchor =
            read_json_in(&dir, ACCOUNT_ANCHOR_FILE, "terminal anchor")?;
        let oms_head = oms.terminal_head();
        let account_head = account.terminal_head();
        oms_anchor_record.check(&oms_expected, &oms_head, &root.authenticity)?;
        account_anchor_record.check(&account_expected, &account_head, &root.authenticity)?;

        let oms_high_water_record: RecoveryHighWater =
            read_json_in(&dir, OMS_HIGH_WATER_FILE, "trusted high-water counter")?;
        let account_high_water_record: RecoveryHighWater =
            read_json_in(&dir, ACCOUNT_HIGH_WATER_FILE, "trusted high-water counter")?;
        oms_high_water_record.check(
            &oms_expected,
            oms_anchor_record.counter,
            &root.authenticity,
        )?;
        account_high_water_record.check(
            &account_expected,
            account_anchor_record.counter,
            &root.authenticity,
        )?;

        #[cfg(test)]
        super::recovery_advance_gate();
        advance_high_water_in_dir(
            &dir,
            &root.authenticity,
            &oms_expected,
            oms_anchor_record.counter,
            OMS_HIGH_WATER_FILE,
        )?;
        advance_high_water_in_dir(
            &dir,
            &root.authenticity,
            &account_expected,
            account_anchor_record.counter,
            ACCOUNT_HIGH_WATER_FILE,
        )?;
        Ok((oms, account))
    }
}

static NEXT_HIGH_WATER_TEMP: AtomicU64 = AtomicU64::new(1);

// Test-only phase gate. It is installed per thread so concurrent recovery
// tests can force a deterministic interleaving between verification and the
// high-water advance without sleeping. Production builds compile it away.
#[cfg(test)]
thread_local! {
    static RECOVERY_ADVANCE_GATE: std::cell::RefCell<
        Option<(
            std::sync::mpsc::Sender<()>,
            std::sync::mpsc::Receiver<()>,
        )>,
    > = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn recovery_advance_gate() {
    RECOVERY_ADVANCE_GATE.with(|slot| {
        if let Some((reached, release)) = slot.borrow().as_ref() {
            let _ = reached.send(());
            let _ = release.recv();
        }
    });
}

/// Owner-side helper that derives the exact binding facts the verifier will
/// recompute, then signs them. It mirrors `bind_recovery_journal` so an owner
/// can seal a journal without duplicating the derivation or reading authority
/// internals.
pub fn recovery_binding_record<T, S>(
    key: &OwnerSigningKey,
    identity: &JournalIdentity,
    records: &[T],
    supplement: &S,
) -> Result<RecoveryBindingRecord, RecoveryError>
where
    T: ChainedRecord + Serialize,
    S: Serialize,
{
    let derived = bind_recovery_journal(records, supplement)?;
    RecoveryBindingRecord::seal(
        key,
        identity,
        derived.record_count,
        derived.first_sequence,
        derived.last_sequence,
        &derived.head_digest,
        &derived.state_digest,
    )
}

fn enforce_os_authenticity(
    journal_paths: &[&Path],
    binding_path: &Path,
    authenticity: &RecoveryAuthenticity,
) -> Result<(), RecoveryError> {
    if authenticity.requires_os_owner() {
        check_owner_journal_paths(journal_paths)?;
        // In an OS-owner-only posture there is no keyed sidecar. If one is
        // present it must still be owner-controlled; it is never required.
        if std::fs::symlink_metadata(binding_path).is_ok() {
            check_owner_journal_paths(&[binding_path])?;
        }
    }
    Ok(())
}

fn enforce_key_authenticity(
    binding_path: &Path,
    identity: &JournalIdentity,
    derived: &DerivedBinding,
    authenticity: &RecoveryAuthenticity,
) -> Result<(), RecoveryError> {
    if let Some(key) = authenticity.key() {
        let record: RecoveryBindingRecord = read_auth_json(binding_path, "owner binding sidecar")?;
        record.verify(key, identity, derived)?;
    }
    Ok(())
}

/// Replay checkpoint a verified snapshot was reconstructed from. `None` means
/// the verifier replayed the whole journal from the genesis digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryCheckpoint {
    pub sequence: u64,
    pub digest: String,
}

/// Bindings recomputed by the verifier from the authoritative journal. Every
/// field is private so a caller cannot forge a count, sequence, digest or
/// identity claim about the journal it is presenting.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SnapshotBinding {
    identity: JournalIdentity,
    schema_version: u32,
    verification_version: u32,
    record_count: u64,
    first_sequence: u64,
    last_sequence: u64,
    head_digest: String,
    state_digest: String,
    replay_checkpoint: Option<RecoveryCheckpoint>,
}

/// The subset of bindings derived from the journal records themselves.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DerivedBinding {
    record_count: u64,
    first_sequence: u64,
    last_sequence: u64,
    head_digest: String,
    state_digest: String,
    replay_checkpoint: Option<RecoveryCheckpoint>,
}

/// Read a required journal. A missing path is a distinct typed failure: it must
/// never be silently converted into a valid empty journal. An empty journal is
/// represented by a file that exists and contains no records; the caller binds
/// that fact with the same signature and chain checks as any other prefix.
fn read_recovery_journal<T: serde::de::DeserializeOwned>(
    path: &Path,
) -> Result<Vec<T>, RecoveryError> {
    let file = File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            RecoveryError::JournalMissing(path.to_path_buf())
        } else {
            RecoveryError::Snapshot(format!("journal {}: {error}", path.display()))
        }
    })?;
    let mut records = Vec::new();
    for (index, line) in std::io::BufReader::new(file).lines().enumerate() {
        let line = line
            .map_err(|error| RecoveryError::Snapshot(format!("journal read failure: {error}")))?;
        if line.trim().is_empty() {
            continue;
        }
        let record: T = serde_json::from_str(&line).map_err(|error| {
            RecoveryError::Snapshot(format!("journal line {}: {error}", index + 1))
        })?;
        records.push(record);
    }
    Ok(records)
}

/// Re-verify the authoritative chain and derive the exact bindings that seal a
/// snapshot. `supplement` carries the owner's other records so the state digest
/// covers everything replayed from that journal, not just its primary stream.
fn bind_recovery_journal<T, S>(
    records: &[T],
    supplement: &S,
) -> Result<DerivedBinding, RecoveryError>
where
    T: ChainedRecord + Serialize,
    S: Serialize,
{
    verify_recovery_chain(records)?;
    for record in records {
        if record.schema_version() != 1 {
            return Err(RecoveryError::Snapshot(format!(
                "unsupported journal schema {}",
                record.schema_version()
            )));
        }
    }
    let record_count = u64::try_from(records.len())
        .map_err(|_| RecoveryError::Snapshot("journal count overflow".to_owned()))?;
    let first_sequence = records.first().map_or(0, ChainedRecord::sequence);
    let last_sequence = records.last().map_or(0, ChainedRecord::sequence);
    let head_digest = records.last().map_or_else(
        || GENESIS_DIGEST.to_owned(),
        |record| record.digest().to_owned(),
    );
    let canonical = serde_json::to_string(&(records, supplement))
        .map_err(|error| RecoveryError::Snapshot(format!("journal serialization: {error}")))?;
    let state_digest = digest::sha256_hex(canonical.as_bytes());
    // This verifier only supports a full replay from the genesis digest.
    let replay_checkpoint = None;
    Ok(DerivedBinding {
        record_count,
        first_sequence,
        last_sequence,
        head_digest,
        state_digest,
        replay_checkpoint,
    })
}

fn check_snapshot_identity(
    binding: &SnapshotBinding,
    account_id: AccountId,
    source: ObservationSource,
    journal_id: &str,
) -> Result<(), RecoveryError> {
    if binding.identity.source != source {
        return Err(RecoveryError::Snapshot(
            "journal source mismatch".to_owned(),
        ));
    }
    if binding.identity.account_id != account_id {
        return Err(RecoveryError::Snapshot(
            "snapshot belongs to a different account".to_owned(),
        ));
    }
    if binding.identity.journal_id != journal_id {
        return Err(RecoveryError::Snapshot(
            "snapshot belongs to a different journal".to_owned(),
        ));
    }
    if binding.verification_version != RECOVERY_SNAPSHOT_VERSION {
        return Err(RecoveryError::Snapshot(
            "unsupported snapshot verification version".to_owned(),
        ));
    }
    if binding.schema_version != 1 {
        return Err(RecoveryError::Snapshot(
            "unsupported journal schema".to_owned(),
        ));
    }
    Ok(())
}

fn check_derived_binding(
    binding: &SnapshotBinding,
    derived: &DerivedBinding,
) -> Result<(), RecoveryError> {
    if binding.record_count != derived.record_count {
        return Err(RecoveryError::Snapshot("record count mismatch".to_owned()));
    }
    if binding.first_sequence != derived.first_sequence {
        return Err(RecoveryError::Snapshot(
            "first sequence mismatch".to_owned(),
        ));
    }
    if binding.last_sequence != derived.last_sequence {
        return Err(RecoveryError::Snapshot("last sequence mismatch".to_owned()));
    }
    if binding.head_digest != derived.head_digest {
        return Err(RecoveryError::Snapshot("head digest mismatch".to_owned()));
    }
    if binding.state_digest != derived.state_digest {
        return Err(RecoveryError::Snapshot("state digest mismatch".to_owned()));
    }
    if binding.replay_checkpoint != derived.replay_checkpoint {
        return Err(RecoveryError::Snapshot(
            "replay checkpoint mismatch".to_owned(),
        ));
    }
    Ok(())
}

/// Opaque, owner-verified snapshot of the authoritative OMS journal. Its fields
/// are private: a caller cannot build one from vectors it fabricated, and
/// startup re-derives every binding before trusting the records.
///
/// ```compile_fail
/// use risk_authority::VerifiedOmsSnapshot;
/// let forged = VerifiedOmsSnapshot {
///     binding: unreachable!(),
///     permits: Vec::new(),
///     executions: Vec::new(),
/// };
/// ```
#[derive(Clone, Debug)]
pub struct VerifiedOmsSnapshot {
    binding: SnapshotBinding,
    permits: Vec<OmsPermitRecord>,
    executions: Vec<ExecutionRecoveryRecord>,
}

impl VerifiedOmsSnapshot {
    /// Read and verify the owner's OMS journal from durable files, then seal a
    /// snapshot bound to `expected`. Both streams must have exact sequence and
    /// digest continuity, and the selected owner authenticity model must pass.
    /// Callers that want paths discovered for them use [`RecoveryJournalRoot::load`].
    pub fn verify(
        permits_path: impl AsRef<Path>,
        executions_path: impl AsRef<Path>,
        expected: &JournalIdentity,
        authenticity: &RecoveryAuthenticity,
    ) -> Result<Self, RecoveryError> {
        if expected.source != ObservationSource::Oms || expected.journal_id.trim().is_empty() {
            return Err(RecoveryError::Snapshot(
                "invalid OMS journal identity".to_owned(),
            ));
        }
        let permits_path = permits_path.as_ref();
        let executions_path = executions_path.as_ref();
        enforce_os_authenticity(
            &[permits_path, executions_path],
            &sidecar_path(permits_path),
            authenticity,
        )?;
        let permits: Vec<OmsPermitRecord> = read_recovery_journal(permits_path)?;
        let executions: Vec<ExecutionRecoveryRecord> = read_recovery_journal(executions_path)?;
        verify_recovery_chain(&executions)?;
        let derived = bind_recovery_journal(&permits, &executions)?;
        enforce_key_authenticity(
            &sidecar_path(permits_path),
            expected,
            &derived,
            authenticity,
        )?;
        let binding = SnapshotBinding {
            identity: expected.clone(),
            schema_version: 1,
            verification_version: RECOVERY_SNAPSHOT_VERSION,
            record_count: derived.record_count,
            first_sequence: derived.first_sequence,
            last_sequence: derived.last_sequence,
            head_digest: derived.head_digest,
            state_digest: derived.state_digest,
            replay_checkpoint: derived.replay_checkpoint,
        };
        Ok(Self {
            binding,
            permits,
            executions,
        })
    }

    pub(crate) fn permits(&self) -> &[OmsPermitRecord] {
        &self.permits
    }

    pub(crate) fn executions(&self) -> &[ExecutionRecoveryRecord] {
        &self.executions
    }

    /// The independently anchored terminal head of this journal.
    #[must_use]
    pub fn terminal_head(&self) -> RecoveryCheckpoint {
        RecoveryCheckpoint {
            sequence: self.binding.last_sequence,
            digest: self.binding.head_digest.clone(),
        }
    }

    pub(crate) fn validate(&self, account_id: AccountId) -> Result<(), RecoveryError> {
        check_snapshot_identity(
            &self.binding,
            account_id,
            ObservationSource::Oms,
            OMS_JOURNAL_ID,
        )?;
        verify_recovery_chain(&self.executions)?;
        let derived = bind_recovery_journal(&self.permits, &self.executions)?;
        check_derived_binding(&self.binding, &derived)
    }
}

/// Opaque, owner-verified snapshot of the authoritative account journal. Its
/// fields are private and startup re-derives every binding before use.
#[derive(Clone, Debug)]
pub struct VerifiedAccountSnapshot {
    binding: SnapshotBinding,
    records: Vec<AccountRecoveryRecord>,
    daily_pnl: DailyPnlRecovery,
}

impl VerifiedAccountSnapshot {
    /// Read and verify the owner's account journal and its sealed daily P&L
    /// bundle from durable files, then bind the snapshot to `expected`. Both
    /// files are required and the selected owner authenticity model must pass.
    pub fn verify(
        accounts_path: impl AsRef<Path>,
        daily_pnl_path: impl AsRef<Path>,
        expected: &JournalIdentity,
        authenticity: &RecoveryAuthenticity,
    ) -> Result<Self, RecoveryError> {
        if expected.source != ObservationSource::AccountJournal
            || expected.journal_id.trim().is_empty()
        {
            return Err(RecoveryError::Snapshot(
                "invalid account journal identity".to_owned(),
            ));
        }
        let accounts_path = accounts_path.as_ref();
        let daily_pnl_path = daily_pnl_path.as_ref();
        enforce_os_authenticity(
            &[accounts_path, daily_pnl_path],
            &sidecar_path(accounts_path),
            authenticity,
        )?;
        let records: Vec<AccountRecoveryRecord> = read_recovery_journal(accounts_path)?;
        let daily_pnl: DailyPnlRecovery = read_required_json(daily_pnl_path)?;
        let derived = bind_recovery_journal(&records, &daily_pnl)?;
        enforce_key_authenticity(
            &sidecar_path(accounts_path),
            expected,
            &derived,
            authenticity,
        )?;
        let binding = SnapshotBinding {
            identity: expected.clone(),
            schema_version: 1,
            verification_version: RECOVERY_SNAPSHOT_VERSION,
            record_count: derived.record_count,
            first_sequence: derived.first_sequence,
            last_sequence: derived.last_sequence,
            head_digest: derived.head_digest,
            state_digest: derived.state_digest,
            replay_checkpoint: derived.replay_checkpoint,
        };
        Ok(Self {
            binding,
            records,
            daily_pnl,
        })
    }

    pub(crate) fn records(&self) -> &[AccountRecoveryRecord] {
        &self.records
    }

    pub(crate) fn daily_pnl(&self) -> &DailyPnlRecovery {
        &self.daily_pnl
    }

    /// The independently anchored terminal head of this journal.
    #[must_use]
    pub fn terminal_head(&self) -> RecoveryCheckpoint {
        RecoveryCheckpoint {
            sequence: self.binding.last_sequence,
            digest: self.binding.head_digest.clone(),
        }
    }

    pub(crate) fn validate(&self, account_id: AccountId) -> Result<(), RecoveryError> {
        check_snapshot_identity(
            &self.binding,
            account_id,
            ObservationSource::AccountJournal,
            ACCOUNT_JOURNAL_ID,
        )?;
        let derived = bind_recovery_journal(&self.records, &self.daily_pnl)?;
        check_derived_binding(&self.binding, &derived)
    }
}

/// Owner-controlled journal root. Application code supplies only the directory
/// the owner provisioned and the code-selected authenticity model; it cannot
/// choose which files are treated as authoritative journals. All journal,
/// binding-sidecar and terminal-anchor names are fixed here.
#[derive(Clone, Debug)]
pub struct RecoveryJournalRoot {
    root: PathBuf,
    authenticity: RecoveryAuthenticity,
}

impl RecoveryJournalRoot {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, authenticity: RecoveryAuthenticity) -> Self {
        Self {
            root: root.into(),
            authenticity,
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Load both owner journals from paths derived from this root, enforce the
    /// configured authenticity model, and reject a journal whose terminal head
    /// does not match its independently protected anchor. An empty journal is
    /// valid only when the file exists, is signed and is anchored at genesis.
    /// The whole read-verify-advance sequence runs under one cross-process lock
    /// on a pinned directory descriptor.
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))]
    pub fn load(
        &self,
        account_id: AccountId,
    ) -> Result<(VerifiedOmsSnapshot, VerifiedAccountSnapshot), RecoveryError> {
        recovery_io::load_pinned(self, account_id)
    }

    /// Recovery is unsupported where directory and kernel-lock primitives are
    /// unavailable; it fails closed instead of using path-based checks.
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    )))]
    pub fn load(
        &self,
        _account_id: AccountId,
    ) -> Result<(VerifiedOmsSnapshot, VerifiedAccountSnapshot), RecoveryError> {
        Err(RecoveryError::Authenticity(
            "recovery transaction requires directory pinning primitives".to_owned(),
        ))
    }
}

/// Complete startup input. All stores are supplied explicitly; there is no
/// implicit in-memory default.
pub struct RecoveryConfig {
    pub account_id: AccountId,
    pub policy: ValidatedRiskPolicy,
    pub instruments: Vec<Instrument>,
    pub trade_date: TradeDate,
    pub control: Box<dyn ControlStore + Send + Sync>,
    pub decisions: Box<dyn DecisionStore + Send + Sync>,
    /// Sealed by the OMS journal owner through [`VerifiedOmsSnapshot::verify`].
    pub oms: VerifiedOmsSnapshot,
    /// Sealed by the account journal owner through
    /// [`VerifiedAccountSnapshot::verify`].
    pub account: VerifiedAccountSnapshot,
    /// Effective-dated instrument-to-sector map supplied by the reference-data
    /// owner. It is required to reconstruct sector exposure consistently.
    pub sectors: BTreeMap<InstrumentId, String>,
    pub ownership_path: PathBuf,
}

/// Durable authority-owned decision stream.
pub trait DecisionStore {
    fn load_decisions(&self) -> Result<Vec<RiskDecisionRecord>, DecisionStoreError>;
    /// Persist one decision and return the sealed record with its assigned
    /// sequence and digest.
    fn append_decision(
        &self,
        record: &RiskDecisionRecord,
    ) -> Result<RiskDecisionRecord, DecisionStoreError>;
    fn is_durable(&self) -> bool {
        true
    }
}

/// Explicit simulation/test decision store. Never a production default.
#[derive(Clone, Debug, Default)]
pub struct InMemoryDecisionStore {
    records: Arc<Mutex<Vec<RiskDecisionRecord>>>,
}

impl InMemoryDecisionStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn records(&self) -> Vec<RiskDecisionRecord> {
        self.records
            .lock()
            .map(|records| records.clone())
            .unwrap_or_default()
    }
}

impl DecisionStore for InMemoryDecisionStore {
    fn load_decisions(&self) -> Result<Vec<RiskDecisionRecord>, DecisionStoreError> {
        Ok(self.records())
    }

    fn append_decision(
        &self,
        record: &RiskDecisionRecord,
    ) -> Result<RiskDecisionRecord, DecisionStoreError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| DecisionStoreError::Corrupt("decision store poisoned".to_owned()))?;
        let sequence = u64::try_from(records.len())
            .map_err(|_| DecisionStoreError::Corrupt("decision count overflow".to_owned()))?
            + 1;
        let previous = records
            .last()
            .map_or_else(|| GENESIS_DIGEST.to_owned(), |last| last.digest.clone());
        let mut sealed = record.clone();
        sealed.seal(sequence, &previous);
        records.push(sealed.clone());
        Ok(sealed)
    }

    fn is_durable(&self) -> bool {
        false
    }
}

/// Fsynced JSON-lines decision store with exact sequence and digest continuity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDecisionStore {
    path: PathBuf,
}

impl FileDecisionStore {
    #[must_use]
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read_records(&self) -> Result<Vec<RiskDecisionRecord>, DecisionStoreError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file =
            File::open(&self.path).map_err(|error| DecisionStoreError::Io(error.to_string()))?;
        let mut records = Vec::new();
        for (index, line) in std::io::BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|error| DecisionStoreError::Io(error.to_string()))?;
            if line.trim().is_empty() {
                continue;
            }
            let record: RiskDecisionRecord = serde_json::from_str(&line).map_err(|error| {
                DecisionStoreError::Corrupt(format!("line {}: {error}", index + 1))
            })?;
            records.push(record);
        }
        verify_recovery_chain(&records)
            .map_err(|error| DecisionStoreError::Corrupt(error.to_string()))?;
        Ok(records)
    }
}

impl DecisionStore for FileDecisionStore {
    fn load_decisions(&self) -> Result<Vec<RiskDecisionRecord>, DecisionStoreError> {
        self.read_records()
    }

    fn append_decision(
        &self,
        record: &RiskDecisionRecord,
    ) -> Result<RiskDecisionRecord, DecisionStoreError> {
        let mut records = self.read_records()?;
        let sequence = u64::try_from(records.len())
            .map_err(|_| DecisionStoreError::Corrupt("decision count overflow".to_owned()))?
            + 1;
        let previous = records
            .last()
            .map_or_else(|| GENESIS_DIGEST.to_owned(), |last| last.digest.clone());
        let mut sealed = record.clone();
        sealed.seal(sequence, &previous);
        records.push(sealed.clone());
        verify_recovery_chain(&records)
            .map_err(|error| DecisionStoreError::Corrupt(error.to_string()))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| DecisionStoreError::Io(error.to_string()))?;
        let line = serde_json::to_string(&sealed)
            .map_err(|error| DecisionStoreError::Serialize(error.to_string()))?;
        file.write_all(line.as_bytes())
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_all())
            .map_err(|error| DecisionStoreError::Io(error.to_string()))?;
        Ok(sealed)
    }
}

/// Exclusive ownership of the authority's reconstructed state. The lock file is
/// held for the lifetime of the value; a stale lock from a crashed process
/// fails the next startup closed until an owner clears it.
#[derive(Debug)]
pub struct AuthorityOwnership {
    path: PathBuf,
    _file: File,
}

impl AuthorityOwnership {
    pub fn acquire(path: impl AsRef<Path>) -> Result<Self, RecoveryError> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                RecoveryError::StateOwnership(format!("{}: {error}", path.display()))
            })?;
        Ok(Self { path, _file: file })
    }
}

impl Drop for AuthorityOwnership {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Writer-side participant in the recovery transaction. A writer that is about
/// to publish a new journal/anchor state holds this guard across the entire
/// publish (records, terminal anchors and trusted high-water records). It uses
/// the same kernel lock identity as [`RecoveryJournalRoot::load`], so a loader
/// cannot verify one counter while a writer commits another underneath it.
///
/// Acquisition order is fixed: the caller first holds the M4
/// [`AuthorityOwnership`] sentinel, then calls
/// [`AuthorityOwnership::recovery_writer`] to take the M3 recovery lock. The
/// borrow of the sentinel makes that order enforced by the type system. On
/// drop the M3 lock is released before the borrowed M4 sentinel can be
/// released, on success and on every error path: each guard is RAII and there
/// is no explicit unlock call to forget.
///
/// Trust boundary: `flock` is an advisory, local, kernel-held lock. It
/// serializes cooperating processes on one host and is released on process
/// death. It is not distributed fencing. An attacker who compromises the
/// owner directory or steals the owner signing key defeats these local
/// protections; neither property is claimed here.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
))]
pub struct RecoveryWriterLock<'a> {
    // Drop order is field order: M3 kernel lock, pinned directory, then the
    // borrowed M4 sentinel (which the caller drops last).
    _recovery: recovery_io::RecoveryLock,
    _dir: recovery_io::PinnedRecoveryDir,
    _owner: &'a AuthorityOwnership,
}

/// Writer-side lock guard on platforms without the required primitives. There
/// is no path-based fallback: writers fail closed just like loaders.
#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
)))]
pub struct RecoveryWriterLock<'a> {
    _owner: &'a AuthorityOwnership,
}

impl AuthorityOwnership {
    /// Opt into the recovery transaction lock while holding this ownership
    /// sentinel. The returned guard must be held across the whole writer
    /// publish; it releases the M3 lock on every success or error path.
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))]
    pub fn recovery_writer(
        &self,
        root: &RecoveryJournalRoot,
    ) -> Result<RecoveryWriterLock<'_>, RecoveryError> {
        let dir = recovery_io::PinnedRecoveryDir::open(&root.root)?;
        let recovery = recovery_io::RecoveryLock::acquire(&dir)?;
        if root.authenticity.requires_os_owner() {
            dir.check_owner_directory()?;
        }
        Ok(RecoveryWriterLock {
            _recovery: recovery,
            _dir: dir,
            _owner: self,
        })
    }

    /// Unsupported platforms fail closed rather than falling back to a
    /// path-based check that other processes could ignore.
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    )))]
    pub fn recovery_writer(
        &self,
        _root: &RecoveryJournalRoot,
    ) -> Result<RecoveryWriterLock<'_>, RecoveryError> {
        Err(RecoveryError::Authenticity(
            "recovery transaction requires directory pinning primitives".to_owned(),
        ))
    }
}

fn checked_paise(value: i64) -> Result<Paise, RecoveryError> {
    if value <= 0 {
        return Err(RecoveryError::Inconsistent(
            "durable decision has a non-positive notional".to_owned(),
        ));
    }
    Ok(Paise(value))
}

fn add_paise(current: Paise, amount: Paise, field: &str) -> Result<Paise, RecoveryError> {
    current
        .checked_add(amount)
        .map_err(|_| RecoveryError::Inconsistent(format!("{field} overflow")))
}

fn sub_paise(current: Paise, amount: Paise, field: &str) -> Result<Paise, RecoveryError> {
    let value = current
        .0
        .checked_sub(amount.0)
        .ok_or_else(|| RecoveryError::Inconsistent(format!("{field} underflow")))?;
    Ok(Paise(value))
}

fn apply_accepted(
    state: &mut AuthorityState,
    record: &RiskDecisionRecord,
    sectors: &BTreeMap<InstrumentId, String>,
    fallback_policy_version: &str,
) -> Result<(), RecoveryError> {
    let notional = checked_paise(record.notional_paise)?;
    if record.permit_id == 0 {
        return Err(RecoveryError::Inconsistent(
            "an accepted decision has no permit id".to_owned(),
        ));
    }
    let reservation = Reservation {
        permit_id: record.permit_id,
        intent_id: record.intent_id,
        account_id: record.intent.account_id,
        strategy_id: record.intent.strategy_id,
        instrument_id: record.intent.instrument_id.clone(),
        venue: record.intent.venue,
        side: record.intent.side,
        quantity: record.intent.quantity,
        reserved_notional_paise: notional,
        snapshot_revision: record.revision,
        assessment: RiskAssessment {
            intent_id: record.intent_id,
            policy_version: if record.policy_version.is_empty() {
                fallback_policy_version.to_owned()
            } else {
                record.policy_version.clone()
            },
            accepted: true,
            checks: Vec::new(),
        },
    };
    reservation
        .validate_invariants()
        .map_err(|error| RecoveryError::Inconsistent(error.to_string()))?;
    state.pending_gross_exposure_paise = add_paise(
        state.pending_gross_exposure_paise,
        notional,
        "pending exposure",
    )?;
    let strategy = state
        .strategy_exposure
        .entry(record.intent.strategy_id)
        .or_insert(Paise(0));
    *strategy = add_paise(*strategy, notional, "strategy exposure")?;
    if let Some(label) = sectors.get(&record.intent.instrument_id) {
        let sector = state
            .sector_exposure
            .entry(label.clone())
            .or_insert(Paise(0));
        *sector = add_paise(*sector, notional, "sector exposure")?;
    }
    let pending = match record.intent.side {
        Side::Buy => &mut state.pending_buy,
        Side::Sell => &mut state.pending_sell,
    };
    let quantity = pending
        .entry(record.intent.instrument_id.clone())
        .or_insert(0);
    *quantity = quantity
        .checked_add(record.intent.quantity.get())
        .ok_or_else(|| RecoveryError::Inconsistent("pending quantity overflow".to_owned()))?;
    state.open_orders = state
        .open_orders
        .checked_add(1)
        .ok_or_else(|| RecoveryError::Inconsistent("open orders overflow".to_owned()))?;
    if record.at_ns > 0 {
        state
            .order_times
            .push_back(LogicalNanos::new(u64::try_from(record.at_ns).map_err(
                |_| RecoveryError::Inconsistent("negative observation time".to_owned()),
            )?));
    }
    state.reservations.insert(record.intent_id, reservation);
    state
        .decided
        .insert(record.intent_id, record.intent.clone());
    Ok(())
}

fn apply_released(
    state: &mut AuthorityState,
    record: &RiskDecisionRecord,
    reservation: &Reservation,
    sectors: &BTreeMap<InstrumentId, String>,
) -> Result<(), RecoveryError> {
    let notional = reservation.reserved_notional_paise;
    state.pending_gross_exposure_paise = sub_paise(
        state.pending_gross_exposure_paise,
        notional,
        "pending exposure",
    )?;
    remove_pending(state, reservation)?;
    decrement_recovery_exposure(
        &mut state.strategy_exposure,
        &reservation.strategy_id,
        notional,
    )?;
    if let Some(label) = sectors.get(&reservation.instrument_id) {
        decrement_recovery_exposure(&mut state.sector_exposure, label, notional)?;
    }
    state.open_orders = state
        .open_orders
        .checked_sub(1)
        .ok_or_else(|| RecoveryError::Inconsistent("open orders underflow".to_owned()))?;
    state.reservations.remove(&record.intent_id);
    state.consumed.insert(record.intent_id);
    Ok(())
}

fn apply_filled(
    state: &mut AuthorityState,
    record: &RiskDecisionRecord,
    reservation: &Reservation,
    sectors: &BTreeMap<InstrumentId, String>,
) -> Result<(), RecoveryError> {
    let notional = reservation.reserved_notional_paise;
    state.pending_gross_exposure_paise = sub_paise(
        state.pending_gross_exposure_paise,
        notional,
        "pending exposure",
    )?;
    state.gross_exposure_paise = add_paise(state.gross_exposure_paise, notional, "gross exposure")?;
    remove_pending(state, reservation)?;
    let cover = state
        .strategy_exposure
        .get(&reservation.strategy_id)
        .copied()
        .unwrap_or(Paise(0));
    if cover.0 < notional.0 {
        return Err(RecoveryError::Inconsistent(
            "strategy exposure does not cover the recovered fill".to_owned(),
        ));
    }
    if let Some(label) = sectors.get(&reservation.instrument_id) {
        let cover = state
            .sector_exposure
            .get(label)
            .copied()
            .unwrap_or(Paise(0));
        if cover.0 < notional.0 {
            return Err(RecoveryError::Inconsistent(
                "sector exposure does not cover the recovered fill".to_owned(),
            ));
        }
    }
    let signed = match reservation.side {
        Side::Buy => reservation.quantity.get(),
        Side::Sell => -reservation.quantity.get(),
    };
    let position = state
        .positions
        .entry(reservation.instrument_id.clone())
        .or_insert(0);
    *position = position
        .checked_add(signed)
        .ok_or_else(|| RecoveryError::Inconsistent("position overflow".to_owned()))?;
    state.open_orders = state
        .open_orders
        .checked_sub(1)
        .ok_or_else(|| RecoveryError::Inconsistent("open orders underflow".to_owned()))?;
    state.reservations.remove(&record.intent_id);
    state.consumed.insert(record.intent_id);
    Ok(())
}

fn remove_pending(
    state: &mut AuthorityState,
    reservation: &Reservation,
) -> Result<(), RecoveryError> {
    let pending = match reservation.side {
        Side::Buy => &mut state.pending_buy,
        Side::Sell => &mut state.pending_sell,
    };
    if let std::collections::btree_map::Entry::Occupied(mut entry) =
        pending.entry(reservation.instrument_id.clone())
    {
        let remaining = entry
            .get()
            .checked_sub(reservation.quantity.get())
            .ok_or_else(|| RecoveryError::Inconsistent("pending quantity underflow".to_owned()))?;
        if remaining <= 0 {
            entry.remove();
        } else {
            *entry.get_mut() = remaining;
        }
    }
    Ok(())
}

fn decrement_recovery_exposure<K>(
    bucket: &mut BTreeMap<K, Paise>,
    key: &K,
    amount: Paise,
) -> Result<(), RecoveryError>
where
    K: Ord + Clone,
{
    match bucket.entry(key.clone()) {
        std::collections::btree_map::Entry::Vacant(_) => Err(RecoveryError::Inconsistent(
            "grouped exposure bucket is missing".to_owned(),
        )),
        std::collections::btree_map::Entry::Occupied(mut entry) => {
            let remaining = Paise(entry.get().0.checked_sub(amount.0).ok_or_else(|| {
                RecoveryError::Inconsistent("grouped exposure underflow".to_owned())
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

/// Execution facts that are incomplete rather than contradictory. Recovery
/// still applies the reported filled quantity, but it cannot certify the money
/// split, so it authors a reconciliation latch and stays NOT-READY.
#[derive(Clone, Debug, PartialEq, Eq)]
struct UncertainExecution {
    reason: String,
    evidence_ref: String,
}

#[derive(Clone, Debug)]
struct EffectiveExecution {
    execution_id: String,
    order_quantity: i64,
    filled_quantity: i64,
    notional_paise: i64,
    fees_paise: i64,
    costs_paise: i64,
    cancelled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExecutionSummary {
    order_quantity: i64,
    filled_quantity: i64,
    leaves_quantity: i64,
    notional_paise: i64,
    fees_paise: i64,
    costs_paise: i64,
    execution_ids: Vec<String>,
}

fn latch_reconciliation(
    state: &mut AuthorityState,
    head: Option<&ReconciliationState>,
    reason: String,
    evidence_ref: String,
) -> Result<(), RecoveryError> {
    let revision = match head {
        Some(record) => record.revision.checked_add(1).ok_or_else(|| {
            RecoveryError::Inconsistent("reconciliation revision overflow".to_owned())
        })?,
        None => 1,
    };
    state.reconciliation_blocked = true;
    state.reconciliation_decision = Some(ReconciliationState::new(
        true,
        reason,
        Some(evidence_ref),
        revision,
        i64::try_from(state.now.get()).unwrap_or(i64::MAX),
    ));
    Ok(())
}

fn same_execution(a: &ExecutionRecoveryRecord, b: &ExecutionRecoveryRecord) -> bool {
    a.permit_id == b.permit_id
        && a.intent_id == b.intent_id
        && a.execution_id == b.execution_id
        && a.report_sequence == b.report_sequence
        && a.kind == b.kind
        && a.corrects == b.corrects
        && a.order_quantity == b.order_quantity
        && a.cumulative_filled_quantity == b.cumulative_filled_quantity
        && a.leaves_quantity == b.leaves_quantity
        && a.cumulative_notional_paise == b.cumulative_notional_paise
        && a.fees_paise == b.fees_paise
        && a.costs_paise == b.costs_paise
}

/// Reduce the cumulative execution reports of one permit to one authoritative
/// order state. Reports are ordered by `report_sequence`, so input order cannot
/// change the result; identical execution identities collapse; corrections and
/// trade cancels supersede the execution they name.
fn reconcile_executions(
    records: &[&ExecutionRecoveryRecord],
    intent_id: &IntentId,
    authorized_quantity: i64,
) -> Result<(Option<ExecutionSummary>, Option<UncertainExecution>), RecoveryError> {
    if records.is_empty() {
        return Ok((None, None));
    }
    let mut unique: BTreeMap<&str, &ExecutionRecoveryRecord> = BTreeMap::new();
    let mut sequences: BTreeMap<u64, String> = BTreeMap::new();
    for record in records {
        if record.schema_version != 1 {
            return Err(RecoveryError::Inconsistent(format!(
                "unsupported execution schema {}",
                record.schema_version
            )));
        }
        if &record.intent_id != intent_id {
            return Err(RecoveryError::Inconsistent(
                "an execution report belongs to another intent".to_owned(),
            ));
        }
        if record.execution_id.trim().is_empty() {
            return Err(RecoveryError::Inconsistent(
                "an execution report has no execution identity".to_owned(),
            ));
        }
        if record.order_quantity <= 0
            || record.cumulative_filled_quantity < 0
            || record.leaves_quantity < 0
            || record.cumulative_filled_quantity > record.order_quantity
            || record.cumulative_notional_paise < 0
            || record.fees_paise < 0
            || record.costs_paise < 0
        {
            return Err(RecoveryError::Inconsistent(format!(
                "execution {} reports impossible quantities or money",
                record.execution_id
            )));
        }
        // FIX OrderQty = CumQty + LeavesQty for every non-terminal report.
        if record
            .cumulative_filled_quantity
            .checked_add(record.leaves_quantity)
            != Some(record.order_quantity)
        {
            return Err(RecoveryError::Inconsistent(format!(
                "execution {} violates OrderQty = CumQty + LeavesQty",
                record.execution_id
            )));
        }
        // An execution report only observes the order the durable decision
        // authorized. Raising OrderQty would inherit approval for exposure that
        // was never decided; lowering it is an amendment that needs its own
        // decision and permit identity. Neither is implicit in a FIX report.
        if record.order_quantity > authorized_quantity {
            return Err(RecoveryError::Inconsistent(format!(
                "execution {} increases OrderQty {} above the authorized quantity {}",
                record.execution_id, record.order_quantity, authorized_quantity
            )));
        }
        if record.order_quantity < authorized_quantity {
            return Err(RecoveryError::Inconsistent(format!(
                "execution {} reduces OrderQty {} without an explicit amendment decision and new permit identity",
                record.execution_id, record.order_quantity
            )));
        }
        match sequences.entry(record.report_sequence) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(record.execution_id.clone());
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                if entry.get() != &record.execution_id {
                    return Err(RecoveryError::Inconsistent(format!(
                        "report sequence {} identifies two executions",
                        record.report_sequence
                    )));
                }
            }
        }
        match unique.entry(&record.execution_id) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(*record);
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                if !same_execution(entry.get(), record) {
                    return Err(RecoveryError::Inconsistent(format!(
                        "execution identity {} has contradictory reports",
                        record.execution_id
                    )));
                }
            }
        }
    }

    let mut ordered: Vec<&ExecutionRecoveryRecord> = unique.values().copied().collect();
    ordered.sort_by(|a, b| {
        a.report_sequence
            .cmp(&b.report_sequence)
            .then_with(|| a.execution_id.cmp(&b.execution_id))
    });
    let mut effective: BTreeMap<String, EffectiveExecution> = BTreeMap::new();
    let mut filled = 0_i64;
    let mut notional = 0_i64;
    let mut fees = 0_i64;
    let mut costs = 0_i64;

    for record in &ordered {
        if record.kind != ExecutionEventKind::Fill {
            continue;
        }
        let delta_quantity = record
            .cumulative_filled_quantity
            .checked_sub(filled)
            .ok_or_else(|| RecoveryError::Inconsistent("execution quantity overflow".to_owned()))?;
        let delta_notional = record
            .cumulative_notional_paise
            .checked_sub(notional)
            .ok_or_else(|| RecoveryError::Inconsistent("execution notional overflow".to_owned()))?;
        let delta_fees = record
            .fees_paise
            .checked_sub(fees)
            .ok_or_else(|| RecoveryError::Inconsistent("execution fees regression".to_owned()))?;
        let delta_costs = record
            .costs_paise
            .checked_sub(costs)
            .ok_or_else(|| RecoveryError::Inconsistent("execution costs regression".to_owned()))?;
        if delta_quantity <= 0 || delta_notional < 0 || delta_fees < 0 || delta_costs < 0 {
            return Err(RecoveryError::Inconsistent(format!(
                "execution {} does not add to the cumulative report",
                record.execution_id
            )));
        }
        effective.insert(
            record.execution_id.clone(),
            EffectiveExecution {
                execution_id: record.execution_id.clone(),
                order_quantity: record.order_quantity,
                filled_quantity: delta_quantity,
                notional_paise: delta_notional,
                fees_paise: delta_fees,
                costs_paise: delta_costs,
                cancelled: false,
            },
        );
        filled = record.cumulative_filled_quantity;
        notional = record.cumulative_notional_paise;
        fees = record.fees_paise;
        costs = record.costs_paise;
    }

    for record in &ordered {
        if record.kind == ExecutionEventKind::Fill {
            continue;
        }
        let Some(target_id) = record.corrects.as_deref() else {
            return Err(RecoveryError::Inconsistent(format!(
                "execution {} is a correction without a target",
                record.execution_id
            )));
        };
        let Some(target) = effective.get_mut(target_id) else {
            return Ok((
                None,
                Some(UncertainExecution {
                    reason: format!(
                        "execution {} references unknown execution {}",
                        record.execution_id, target_id
                    ),
                    evidence_ref: format!("permit-{}", record.permit_id),
                }),
            ));
        };
        let other_quantity = filled.checked_sub(target.filled_quantity).ok_or_else(|| {
            RecoveryError::Inconsistent("execution correction quantity underflow".to_owned())
        })?;
        let other_notional = notional.checked_sub(target.notional_paise).ok_or_else(|| {
            RecoveryError::Inconsistent("execution correction notional underflow".to_owned())
        })?;
        let other_fees = fees.checked_sub(target.fees_paise).ok_or_else(|| {
            RecoveryError::Inconsistent("execution correction fees underflow".to_owned())
        })?;
        let other_costs = costs.checked_sub(target.costs_paise).ok_or_else(|| {
            RecoveryError::Inconsistent("execution correction costs underflow".to_owned())
        })?;
        if record.kind == ExecutionEventKind::TradeCancel {
            // All four cumulative facts must agree with the executions that
            // remain: quantity, notional, fees and costs are cleared together
            // and any disagreement is a contradiction, not a usable estimate.
            if record.cumulative_filled_quantity != other_quantity
                || record.cumulative_notional_paise != other_notional
                || record.fees_paise != other_fees
                || record.costs_paise != other_costs
            {
                return Err(RecoveryError::Inconsistent(format!(
                    "trade cancel {} disagrees with the remaining executions on quantity, notional, fees or costs",
                    record.execution_id
                )));
            }
            target.filled_quantity = 0;
            target.notional_paise = 0;
            target.fees_paise = 0;
            target.costs_paise = 0;
            target.cancelled = true;
        } else {
            if target.cancelled {
                return Err(RecoveryError::Inconsistent(format!(
                    "execution correction {} targets an already cancelled execution",
                    record.execution_id
                )));
            }
            let new_quantity = record
                .cumulative_filled_quantity
                .checked_sub(other_quantity)
                .ok_or_else(|| {
                    RecoveryError::Inconsistent("corrected quantity underflow".to_owned())
                })?;
            let new_notional = record
                .cumulative_notional_paise
                .checked_sub(other_notional)
                .ok_or_else(|| {
                    RecoveryError::Inconsistent("corrected notional underflow".to_owned())
                })?;
            let new_fees = record.fees_paise.checked_sub(other_fees).ok_or_else(|| {
                RecoveryError::Inconsistent("corrected fees underflow".to_owned())
            })?;
            let new_costs = record.costs_paise.checked_sub(other_costs).ok_or_else(|| {
                RecoveryError::Inconsistent("corrected costs underflow".to_owned())
            })?;
            if new_quantity < 0 || new_notional < 0 || new_fees < 0 || new_costs < 0 {
                return Err(RecoveryError::Inconsistent(format!(
                    "execution correction {} reports impossible terms",
                    record.execution_id
                )));
            }
            target.order_quantity = record.order_quantity;
            target.filled_quantity = new_quantity;
            target.notional_paise = new_notional;
            target.fees_paise = new_fees;
            target.costs_paise = new_costs;
        }
        filled = record.cumulative_filled_quantity;
        notional = record.cumulative_notional_paise;
        fees = record.fees_paise;
        costs = record.costs_paise;
    }

    let mut summary_filled = 0_i64;
    let mut summary_notional = 0_i64;
    let mut summary_fees = 0_i64;
    let mut summary_costs = 0_i64;
    let mut execution_ids = Vec::new();
    for execution in effective.values() {
        if execution.cancelled {
            continue;
        }
        summary_filled = summary_filled
            .checked_add(execution.filled_quantity)
            .ok_or_else(|| RecoveryError::Inconsistent("execution quantity overflow".to_owned()))?;
        summary_notional = summary_notional
            .checked_add(execution.notional_paise)
            .ok_or_else(|| RecoveryError::Inconsistent("execution notional overflow".to_owned()))?;
        summary_fees = summary_fees
            .checked_add(execution.fees_paise)
            .ok_or_else(|| RecoveryError::Inconsistent("execution fees overflow".to_owned()))?;
        summary_costs = summary_costs
            .checked_add(execution.costs_paise)
            .ok_or_else(|| RecoveryError::Inconsistent("execution costs overflow".to_owned()))?;
        execution_ids.push(execution.execution_id.clone());
    }
    let order_quantity = ordered
        .last()
        .map_or(authorized_quantity, |record| record.order_quantity);
    let leaves_quantity = order_quantity
        .checked_sub(summary_filled)
        .ok_or_else(|| RecoveryError::Inconsistent("leaves quantity underflow".to_owned()))?;
    if leaves_quantity < 0 {
        return Err(RecoveryError::Inconsistent(
            "cumulative executions exceed the order quantity".to_owned(),
        ));
    }
    Ok((
        Some(ExecutionSummary {
            order_quantity,
            filled_quantity: summary_filled,
            leaves_quantity,
            notional_paise: summary_notional,
            fees_paise: summary_fees,
            costs_paise: summary_costs,
            execution_ids,
        }),
        None,
    ))
}

/// Deduct a known filled quantity from the pending bucket of one instrument,
/// leaving the unexecuted leaves quantity pending.
fn reduce_pending_quantity(
    state: &mut AuthorityState,
    reservation: &Reservation,
    quantity: i64,
) -> Result<(), RecoveryError> {
    if quantity == 0 {
        return Ok(());
    }
    let pending = match reservation.side {
        Side::Buy => &mut state.pending_buy,
        Side::Sell => &mut state.pending_sell,
    };
    if let std::collections::btree_map::Entry::Occupied(mut entry) =
        pending.entry(reservation.instrument_id.clone())
    {
        let remaining = entry
            .get()
            .checked_sub(quantity)
            .ok_or_else(|| RecoveryError::Inconsistent("pending quantity underflow".to_owned()))?;
        if remaining <= 0 {
            entry.remove();
        } else {
            *entry.get_mut() = remaining;
        }
    }
    Ok(())
}

/// Apply the cumulative executed portion of one permit. The filled notional
/// moves from pending into gross exposure and into the signed position; the
/// reservation is reduced to the leaves quantity and its unspent notional.
#[allow(clippy::too_many_arguments)] // every argument is a required reconciliation fact
fn apply_execution_to_state(
    state: &mut AuthorityState,
    decision: &RiskDecisionRecord,
    sectors: &BTreeMap<InstrumentId, String>,
    filled_quantity: i64,
    filled_notional: Paise,
    leaves_quantity: i64,
    leaves_notional: Paise,
    outcome: OmsPermitState,
    execution: Option<ExecutionExposure>,
) -> Result<(), RecoveryError> {
    let authorized_notional = checked_paise(decision.notional_paise)?;
    if filled_quantity < 0 || leaves_quantity < 0 || filled_notional.0 < 0 || leaves_notional.0 < 0
    {
        return Err(RecoveryError::Inconsistent(
            "an OMS execution reports a negative quantity or notional".to_owned(),
        ));
    }
    let accounted = filled_notional
        .0
        .checked_add(leaves_notional.0)
        .ok_or_else(|| RecoveryError::Inconsistent("execution notional overflow".to_owned()))?;
    if accounted != authorized_notional.0 {
        return Err(RecoveryError::Inconsistent(
            "filled and leaves notional do not cover the authorized notional".to_owned(),
        ));
    }
    let reservation = state
        .reservations
        .get(&decision.intent_id)
        .cloned()
        .ok_or_else(|| {
            RecoveryError::Inconsistent("an execution has no matching reservation".to_owned())
        })?;

    state.pending_gross_exposure_paise = sub_paise(
        state.pending_gross_exposure_paise,
        filled_notional,
        "pending exposure",
    )?;
    state.gross_exposure_paise = add_paise(
        state.gross_exposure_paise,
        filled_notional,
        "gross exposure",
    )?;
    reduce_pending_quantity(state, &reservation, filled_quantity)?;
    if filled_quantity > 0 {
        let signed = match reservation.side {
            Side::Buy => filled_quantity,
            Side::Sell => -filled_quantity,
        };
        let position = state
            .positions
            .entry(reservation.instrument_id.clone())
            .or_insert(0);
        let updated = position
            .checked_add(signed)
            .ok_or_else(|| RecoveryError::Inconsistent("recovered position overflow".to_owned()))?;
        if updated == 0 {
            state.positions.remove(&reservation.instrument_id);
        } else {
            *position = updated;
        }
    }

    let cover = state
        .strategy_exposure
        .get(&reservation.strategy_id)
        .copied()
        .unwrap_or(Paise(0));
    if cover.0 < authorized_notional.0 {
        return Err(RecoveryError::Inconsistent(
            "strategy exposure does not cover the recovered execution".to_owned(),
        ));
    }
    if let Some(label) = sectors.get(&reservation.instrument_id) {
        let cover = state
            .sector_exposure
            .get(label)
            .copied()
            .unwrap_or(Paise(0));
        if cover.0 < authorized_notional.0 {
            return Err(RecoveryError::Inconsistent(
                "sector exposure does not cover the recovered execution".to_owned(),
            ));
        }
    }

    match outcome {
        OmsPermitState::Open | OmsPermitState::Unknown => {
            if leaves_quantity <= 0 {
                return Err(RecoveryError::Inconsistent(
                    "an unfinished OMS permit has no leaves quantity".to_owned(),
                ));
            }
            let mut remaining = reservation.clone();
            remaining.quantity = Quantity::new(leaves_quantity).map_err(|error| {
                RecoveryError::Inconsistent(format!("invalid leaves quantity: {error}"))
            })?;
            remaining.reserved_notional_paise = leaves_notional;
            state.reservations.insert(decision.intent_id, remaining);
            if filled_quantity > 0 {
                // Only a reconciled execution journal may create an exposure.
                // An aggregate-only fill has no execution identity and no
                // certified fee or cost fact; inventing zeros here would turn
                // missing evidence into an apparent reconciliation.
                if let Some(value) = execution {
                    state.executions.insert(decision.intent_id, value);
                }
            }
        }
        OmsPermitState::Filled => {
            state.pending_gross_exposure_paise = sub_paise(
                state.pending_gross_exposure_paise,
                leaves_notional,
                "pending exposure",
            )?;
            state.reservations.remove(&decision.intent_id);
            state.executions.remove(&decision.intent_id);
            state.consumed.insert(decision.intent_id);
            state.open_orders = state
                .open_orders
                .checked_sub(1)
                .ok_or_else(|| RecoveryError::Inconsistent("open orders underflow".to_owned()))?;
        }
        OmsPermitState::Cancelled => {
            state.pending_gross_exposure_paise = sub_paise(
                state.pending_gross_exposure_paise,
                leaves_notional,
                "pending exposure",
            )?;
            decrement_recovery_exposure(
                &mut state.strategy_exposure,
                &reservation.strategy_id,
                leaves_notional,
            )?;
            if let Some(label) = sectors.get(&reservation.instrument_id) {
                decrement_recovery_exposure(&mut state.sector_exposure, label, leaves_notional)?;
            }
            state.reservations.remove(&decision.intent_id);
            state.executions.remove(&decision.intent_id);
            state.consumed.insert(decision.intent_id);
            state.open_orders = state
                .open_orders
                .checked_sub(1)
                .ok_or_else(|| RecoveryError::Inconsistent("open orders underflow".to_owned()))?;
        }
    }
    Ok(())
}

fn reconstruct(
    config: &RecoveryConfig,
    sectors: &BTreeMap<InstrumentId, String>,
) -> Result<AuthorityState, RecoveryError> {
    if !config.control.is_durable() || !config.decisions.is_durable() {
        return Err(RecoveryError::NonDurableStore);
    }
    let kill = config
        .control
        .load_kill_switch()
        .map_err(|error| RecoveryError::Control(error.to_string()))?;
    let reconciliation = config
        .control
        .load_reconciliation()
        .map_err(|error| RecoveryError::Control(error.to_string()))?;
    let decisions = config
        .decisions
        .load_decisions()
        .map_err(|error| RecoveryError::DecisionStore(error.to_string()))?;
    verify_recovery_chain(&decisions)?;
    // The owner journals are never accepted as caller vectors. Both snapshots
    // must carry bindings that re-derive exactly from their sealed records.
    config.oms.validate(config.account_id)?;
    config.account.validate(config.account_id)?;

    let mut state = AuthorityState {
        kill_switch_engaged: kill.as_ref().is_none_or(|record| record.enabled),
        reconciliation_decision: reconciliation.clone(),
        reconciliation_blocked: reconciliation.as_ref().is_none_or(|record| record.blocked),
        ..AuthorityState::default()
    };
    if let Some(record) = reconciliation.as_ref()
        && !record.blocked
        && record
            .evidence_ref
            .as_ref()
            .is_none_or(|reference| reference.trim().is_empty())
    {
        return Err(RecoveryError::Inconsistent(
            "a persisted reconciliation resolution has no evidence".to_owned(),
        ));
    }

    let mut by_permit: BTreeMap<u64, &RiskDecisionRecord> = BTreeMap::new();
    let mut applied: BTreeMap<IntentId, DecisionOutcome> = BTreeMap::new();
    let mut max_revision = kill.as_ref().map_or(0, |record| record.revision);
    max_revision = max_revision.max(reconciliation.as_ref().map_or(0, |record| record.revision));
    let mut max_at_ns = 0_i64;
    let mut max_permit = 0_u64;

    for record in &decisions {
        if record.schema_version != 1 {
            return Err(RecoveryError::Inconsistent(format!(
                "unsupported decision schema {}",
                record.schema_version
            )));
        }
        if record.intent.account_id != config.account_id {
            return Err(RecoveryError::Inconsistent(
                "a durable decision belongs to another account".to_owned(),
            ));
        }
        record.intent.validate().map_err(|error| {
            RecoveryError::Inconsistent(format!("durable intent is invalid: {error}"))
        })?;
        max_revision = max_revision.max(record.revision);
        max_at_ns = max_at_ns.max(record.at_ns);
        match record.outcome {
            DecisionOutcome::Accepted => {
                apply_accepted(&mut state, record, sectors, &config.policy.version)?;
                by_permit.insert(record.permit_id, record);
                max_permit = max_permit.max(record.permit_id);
                applied.insert(record.intent_id, DecisionOutcome::Accepted);
            }
            DecisionOutcome::Rejected => {
                state
                    .decided
                    .insert(record.intent_id, record.intent.clone());
                applied.insert(record.intent_id, DecisionOutcome::Rejected);
            }
            DecisionOutcome::Released | DecisionOutcome::Filled => {
                let reservation = state
                    .reservations
                    .get(&record.intent_id)
                    .cloned()
                    .ok_or_else(|| {
                        RecoveryError::Inconsistent(
                            "a terminal decision has no prior reservation".to_owned(),
                        )
                    })?;
                if record.outcome == DecisionOutcome::Released {
                    apply_released(&mut state, record, &reservation, sectors)?;
                } else {
                    apply_filled(&mut state, record, &reservation, sectors)?;
                }
                applied.insert(record.intent_id, record.outcome);
            }
        }
    }

    let mut executions_by_permit: BTreeMap<u64, Vec<&ExecutionRecoveryRecord>> = BTreeMap::new();
    for execution in config.oms.executions() {
        executions_by_permit
            .entry(execution.permit_id)
            .or_default()
            .push(execution);
    }
    let no_executions: &[&ExecutionRecoveryRecord] = &[];

    let mut consumed_permits = BTreeSet::new();
    for oms in config.oms.permits() {
        if oms.schema_version != 1 {
            return Err(RecoveryError::Inconsistent(
                "unsupported OMS recovery schema".to_owned(),
            ));
        }
        let decision = by_permit.get(&oms.permit_id).ok_or_else(|| {
            RecoveryError::Inconsistent(format!(
                "OMS references permit {} with no accepted decision",
                oms.permit_id
            ))
        })?;
        if oms.intent_id != decision.intent_id {
            return Err(RecoveryError::Inconsistent(
                "OMS intent identity differs from the durable decision".to_owned(),
            ));
        }
        if !consumed_permits.insert(oms.permit_id) {
            return Err(RecoveryError::Inconsistent(
                "a permit was consumed by two OMS orders".to_owned(),
            ));
        }
        let expected = permit_digest(
            oms.permit_id,
            if decision.policy_version.is_empty() {
                &config.policy.version
            } else {
                &decision.policy_version
            },
            decision.revision,
            &decision.intent,
        );
        if expected != oms.permit_digest {
            return Err(RecoveryError::Digest(format!(
                "permit {} digest does not bind the durable intent",
                oms.permit_id
            )));
        }

        let authorized_quantity = decision.intent.quantity.get();
        let authorized_notional = checked_paise(decision.notional_paise)?;
        let ledger = executions_by_permit
            .get(&oms.permit_id)
            .map_or(no_executions, Vec::as_slice);
        let (summary, uncertain) =
            reconcile_executions(ledger, &decision.intent_id, authorized_quantity)?;
        let order_quantity = summary
            .as_ref()
            .map_or(authorized_quantity, |value| value.order_quantity);
        if matches!(
            oms.state,
            OmsPermitState::Filled | OmsPermitState::Cancelled
        ) && order_quantity != authorized_quantity
        {
            return Err(RecoveryError::Inconsistent(
                "a terminal OMS permit reports a corrected order quantity".to_owned(),
            ));
        }
        if oms.filled_quantity < 0
            || oms.filled_quantity > order_quantity
            || oms.filled_notional < 0
        {
            return Err(RecoveryError::Inconsistent(
                "an OMS permit reports an impossible execution".to_owned(),
            ));
        }
        // The OMS cumulative summary and the execution journal must describe
        // the same filled quantity and notional; neither is silently preferred.
        if let Some(found) = &summary
            && (oms.filled_quantity != found.filled_quantity
                || oms.filled_notional != found.notional_paise)
        {
            return Err(RecoveryError::Inconsistent(format!(
                "permit {} cumulative execution disagrees with the execution journal",
                oms.permit_id
            )));
        }
        if oms.filled_notional > authorized_notional.0 {
            return Err(RecoveryError::Inconsistent(
                "OMS filled notional exceeds the authorized notional".to_owned(),
            ));
        }
        let filled_quantity = oms.filled_quantity;
        let filled_notional = Paise(oms.filled_notional);
        let leaves_quantity = order_quantity
            .checked_sub(filled_quantity)
            .ok_or_else(|| RecoveryError::Inconsistent("leaves quantity underflow".to_owned()))?;
        if leaves_quantity < 0 {
            return Err(RecoveryError::Inconsistent(
                "cumulative executions exceed the order quantity".to_owned(),
            ));
        }
        if let Some(reported) = oms.leaves_quantity
            && matches!(oms.state, OmsPermitState::Open | OmsPermitState::Unknown)
            && reported != leaves_quantity
        {
            return Err(RecoveryError::Inconsistent(format!(
                "permit {} reports LeavesQty inconsistent with OrderQty and CumQty",
                oms.permit_id
            )));
        }
        match oms.state {
            OmsPermitState::Open => {
                if leaves_quantity <= 0 {
                    return Err(RecoveryError::Inconsistent(
                        "an open OMS permit has no leaves quantity".to_owned(),
                    ));
                }
            }
            OmsPermitState::Filled => {
                if filled_quantity != order_quantity || filled_notional != authorized_notional {
                    return Err(RecoveryError::Inconsistent(
                        "an OMS fill disagrees with the authorized terms".to_owned(),
                    ));
                }
            }
            OmsPermitState::Cancelled | OmsPermitState::Unknown => {}
        }

        let reservation = state.reservations.get(&decision.intent_id).cloned();
        let exposure = match &summary {
            Some(found) if found.filled_quantity > 0 => Some(ExecutionExposure::new(
                authorized_quantity,
                found.filled_quantity,
                found.leaves_quantity,
                Paise(found.notional_paise),
                Paise(found.fees_paise),
                Paise(found.costs_paise),
                found.execution_ids.clone(),
            )?),
            _ => None,
        };
        let leaves_notional = Paise(authorized_notional.0 - filled_notional.0);
        match oms.state {
            OmsPermitState::Open | OmsPermitState::Unknown => {
                apply_execution_to_state(
                    &mut state,
                    decision,
                    sectors,
                    filled_quantity,
                    filled_notional,
                    leaves_quantity,
                    leaves_notional,
                    oms.state,
                    exposure,
                )?;
            }
            OmsPermitState::Filled | OmsPermitState::Cancelled => {
                let expected = if oms.state == OmsPermitState::Filled {
                    DecisionOutcome::Filled
                } else {
                    DecisionOutcome::Released
                };
                match reservation {
                    Some(_) => apply_execution_to_state(
                        &mut state,
                        decision,
                        sectors,
                        filled_quantity,
                        filled_notional,
                        leaves_quantity,
                        leaves_notional,
                        oms.state,
                        None,
                    )?,
                    None if applied.get(&decision.intent_id) == Some(&expected) => {}
                    None => {
                        return Err(RecoveryError::Inconsistent(
                            "an OMS terminal outcome has no matching reservation".to_owned(),
                        ));
                    }
                }
            }
        }
        // A reported fill with no execution-level evidence is not reconciled:
        // no summary was synthesized, no zero-fee exposure was inserted, and
        // the authority stays NOT-READY until the owner resolves the gap.
        if filled_quantity > 0 && summary.is_none() && uncertain.is_none() {
            latch_reconciliation(
                &mut state,
                reconciliation.as_ref(),
                "an OMS fill has no execution-level evidence and cannot be reconciled".to_owned(),
                format!("permit-{}", oms.permit_id),
            )?;
        }
        if let Some(uncertain) = uncertain {
            latch_reconciliation(
                &mut state,
                reconciliation.as_ref(),
                uncertain.reason,
                uncertain.evidence_ref,
            )?;
        }
        if filled_quantity > 0 && filled_notional.0 == 0 {
            latch_reconciliation(
                &mut state,
                reconciliation.as_ref(),
                "an OMS fill reports no notional and cannot be priced".to_owned(),
                format!("permit-{}", oms.permit_id),
            )?;
        }
        if oms.state == OmsPermitState::Unknown {
            state.reconciliation_blocked = true;
            state.unresolved.insert(decision.intent_id);
        }
    }
    if executions_by_permit
        .keys()
        .any(|permit| !consumed_permits.contains(permit))
    {
        return Err(RecoveryError::Inconsistent(
            "execution reports reference a permit with no OMS record".to_owned(),
        ));
    }

    let mut account_positions: BTreeMap<InstrumentId, i64> = BTreeMap::new();
    for record in config.account.records() {
        if record.schema_version != 1 {
            return Err(RecoveryError::Inconsistent(
                "unsupported account recovery schema".to_owned(),
            ));
        }
        if record.account_id != config.account_id {
            return Err(RecoveryError::Inconsistent(
                "an account record belongs to another account".to_owned(),
            ));
        }
        let position = account_positions
            .entry(record.instrument_id.clone())
            .or_insert(0);
        *position = position.checked_add(record.quantity_delta).ok_or_else(|| {
            RecoveryError::Inconsistent("account position delta overflow".to_owned())
        })?;
    }
    account_positions.retain(|_, quantity| *quantity != 0);
    if account_positions != state.positions {
        return Err(RecoveryError::Inconsistent(
            "account journal positions disagree with the derived fill set".to_owned(),
        ));
    }

    state.next_permit = max_permit
        .checked_add(1)
        .ok_or_else(|| RecoveryError::Inconsistent("permit counter overflow".to_owned()))?;
    state.revision = max_revision;
    if max_at_ns > 0 {
        state.now = LogicalNanos::new(
            u64::try_from(max_at_ns)
                .map_err(|_| RecoveryError::Inconsistent("invalid recovered clock".to_owned()))?,
        );
        state.prune_order_times();
    }

    // Startup never invents a zero P&L. A missing or contradictory sealed
    // bundle is an explicit recovery failure, so the authority cannot reach a
    // state where the daily-loss limit silently sees no loss.
    let sealed_pnl = config.account.daily_pnl();
    let daily = sealed_pnl
        .record
        .as_ref()
        .ok_or_else(|| RecoveryError::DailyPnl("no sealed record".to_owned()))?;
    let calendar = sealed_pnl
        .calendar
        .as_ref()
        .ok_or_else(|| RecoveryError::DailyPnl("no business-day calendar".to_owned()))?;
    let snapshot = derive_daily_pnl_snapshot(
        daily,
        config.account.records(),
        &sealed_pnl.valuations,
        calendar,
        &state.positions,
        config.account_id,
        config.trade_date,
        state.now,
        ObservationPolicy::default().max_account_journal_age_ns,
    )
    .map_err(|error| RecoveryError::DailyPnl(error.to_string()))?;
    state.daily_pnl = Some(snapshot);
    Ok(state)
}

impl RiskAuthority {
    /// Reconstruct authority state from durable records before permitting any
    /// new risk. Ownership is exclusive, control and decision stores must be
    /// durable, and every supplied journal must have exact sequence and digest
    /// continuity. The resulting authority is still not-ready: fresh owner
    /// observations and an authenticated control resolution are required before
    /// a reservation can be issued.
    pub fn recover(config: RecoveryConfig) -> Result<Self, RecoveryError> {
        let ownership = AuthorityOwnership::acquire(&config.ownership_path)?;
        let sectors = config.sectors.clone();
        let state = reconstruct(&config, &sectors)?;
        let mut value = Self::build(
            config.account_id,
            config.policy,
            config.instruments,
            config.trade_date,
            BuildOptions {
                control: config.control,
                decision_store: Some(config.decisions),
                ownership: Some(ownership),
                startup_error: None,
                simulation: false,
                state,
            },
        );
        value.sectors = sectors;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::FileControlStore;
    use risk_engine::{
        RiskPolicy, //
        ValidatedRiskPolicy,
    };
    use std::process::Command;
    use std::thread;
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

    fn test_policy() -> ValidatedRiskPolicy {
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
        ValidatedRiskPolicy::from_policy(&raw).expect("policy")
    }

    fn test_authority() -> RiskAuthority {
        RiskAuthority::new(
            AccountId::new(1).expect("account"),
            test_policy(),
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

        let calendar = DailyPnlCalendar::new(
            "Asia/Kolkata".to_owned(),
            TradeDate::new(2026, 1, 1).expect("date"),
            -86_400_000_000_000,
            86_400_000_000_000,
        );
        let mut account = AccountRecoveryRecord::new(
            "unit-pnl".to_owned(),
            AccountId::new(1).expect("account"),
            InstrumentId::new("nse.cash.test").expect("instrument"),
            0,
        );
        account.seal(1, GENESIS_DIGEST);
        let mut record = DailyPnlRecord::new(
            AccountId::new(1).expect("account"),
            TradeDate::new(2026, 1, 1).expect("date"),
            "INR".to_owned(),
            DailyPnlBasis::RealizedPlusUnrealized,
            &calendar,
            0,
        )
        .with_journal(1, GENESIS_DIGEST.to_owned())
        .with_digests(
            daily_pnl_positions_digest(&BTreeMap::new()),
            GENESIS_DIGEST.to_owned(),
        );
        record.seal(1, GENESIS_DIGEST);

        let failures = [
            authority.set_now(LogicalNanos::new(1)).err(),
            authority.set_session_open(true).err(),
            authority.set_snapshot_available(true).err(),
            authority
                .publish_daily_pnl(
                    tokens.account_journal,
                    record,
                    std::slice::from_ref(&account),
                    &[],
                    &calendar,
                )
                .err()
                .map(|_| AuthorityError::AccountingOverflow("revision".to_owned())),
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

    #[test]
    fn recovery_digest_matches_the_sha256_reference_vector() {
        assert_eq!(
            digest::sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "the dependency-free digest matches the canonical vector"
        );
    }

    #[test]
    fn missing_journal_is_a_typed_failure_but_an_existing_empty_journal_is_not() {
        let missing = unique_path("missing-journal");
        let empty = unique_path("empty-journal");
        let _ = std::fs::remove_file(&missing);
        assert!(
            matches!(
                read_recovery_journal::<OmsPermitRecord>(&missing),
                Err(RecoveryError::JournalMissing(path)) if path == missing
            ),
            "an absent required journal never collapses into an empty journal"
        );

        write_lines(&empty, &Vec::<OmsPermitRecord>::new());
        assert_eq!(
            read_recovery_journal::<OmsPermitRecord>(&empty).expect("existing empty journal"),
            Vec::<OmsPermitRecord>::new(),
            "an existing journal with no records is a valid empty journal"
        );
        verify_recovery_chain(&Vec::<OmsPermitRecord>::new())
            .expect("an empty chain verifies from the genesis digest");

        let _ = std::fs::remove_file(&empty);
    }

    #[test]
    fn owner_signature_is_bound_to_identity_sequence_head_and_schema() {
        let key = OwnerSigningKey::new(b"unit-owner-key".to_vec()).expect("key");
        let identity = oms_identity();
        let derived =
            bind_recovery_journal(&[sealed_oms()], &Vec::<ExecutionRecoveryRecord>::new())
                .expect("derived binding");
        let binding = RecoveryBindingRecord::seal(
            &key,
            &identity,
            derived.record_count,
            derived.first_sequence,
            derived.last_sequence,
            &derived.head_digest,
            &derived.state_digest,
        )
        .expect("seal");
        binding
            .verify(&key, &identity, &derived)
            .expect("a correctly signed binding verifies");

        let mut forged_identity = identity.clone();
        forged_identity.journal_id = "some-other-journal".to_owned();
        assert!(
            matches!(
                binding.verify(&key, &forged_identity, &derived),
                Err(RecoveryError::Authenticity(_))
            ),
            "a signature cannot be replayed under another journal identity"
        );

        let other_key = OwnerSigningKey::new(b"attacker-key".to_vec()).expect("key");
        assert!(
            matches!(
                binding.verify(&other_key, &identity, &derived),
                Err(RecoveryError::Authenticity(_))
            ),
            "a signature made with another key is rejected"
        );

        let mut advanced = derived.clone();
        advanced.last_sequence = 2;
        assert!(
            matches!(
                binding.verify(&key, &identity, &advanced),
                Err(RecoveryError::Authenticity(_))
            ),
            "a signature does not authorize a different sequence"
        );

        assert_eq!(
            digest::hmac_sha256_hex(b"key", b"hello"),
            digest::hmac_sha256_hex(b"key", b"hello"),
            "the keyed digest is deterministic"
        );
        assert_ne!(
            digest::hmac_sha256_hex(b"key", b"hello"),
            digest::hmac_sha256_hex(b"other", b"hello"),
            "the keyed digest depends on the key"
        );
    }

    // Test-only constructors. They live under #[cfg(test)] and are the only
    // way a test can seal a snapshot without going through the journal reader;
    // production callers have no equivalent path.
    fn oms_identity() -> JournalIdentity {
        JournalIdentity::new(
            ObservationSource::Oms,
            AccountId::new(1).expect("account"),
            OMS_JOURNAL_ID,
        )
    }

    fn account_identity() -> JournalIdentity {
        JournalIdentity::new(
            ObservationSource::AccountJournal,
            AccountId::new(1).expect("account"),
            ACCOUNT_JOURNAL_ID,
        )
    }

    fn test_oms(
        permits: Vec<OmsPermitRecord>,
        executions: Vec<ExecutionRecoveryRecord>,
    ) -> VerifiedOmsSnapshot {
        let derived = bind_recovery_journal(&permits, &executions).expect("valid OMS journal");
        VerifiedOmsSnapshot {
            binding: SnapshotBinding {
                identity: oms_identity(),
                schema_version: 1,
                verification_version: RECOVERY_SNAPSHOT_VERSION,
                record_count: derived.record_count,
                first_sequence: derived.first_sequence,
                last_sequence: derived.last_sequence,
                head_digest: derived.head_digest,
                state_digest: derived.state_digest,
                replay_checkpoint: derived.replay_checkpoint,
            },
            permits,
            executions,
        }
    }

    fn test_account(
        records: Vec<AccountRecoveryRecord>,
        daily_pnl: DailyPnlRecovery,
    ) -> VerifiedAccountSnapshot {
        let derived = bind_recovery_journal(&records, &daily_pnl).expect("valid account journal");
        VerifiedAccountSnapshot {
            binding: SnapshotBinding {
                identity: account_identity(),
                schema_version: 1,
                verification_version: RECOVERY_SNAPSHOT_VERSION,
                record_count: derived.record_count,
                first_sequence: derived.first_sequence,
                last_sequence: derived.last_sequence,
                head_digest: derived.head_digest,
                state_digest: derived.state_digest,
                replay_checkpoint: derived.replay_checkpoint,
            },
            records,
            daily_pnl,
        }
    }

    fn sealed_oms() -> OmsPermitRecord {
        let mut record = OmsPermitRecord::new(
            1,
            IntentId::new(1).expect("intent"),
            "a".repeat(64),
            OmsPermitState::Open,
            0,
            0,
        );
        record.seal(1, genesis_digest());
        record
    }

    static NEXT_TEST_PATH: AtomicU64 = AtomicU64::new(1);

    fn unique_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "risk-authority-sealed-{label}-{}-{}.jsonl",
            std::process::id(),
            NEXT_TEST_PATH.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn write_lines<T: Serialize>(path: &Path, records: &[T]) {
        let mut text = String::new();
        for record in records {
            text.push_str(&serde_json::to_string(record).expect("serialize"));
            text.push('\n');
        }
        std::fs::write(path, text).expect("write journal");
    }

    struct TestRecoveryFiles {
        control: PathBuf,
        decisions: PathBuf,
        lock: PathBuf,
    }

    impl TestRecoveryFiles {
        fn new(label: &str) -> Self {
            Self {
                control: unique_path(&format!("{label}-control")),
                decisions: unique_path(&format!("{label}-decisions")),
                lock: unique_path(&format!("{label}-lock")),
            }
        }

        fn config(
            &self,
            oms: VerifiedOmsSnapshot,
            account: VerifiedAccountSnapshot,
        ) -> RecoveryConfig {
            RecoveryConfig {
                account_id: AccountId::new(1).expect("account"),
                policy: test_policy(),
                instruments: Vec::new(),
                trade_date: TradeDate::new(2026, 1, 1).expect("date"),
                control: Box::new(FileControlStore::new(&self.control)),
                decisions: Box::new(FileDecisionStore::new(&self.decisions)),
                oms,
                account,
                sectors: BTreeMap::new(),
                ownership_path: self.lock.clone(),
            }
        }

        fn cleanup(&self) {
            let _ = std::fs::remove_file(&self.control);
            let _ = std::fs::remove_file(&self.decisions);
            let _ = std::fs::remove_file(&self.lock);
        }
    }

    #[test]
    fn snapshot_verifier_rejects_corrupt_journal_data() {
        let permits = unique_path("corrupt-permits");
        let executions = unique_path("corrupt-executions");
        let accounts = unique_path("corrupt-accounts");
        let pnl = unique_path("corrupt-pnl");
        let broken_chain = unique_path("broken-chain");
        // A required companion journal still has to exist; it is an empty,
        // existing record stream, not a missing path silently read as empty.
        write_lines(&executions, &Vec::<ExecutionRecoveryRecord>::new());
        std::fs::write(&permits, "{not-json}\n").expect("write corrupt journal");
        // Chain-corruption checks use a signing-key posture so the test does
        // not depend on host-specific /tmp directory ownership semantics.
        let owner_directory = RecoveryAuthenticity::OwnerSignature(
            OwnerSigningKey::new(b"unit-owner-key".to_vec()).expect("key"),
        );
        assert!(
            matches!(
                VerifiedOmsSnapshot::verify(
                    &permits,
                    &executions,
                    &oms_identity(),
                    &owner_directory
                ),
                Err(RecoveryError::Snapshot(_))
            ),
            "an unparseable OMS journal cannot be sealed"
        );

        let mut out_of_sequence = sealed_oms();
        out_of_sequence.seal(2, genesis_digest());
        write_lines(&broken_chain, std::slice::from_ref(&out_of_sequence));
        assert!(
            matches!(
                VerifiedOmsSnapshot::verify(
                    &broken_chain,
                    &executions,
                    &oms_identity(),
                    &owner_directory
                ),
                Err(RecoveryError::Sequence(_))
            ),
            "a broken OMS chain cannot be sealed"
        );

        std::fs::write(&accounts, "{not-json}\n").expect("write corrupt account journal");
        assert!(
            matches!(
                VerifiedAccountSnapshot::verify(
                    &accounts,
                    &pnl,
                    &account_identity(),
                    &owner_directory
                ),
                Err(RecoveryError::Snapshot(_))
            ),
            "an unparseable account journal cannot be sealed"
        );
        std::fs::write(&pnl, "{not-json}\n").expect("write corrupt pnl bundle");
        assert!(
            VerifiedAccountSnapshot::verify(
                &executions,
                &pnl,
                &account_identity(),
                &owner_directory
            )
            .is_err(),
            "a corrupt daily pnl bundle cannot be sealed"
        );

        for path in [&permits, &executions, &accounts, &pnl, &broken_chain] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn snapshot_binding_rejects_wrong_identity_and_version() {
        let account = AccountId::new(1).expect("account");
        let base = test_oms(Vec::new(), Vec::new());
        assert!(
            base.validate(account).is_ok(),
            "a correctly bound fixture validates"
        );

        let mut wrong_account = base.clone();
        wrong_account.binding.identity.account_id = AccountId::new(9).expect("other account");
        assert!(
            matches!(
                wrong_account.validate(account),
                Err(RecoveryError::Snapshot(_))
            ),
            "a snapshot from another account is rejected"
        );

        let mut wrong_journal = base.clone();
        wrong_journal.binding.identity.journal_id = "another-journal".to_owned();
        assert!(
            matches!(
                wrong_journal.validate(account),
                Err(RecoveryError::Snapshot(_))
            ),
            "a snapshot from another journal is rejected"
        );

        let mut wrong_source = base.clone();
        wrong_source.binding.identity.source = ObservationSource::AccountJournal;
        assert!(
            matches!(
                wrong_source.validate(account),
                Err(RecoveryError::Snapshot(_))
            ),
            "a snapshot from the wrong owner is rejected"
        );

        let mut wrong_version = base.clone();
        wrong_version.binding.verification_version = RECOVERY_SNAPSHOT_VERSION + 1;
        assert!(
            matches!(
                wrong_version.validate(account),
                Err(RecoveryError::Snapshot(_))
            ),
            "an unsupported verification version is rejected"
        );

        let mut wrong_schema = base;
        wrong_schema.binding.schema_version = 2;
        assert!(
            matches!(
                wrong_schema.validate(account),
                Err(RecoveryError::Snapshot(_))
            ),
            "an unsupported journal schema is rejected"
        );
    }

    #[test]
    fn snapshot_binding_rejects_count_sequence_head_and_state_mismatches() {
        let account = AccountId::new(1).expect("account");
        let base = test_oms(vec![sealed_oms()], Vec::new());
        assert!(
            base.validate(account).is_ok(),
            "a correctly bound fixture validates"
        );

        type BindingEdit = fn(&mut VerifiedOmsSnapshot);
        let cases: [(&str, BindingEdit); 6] = [
            ("record count mismatch", |snapshot| {
                snapshot.binding.record_count = 2;
            }),
            ("first sequence mismatch", |snapshot| {
                snapshot.binding.first_sequence = 2;
            }),
            ("last sequence mismatch", |snapshot| {
                snapshot.binding.last_sequence = 2;
            }),
            ("head digest mismatch", |snapshot| {
                snapshot.binding.head_digest = "f".repeat(64);
            }),
            ("state digest mismatch", |snapshot| {
                snapshot.binding.state_digest = "f".repeat(64);
            }),
            ("replay checkpoint mismatch", |snapshot| {
                snapshot.binding.replay_checkpoint = Some(RecoveryCheckpoint {
                    sequence: 1,
                    digest: "f".repeat(64),
                });
            }),
        ];
        for (expected, edit) in cases {
            let mut tampered = base.clone();
            edit(&mut tampered);
            assert!(
                matches!(
                    tampered.validate(account),
                    Err(RecoveryError::Snapshot(message)) if message == expected
                ),
                "{expected} is detected by re-derivation"
            );
        }

        let mut corrupt_records = base;
        corrupt_records.permits[0].digest = "f".repeat(64);
        assert!(
            matches!(
                corrupt_records.validate(account),
                Err(RecoveryError::Digest(_))
            ),
            "corrupted records are re-verified before the snapshot is trusted"
        );
    }

    #[test]
    fn recover_rejects_foreign_and_tampered_snapshots() {
        let files = TestRecoveryFiles::new("sealed-reject");
        let oms = test_oms(Vec::new(), Vec::new());

        let mut foreign = test_account(Vec::new(), DailyPnlRecovery::default());
        foreign.binding.identity.account_id = AccountId::new(9).expect("other account");
        let config = files.config(oms.clone(), foreign);
        assert!(
            matches!(
                reconstruct(&config, &BTreeMap::new()),
                Err(RecoveryError::Snapshot(_))
            ),
            "recovery rejects an account snapshot from another account"
        );

        let mut mismatched = test_account(Vec::new(), DailyPnlRecovery::default());
        mismatched.binding.head_digest = "f".repeat(64);
        let config = files.config(oms, mismatched);
        assert!(
            matches!(
                reconstruct(&config, &BTreeMap::new()),
                Err(RecoveryError::Snapshot(_))
            ),
            "recovery rejects a tampered account snapshot binding"
        );
        files.cleanup();
    }

    // --- Cross-process recovery transaction invariants ---------------------

    fn recovery_test_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "nirnay-recovery-tx-{label}-{}-{}",
            std::process::id(),
            NEXT_TEST_PATH.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("recovery fixture root");
        root
    }

    fn write_recovery_value<T: Serialize>(path: &Path, value: &T) {
        let text = serde_json::to_string(value).expect("serialize recovery value");
        std::fs::write(path, text).expect("write recovery value");
    }

    /// Minimal, fully signed recovery root at a chosen anchor/high-water
    /// counter. Empty journals are valid when they are signed and anchored.
    fn write_signed_recovery_root(root: &Path, key: &OwnerSigningKey, counter: u64) {
        let empty_executions: Vec<ExecutionRecoveryRecord> = Vec::new();
        let empty_permits: Vec<OmsPermitRecord> = Vec::new();
        let empty_accounts: Vec<AccountRecoveryRecord> = Vec::new();
        write_lines(&root.join(OMS_PERMITS_FILE), &empty_permits);
        write_lines(&root.join(OMS_EXECUTIONS_FILE), &empty_executions);
        write_lines(&root.join(ACCOUNT_JOURNAL_FILE), &empty_accounts);
        write_recovery_value(&root.join(DAILY_PNL_FILE), &DailyPnlRecovery::default());
        let oms_binding =
            recovery_binding_record(key, &oms_identity(), &empty_permits, &empty_executions)
                .expect("oms binding");
        let account_binding = recovery_binding_record(
            key,
            &account_identity(),
            &empty_accounts,
            &DailyPnlRecovery::default(),
        )
        .expect("account binding");
        write_recovery_value(
            &root.join(format!("{OMS_PERMITS_FILE}{BINDING_SUFFIX}")),
            &oms_binding,
        );
        write_recovery_value(
            &root.join(format!("{ACCOUNT_JOURNAL_FILE}{BINDING_SUFFIX}")),
            &account_binding,
        );
        let genesis = genesis_digest();
        write_recovery_value(
            &root.join(OMS_ANCHOR_FILE),
            &RecoveryAnchor::seal(key, &oms_identity(), counter, 0, genesis).expect("oms anchor"),
        );
        write_recovery_value(
            &root.join(ACCOUNT_ANCHOR_FILE),
            &RecoveryAnchor::seal(key, &account_identity(), counter, 0, genesis)
                .expect("account anchor"),
        );
        write_recovery_value(
            &root.join(OMS_HIGH_WATER_FILE),
            &RecoveryHighWater::seal(key, &oms_identity(), counter).expect("oms high water"),
        );
        write_recovery_value(
            &root.join(ACCOUNT_HIGH_WATER_FILE),
            &RecoveryHighWater::seal(key, &account_identity(), counter)
                .expect("account high water"),
        );
    }

    fn read_high_water_counter(path: &Path) -> u64 {
        let text = std::fs::read_to_string(path).expect("read high water");
        let record: RecoveryHighWater = serde_json::from_str(&text).expect("parse high water");
        record.counter
    }

    fn recovery_gate_channels() -> (
        std::sync::mpsc::Sender<()>,
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::Sender<()>,
    ) {
        let (reached_tx, reached_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        (reached_tx, reached_rx, release_rx, release_tx)
    }

    /// Install the gate on the calling thread. Recovery threads call this
    /// before `load`; the gate is thread-local so unrelated loads are no-ops.
    fn install_recovery_gate(
        reached_tx: std::sync::mpsc::Sender<()>,
        release_rx: std::sync::mpsc::Receiver<()>,
    ) {
        RECOVERY_ADVANCE_GATE.with(|slot| {
            *slot.borrow_mut() = Some((reached_tx, release_rx));
        });
    }

    #[cfg(unix)]
    #[allow(unsafe_code)] // kernel file lock is only exposed through libc FFI
    fn recovery_lock_is_held(root: &Path) -> bool {
        use std::os::unix::io::AsRawFd;

        unsafe extern "C" {
            fn flock(fd: i32, operation: i32) -> i32;
        }

        let path = root.join(RECOVERY_LOCK_FILE);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .expect("open recovery lock file");
        // LOCK_EX | LOCK_NB: a held kernel lock makes this fail immediately.
        unsafe { flock(file.as_raw_fd(), 2 | 4) != 0 }
    }

    fn marker_path(root: &Path, name: &str) -> PathBuf {
        root.join(format!("{name}.marker"))
    }

    fn publish_marker(path: &Path) {
        let staged = PathBuf::from(format!("{}.tmp", path.display()));
        std::fs::write(&staged, b"1").expect("stage marker");
        std::fs::rename(&staged, path).expect("publish marker");
    }

    /// Bounded, time-free wait for a cross-process marker. Ordering comes from
    /// file publication, never from a sleep, PID or lock-file age heuristic.
    fn wait_for_marker(path: &Path) {
        for _ in 0..10_000_000 {
            if path.exists() {
                return;
            }
            thread::yield_now();
        }
        panic!("marker was not published: {}", path.display());
    }

    #[test]
    #[cfg(unix)]
    fn concurrent_loaders_never_regress_the_high_water() {
        let key = OwnerSigningKey::new(b"recovery-loader-lock-key".to_vec()).expect("key");
        let root = recovery_test_root("loader-contention");
        write_signed_recovery_root(&root, &key, 12);
        let high_water = root.join(OMS_HIGH_WATER_FILE);
        let mut observed = vec![read_high_water_counter(&high_water)];

        let (first_reached_tx, first_reached_rx, first_release_rx, first_release_tx) =
            recovery_gate_channels();
        let first_root = root.clone();
        let first_key = key.clone();
        let (first_done_tx, first_done_rx) = std::sync::mpsc::channel();
        let first = thread::spawn(move || {
            install_recovery_gate(first_reached_tx, first_release_rx);
            let result = RecoveryJournalRoot::new(
                first_root.clone(),
                RecoveryAuthenticity::OwnerSignature(first_key),
            )
            .load(AccountId::new(1).expect("account"));
            let committed = read_high_water_counter(&first_root.join(OMS_HIGH_WATER_FILE));
            first_done_tx
                .send((result, committed))
                .expect("send first loader result");
        });

        first_reached_rx
            .recv()
            .expect("first loader reached the advance gate");
        assert!(
            recovery_lock_is_held(&root),
            "the first loader holds the kernel lock"
        );

        let second_root = root.clone();
        let second_key = key.clone();
        let (second_reached_tx, second_reached_rx, second_release_rx, second_release_tx) =
            recovery_gate_channels();
        let (second_done_tx, second_done_rx) = std::sync::mpsc::channel();
        let second = thread::spawn(move || {
            install_recovery_gate(second_reached_tx, second_release_rx);
            let result = RecoveryJournalRoot::new(
                second_root.clone(),
                RecoveryAuthenticity::OwnerSignature(second_key),
            )
            .load(AccountId::new(1).expect("account"));
            let committed = read_high_water_counter(&second_root.join(OMS_HIGH_WATER_FILE));
            second_done_tx
                .send((result, committed))
                .expect("send second loader result");
        });

        first_release_tx.send(()).expect("release first loader");
        let (first_result, first_committed) = first_done_rx.recv().expect("first loader completed");
        assert!(first_result.is_ok(), "the first loader verified its anchor");
        // The second loader reaches its gate only after it acquires the lock
        // the first loader released, and it has not advanced yet.
        second_reached_rx
            .recv()
            .expect("second loader reached the advance gate");
        assert_eq!(
            first_committed, 12,
            "the older loader committed the counter it verified"
        );
        let intermediate = read_high_water_counter(&high_water);
        assert_eq!(
            intermediate, first_committed,
            "the on-disk counter immediately after the first commit is observed"
        );
        observed.push(intermediate);

        second_release_tx.send(()).expect("release second loader");
        let (second_result, second_committed) =
            second_done_rx.recv().expect("second loader completed");
        assert!(
            second_result.is_ok(),
            "the second loader verified its anchor"
        );
        assert_eq!(
            second_committed, 12,
            "the cooperating loader commits the same counter"
        );
        let _ = first.join();
        let _ = second.join();
        observed.push(read_high_water_counter(&high_water));

        assert!(
            observed.windows(2).all(|pair| pair[0] <= pair[1]),
            "two cooperating loaders never regress the high-water: {observed:?}"
        );
        assert_eq!(
            observed.last(),
            Some(&12),
            "two cooperating loaders persist the maximum: {observed:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn recovery_writer_contending_with_loader_never_regresses_the_high_water() {
        let key = OwnerSigningKey::new(b"recovery-writer-lock-key".to_vec()).expect("key");
        let root = recovery_test_root("writer-contention");
        write_signed_recovery_root(&root, &key, 11);
        let high_water = root.join(OMS_HIGH_WATER_FILE);
        let ownership_path = root.with_extension("ownership.sentinel");
        let mut observed = vec![read_high_water_counter(&high_water)];

        let (reached_tx, reached_rx, release_rx, release_tx) = recovery_gate_channels();
        let loader_root = root.clone();
        let loader_key = key.clone();
        let (loader_done_tx, loader_done_rx) = std::sync::mpsc::channel();
        let loader = thread::spawn(move || {
            install_recovery_gate(reached_tx, release_rx);
            let result = RecoveryJournalRoot::new(
                loader_root.clone(),
                RecoveryAuthenticity::OwnerSignature(loader_key),
            )
            .load(AccountId::new(1).expect("account"));
            let committed = read_high_water_counter(&loader_root.join(OMS_HIGH_WATER_FILE));
            loader_done_tx
                .send((result, committed))
                .expect("send loader result");
        });
        reached_rx.recv().expect("loader reached the advance gate");
        assert!(
            recovery_lock_is_held(&root),
            "the loader holds the M3 kernel lock"
        );

        let writer_root = root.clone();
        let writer_key = key.clone();
        let writer_ownership = ownership_path.clone();
        let (attempt_tx, attempt_rx) = std::sync::mpsc::channel();
        let (writer_done_tx, writer_done_rx) = std::sync::mpsc::channel();
        let writer = thread::spawn(move || {
            attempt_tx.send(()).expect("signal writer attempt");
            let ownership = AuthorityOwnership::acquire(&writer_ownership).expect("M4 sentinel");
            let journal_root = RecoveryJournalRoot::new(
                writer_root.clone(),
                RecoveryAuthenticity::OwnerSignature(writer_key.clone()),
            );
            let guard = ownership
                .recovery_writer(&journal_root)
                .expect("M3 writer lock");
            // Publish the newer counter only under the same lock the loader
            // uses; the old helper write bypassed this protocol.
            write_signed_recovery_root(&writer_root, &writer_key, 12);
            drop(guard);
            drop(ownership);
            writer_done_tx.send(()).expect("signal writer done");
        });

        attempt_rx.recv().expect("writer attempted to publish");
        assert!(
            matches!(
                writer_done_rx.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ),
            "the writer cannot finish while the loader holds the kernel lock"
        );
        assert_eq!(
            read_high_water_counter(&high_water),
            11,
            "the writer cannot publish while the loader holds the kernel lock"
        );

        release_tx.send(()).expect("release loader");
        let (loader_result, loader_committed) = loader_done_rx.recv().expect("loader completed");
        assert!(loader_result.is_ok(), "the loader verified counter 11");
        assert_eq!(
            loader_committed, 11,
            "the loader committed the older counter"
        );
        let intermediate = read_high_water_counter(&high_water);
        assert_eq!(
            intermediate, loader_committed,
            "the on-disk counter immediately after the loader commit is observed"
        );
        observed.push(intermediate);

        writer_done_rx.recv().expect("writer completed");
        let _ = writer.join();
        let _ = loader.join();
        observed.push(read_high_water_counter(&high_water));
        assert!(
            !ownership_path.exists(),
            "the M4 sentinel is released when the writer returns"
        );
        assert!(
            !recovery_lock_is_held(&root),
            "the M3 lock is released when the writer returns"
        );
        assert!(
            observed.windows(2).all(|pair| pair[0] <= pair[1]),
            "a protocol-compliant writer never regresses the high-water: {observed:?}"
        );
        assert_eq!(
            observed.last().copied(),
            Some(12),
            "the writer's newer counter is persisted: {observed:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn spawned_recovery_processes_never_regress_the_high_water() {
        let root = recovery_test_root("spawned-processes");
        let key = OwnerSigningKey::new(b"recovery-serialization-key".to_vec()).expect("key");
        write_signed_recovery_root(&root, &key, 11);
        let high_water = root.join(OMS_HIGH_WATER_FILE);
        let older_at_gate = marker_path(&root, "older-at-gate");
        let older_release = marker_path(&root, "older-release");
        let older_done = marker_path(&root, "older-done");
        let writer_attempting = marker_path(&root, "writer-attempting");
        let writer_done = marker_path(&root, "writer-done");
        let parent_sampled = marker_path(&root, "parent-sampled");
        let newer_done = marker_path(&root, "newer-done");
        let binary = std::env::current_exe().expect("test binary");

        let spawn = |role: &str| {
            Command::new(&binary)
                .arg("--ignored")
                .arg("--exact")
                .arg("authority::tests::recovery_child_worker")
                .env("NIRNAY_M3_ROOT", &root)
                .env("NIRNAY_M3_ROLE", role)
                .spawn()
                .expect("spawn recovery child")
        };

        let mut observed = vec![read_high_water_counter(&high_water)];
        let mut older = spawn("older_loader");
        wait_for_marker(&older_at_gate);
        assert!(
            recovery_lock_is_held(&root),
            "the older child holds the kernel lock"
        );

        let mut writer = spawn("writer");
        wait_for_marker(&writer_attempting);
        assert_eq!(
            read_high_water_counter(&high_water),
            11,
            "the writer child waits for the kernel lock"
        );

        publish_marker(&older_release);
        wait_for_marker(&older_done);
        let older_counter = std::fs::read_to_string(&older_done).expect("read older counter");
        assert_eq!(
            older_counter.trim(),
            "11",
            "the older child committed its verified counter"
        );
        let committed = older_counter.trim().parse::<u64>().expect("parse counter");
        let intermediate = read_high_water_counter(&high_water);
        assert_eq!(intermediate, committed, "disk counter after older commit");
        observed.push(intermediate);
        // The writer holds the lock by now but waits for this marker before
        // publishing, so the parent's intermediate observation is deterministic.
        publish_marker(&parent_sampled);

        wait_for_marker(&writer_done);
        observed.push(read_high_water_counter(&high_water));

        let mut newer = spawn("newer_loader");
        wait_for_marker(&newer_done);
        observed.push(read_high_water_counter(&high_water));

        assert!(
            older.wait().expect("older child status").success(),
            "the older child succeeded"
        );
        assert!(
            writer.wait().expect("writer child status").success(),
            "the writer child succeeded"
        );
        assert!(
            newer.wait().expect("newer child status").success(),
            "the newer child succeeded"
        );
        assert!(
            observed.windows(2).all(|pair| pair[0] <= pair[1]),
            "persisted high-water never decreases across processes: {observed:?}"
        );
        assert_eq!(
            observed.last().copied(),
            Some(12),
            "the maximum committed counter is persisted: {observed:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Child-only worker. The parent forces ordering with published marker
    /// files; the child never uses a sleep, PID or lock age to decide.
    #[test]
    #[ignore = "child-only recovery worker invoked by the parent test"]
    #[cfg(unix)]
    fn recovery_child_worker() {
        let root = PathBuf::from(std::env::var_os("NIRNAY_M3_ROOT").expect("child root"));
        let role = std::env::var("NIRNAY_M3_ROLE").expect("child role");
        let key = OwnerSigningKey::new(b"recovery-serialization-key".to_vec()).expect("key");
        match role.as_str() {
            "older_loader" => {
                let (reached_tx, reached_rx, release_rx, release_tx) = recovery_gate_channels();
                install_recovery_gate(reached_tx, release_rx);
                let reached = marker_path(&root, "older-at-gate");
                let release = marker_path(&root, "older-release");
                let notifier = thread::spawn(move || {
                    reached_rx.recv().expect("child reached the advance gate");
                    publish_marker(&reached);
                });
                let releaser = thread::spawn(move || {
                    wait_for_marker(&release);
                    release_tx.send(()).expect("release child gate");
                });
                let result = RecoveryJournalRoot::new(
                    root.clone(),
                    RecoveryAuthenticity::OwnerSignature(key),
                )
                .load(AccountId::new(1).expect("account"));
                assert!(result.is_ok(), "older child load succeeded");
                let counter = read_high_water_counter(&root.join(OMS_HIGH_WATER_FILE));
                notifier.join().expect("child notifier");
                releaser.join().expect("child releaser");
                std::fs::write(marker_path(&root, "older-done"), counter.to_string())
                    .expect("write older counter");
            }
            "writer" => {
                wait_for_marker(&marker_path(&root, "older-at-gate"));
                publish_marker(&marker_path(&root, "writer-attempting"));
                let sentinel = root.with_extension("child-ownership.sentinel");
                let ownership = AuthorityOwnership::acquire(&sentinel).expect("child M4 sentinel");
                let journal_root = RecoveryJournalRoot::new(
                    root.clone(),
                    RecoveryAuthenticity::OwnerSignature(key.clone()),
                );
                let guard = ownership
                    .recovery_writer(&journal_root)
                    .expect("child M3 writer lock");
                wait_for_marker(&marker_path(&root, "parent-sampled"));
                write_signed_recovery_root(&root, &key, 12);
                drop(guard);
                drop(ownership);
                publish_marker(&marker_path(&root, "writer-done"));
            }
            "newer_loader" => {
                wait_for_marker(&marker_path(&root, "writer-done"));
                let result =
                    RecoveryJournalRoot::new(&root, RecoveryAuthenticity::OwnerSignature(key))
                        .load(AccountId::new(1).expect("account"));
                assert!(result.is_ok(), "newer child load succeeded");
                publish_marker(&marker_path(&root, "newer-done"));
            }
            other => panic!("unknown recovery child role: {other}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn writer_guard_releases_both_locks_on_every_path() {
        let key = OwnerSigningKey::new(b"recovery-writer-error-key".to_vec()).expect("key");
        let root = recovery_test_root("writer-error-paths");
        write_signed_recovery_root(&root, &key, 3);
        let ownership_path = root.with_extension("ownership-error.sentinel");

        std::fs::write(&ownership_path, b"stale").expect("stale sentinel");
        assert!(
            AuthorityOwnership::acquire(&ownership_path).is_err(),
            "a stale M4 sentinel is refused"
        );
        assert!(
            !recovery_lock_is_held(&root),
            "no M3 lock is taken when M4 acquisition fails"
        );
        std::fs::remove_file(&ownership_path).expect("remove stale sentinel");

        let target = root.with_extension("lock-target");
        let _ = std::fs::remove_file(root.join(RECOVERY_LOCK_FILE));
        std::os::unix::fs::symlink(&target, root.join(RECOVERY_LOCK_FILE))
            .expect("symlink recovery lock");
        let ownership = AuthorityOwnership::acquire(&ownership_path).expect("M4 sentinel");
        let journal_root =
            RecoveryJournalRoot::new(root.clone(), RecoveryAuthenticity::OwnerSignature(key));
        assert!(
            ownership.recovery_writer(&journal_root).is_err(),
            "a non-regular recovery lock is refused"
        );
        drop(ownership);
        assert!(
            !ownership_path.exists(),
            "the M4 sentinel is removed after M3 acquisition fails"
        );
        assert!(!recovery_lock_is_held(&root), "no M3 lock was acquired");

        std::fs::remove_file(root.join(RECOVERY_LOCK_FILE)).expect("remove symlink");
        let ownership = AuthorityOwnership::acquire(&ownership_path).expect("M4 sentinel");
        let guard = ownership
            .recovery_writer(&journal_root)
            .expect("M3 writer lock");
        assert!(recovery_lock_is_held(&root), "the guard holds the M3 lock");
        drop(guard);
        assert!(
            !recovery_lock_is_held(&root),
            "the guard releases the M3 lock on drop"
        );
        assert!(
            ownership_path.exists(),
            "the M4 sentinel outlives the guard"
        );
        drop(ownership);
        assert!(
            !ownership_path.exists(),
            "the M4 sentinel is released on drop"
        );

        let _ = std::fs::remove_file(&target);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn recovery_lock_is_held_until_the_transaction_finishes() {
        let key = OwnerSigningKey::new(b"recovery-lock-key".to_vec()).expect("key");
        let root = recovery_test_root("lock-scope");
        write_signed_recovery_root(&root, &key, 3);

        let (reached_tx, reached_rx, release_rx, release_tx) = recovery_gate_channels();
        let loader_root = root.clone();
        let loader_key = key.clone();
        let loader = thread::spawn(move || {
            install_recovery_gate(reached_tx, release_rx);
            RecoveryJournalRoot::new(
                loader_root,
                RecoveryAuthenticity::OwnerSignature(loader_key),
            )
            .load(AccountId::new(1).expect("account"))
        });

        reached_rx
            .recv()
            .expect("recovery reached the advance gate");
        assert!(
            recovery_lock_is_held(&root),
            "the kernel lock is held across verification and advance"
        );
        release_tx.send(()).expect("release recovery");
        assert!(
            loader.join().expect("loader thread").is_ok(),
            "the gated recovery succeeds"
        );
        assert!(
            !recovery_lock_is_held(&root),
            "the kernel lock is released only after the transaction finishes"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(unix)]
    fn pinned_directory_identity_change_is_rejected() {
        let key = OwnerSigningKey::new(b"recovery-dir-pin-key".to_vec()).expect("key");
        let root = recovery_test_root("dir-swap");
        write_signed_recovery_root(&root, &key, 3);
        let swapped = root.with_extension("swapped");
        let _ = std::fs::remove_dir_all(&swapped);

        let (reached_tx, reached_rx, release_rx, release_tx) = recovery_gate_channels();
        let loader_root = root.clone();
        let loader_key = key.clone();
        let loader = thread::spawn(move || {
            install_recovery_gate(reached_tx, release_rx);
            RecoveryJournalRoot::new(
                loader_root,
                RecoveryAuthenticity::OwnerSignature(loader_key),
            )
            .load(AccountId::new(1).expect("account"))
        });

        reached_rx
            .recv()
            .expect("recovery reached the advance gate");
        std::fs::rename(&root, &swapped).expect("swap recovery directory");
        std::fs::create_dir_all(&root).expect("replacement recovery directory");
        write_signed_recovery_root(&root, &key, 3);
        release_tx.send(()).expect("release recovery");

        let result = loader.join().expect("loader thread");
        assert!(
            matches!(result, Err(RecoveryError::Authenticity(_))),
            "a directory identity change after pinning is rejected"
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&swapped);
    }

    #[test]
    #[cfg(unix)]
    fn high_water_symlink_is_refused() {
        let key = OwnerSigningKey::new(b"recovery-nofollow-key".to_vec()).expect("key");
        let root = recovery_test_root("nofollow");
        write_signed_recovery_root(&root, &key, 3);
        let target = root.with_extension("high-water-target");
        write_recovery_value(
            &target,
            &RecoveryHighWater::seal(&key, &oms_identity(), 3).expect("target high water"),
        );
        let high_water = root.join(OMS_HIGH_WATER_FILE);
        std::fs::remove_file(&high_water).expect("remove regular high water");
        std::os::unix::fs::symlink(&target, &high_water).expect("link high water");

        let result =
            RecoveryJournalRoot::new(root.clone(), RecoveryAuthenticity::OwnerSignature(key))
                .load(AccountId::new(1).expect("account"));
        assert!(
            matches!(result, Err(RecoveryError::Authenticity(_))),
            "a symlinked high-water record is refused"
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&target);
    }
}
