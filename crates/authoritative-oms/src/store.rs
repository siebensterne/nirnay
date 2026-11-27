//! The single authoritative OMS store: an append-only commit journal that
//! persists the complete order, attempt/outbox and event state after every
//! successful transition. One append + `sync_all` is the atomic commit point;
//! replay requires a contiguous sequence chain, so a truncated or corrupt tail
//! is refused rather than silently accepted.
//!
//! Acknowledgement levels (all local to this process and its filesystem):
//! 1. local queue admission — the command is accepted into the in-process outbox;
//! 2. local write — the record bytes were handed to the local writer;
//! 3. local flush — the buffered writer was flushed to the OS;
//! 4. local sync — `sync_all` returned and the append is locally durable.
//!
//! No replication is claimed at any level, and no D4/D5 exposure is implied.
//! Fencing terms are persisted before a new owner is allowed to write; `flock`
//! from M3 is local mutual exclusion only and is not distributed fencing.
//!
//! Every durable record carries the current term in its replayed state. The
//! legacy write methods remain for single-process callers that hold the local
//! lock. The `*_fenced` methods are the authoritative owner-facing surface:
//! they require a token and reject stale terms at the sink before any write.

use std::{
    fmt, //
    fs::{
        File, //
        OpenOptions,
    },
    io::{
        BufRead, //
        BufReader,
        ErrorKind,
        Write,
    },
    num::NonZeroU64,
    path::{
        Path, //
        PathBuf,
    },
};

use fs2::FileExt;
use trading_core::Side;

use serde::{
    Deserialize, //
    Serialize,
};
use sha2::{
    Digest, //
    Sha256,
};
use trading_core::{
    OrderIntent, //
    permit_digest,
};

use crate::{
    error::OmsError, //
    lifecycle::{OmsOrderState, OutboxState},
};

/// Client order identity, assigned by the caller and idempotent across retries.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClientOrderId(NonZeroU64);

impl ClientOrderId {
    pub fn new(value: u64) -> Result<Self, OmsError> {
        NonZeroU64::new(value).map(Self).ok_or_else(|| {
            OmsError::InvalidIdentifier("client order id must be non-zero".to_owned())
        })
    }
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Display for ClientOrderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0.get())
    }
}

/// Opaque broker order identity bound only after an acknowledgement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BrokerOrderId(String);

impl BrokerOrderId {
    pub fn new(value: impl Into<String>) -> Result<Self, OmsError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(OmsError::InvalidIdentifier(
                "broker order id must not be empty".to_owned(),
            ));
        }
        Ok(Self(value))
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque reference to the risk permit that authorized this order. The OMS
/// stores it as evidence; it does not depend on the risk-authority crate.
/// `digest` is the deterministic, unkeyed SHA-256 binding to the exact
/// `OrderIntent`; the OMS recomputes it on `create` and denies a mismatch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermitRef {
    pub permit_id: u64,
    pub policy_version: String,
    pub snapshot_revision: u64,
    pub digest: String,
}

/// A claim returned to a dispatcher. It carries the complete typed order
/// request; the caller must submit exactly these terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchClaim {
    pub attempt_id: i64,
    pub client_order_id: ClientOrderId,
    pub command_type: String,
    pub intent: OrderIntent,
}

/// Read-only view of one order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderView {
    pub client_order_id: ClientOrderId,
    pub intent: OrderIntent,
    pub permit: PermitRef,
    pub state: OmsOrderState,
    pub broker_order_id: Option<BrokerOrderId>,
    pub filled_quantity: i64,
    pub filled_notional: i64,
    pub created_at_ns: i64,
    pub updated_at_ns: i64,
}

/// Read-only view of one outbox / attempt row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboxView {
    pub attempt_id: i64,
    pub client_order_id: ClientOrderId,
    pub command_type: String,
    pub state: OutboxState,
    pub dispatched_at_ns: Option<i64>,
    pub broker_order_id: Option<BrokerOrderId>,
}

/// How a reconciled UNKNOWN order resolved at the broker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    Working {
        broker_order_id: BrokerOrderId,
    },
    Filled {
        broker_order_id: BrokerOrderId,
        filled_quantity: i64,
        filled_notional: i64,
    },
    Rejected {
        broker_order_id: Option<BrokerOrderId>,
    },
    Cancelled {
        broker_order_id: Option<BrokerOrderId>,
    },
}

/// A monotonically increasing durable fencing term. The sink (this store) owns
/// the term; an owner never invents one. Terms start at 1 when an owner is
/// fenced; 0 is the genesis term before any owner has been granted a token.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FencingTerm(u64);

impl FencingTerm {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Capability presented with every fenced write. It binds one owner to one
/// term; the sink refuses a write whose term is older than the persisted term.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FencingToken {
    pub term: FencingTerm,
    pub owner: String,
}

/// Typed fencing failures. A stale term is never silently accepted or clamped
/// to the persisted term; the caller receives a distinct error.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FencingError {
    #[error("stale fencing term {presented}; persisted term is {persisted}")]
    StaleTerm { presented: u64, persisted: u64 },
    #[error("fencing term {presented} was never issued; persisted term is {persisted}")]
    UnknownTerm { presented: u64, persisted: u64 },
    #[error("fencing term {term} is held by `{holder}`, not `{presented}`")]
    OwnerMismatch {
        presented: String,
        holder: String,
        term: u64,
    },
    #[error("fencing owner must not be empty")]
    InvalidOwner,
    #[error("fencing term space is exhausted")]
    TermExhausted,
    #[error("storage error: {0}")]
    Storage(String),
    #[error("oms failure: {0}")]
    Oms(OmsError),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct OrderRecord {
    client_order_id: ClientOrderId,
    intent: OrderIntent,
    permit: PermitRef,
    state: OmsOrderState,
    broker_order_id: Option<BrokerOrderId>,
    filled_quantity: i64,
    filled_notional: i64,
    created_at_ns: i64,
    updated_at_ns: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct AttemptRecord {
    attempt_id: i64,
    client_order_id: ClientOrderId,
    command_type: String,
    state: OutboxState,
    lease_owner: Option<String>,
    lease_until_ns: Option<i64>,
    broker_order_id: Option<BrokerOrderId>,
    created_at_ns: i64,
    dispatched_at_ns: Option<i64>,
    reason: Option<String>,
}

/// Whether charge values are an estimate or broker-confirmed amounts.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeStatus {
    Estimated,
    BrokerConfirmed,
}

