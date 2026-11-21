//! Broker-neutral request and event contract.
//!
//! The adapter must translate the *complete typed order request*. A permit
//! covering only broker, account, strategy, instrument and notional is not
//! enough: the exact venue, product, side, quantity, limit price, validity and
//! decision evidence are bound from the OMS dispatch claim.

use authoritative_oms::{
    DispatchClaim, //
    PermitRef,
};
use trading_core::OrderIntent;

use crate::error::AdapterError;

/// The complete typed order request handed to a broker adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrokerOrderRequest {
    client_order_id: authoritative_oms::ClientOrderId,
    submission_attempt_id: i64,
    /// Complete typed order terms from the authoritative OMS.
    intent: OrderIntent,
    /// Opaque permit evidence; not a substitute for the order terms.
    permit: PermitRef,
    /// Adapter target identity, fixed by adapter construction.
    broker: String,
}

impl BrokerOrderRequest {
    /// Build a request from an OMS dispatch claim. Fields are private, so a
    /// caller cannot construct or edit a weaker request.
    pub fn from_claim(
        claim: &DispatchClaim,
        permit: &PermitRef,
        broker: impl Into<String>,
    ) -> Result<Self, AdapterError> {
        let broker = broker.into();
        if broker.trim().is_empty() {
            return Err(AdapterError::IncompleteRequest);
        }
        if claim.command_type != "SUBMIT" {
            return Err(AdapterError::IncompleteRequest);
        }
        Ok(Self {
            client_order_id: claim.client_order_id,
            submission_attempt_id: claim.attempt_id,
            intent: claim.intent.clone(),
            permit: permit.clone(),
            broker,
        })
    }

    #[must_use]
    pub const fn client_order_id(&self) -> authoritative_oms::ClientOrderId {
        self.client_order_id
    }

    #[must_use]
    pub const fn submission_attempt_id(&self) -> i64 {
        self.submission_attempt_id
    }

    #[must_use]
    pub const fn intent(&self) -> &OrderIntent {
        &self.intent
    }

    #[must_use]
    pub const fn permit(&self) -> &PermitRef {
        &self.permit
    }

    #[must_use]
    pub fn broker(&self) -> &str {
        &self.broker
    }

    /// Verify the request still equals the OMS claim and evidence it came
    /// from, including the permit and the target adapter identity.
    #[must_use]
    pub fn matches(&self, claim: &DispatchClaim, permit: &PermitRef, broker: &str) -> bool {
        claim.command_type == "SUBMIT"
            && self.client_order_id == claim.client_order_id
            && self.submission_attempt_id == claim.attempt_id
            && self.intent == claim.intent
            && self.permit == *permit
            && self.broker == broker
    }

    /// Verify the request binds the exact claim it came from.
    #[must_use]
    pub fn matches_claim(&self, claim: &DispatchClaim) -> bool {
        claim.command_type == "SUBMIT"
            && self.client_order_id == claim.client_order_id
            && self.submission_attempt_id == claim.attempt_id
            && self.intent == claim.intent
    }
}

/// Exact cancellation request; it identifies both the broker order and the
/// client order so no other order can be cancelled by a stale message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CancelRequest {
    pub client_order_id: authoritative_oms::ClientOrderId,
    pub broker_order_id: String,
    pub broker_event_id: String,
}

/// acknowledgement bound to one submission attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmissionAck {
    pub submission_attempt_id: i64,
    pub broker_order_id: String,
    pub broker_event_id: String,
    pub accepted_at_ns: i64,
}

/// Outcome of a submission. `TimedOut` is ambiguous by design: it must be
/// mapped to UNKNOWN at the OMS, never auto-retried.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmissionOutcome {
    Acknowledged(SubmissionAck),
    Rejected {
        submission_attempt_id: i64,
        reason: String,
        broker_event_id: String,
    },
    TimedOut {
        submission_attempt_id: i64,
    },
}

/// Broker-neutral order status for reconciliation queries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrokerOrderStatus {
    Working,
    PartiallyFilled { filled_quantity: i64 },
    Filled { filled_quantity: i64 },
    Cancelled,
    Rejected { reason: String },
    Unknown,
}

/// A broker event to be applied to the authoritative OMS. Each event carries
/// a unique broker event id and execution id where relevant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrokerEvent {
    Acknowledged {
        broker_order_id: String,
        broker_event_id: String,
        sequence: u64,
    },
    Rejected {
        broker_order_id: String,
        reason: String,
        broker_event_id: String,
        sequence: u64,
    },
    Filled {
        broker_order_id: String,
        execution_id: String,
        quantity: i64,
        price_paise: i64,
        broker_event_id: String,
        sequence: u64,
    },
    Cancelled {
        broker_order_id: String,
        broker_event_id: String,
        sequence: u64,
    },
}

impl BrokerEvent {
    #[must_use]
    pub fn broker_event_id(&self) -> &str {
        match self {
            Self::Acknowledged {
                broker_event_id, ..
            }
            | Self::Rejected {
                broker_event_id, ..
            }
            | Self::Filled {
                broker_event_id, ..
            }
            | Self::Cancelled {
                broker_event_id, ..
            } => broker_event_id,
        }
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        match self {
            Self::Acknowledged { sequence, .. }
            | Self::Rejected { sequence, .. }
            | Self::Filled { sequence, .. }
            | Self::Cancelled { sequence, .. } => *sequence,
        }
    }
}

/// The broker-neutral interface. Strategies and the risk engine never see it.
pub trait BrokerAdapter {
    #[must_use]
    fn adapter_id(&self) -> &str;

    fn submit(&mut self, request: &BrokerOrderRequest) -> Result<SubmissionOutcome, AdapterError>;

    fn cancel(&mut self, request: &CancelRequest) -> Result<SubmissionOutcome, AdapterError>;

    fn poll_events(&mut self) -> Result<Vec<BrokerEvent>, AdapterError>;

    fn query_order(
        &mut self,
        broker_order_id: &str,
    ) -> Result<Option<BrokerOrderStatus>, AdapterError>;
}
