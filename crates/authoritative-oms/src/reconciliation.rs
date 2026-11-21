//! Independent OMS-vs-ledger reconciliation.
//!
//! The comparator recomputes positions and order aggregates from both durable
//! sources instead of trusting either summary. It classifies every delivery
//! anomaly in the M3.5.3 matrix and emits the typed [`ReconciliationReport`]
//! consumed by the risk authority. A missing view or a digest failure is
//! `Unavailable`, which blocks new risk exactly like `Mismatch`.

use std::collections::{
    BTreeMap, //
    BTreeSet,
};

use sha2::{
    Digest, //
    Sha256,
};
use thiserror::Error;
use trading_core::{
    InstrumentId, //
    OrderId,
    ReconciliationReport,
    ReconciliationStatus,
};

use crate::{
    account_projection::{ProjectionError, Projector},
    store::{FillEvent, OmsStore},
};

/// Typed anomaly classifications. Strings are only rendered at the report
/// boundary; control flow uses these variants.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ReconciliationDifference {
    #[error("duplicate delivery of execution `{execution_id}` is idempotent")]
    DuplicateDelivery {
        execution_id: String,
        digest: String,
    },
    #[error("execution `{execution_id}` was delivered with different content")]
    ContentConflict { execution_id: String },
    #[error("order `{order_id}` fill sum {observed} differs from OMS aggregate {expected}")]
    PartialFillSum {
        order_id: u64,
        expected: i64,
        observed: i64,
    },
    #[error("late fill `{execution_id}` arrived after order `{order_id}` reached a terminal state")]
    LateFillAfterCancel { execution_id: String, order_id: u64 },
    #[error("execution `{execution_id}` was delivered out of broker-time order")]
    OutOfOrderDelivery { execution_id: String },
    #[error("ledger replay digest does not match the committed state")]
    ReplayDigestMismatch,
    #[error("OMS journal digest mismatch or poisoned writer")]
    OmsUnavailable,
    #[error(
        "position differs for scope `{scope}`, instrument `{instrument}`: ledger {expected}, OMS {observed}"
    )]
    PositionMismatch {
        scope: String,
        instrument: String,
        expected: i64,
        observed: i64,
    },
    #[error("no instrument mapping for execution `{execution_id}`")]
    MissingInstrument { execution_id: String },
    #[error("unknown execution `{execution_id}` in the delivery log")]
    UnknownDelivery { execution_id: String },
}

impl ReconciliationDifference {
    /// Duplicate and out-of-order deliveries are recorded as evidence but do
    /// not block: both layers dedup and the projection is confluent.
    #[must_use]
    pub const fn is_blocking(&self) -> bool {
        !matches!(
            self,
            Self::DuplicateDelivery { .. } | Self::OutOfOrderDelivery { .. }
        )
    }
}

/// Raw delivery log plus the independent instrument mapping. The log may
/// contain duplicate and out-of-order events; the OMS store holds only the
/// accepted prefix.
#[derive(Clone, Debug, Default)]
pub struct ReconciliationInput {
    pub deliveries: Vec<FillEvent>,
    pub instruments: BTreeMap<String, InstrumentId>,
}

/// Full reconciliation outcome: the domain report plus the typed anomalies
/// and both digest chains for evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconciliationOutcome {
    pub report: ReconciliationReport,
    pub differences: Vec<ReconciliationDifference>,
    pub oms_state_digest: String,
    pub ledger_state_digest: String,
}