/// One durable, independently identifiable execution event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillEvent {
    pub schema_version: u32,
    pub execution_id: String,
    pub broker_event_id: String,
    pub client_order_id: ClientOrderId,
    pub oms_order_id: String,
    pub side: Side,
    pub quantity: i64,
    pub price_paise: i64,
    pub currency: String,
    pub broker_timestamp_ns: i64,
    pub received_at_ns: i64,
    pub source: String,
    pub account_scope: String,
    pub charges_paise: i64,
    pub charge_status: ChargeStatus,
    pub provenance: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct OmsState {
    seq: u64,
    next_attempt: i64,
    orders: std::collections::BTreeMap<String, OrderRecord>,
    outbox: std::collections::BTreeMap<i64, AttemptRecord>,
    events: std::collections::BTreeSet<String>,
    #[serde(default)]
    fills: std::collections::BTreeMap<String, FillEvent>,
    /// Permit ids that have already opened an order. Replayed from the journal
    /// on every recovery, so a permit cannot be spent twice after restart.
    #[serde(default)]
    consumed_permits: std::collections::BTreeSet<u64>,
    /// Durable fencing term, advanced with checked arithmetic and persisted in
    /// the journal before a new owner is granted a token.
    #[serde(default)]
    fencing_term: u64,
    /// Owner holding the persisted fencing term.
    #[serde(default)]
    fencing_owner: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CommitRecord {
    seq: u64,
    op: String,
    state: OmsState,
    /// Digest of the previous record; the first record uses 64 zero chars.
    prev_digest: String,
    /// Digest of this record's canonical bytes, so semantic tampering is
    /// detectable on replay rather than only truncation.
    digest: String,
}

fn genesis_digest() -> String {
    "00".repeat(32)
}

fn digest_of(seq: u64, op: &str, state: &OmsState, prev_digest: &str) -> String {
    let preimage = serde_json::json!({
        "seq": seq,
        "op": op,
        "state": state,
        "prev_digest": prev_digest,
    })
    .to_string();
    let raw = Sha256::digest(preimage.as_bytes());
    raw.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Semantic identity of a fill event, independent of journal order and of the
/// OMS chain digest. Every economic field is included: ids, side, quantity,
/// price, currency, charges, charge status, source, scope and provenance.
///
/// The two local-clock fields (`broker_timestamp_ns`, `received_at_ns`) are
/// excluded by design. They are observation times assigned by the receiver,
/// so a redelivery of the same broker event carries different local times;
/// including them would turn an idempotent duplicate into a false conflict.
#[must_use]
pub fn fill_content_digest(event: &FillEvent) -> String {
    let preimage = serde_json::json!({
        "schema_version": event.schema_version,
        "execution_id": &event.execution_id,
        "broker_event_id": &event.broker_event_id,
        "client_order_id": event.client_order_id.get(),
        "oms_order_id": &event.oms_order_id,
        "side": event.side,
        "quantity": event.quantity,
        "price_paise": event.price_paise,
        "currency": &event.currency,
        "source": &event.source,
        "account_scope": &event.account_scope,
        "charges_paise": event.charges_paise,
        "charge_status": event.charge_status,
        "provenance": &event.provenance,
    })
    .to_string();
    let raw = Sha256::digest(format!("nirnay.fill.event.v1\0{preimage}").as_bytes());
    raw.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The authoritative store. A single instance owns the journal handle and the
/// replayed state; every transition is one fsynced append.
pub struct OmsStore {
    path: PathBuf,
    journal: File,
    state: OmsState,
    last_digest: String,
    /// Set before an uncertain append and cleared only after a successful
    /// `sync_all`. A poisoned writer refuses every subsequent mutation.
    poisoned: bool,
    /// Deterministic fault injection for durability tests. Never set in
    /// production paths.
    commit_fault: Option<CommitFault>,
}

/// Injected commit faults for durability tests.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CommitFault {
    /// Write only part of the record, then fail.
    PartialWrite,
    /// Write the complete record but fail the durability step.
    SyncFailure,
}

impl OmsStore {
    /// Open or create a store. Replays and validates the whole journal; any
    /// gap or malformed record is refused.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, OmsError> {
        let path = path.as_ref().to_path_buf();
        let mut state = OmsState {
            next_attempt: 1,
            ..OmsState::default()
        };
        let mut last_digest = genesis_digest();
        if path.exists() {
            let file = File::open(&path).map_err(|error| OmsError::Storage(error.to_string()))?;
            for (index, line) in BufReader::new(file).lines().enumerate() {
                let line = line.map_err(|error| OmsError::Storage(error.to_string()))?;
                let record: CommitRecord = serde_json::from_str(&line).map_err(|error| {
                    OmsError::CorruptRecord(format!("journal line {}: {error}", index + 1))
                })?;
                let expected = (index as u64) + 1;
                if record.seq != expected {
                    return Err(OmsError::CorruptRecord(format!(
                        "journal seq {} at line {}; expected {expected}",
                        record.seq,
                        index + 1
                    )));
                }
                if record.prev_digest != last_digest {
                    return Err(OmsError::CorruptRecord(format!(
                        "journal link broken at line {}",
                        index + 1
                    )));
                }
                let computed =
                    digest_of(record.seq, &record.op, &record.state, &record.prev_digest);
                if computed != record.digest {
                    return Err(OmsError::CorruptRecord(format!(
                        "journal digest mismatch at line {}",
                        index + 1
                    )));
                }
                last_digest = record.digest;
                state = record.state;
            }
        }
        let journal = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| OmsError::Storage(error.to_string()))?;
        journal
            .try_lock_exclusive()
            .map_err(|error| match error.kind() {
                ErrorKind::WouldBlock => OmsError::WriterLocked,
                _ => OmsError::Storage(error.to_string()),
            })?;
        Ok(Self {
            path,
            journal,
            state,
            last_digest,
            poisoned: false,
            commit_fault: None,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Re-open and replay the journal into a fresh writer. The poisoned
    /// instance remains unusable; recovery is an explicit owner action.
    pub fn recover(path: impl AsRef<Path>) -> Result<Self, OmsError> {
        Self::open(path)
    }

    #[must_use]
    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    /// Inject a commit fault for deterministic durability tests.
    pub fn force_commit_fault(&mut self, fault: CommitFault) {
        self.commit_fault = Some(fault);
    }

    /// The persisted fencing term, replayed from the journal on every open.
    #[must_use]
    pub fn fencing_term(&self) -> FencingTerm {
        FencingTerm(self.state.fencing_term)
    }

    /// The owner holding the persisted fencing term.
    #[must_use]
    pub fn fencing_owner(&self) -> Option<&str> {
        self.state.fencing_owner.as_deref()
    }

    /// Grant an owner the next fencing term. The `fence` record is appended and
    /// synced before the token is returned, so the term is durable before the
    /// new owner is allowed to accept writes. Checked arithmetic prevents a
    /// wrap-around from reviving a stale term.
    pub fn fence(&mut self, owner: &str) -> Result<FencingToken, FencingError> {
        if owner.trim().is_empty() {
            return Err(FencingError::InvalidOwner);
        }
        let term = self
            .state
            .fencing_term
            .checked_add(1)
            .ok_or(FencingError::TermExhausted)?;
        let mut next = self.state.clone();
        next.fencing_term = term;
        next.fencing_owner = Some(owner.to_owned());
        self.append("fence", next).map_err(FencingError::Oms)?;
        Ok(FencingToken {
            term: FencingTerm(term),
            owner: owner.to_owned(),
        })
    }

    /// Reject a token whose term is older than the persisted term, was never
    /// issued, or belongs to a different owner. Equal current terms with the
    /// right owner pass through to the existing identity/idempotency rules.
    fn guard(&self, token: &FencingToken) -> Result<(), FencingError> {
        let persisted = self.state.fencing_term;
        let presented = token.term.get();
        if presented < persisted {
            return Err(FencingError::StaleTerm {
                presented,
                persisted,
            });
        }
        if presented > persisted {
            return Err(FencingError::UnknownTerm {
                presented,
                persisted,
            });
        }
        match self.state.fencing_owner.as_deref() {
            Some(holder) if holder == token.owner => Ok(()),
            Some(holder) => Err(FencingError::OwnerMismatch {
                presented: token.owner.clone(),
                holder: holder.to_owned(),
                term: presented,
            }),
            None => Err(FencingError::OwnerMismatch {
                presented: token.owner.clone(),
                holder: "none".to_owned(),
                term: presented,
            }),
        }
    }

    /// Fenced `create`. A repeated write at the current term is deduplicated by
    /// the existing client-order/permit identity rules; the term is never lowered.
    pub fn create_fenced(
        &mut self,
        token: &FencingToken,
        client_order_id: ClientOrderId,
        intent: &OrderIntent,
        permit: &PermitRef,
        now_ns: i64,
    ) -> Result<(), FencingError> {
        self.guard(token)?;
        self.create(client_order_id, intent, permit, now_ns)
            .map_err(FencingError::Oms)
    }

    /// Fenced `accept_risk_approved`.
    pub fn accept_risk_approved_fenced(
        &mut self,
        token: &FencingToken,
        client_order_id: ClientOrderId,
        now_ns: i64,
    ) -> Result<(), FencingError> {
        self.guard(token)?;
        self.accept_risk_approved(client_order_id, now_ns)
            .map_err(FencingError::Oms)
    }

    /// Fenced `claim`.
    pub fn claim_fenced(
        &mut self,
        token: &FencingToken,
        worker: &str,
        now_ns: i64,
        lease_ns: i64,
    ) -> Result<Option<DispatchClaim>, FencingError> {
        self.guard(token)?;
        self.claim(worker, now_ns, lease_ns)
            .map_err(FencingError::Oms)
    }

    /// Fenced `mark_dispatching`.
    pub fn mark_dispatching_fenced(
        &mut self,
        token: &FencingToken,
        attempt_id: i64,
        now_ns: i64,
    ) -> Result<(), FencingError> {
        self.guard(token)?;
        self.mark_dispatching(attempt_id, now_ns)
            .map_err(FencingError::Oms)
    }

    /// Fenced `acknowledge`.
    pub fn acknowledge_fenced(
        &mut self,
        token: &FencingToken,
        attempt_id: i64,
        broker_order_id: &BrokerOrderId,
        broker_event_id: &str,
        now_ns: i64,
    ) -> Result<bool, FencingError> {
        self.guard(token)?;
        self.acknowledge(attempt_id, broker_order_id, broker_event_id, now_ns)
            .map_err(FencingError::Oms)
    }

    /// Fenced `apply_typed_fill`.
    pub fn apply_typed_fill_fenced(
        &mut self,
        token: &FencingToken,
        event: FillEvent,
    ) -> Result<bool, FencingError> {
        self.guard(token)?;
        self.apply_typed_fill(event).map_err(FencingError::Oms)
    }

    /// Fenced `apply_fill`.
    #[allow(clippy::too_many_arguments)] // mirrors apply_fill plus the mandatory fencing token
    pub fn apply_fill_fenced(
        &mut self,
        token: &FencingToken,
        client_order_id: ClientOrderId,
        execution_id: &str,
        broker_event_id: &str,
        quantity: i64,
        price_paise: i64,
        now_ns: i64,
    ) -> Result<bool, FencingError> {
        self.guard(token)?;
        self.apply_fill(
            client_order_id,
            execution_id,
            broker_event_id,
            quantity,
            price_paise,
            now_ns,
        )
        .map_err(FencingError::Oms)
    }

    /// Fenced `expire_dispatched`.
    pub fn expire_dispatched_fenced(
        &mut self,
        token: &FencingToken,
        now_ns: i64,
        timeout_ns: i64,
    ) -> Result<usize, FencingError> {
        self.guard(token)?;
        self.expire_dispatched(now_ns, timeout_ns)
            .map_err(FencingError::Oms)
    }

    /// Fenced `request_cancel`.
    pub fn request_cancel_fenced(
        &mut self,
        token: &FencingToken,
        client_order_id: ClientOrderId,
        now_ns: i64,
    ) -> Result<i64, FencingError> {
        self.guard(token)?;
        self.request_cancel(client_order_id, now_ns)
            .map_err(FencingError::Oms)
    }

    /// Fenced `apply_cancel`.
    pub fn apply_cancel_fenced(
        &mut self,
        token: &FencingToken,
        client_order_id: ClientOrderId,
        broker_event_id: &str,
        now_ns: i64,
    ) -> Result<bool, FencingError> {
        self.guard(token)?;
        self.apply_cancel(client_order_id, broker_event_id, now_ns)
            .map_err(FencingError::Oms)
    }

    /// Fenced `begin_reconciliation`.
    pub fn begin_reconciliation_fenced(
        &mut self,
        token: &FencingToken,
        client_order_id: ClientOrderId,
        now_ns: i64,
    ) -> Result<(), FencingError> {
        self.guard(token)?;
        self.begin_reconciliation(client_order_id, now_ns)
            .map_err(FencingError::Oms)
    }

    /// Fenced `resolve_unknown`.
    pub fn resolve_unknown_fenced(
        &mut self,
        token: &FencingToken,
        client_order_id: ClientOrderId,
        resolution: Resolution,
        broker_event_id: &str,
        now_ns: i64,
    ) -> Result<(), FencingError> {
        self.guard(token)?;
        self.resolve_unknown(client_order_id, resolution, broker_event_id, now_ns)
            .map_err(FencingError::Oms)
    }

    fn append(&mut self, op: &str, mut next: OmsState) -> Result<(), OmsError> {
        if self.poisoned {
            return Err(OmsError::WriterPoisoned);
        }
        next.seq = self
            .state
            .seq
            .checked_add(1)
            .ok_or_else(|| OmsError::CorruptRecord("journal sequence exhausted".to_owned()))?;
        let digest = digest_of(next.seq, op, &next, &self.last_digest);
        let record = CommitRecord {
            seq: next.seq,
            op: op.to_owned(),
            state: next.clone(),
            prev_digest: self.last_digest.clone(),
            digest: digest.clone(),
        };
        let line = serde_json::to_string(&record).map_err(|e| OmsError::Storage(e.to_string()))?;
        // Any error after this point leaves the writer poisoned: the commit
        // may be partial or may not be durable, so this instance cannot safely
        // append again. Recovery is a fresh replay through `Self::recover`.
        self.poisoned = true;
        let outcome = self.write_commit(&line);
        outcome.map_err(|e| OmsError::Storage(e.to_string()))?;
        self.state = next;
        self.last_digest = digest;
        self.poisoned = false;
        Ok(())
    }

    fn write_commit(&mut self, line: &str) -> std::io::Result<()> {
        if self.commit_fault == Some(CommitFault::PartialWrite) {
            let split = line.len() / 2;
            self.journal.write_all(&line.as_bytes()[..split])?;
            return Err(std::io::Error::other("injected partial write"));
        }
        self.journal.write_all(line.as_bytes())?;
        self.journal.write_all(b"\n")?;
        if self.commit_fault == Some(CommitFault::SyncFailure) {
            return Err(std::io::Error::other("injected sync failure"));
        }
        self.journal.sync_all()?;
        Ok(())
    }

    fn apply<T, F>(&mut self, op: &str, function: F) -> Result<T, OmsError>
    where
        F: FnOnce(&OmsState) -> Result<(OmsState, T), OmsError>,
    {
        let (next, value) = function(&self.state)?;
        self.append(op, next)?;
        Ok(value)
    }

    /// Create an order in `Created` state. Idempotent on identical terms.
    pub fn create(
        &mut self,
        client_order_id: ClientOrderId,
        intent: &OrderIntent,
        permit: &PermitRef,
        now_ns: i64,
    ) -> Result<(), OmsError> {
        intent
            .validate()
            .map_err(|error| OmsError::InvalidIdentifier(error.to_string()))?;
        let key = client_order_id.to_string();
        let expected_digest = permit_digest(
            permit.permit_id,
            &permit.policy_version,
            permit.snapshot_revision,
            intent,
        );
        self.apply("create", |state| {
            if let Some(existing) = state.orders.get(&key) {
                if existing.intent == *intent && existing.permit == *permit {
                    return Ok((state.clone(), ()));
                }
                return Err(OmsError::ConflictingClientOrder(key.clone()));
            }
            // A permit is single-use across the durable journal: an identical
            // replay with the same client id was handled above; any other
            // client id must be denied even after an authority restart.
            if state.consumed_permits.contains(&permit.permit_id) {
                return Err(OmsError::PermitAlreadyConsumed {
                    permit_id: permit.permit_id,
                });
            }
            if expected_digest != permit.digest {
                return Err(OmsError::PermitDigestMismatch {
                    permit_id: permit.permit_id,
                });
            }
            let mut next = state.clone();
            next.consumed_permits.insert(permit.permit_id);
            next.orders.insert(
                key.clone(),
                OrderRecord {
                    client_order_id,
                    intent: intent.clone(),
                    permit: permit.clone(),
                    state: OmsOrderState::Created,
                    broker_order_id: None,
                    filled_quantity: 0,
                    filled_notional: 0,
                    created_at_ns: now_ns,
                    updated_at_ns: now_ns,
                },
            );
            Ok((next, ()))
        })
    }

    /// Accept a risk-approved order and enqueue its first submission attempt.
    pub fn accept_risk_approved(
        &mut self,
        client_order_id: ClientOrderId,
        now_ns: i64,
    ) -> Result<(), OmsError> {
        let key = client_order_id.to_string();
        self.apply("accept_risk_approved", |state| {
            let mut next = state.clone();
            let order = next
                .orders
                .get_mut(&key)
                .ok_or_else(|| OmsError::UnknownOrder(key.clone()))?;
            transition_order(order, OmsOrderState::RiskAccepted, now_ns)?;
            transition_order(order, OmsOrderState::PendingSubmission, now_ns)?;
            let attempt_id = next.next_attempt;
            next.next_attempt = next
                .next_attempt
                .checked_add(1)
                .ok_or_else(|| OmsError::Storage("attempt id exhausted".to_owned()))?;
            next.outbox.insert(
                attempt_id,
                AttemptRecord {
                    attempt_id,
                    client_order_id,
                    command_type: "SUBMIT".to_owned(),
                    state: OutboxState::Pending,
                    lease_owner: None,
                    lease_until_ns: None,
                    broker_order_id: None,
                    created_at_ns: now_ns,
                    dispatched_at_ns: None,
                    reason: None,
                },
            );
            record_event(
                &mut next,
                &format!("accept:{client_order_id}"),
                client_order_id,
                "RISK_ACCEPTED",
                now_ns,
            )?;
            Ok((next, ()))
        })
    }

    /// Claim one pending outbox command for a dispatcher.
    pub fn claim(
        &mut self,
        worker: &str,
        now_ns: i64,
        lease_ns: i64,
    ) -> Result<Option<DispatchClaim>, OmsError> {
        self.apply("claim", |state| {
            let target = state
                .outbox
                .values()
                .filter(|attempt| attempt.state == OutboxState::Pending)
                .min_by_key(|attempt| attempt.attempt_id)
                .map(|attempt| {
                    (
                        attempt.attempt_id,
                        attempt.client_order_id,
                        attempt.command_type.clone(),
                    )
                });
            let Some((attempt_id, client_order_id, command_type)) = target else {
                return Ok((state.clone(), None));
            };
            let mut next = state.clone();
            let attempt = next
                .outbox
                .get_mut(&attempt_id)
                .ok_or(OmsError::UnknownAttempt(attempt_id))?;
            transition_attempt(attempt, OutboxState::Claimed)?;
            attempt.lease_owner = Some(worker.to_owned());
            attempt.lease_until_ns = now_ns.checked_add(lease_ns);
            let intent = next
                .orders
                .get(&client_order_id.to_string())
                .ok_or_else(|| OmsError::UnknownOrder(client_order_id.to_string()))?
                .intent
                .clone();
            Ok((
                next,
                Some(DispatchClaim {
                    attempt_id,
                    client_order_id,
                    command_type,
                    intent,
                }),
            ))
        })
    }

    /// Mark a claimed attempt as dispatched. From this point a missing broker
    /// acknowledgement is UNKNOWN, never an automatic retry.
    pub fn mark_dispatching(&mut self, attempt_id: i64, now_ns: i64) -> Result<(), OmsError> {
        self.apply("mark_dispatching", |state| {
            let mut next = state.clone();
            let attempt = next
                .outbox
                .get_mut(&attempt_id)
                .ok_or(OmsError::UnknownAttempt(attempt_id))?;
            transition_attempt(attempt, OutboxState::Dispatching)?;
            attempt.dispatched_at_ns = Some(now_ns);
            Ok((next, ()))
        })
    }

    /// Bind the broker order id on acknowledgement and move the order to
    /// Working. Duplicate broker events are ignored.
    pub fn acknowledge(
        &mut self,
        attempt_id: i64,
        broker_order_id: &BrokerOrderId,
        broker_event_id: &str,
        now_ns: i64,
    ) -> Result<bool, OmsError> {
        if event_seen(&self.state, broker_event_id) {
            return Ok(false);
        }
        self.apply("acknowledge", |state| {
            let mut next = state.clone();
            let client_order_id = {
                let attempt = next
                    .outbox
                    .get(&attempt_id)
                    .ok_or(OmsError::UnknownAttempt(attempt_id))?;
                attempt.client_order_id
            };
            let attempt = next
                .outbox
                .get_mut(&attempt_id)
                .ok_or(OmsError::UnknownAttempt(attempt_id))?;
            transition_attempt(attempt, OutboxState::Acknowledged)?;
            attempt.broker_order_id = Some(broker_order_id.clone());
            let order = next
                .orders
                .get_mut(&client_order_id.to_string())
                .ok_or_else(|| OmsError::UnknownOrder(client_order_id.to_string()))?;
            transition_order(order, OmsOrderState::Working, now_ns)?;
            order.broker_order_id = Some(broker_order_id.clone());
            record_event(
                &mut next,
                broker_event_id,
                client_order_id,
                "BROKER_ACK",
                now_ns,
            )?;
            Ok((next, true))
        })
    }

    /// Expire dispatched attempts with no acknowledgement. The order becomes
    /// UNKNOWN and no new attempt is created.
    pub fn expire_dispatched(&mut self, now_ns: i64, timeout_ns: i64) -> Result<usize, OmsError> {
        self.apply("expire_dispatched", |state| {
            let targets: Vec<i64> = state
                .outbox
                .values()
                .filter(|attempt| {
                    attempt.state == OutboxState::Dispatching
                        && attempt
                            .dispatched_at_ns
                            .and_then(|at| at.checked_add(timeout_ns))
                            .is_some_and(|deadline| deadline <= now_ns)
                })
                .map(|attempt| attempt.attempt_id)
                .collect();
            let mut next = state.clone();
            for attempt_id in &targets {
                let attempt = next
                    .outbox
                    .get_mut(attempt_id)
                    .ok_or(OmsError::UnknownAttempt(*attempt_id))?;
                transition_attempt(attempt, OutboxState::Unknown)?;
                let client_order_id = attempt.client_order_id;
                let order = next
                    .orders
                    .get_mut(&client_order_id.to_string())
                    .ok_or_else(|| OmsError::UnknownOrder(client_order_id.to_string()))?;
                transition_order(order, OmsOrderState::Unknown, now_ns)?;
                record_event(
                    &mut next,
                    &format!("submission_unknown:{attempt_id}"),
                    client_order_id,
                    "SUBMISSION_OUTCOME_UNKNOWN",
                    now_ns,
                )?;
            }
            Ok((next, targets.len()))
        })
    }

    /// Reject a dispatched attempt.
    pub fn reject_submission(
        &mut self,
        attempt_id: i64,
        reason: &str,
        broker_event_id: &str,
        now_ns: i64,
    ) -> Result<bool, OmsError> {
        if event_seen(&self.state, broker_event_id) {
            return Ok(false);
        }
        self.apply("reject_submission", |state| {
            let mut next = state.clone();
            let client_order_id = {
                let attempt = next
                    .outbox
                    .get(&attempt_id)
                    .ok_or(OmsError::UnknownAttempt(attempt_id))?;
                attempt.client_order_id
            };
            let attempt = next
                .outbox
                .get_mut(&attempt_id)
                .ok_or(OmsError::UnknownAttempt(attempt_id))?;
            transition_attempt(attempt, OutboxState::Rejected)?;
            attempt.reason = Some(reason.to_owned());
            let order = next
                .orders
                .get_mut(&client_order_id.to_string())
                .ok_or_else(|| OmsError::UnknownOrder(client_order_id.to_string()))?;
            transition_order(order, OmsOrderState::Rejected, now_ns)?;
            record_event(
                &mut next,
                broker_event_id,
                client_order_id,
                "BROKER_REJECT",
                now_ns,
            )?;
            Ok((next, true))
        })
    }

    /// Apply a typed execution event. Duplicate execution ids and broker
    /// event ids are ignored; the full event is persisted in the journal.
    pub fn apply_typed_fill(&mut self, event: FillEvent) -> Result<bool, OmsError> {
        let execution_id = event.execution_id.clone();
        let broker_event_id = event.broker_event_id.clone();
        let client_order_id = event.client_order_id;
        let quantity = event.quantity;
        let price_paise = event.price_paise;
        let now_ns = event.received_at_ns;
        if event.schema_version != 1 {
            return Err(OmsError::InvalidIdentifier(
                "unsupported fill event schema version".to_owned(),
            ));
        }
        if execution_id.is_empty() || broker_event_id.is_empty() {
            return Err(OmsError::InvalidIdentifier(
                "fill requires stable execution and broker event ids".to_owned(),
            ));
        }
        let execution_key = format!("execution:{execution_id}");
        if event_seen(&self.state, &broker_event_id) || event_seen(&self.state, &execution_key) {
            // A repeated delivery is an idempotent no-op only when the content
            // is identical. Reusing an execution id for different economics is
            // a hard conflict and must surface as a reconciliation mismatch.
            if let Some(stored) = self.state.fills.get(&execution_id)
                && fill_content_digest(stored) != fill_content_digest(&event)
            {
                return Err(OmsError::FillContentConflict {
                    execution_id: execution_id.clone(),
                });
            }
            return Ok(false);
        }
        if quantity <= 0 || price_paise <= 0 {
            return Err(OmsError::InvalidIdentifier(
                "fill must have positive quantity and price".to_owned(),
            ));
        }
        self.apply("apply_typed_fill", |state| {
            let mut next = state.clone();
            let order = next
                .orders
                .get_mut(&client_order_id.to_string())
                .ok_or_else(|| OmsError::UnknownOrder(client_order_id.to_string()))?;
            if !matches!(
                order.state,
                OmsOrderState::Working | OmsOrderState::PartiallyFilled
            ) {
                return Err(OmsError::IllegalOrderTransition {
                    from: order.state,
                    to: OmsOrderState::PartiallyFilled,
                });
            }
            let new_filled = order
                .filled_quantity
                .checked_add(quantity)
                .ok_or(OmsError::OverFill)?;
            if new_filled > order.intent.quantity.get() {
                return Err(OmsError::OverFill);
            }
            let new_notional = order
                .filled_notional
                .checked_add(
                    quantity
                        .checked_mul(price_paise)
                        .ok_or(OmsError::OverFill)?,
                )
                .ok_or(OmsError::OverFill)?;
            let next_state = if new_filled == order.intent.quantity.get() {
                OmsOrderState::Filled
            } else {
                OmsOrderState::PartiallyFilled
            };
            transition_order(order, next_state, now_ns)?;
            order.filled_quantity = new_filled;
            order.filled_notional = new_notional;
            record_event(
                &mut next,
                &broker_event_id,
                client_order_id,
                "BROKER_FILL",
                now_ns,
            )?;
            record_event(
                &mut next,
                &execution_key,
                client_order_id,
                "EXECUTION",
                now_ns,
            )?;
            if next.fills.contains_key(&execution_id) {
                return Err(OmsError::DuplicateBrokerEvent(execution_id));
            }
            next.fills.insert(execution_id, event);
            Ok((next, true))
        })
    }

    /// Apply a broker fill from a minimal simulated report. The typed event
    /// records estimated charges and paper provenance; broker-confirmed charge
    /// values require `apply_typed_fill` with real evidence.
    pub fn apply_fill(
        &mut self,
        client_order_id: ClientOrderId,
        execution_id: &str,
        broker_event_id: &str,
        quantity: i64,
        price_paise: i64,
        now_ns: i64,
    ) -> Result<bool, OmsError> {
        let event = FillEvent {
            schema_version: 1,
            execution_id: execution_id.to_owned(),
            broker_event_id: broker_event_id.to_owned(),
            client_order_id,
            oms_order_id: client_order_id.to_string(),
            side: Side::Buy,
            quantity,
            price_paise,
            currency: "INR".to_owned(),
            broker_timestamp_ns: now_ns,
            received_at_ns: now_ns,
            source: "paper-simulated-broker".to_owned(),
            account_scope: "paper".to_owned(),
            charges_paise: 0,
            charge_status: ChargeStatus::Estimated,
            provenance: "paper-runner fill report".to_owned(),
        };
        self.apply_typed_fill(event)
    }

    /// Read-only view of a persisted execution event.
    pub fn fill(&self, execution_id: &str) -> Option<&FillEvent> {
        self.state.fills.get(execution_id)
    }

    /// All persisted execution events in execution-id order.
    pub fn fills(&self) -> Vec<FillEvent> {
        self.state.fills.values().cloned().collect()
    }

    /// Digest of the last committed OMS journal record, for replay equality
    /// checks against an independently reopened store.
    #[must_use]
    pub fn state_digest(&self) -> &str {
        &self.last_digest
    }

    /// Read-only view of every order, for independent OMS-vs-ledger
    /// reconciliation. Reconciliation recomputes from rows, never from a
    /// caller-supplied summary.
    pub fn orders(&self) -> Result<Vec<OrderView>, OmsError> {
        let mut views = Vec::new();
        for record in self.state.orders.values() {
            if let Some(view) = self.order(record.client_order_id)? {
                views.push(view);
            }
        }
        Ok(views)
    }

    pub fn request_cancel(
        &mut self,
        client_order_id: ClientOrderId,
        now_ns: i64,
    ) -> Result<i64, OmsError> {
        let key = client_order_id.to_string();
        self.apply("request_cancel", |state| {
            let mut next = state.clone();
            let order = next
                .orders
                .get_mut(&key)
                .ok_or_else(|| OmsError::UnknownOrder(key.clone()))?;
            transition_order(order, OmsOrderState::CancelPending, now_ns)?;
            let attempt_id = next.next_attempt;
            next.next_attempt = next
                .next_attempt
                .checked_add(1)
                .ok_or_else(|| OmsError::Storage("attempt id exhausted".to_owned()))?;
            next.outbox.insert(
                attempt_id,
                AttemptRecord {
                    attempt_id,
                    client_order_id,
                    command_type: "CANCEL".to_owned(),
                    state: OutboxState::Pending,
                    lease_owner: None,
                    lease_until_ns: None,
                    broker_order_id: None,
                    created_at_ns: now_ns,
                    dispatched_at_ns: None,
                    reason: None,
                },
            );
            Ok((next, attempt_id))
        })
    }

    /// Apply a broker cancellation acknowledgement.
    pub fn apply_cancel(
        &mut self,
        client_order_id: ClientOrderId,
        broker_event_id: &str,
        now_ns: i64,
    ) -> Result<bool, OmsError> {
        if event_seen(&self.state, broker_event_id) {
            return Ok(false);
        }
        self.apply("apply_cancel", |state| {
            let mut next = state.clone();
            let order = next
                .orders
                .get_mut(&client_order_id.to_string())
                .ok_or_else(|| OmsError::UnknownOrder(client_order_id.to_string()))?;
            transition_order(order, OmsOrderState::Cancelled, now_ns)?;
            record_event(
                &mut next,
                broker_event_id,
                client_order_id,
                "BROKER_CANCEL",
                now_ns,
            )?;
            Ok((next, true))
        })
    }

    /// Begin reconciliation of an UNKNOWN order.
    pub fn begin_reconciliation(
        &mut self,
        client_order_id: ClientOrderId,
        now_ns: i64,
    ) -> Result<(), OmsError> {
        self.apply("begin_reconciliation", |state| {
            let mut next = state.clone();
            let order = next
                .orders
                .get_mut(&client_order_id.to_string())
                .ok_or_else(|| OmsError::UnknownOrder(client_order_id.to_string()))?;
            transition_order(order, OmsOrderState::Reconciling, now_ns)?;
            Ok((next, ()))
        })
    }

    /// Resolve a reconciled UNKNOWN order from broker evidence. This is the
    /// only path out of `Reconciling`, and no new submission is created.
    pub fn resolve_unknown(
        &mut self,
        client_order_id: ClientOrderId,
        resolution: Resolution,
        broker_event_id: &str,
        now_ns: i64,
    ) -> Result<(), OmsError> {
        if broker_event_id.trim().is_empty() {
            return Err(OmsError::UnresolvedUnknown(
                "resolution requires broker evidence".to_owned(),
            ));
        }
        if event_seen(&self.state, broker_event_id) {
            return Ok(());
        }
        self.apply("resolve_unknown", |state| {
            let mut next = state.clone();
            let order = next
                .orders
                .get_mut(&client_order_id.to_string())
                .ok_or_else(|| OmsError::UnknownOrder(client_order_id.to_string()))?;
            transition_order(order, OmsOrderState::Resolved, now_ns)?;
            let (final_state, broker_id) = match resolution {
                Resolution::Working { broker_order_id } => {
                    (OmsOrderState::Working, Some(broker_order_id))
                }
                Resolution::Filled {
                    broker_order_id,
                    filled_quantity,
                    filled_notional,
                } => {
                    if filled_quantity <= 0 || filled_notional <= 0 {
                        return Err(OmsError::OverFill);
                    }
                    if filled_quantity != order.intent.quantity.get() {
                        return Err(OmsError::OverFill);
                    }
                    order.filled_quantity = filled_quantity;
                    order.filled_notional = filled_notional;
                    (OmsOrderState::Filled, Some(broker_order_id))
                }
                Resolution::Rejected { broker_order_id } => {
                    (OmsOrderState::Rejected, broker_order_id)
                }
                Resolution::Cancelled { broker_order_id } => {
                    (OmsOrderState::Cancelled, broker_order_id)
                }
            };
            transition_order(order, final_state, now_ns)?;
            if let Some(broker_id) = broker_id {
                order.broker_order_id = Some(broker_id);
            }
            for attempt in next.outbox.values_mut() {
                if attempt.client_order_id == client_order_id
                    && attempt.state == OutboxState::Unknown
                {
                    attempt.state = OutboxState::Acknowledged;
                }
            }
            record_event(
                &mut next,
                broker_event_id,
                client_order_id,
                "RECONCILIATION_RESOLVED",
                now_ns,
            )?;
            Ok((next, ()))
        })
    }

    /// Read one order.
    pub fn order(&self, client_order_id: ClientOrderId) -> Result<Option<OrderView>, OmsError> {
        Ok(self
            .state
            .orders
            .get(&client_order_id.to_string())
            .map(|record| OrderView {
                client_order_id,
                intent: record.intent.clone(),
                permit: record.permit.clone(),
                state: record.state,
                broker_order_id: record.broker_order_id.clone(),
                filled_quantity: record.filled_quantity,
                filled_notional: record.filled_notional,
                created_at_ns: record.created_at_ns,
                updated_at_ns: record.updated_at_ns,
            }))
    }

    /// All outbox rows, for operating inspection.
    pub fn outbox(&self) -> Result<Vec<OutboxView>, OmsError> {
        Ok(self
            .state
            .outbox
            .values()
            .map(|attempt| OutboxView {
                attempt_id: attempt.attempt_id,
                client_order_id: attempt.client_order_id,
                command_type: attempt.command_type.clone(),
                state: attempt.state,
                dispatched_at_ns: attempt.dispatched_at_ns,
                broker_order_id: attempt.broker_order_id.clone(),
            })
            .collect())
    }

    /// Orders currently in Unknown or Reconciling, for reconciliation control.
    pub fn unresolved(&self) -> Result<Vec<OrderView>, OmsError> {
        let mut out = Vec::new();
        for record in self.state.orders.values() {
            if matches!(
                record.state,
                OmsOrderState::Unknown | OmsOrderState::Reconciling
            ) && let Some(view) = self.order(record.client_order_id)?
            {
                out.push(view);
            }
        }
        Ok(out)
    }
}

fn transition_order(
    order: &mut OrderRecord,
    to: OmsOrderState,
    now_ns: i64,
) -> Result<(), OmsError> {
    if !order.state.permits(to) {
        return Err(OmsError::IllegalOrderTransition {
            from: order.state,
            to,
        });
    }
    order.state = to;
    order.updated_at_ns = now_ns;
    Ok(())
}

fn transition_attempt(attempt: &mut AttemptRecord, to: OutboxState) -> Result<(), OmsError> {
    if !attempt.state.permits(to) {
        return Err(OmsError::IllegalOutboxTransition {
            from: attempt.state,
            to,
        });
    }
    attempt.state = to;
    Ok(())
}

fn event_seen(state: &OmsState, key: &str) -> bool {
    state.events.contains(key)
}

fn record_event(
    state: &mut OmsState,
    key: &str,
    client_order_id: ClientOrderId,
    event_type: &str,
    now_ns: i64,
) -> Result<(), OmsError> {
    if event_seen(state, key) {
        return Err(OmsError::DuplicateBrokerEvent(key.to_owned()));
    }
    state.events.insert(key.to_owned());
    let _ = (client_order_id, event_type, now_ns);
    Ok(())
}

#[cfg(test)]
mod fencing_tests {
    //! M7 fencing-term tests. Authored before the implementation so the
    //! missing durable-term capability fails deterministically.

    use super::{
        BrokerOrderId, //
        ClientOrderId,
        FencingError,
        FencingTerm,
        FencingToken,
        FillEvent,
        OmsStore,
        PermitRef,
        Resolution,
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
            reason: "m7 fencing fixture".to_owned(),
            evidence: vec![JournalRef {
                sequence: 1,
                hash_hex: "ab".repeat(32),
            }],
            expires_at: LogicalNanos::new(100_000),
        })
    }

    fn permit(
        id: u64,
        order_id: u64,
        quantity: i64,
    ) -> Result<PermitRef, Box<dyn std::error::Error>> {
        let canonical = intent(order_id, quantity)?;
        Ok(PermitRef {
            permit_id: id,
            policy_version: "risk-v1".to_owned(),
            snapshot_revision: 3,
            digest: permit_digest(id, "risk-v1", 3, &canonical),
        })
    }

    fn fill(execution: &str, broker_event: &str) -> FillEvent {
        FillEvent {
            schema_version: 1,
            execution_id: execution.to_owned(),
            broker_event_id: broker_event.to_owned(),
            client_order_id: ClientOrderId::new(1).expect("fixture client id"),
            oms_order_id: "1".to_owned(),
            side: Side::Buy,
            quantity: 1,
            price_paise: 1_000,
            currency: "INR".to_owned(),
            broker_timestamp_ns: 1,
            received_at_ns: 1,
            source: "m7-fixture".to_owned(),
            account_scope: "paper".to_owned(),
            charges_paise: 0,
            charge_status: super::ChargeStatus::Estimated,
            provenance: "m7 fixture".to_owned(),
        }
    }

    macro_rules! assert_stale {
        ($result:expr) => {
            match $result {
                Err(FencingError::StaleTerm { .. }) => {}
                other => panic!("expected StaleTerm, got {other:?}"),
            }
        };
    }

    #[test]
    fn owner_termination_makes_previous_token_stale() -> TestResult {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("oms.jsonl");
        let old_token;
        {
            let mut store = OmsStore::open(&path)?;
            old_token = store.fence("owner-a").expect("fence owner-a");
            store.create_fenced(
                &old_token,
                ClientOrderId::new(1)?,
                &intent(1, 10)?,
                &permit(1, 1, 10)?,
                1,
            )?;
        }
        let mut store = OmsStore::open(&path)?;
        let current = store.fence("owner-b").expect("fence owner-b");
        store.create_fenced(
            &current,
            ClientOrderId::new(2)?,
            &intent(2, 10)?,
            &permit(2, 2, 10)?,
            2,
        )?;
        assert!(
            matches!(
                store.create_fenced(
                    &old_token,
                    ClientOrderId::new(3)?,
                    &intent(3, 10)?,
                    &permit(3, 3, 10)?,
                    3,
                ),
                Err(FencingError::StaleTerm { .. })
            ),
            "a terminated owner's token must be rejected"
        );
        Ok(())
    }

    #[test]
    fn paused_owner_resuming_after_reassignment_is_rejected() -> TestResult {
        let dir = tempfile::tempdir()?;
        let mut store = OmsStore::open(dir.path().join("oms.jsonl"))?;
        let paused = store.fence("paused-owner").expect("fence paused owner");
        let reassigned = store.fence("new-owner").expect("reassign owner");
        store.create_fenced(
            &reassigned,
            ClientOrderId::new(1)?,
            &intent(1, 10)?,
            &permit(1, 1, 10)?,
            2,
        )?;
        assert_stale!(store.create_fenced(
            &paused,
            ClientOrderId::new(2)?,
            &intent(2, 10)?,
            &permit(2, 2, 10)?,
            3,
        ));
        Ok(())
    }

    #[test]
    fn two_competing_owners_only_latest_term_writes() -> TestResult {
        let dir = tempfile::tempdir()?;
        let mut store = OmsStore::open(dir.path().join("oms.jsonl"))?;
        let first = store.fence("competitor-a").expect("fence competitor-a");
        let second = store.fence("competitor-b").expect("fence competitor-b");
        assert_eq!(first.term.get(), 1, "first competitor term");
        assert_eq!(second.term.get(), 2, "second competitor term");
        store.create_fenced(
            &second,
            ClientOrderId::new(1)?,
            &intent(1, 10)?,
            &permit(1, 1, 10)?,
            2,
        )?;
        assert_stale!(store.create_fenced(
            &first,
            ClientOrderId::new(2)?,
            &intent(2, 10)?,
            &permit(2, 2, 10)?,
            3,
        ));
        assert_eq!(
            store.orders()?.len(),
            1,
            "only the current owner's write landed"
        );
        Ok(())
    }

    #[test]
    fn stale_token_is_rejected_without_accepting_the_write() -> TestResult {
        let dir = tempfile::tempdir()?;
        let mut store = OmsStore::open(dir.path().join("oms.jsonl"))?;
        let stale = store.fence("owner-a").expect("fence owner-a");
        let _current = store.fence("owner-b").expect("fence owner-b");
        assert_stale!(store.create_fenced(
            &stale,
            ClientOrderId::new(1)?,
            &intent(1, 10)?,
            &permit(1, 1, 10)?,
            3,
        ));
        assert!(
            store.orders()?.is_empty(),
            "rejected write must not be persisted"
        );
        Ok(())
    }

    #[test]
    fn restart_preserves_persisted_fencing_term() -> TestResult {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("oms.jsonl");
        let old_token;
        {
            let mut store = OmsStore::open(&path)?;
            old_token = store.fence("owner-a").expect("fence owner-a");
            store.create_fenced(
                &old_token,
                ClientOrderId::new(1)?,
                &intent(1, 10)?,
                &permit(1, 1, 10)?,
                1,
            )?;
        }
        let mut store = OmsStore::open(&path)?;
        assert_eq!(store.fencing_term().get(), 1, "term survives restart");
        assert_eq!(
            store.fencing_owner(),
            Some("owner-a"),
            "owner survives restart"
        );
        let next = store.fence("owner-b").expect("fence after restart");
        assert_eq!(
            next.term.get(),
            2,
            "term advances monotonically after restart"
        );
        assert_stale!(store.create_fenced(
            &old_token,
            ClientOrderId::new(2)?,
            &intent(2, 10)?,
            &permit(2, 2, 10)?,
            2,
        ));
        Ok(())
    }

    #[test]
    fn simulated_partition_and_rejoin_rejects_stale_owner() -> TestResult {
        let dir = tempfile::tempdir()?;
        let mut store = OmsStore::open(dir.path().join("oms.jsonl"))?;
        // owner-a is fenced, then loses visibility of the journal (partition).
        let partitioned = store.fence("owner-a").expect("fence owner-a");
        // owner-b takes over while owner-a cannot observe the term advance.
        let rejoined = store.fence("owner-b").expect("fence owner-b");
        store.create_fenced(
            &rejoined,
            ClientOrderId::new(1)?,
            &intent(1, 10)?,
            &permit(1, 1, 10)?,
            2,
        )?;
        // owner-a rejoins and presents its pre-partition token.
        assert_stale!(store.create_fenced(
            &partitioned,
            ClientOrderId::new(2)?,
            &intent(2, 10)?,
            &permit(2, 2, 10)?,
            3,
        ));
        Ok(())
    }

    #[test]
    fn duplicate_replay_after_restart_is_idempotent_without_lowering_term() -> TestResult {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("oms.jsonl");
        let token;
        {
            let mut store = OmsStore::open(&path)?;
            token = store.fence("owner-a").expect("fence owner-a");
            store.create_fenced(
                &token,
                ClientOrderId::new(1)?,
                &intent(1, 10)?,
                &permit(1, 1, 10)?,
                1,
            )?;
        }
        let mut store = OmsStore::open(&path)?;
        assert_eq!(
            store.fencing_term().get(),
            1,
            "term persisted before restart"
        );
        // Same owner, same term: identity rules make the replay a no-op.
        store.create_fenced(
            &token,
            ClientOrderId::new(1)?,
            &intent(1, 10)?,
            &permit(1, 1, 10)?,
            2,
        )?;
        assert_eq!(
            store.fencing_term().get(),
            1,
            "duplicate must not lower the term"
        );
        let next = store.fence("owner-b").expect("reassign after restart");
        assert_stale!(store.create_fenced(
            &token,
            ClientOrderId::new(1)?,
            &intent(1, 10)?,
            &permit(1, 1, 10)?,
            3,
        ));
        assert_eq!(
            store.fencing_term().get(),
            next.term.get(),
            "term is not lowered by stale replay"
        );
        Ok(())
    }

    #[test]
    fn every_fenced_write_rejects_a_stale_term() -> TestResult {
        let dir = tempfile::tempdir()?;
        let mut store = OmsStore::open(dir.path().join("oms.jsonl"))?;
        let stale = store.fence("owner-a").expect("fence owner-a");
        let current = store.fence("owner-b").expect("fence owner-b");
        let client = ClientOrderId::new(1)?;
        let order_intent = intent(1, 10)?;
        let order_permit = permit(1, 1, 10)?;
        store.create_fenced(&current, client, &order_intent, &order_permit, 1)?;
        assert_stale!(store.create_fenced(&stale, client, &order_intent, &order_permit, 2));
        assert_stale!(store.accept_risk_approved_fenced(&stale, client, 2));
        assert_stale!(store.claim_fenced(&stale, "worker", 2, 1_000));
        assert_stale!(store.mark_dispatching_fenced(&stale, 1, 2));
        assert_stale!(store.acknowledge_fenced(
            &stale,
            1,
            &BrokerOrderId::new("BROKER-1")?,
            "evt-ack",
            2,
        ));
        assert_stale!(store.apply_typed_fill_fenced(&stale, fill("exec-1", "evt-fill-1")));
        assert_stale!(store.apply_fill_fenced(&stale, client, "exec-2", "evt-fill-2", 1, 1_000, 2));
        assert_stale!(store.request_cancel_fenced(&stale, client, 2));
        assert_stale!(store.apply_cancel_fenced(&stale, client, "evt-cancel", 2));
        assert_stale!(store.begin_reconciliation_fenced(&stale, client, 2));
        assert_stale!(store.resolve_unknown_fenced(
            &stale,
            client,
            Resolution::Rejected {
                broker_order_id: None,
            },
            "evt-resolve",
            2,
        ));
        assert_stale!(store.expire_dispatched_fenced(&stale, 2, 1));
        Ok(())
    }

    #[test]
    fn forged_term_and_wrong_owner_are_rejected() -> TestResult {
        let dir = tempfile::tempdir()?;
        let mut store = OmsStore::open(dir.path().join("oms.jsonl"))?;
        let current = store.fence("rightful-owner").expect("fence rightful owner");
        assert_eq!(current.term, FencingTerm::new(1), "first term is one");
        let forged = FencingToken {
            term: FencingTerm::new(9),
            owner: "rightful-owner".to_owned(),
        };
        assert!(
            matches!(
                store.create_fenced(
                    &forged,
                    ClientOrderId::new(1)?,
                    &intent(1, 10)?,
                    &permit(1, 1, 10)?,
                    1,
                ),
                Err(FencingError::UnknownTerm {
                    presented: 9,
                    persisted: 1
                })
            ),
            "a term that was never issued must be rejected"
        );
        let wrong_owner = FencingToken {
            term: current.term,
            owner: "impostor".to_owned(),
        };
        assert!(
            matches!(
                store.create_fenced(
                    &wrong_owner,
                    ClientOrderId::new(1)?,
                    &intent(1, 10)?,
                    &permit(1, 1, 10)?,
                    1,
                ),
                Err(FencingError::OwnerMismatch { .. })
            ),
            "a token from another owner must be rejected"
        );
        Ok(())
    }
}
