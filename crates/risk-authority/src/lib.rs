#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.4 stateful risk authority with atomic exposure reservation.
//!
//! Sits in the engines layer and depends only on lower-layer crates. It owns
//! account-global state, validates typed versioned observations from six
//! owners and derives health inside one mutex. A caller supplies neither a
//! `Healthy` flag nor an exposure figure. It produces reservations (permits)
//! only; it has no OMS, broker, order submission or live-routing capability.

pub mod authority;
pub mod control;
pub mod error;
pub mod observation;

pub use authority::{
    ACCOUNT_JOURNAL_ID,    //
    AccountRecoveryRecord, //
    AuthorityOwnership,
    AuthoritySnapshot,
    ChainedRecord,
    DecisionOutcome,
    DecisionStore,
    DecisionStoreError,
    FileDecisionStore,
    GENESIS_DIGEST,
    InMemoryDecisionStore,
    JournalIdentity,
    OMS_JOURNAL_ID,
    OmsPermitRecord,
    OmsPermitState,
    RECOVERY_SNAPSHOT_VERSION,
    RecoveryCheckpoint,
    RecoveryConfig,
    RecoveryError,
    Reservation,
    RiskAuthority,
    RiskDecisionRecord,
    VerifiedAccountSnapshot,
    VerifiedOmsSnapshot,
    genesis_digest,
    verify_recovery_chain,
};
pub use control::{
    ControlError, //
    ControlStore,
    FileControlStore,
    KillSwitchState,
    NoopControlStore,
    ReconciliationState,
};
pub use error::{
    AuthorityError, //
    OpsThresholdKind,
};
pub use observation::{
    AccountJournalObservation,
    FeedObservation,
    MarketObservation,
    OBSERVATION_SCHEMA_VERSION, //
    Observation,
    ObservationPolicy,
    ObservationSource,
    Observations,
    OmsObservation,
    OperatorObservation,
    OpsObservation,
    OwnerToken,
    OwnerTokens,
    ReconciliationObservation,
    SequenceGap,
};
