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
    ACCOUNT_ANCHOR_FILE, //
    ACCOUNT_HIGH_WATER_FILE,
    ACCOUNT_JOURNAL_FILE,
    ACCOUNT_JOURNAL_ID,
    AccountRecoveryRecord,
    AuthorityOwnership,
    AuthoritySnapshot,
    BINDING_SUFFIX,
    CLOCK_BINDING_IDENTITY,
    ChainedRecord,
    DAILY_PNL_FILE,
    DECISION_FEATURE_VERSION,
    DECISION_LINEAGE_VERSION,
    DECISION_MODEL_VERSION,
    DecisionLineage,
    DecisionOutcome,
    DecisionReplay,
    DecisionReplayInputs,
    DecisionStore,
    DecisionStoreError,
    EvaluatorIdentity,
    FileDecisionStore,
    GENESIS_DIGEST,
    InMemoryDecisionStore,
    InputRef,
    JournalIdentity,
    ModelVersions,
    OMS_ANCHOR_FILE,
    OMS_EXECUTIONS_FILE,
    OMS_HIGH_WATER_FILE,
    OMS_JOURNAL_ID,
    OMS_PERMITS_FILE,
    OmsPermitRecord,
    OmsPermitState,
    OwnerSigningKey,
    RECOVERY_LOCK_FILE,
    RECOVERY_SNAPSHOT_VERSION,
    RecoveryAnchor,
    RecoveryAuthenticity,
    RecoveryBindingRecord,
    RecoveryCheckpoint,
    RecoveryConfig,
    RecoveryError,
    RecoveryHighWater,
    RecoveryJournalRoot,
    RecoveryWriterLock,
    ReplayError,
    Reservation,
    RiskAuthority,
    RiskDecisionRecord,
    SourceBinding,
    VerifiedAccountSnapshot,
    VerifiedOmsSnapshot,
    genesis_digest,
    recovery_binding_record,
    replay_decision,
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
