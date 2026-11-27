#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.5 authoritative OMS.
//!
//! One append-only durable commit journal owns the client order identity, the complete
//! typed order request, submission-attempt/outbox state, broker identities,
//! fills and the full order lifecycle. A timeout after dispatch yields
//! `Unknown`; it is never auto-retried and can only leave through explicit
//! reconciliation with broker evidence. No strategy, risk, broker SDK, live
//! routing or automatic deployment dependency exists here.

pub mod account_projection;
pub mod error;
pub mod lifecycle;
pub mod reconciliation;
pub mod store;

/// Freshness marker for the authoritative OMS store's local fencing terms.
pub use account_journal::FencingToken as JournalFencingToken;
pub use account_projection::{
    LEDGER_STREAM_ID, //
    LedgerCursor,
    LedgerState,
    ProjectionError,
    Projector,
    ReplayVerification,
};
pub use error::OmsError;
pub use lifecycle::{
    OmsOrderState, //
    OutboxState,
};
pub use reconciliation::{
    ReconciliationDifference, //
    ReconciliationInput,
    ReconciliationOutcome,
    reconcile,
};
pub use store::{
    BrokerOrderId, //
    ChargeStatus,
    ClientOrderId,
    CommitFault,
    DispatchClaim,
    FencingError,
    FencingTerm,
    FencingToken,
    FillEvent,
    OmsStore,
    OrderView,
    OutboxView,
    PermitRef,
    Resolution,
};