/// Reconcile the authoritative OMS with the projected ledger.
pub fn reconcile(
    store: &OmsStore,
    projector: &Projector,
    input: &ReconciliationInput,
) -> Result<ReconciliationOutcome, ProjectionError> {
    let mut differences = Vec::new();
    let mut hard_conflict = false;
    let mut unavailable = false;

    let fills = store.fills();
    let orders = store.orders()?;

    // 1. Duplicate delivery and same-id/different-content conflicts are
    //    detected from the raw delivery log, before OMS dedup hides them.
    let mut by_execution: BTreeMap<String, Vec<&FillEvent>> = BTreeMap::new();
    for delivery in &input.deliveries {
        by_execution
            .entry(delivery.execution_id.clone())
            .or_default()
            .push(delivery);
    }
    for (execution_id, delivered) in by_execution {
        let digests: BTreeSet<String> = delivered
            .iter()
            .map(|event| crate::store::fill_content_digest(event))
            .collect();
        if digests.len() > 1 {
            differences.push(ReconciliationDifference::ContentConflict {
                execution_id: execution_id.clone(),
            });
            hard_conflict = true;
        } else if delivered.len() > 1 {
            differences.push(ReconciliationDifference::DuplicateDelivery {
                execution_id: execution_id.clone(),
                digest: digests.into_iter().next().unwrap_or_default(),
            });
        }
    }

    // 2. Out-of-order delivery is informational: projection is confluent and
    //    final state must be identical for any delivery order.
    let mut last_broker_time = i64::MIN;
    for delivery in &input.deliveries {
        if delivery.broker_timestamp_ns < last_broker_time {
            differences.push(ReconciliationDifference::OutOfOrderDelivery {
                execution_id: delivery.execution_id.clone(),
            });
        }
        last_broker_time = last_broker_time.max(delivery.broker_timestamp_ns);
    }

    // 3. Partial-fill aggregates and late fills after a terminal cancel.
    for order in &orders {
        let observed: i64 = fills
            .iter()
            .filter(|fill| fill.client_order_id == order.client_order_id)
            .map(|fill| fill.quantity)
            .sum();
        if observed != order.filled_quantity {
            differences.push(ReconciliationDifference::PartialFillSum {
                order_id: order.client_order_id.get(),
                expected: order.filled_quantity,
                observed,
            });
            hard_conflict = true;
        }
        if order.state.terminal() {
            for delivery in &input.deliveries {
                if delivery.client_order_id == order.client_order_id
                    && delivery.broker_timestamp_ns > order.updated_at_ns
                {
                    differences.push(ReconciliationDifference::LateFillAfterCancel {
                        execution_id: delivery.execution_id.clone(),
                        order_id: order.client_order_id.get(),
                    });
                    hard_conflict = true;
                }
            }
        }
    }

    // 4. OMS-derived positions, recomputed independently from fills.
    let mut observed_by_scope: BTreeMap<String, BTreeMap<InstrumentId, i64>> = BTreeMap::new();
    for fill in &fills {
        let Some(instrument) = input.instruments.get(&fill.execution_id) else {
            differences.push(ReconciliationDifference::MissingInstrument {
                execution_id: fill.execution_id.clone(),
            });
            unavailable = true;
            continue;
        };
        let signed = if fill.side == trading_core::Side::Buy {
            fill.quantity
        } else {
            -fill.quantity
        };
        let entry = observed_by_scope
            .entry(fill.account_scope.clone())
            .or_default()
            .entry(instrument.clone())
            .or_insert(0);
        *entry = entry.saturating_add(signed);
    }
    for delivery in &input.deliveries {
        if !fills
            .iter()
            .any(|fill| fill.execution_id == delivery.execution_id)
            && !input.instruments.contains_key(&delivery.execution_id)
        {
            differences.push(ReconciliationDifference::UnknownDelivery {
                execution_id: delivery.execution_id.clone(),
            });
            // UnknownDelivery is a blocking anomaly; it must move the report
            // to Mismatch like any other `is_blocking()` difference.
            hard_conflict = true;
        }
    }

    let ledger_state = projector.state();
    for (scope, positions) in &ledger_state.positions {
        for (instrument, expected) in positions {
            let observed = observed_by_scope
                .get(scope)
                .and_then(|by_instrument| by_instrument.get(instrument))
                .copied()
                .unwrap_or(0);
            if observed != *expected {
                differences.push(ReconciliationDifference::PositionMismatch {
                    scope: scope.clone(),
                    instrument: instrument.as_str().to_owned(),
                    expected: *expected,
                    observed,
                });
                hard_conflict = true;
            }
        }
    }

    // 5. Replay and digest verification: a failed chain is unavailable, not a
    //    clean match, and must block new risk.
    let replay = projector.verify_replay()?;
    if !replay.matched {
        differences.push(ReconciliationDifference::ReplayDigestMismatch);
        unavailable = true;
    }
    if store.is_poisoned() || store.state_digest().len() != 64 {
        differences.push(ReconciliationDifference::OmsUnavailable);
        unavailable = true;
    }

    let expected_positions = flatten(ledger_state.positions.clone());
    let observed_positions = if unavailable {
        None
    } else {
        Some(flatten(observed_by_scope))
    };
    let active_orders = orders
        .iter()
        .filter(|order| !order.state.terminal())
        .map(|order| OrderId::new(order.client_order_id.get()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let status = if unavailable {
        ReconciliationStatus::Unavailable
    } else if hard_conflict {
        ReconciliationStatus::Mismatch
    } else {
        ReconciliationStatus::Matched
    };
    let report_id_material = format!(
        "{}|{}|{}",
        store.state_digest(),
        replay.cursor.ledger_state_digest,
        status_name(status)
    );
    let report_id = format!(
        "recon-{}",
        Sha256::digest(report_id_material.as_bytes())
            .iter()
            .take(16)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let report = ReconciliationReport {
        reconciliation_id: report_id,
        status,
        expected_positions,
        observed_positions,
        expected_active_orders: active_orders.clone(),
        observed_active_orders: if unavailable {
            None
        } else {
            Some(active_orders)
        },
        differences: differences.iter().map(ToString::to_string).collect(),
    };
    Ok(ReconciliationOutcome {
        report,
        differences,
        oms_state_digest: store.state_digest().to_owned(),
        ledger_state_digest: replay.cursor.ledger_state_digest,
    })
}

fn status_name(status: ReconciliationStatus) -> &'static str {
    match status {
        ReconciliationStatus::Matched => "matched",
        ReconciliationStatus::Mismatch => "mismatch",
        ReconciliationStatus::Unavailable => "unavailable",
    }
}

fn flatten(
    positions: BTreeMap<String, BTreeMap<InstrumentId, i64>>,
) -> BTreeMap<InstrumentId, i64> {
    let mut flat: BTreeMap<InstrumentId, i64> = BTreeMap::new();
    for by_instrument in positions.into_values() {
        for (instrument, quantity) in by_instrument {
            let entry = flat.entry(instrument).or_insert(0);
            *entry = (*entry).saturating_add(quantity);
        }
    }
    flat
}
