//! The single authoritative OMS store: an append-only commit journal that
//! persists the complete order, attempt/outbox and event state after every
//! successful transition. One append + `sync_all` is the atomic commit point;
//! replay requires a contiguous sequence chain, so a truncated or corrupt tail
//! is refused rather than silently accepted.

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
