//! Deterministic broker simulator. No network, SDK or real endpoint.
//!
//! The simulator is scripted so contract tests can reproduce acknowledgement,
//! rejection, timeout-after-submission, partial fills, duplicate events and
//! out-of-order updates exactly.

use std::collections::{
    BTreeMap, //
    VecDeque,
};

use crate::{
    contract::{
        BrokerAdapter, //
        BrokerEvent,
        BrokerOrderRequest,
        BrokerOrderStatus,
        CancelRequest,
        SubmissionAck,
        SubmissionOutcome,
    },
    error::AdapterError,
};

/// How a submit call behaves. Every mode is deterministic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmitBehavior {
    ImmediateAck,
    /// The broker may have accepted the order but returns no acknowledgement.
    Timeout,
    Reject {
        reason: String,
    },
    AckThenFills {
        fills: Vec<(i64, i64)>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CancelBehavior {
    Acknowledge,
    Timeout,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimulatorConfig {
    pub adapter_id: String,
    pub behavior: SubmitBehavior,
    pub cancel_behavior: CancelBehavior,
    /// Emit each event twice with the same broker event id.
    pub duplicate_events: bool,
    /// Emit the queued events in reverse arrival order.
    pub reverse_events: bool,
}

impl SimulatorConfig {
    pub fn new(adapter_id: impl Into<String>, behavior: SubmitBehavior) -> Self {
        Self {
            adapter_id: adapter_id.into(),
            behavior,
            cancel_behavior: CancelBehavior::Acknowledge,
            duplicate_events: false,
            reverse_events: false,
        }
    }
}

/// A deterministic in-process broker used only for adapter contract tests.
pub struct SimulatedBroker {
    config: SimulatorConfig,
    authenticated: bool,
    sequence: u64,
    events: VecDeque<BrokerEvent>,
    orders: BTreeMap<String, (BrokerOrderStatus, BrokerOrderRequest)>,
}

impl SimulatedBroker {
    pub fn new(config: SimulatorConfig) -> Result<Self, AdapterError> {
        if config.adapter_id.trim().is_empty() {
            return Err(AdapterError::InvalidScenario(
                "adapter id must not be empty".to_owned(),
            ));
        }
        if let SubmitBehavior::AckThenFills { fills } = &config.behavior
            && (fills.is_empty()
                || fills
                    .iter()
                    .any(|(quantity, price)| *quantity <= 0 || *price <= 0))
        {
            return Err(AdapterError::InvalidScenario(
                "fills must be non-empty and positive".to_owned(),
            ));
        }
        Ok(Self {
            config,
            authenticated: false,
            sequence: 0,
            events: VecDeque::new(),
            orders: BTreeMap::new(),
        })
    }

    pub fn authenticate(&mut self) {
        self.authenticated = true;
    }

    pub fn is_authenticated(&self) -> bool {
        self.authenticated
    }

    fn next_sequence(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }

    fn event_id(&mut self) -> String {
        let sequence = self.next_sequence();
        let adapter_id = self.config.adapter_id.clone();
        format!("{adapter_id}-evt-{sequence}")
    }

    fn broker_order_id(attempt_id: i64) -> String {
        format!("SIM-{attempt_id}")
    }

    fn enqueue(&mut self, event: BrokerEvent) {
        if self.config.duplicate_events {
            self.push_event(event.clone());
        }
        self.push_event(event);
    }

    fn push_event(&mut self, event: BrokerEvent) {
        if self.config.reverse_events {
            self.events.push_front(event);
        } else {
            self.events.push_back(event);
        }
    }

    /// Broker delivery order before adapter normalization. Used by contract
    /// tests to prove the simulator delivers out-of-order events.
    pub fn drain_raw_events(&mut self) -> Vec<BrokerEvent> {
        self.events.drain(..).collect()
    }
}

impl BrokerAdapter for SimulatedBroker {
    fn adapter_id(&self) -> &str {
        &self.config.adapter_id
    }

    fn submit(&mut self, request: &BrokerOrderRequest) -> Result<SubmissionOutcome, AdapterError> {
        if !self.authenticated {
            return Err(AdapterError::NotAuthenticated);
        }
        if request.broker() != self.adapter_id() {
            return Err(AdapterError::IncompleteRequest);
        }
        let attempt_id = request.submission_attempt_id();
        let broker_order_id = Self::broker_order_id(attempt_id);
        match self.config.behavior.clone() {
            SubmitBehavior::ImmediateAck => {
                self.orders.insert(
                    broker_order_id.clone(),
                    (BrokerOrderStatus::Working, request.clone()),
                );
                let event_id = self.event_id();
                self.enqueue(BrokerEvent::Acknowledged {
                    broker_order_id: broker_order_id.clone(),
                    broker_event_id: event_id.clone(),
                    sequence: self.sequence,
                });
                Ok(SubmissionOutcome::Acknowledged(SubmissionAck {
                    submission_attempt_id: attempt_id,
                    broker_order_id,
                    broker_event_id: event_id,
                    accepted_at_ns: i64::try_from(self.sequence).unwrap_or(i64::MAX),
                }))
            }
            SubmitBehavior::Timeout => {
                // The broker may hold the order; no acknowledgement is
                // returned. Reconciliation can discover the true state.
                self.orders.insert(
                    broker_order_id,
                    (BrokerOrderStatus::Unknown, request.clone()),
                );
                Ok(SubmissionOutcome::TimedOut {
                    submission_attempt_id: attempt_id,
                })
            }
            SubmitBehavior::Reject { reason } => {
                let event_id = self.event_id();
                self.enqueue(BrokerEvent::Rejected {
                    broker_order_id: broker_order_id.clone(),
                    reason: reason.clone(),
                    broker_event_id: event_id.clone(),
                    sequence: self.sequence,
                });
                Ok(SubmissionOutcome::Rejected {
                    submission_attempt_id: attempt_id,
                    reason,
                    broker_event_id: event_id,
                })
            }
            SubmitBehavior::AckThenFills { fills } => {
                let total: i64 = fills.iter().map(|(quantity, _)| *quantity).sum();
                let status = if total >= request.intent().quantity.get() {
                    BrokerOrderStatus::Filled {
                        filled_quantity: total,
                    }
                } else {
                    BrokerOrderStatus::PartiallyFilled {
                        filled_quantity: total,
                    }
                };
                self.orders
                    .insert(broker_order_id.clone(), (status, request.clone()));
                let ack_id = self.event_id();
                self.enqueue(BrokerEvent::Acknowledged {
                    broker_order_id: broker_order_id.clone(),
                    broker_event_id: ack_id.clone(),
                    sequence: self.sequence,
                });
                for (quantity, price) in fills {
                    let event_id = self.event_id();
                    let execution_id = format!("exec-{}", self.sequence);
                    self.enqueue(BrokerEvent::Filled {
                        broker_order_id: broker_order_id.clone(),
                        execution_id,
                        quantity,
                        price_paise: price,
                        broker_event_id: event_id.clone(),
                        sequence: self.sequence,
                    });
                }
                Ok(SubmissionOutcome::Acknowledged(SubmissionAck {
                    submission_attempt_id: attempt_id,
                    broker_order_id,
                    broker_event_id: ack_id,
                    accepted_at_ns: i64::try_from(self.sequence).unwrap_or(i64::MAX),
                }))
            }
        }
    }

    fn cancel(&mut self, request: &CancelRequest) -> Result<SubmissionOutcome, AdapterError> {
        if !self.authenticated {
            return Err(AdapterError::NotAuthenticated);
        }
        let bound_client = self
            .orders
            .get(&request.broker_order_id)
            .map(|(_, original)| original.client_order_id());
        let bound_client = bound_client
            .ok_or_else(|| AdapterError::UnknownBrokerOrder(request.broker_order_id.clone()))?;
        if bound_client != request.client_order_id {
            return Err(AdapterError::IncompleteRequest);
        }
        if let Some(entry) = self.orders.get_mut(&request.broker_order_id) {
            entry.0 = BrokerOrderStatus::Cancelled;
        }
        match self.config.cancel_behavior {
            CancelBehavior::Acknowledge => {
                let sequence = self.next_sequence();
                self.enqueue(BrokerEvent::Cancelled {
                    broker_order_id: request.broker_order_id.clone(),
                    broker_event_id: request.broker_event_id.clone(),
                    sequence,
                });
                Ok(SubmissionOutcome::Acknowledged(SubmissionAck {
                    submission_attempt_id: 0,
                    broker_order_id: request.broker_order_id.clone(),
                    broker_event_id: request.broker_event_id.clone(),
                    accepted_at_ns: i64::try_from(self.sequence).unwrap_or(i64::MAX),
                }))
            }
            CancelBehavior::Timeout => Ok(SubmissionOutcome::TimedOut {
                submission_attempt_id: 0,
            }),
        }
    }

    fn poll_events(&mut self) -> Result<Vec<BrokerEvent>, AdapterError> {
        if !self.authenticated {
            return Err(AdapterError::NotAuthenticated);
        }
        let mut events: Vec<BrokerEvent> = self.events.drain(..).collect();
        // Broker events may arrive out of order. The adapter restores their
        // declared sequence so the authoritative OMS never sees a fill before
        // the acknowledgement it depends on.
        events.sort_by_key(BrokerEvent::sequence);
        Ok(events)
    }

    fn query_order(
        &mut self,
        broker_order_id: &str,
    ) -> Result<Option<BrokerOrderStatus>, AdapterError> {
        if !self.authenticated {
            return Err(AdapterError::NotAuthenticated);
        }
        Ok(self
            .orders
            .get(broker_order_id)
            .map(|(status, _)| status.clone()))
    }
}
