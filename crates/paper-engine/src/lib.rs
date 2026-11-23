#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Deterministic paper-trading engine for the India platform.
//!
//! Derived from the internal Nirnay `crates/nirnay-engine/src/{engine,state,market}.rs` at
//! source snapshot `cc3a34c317a86a9984401efe90fa3f43d67a1787`.
//!
//! Responsibilities stay separated: `trading-core` owns contracts,
//! [`risk_engine`] owns pre-trade risk, [`cost_model`] owns charge schedules
//! and [`account_journal`] owns durable event storage. This crate owns order
//! lifecycle, paper matching and the account projection.
//!
//! Matching consumes the displayed top of book. It is a deterministic fill
//! model, not a realistic historical execution simulator.

mod capsule;
mod engine;
mod market;
mod state;

pub use capsule::{
    CapsuleError, DecisionCapsule, EvidenceEvent, FillEvidence, OrderEvidence,
    ReconciliationEvidence, RiskEvidence,
};
pub use engine::{
    CancelOutcome, Engine, EngineConfig, EngineError, PaperProfile, QuoteOutcome, ReconcileInput,
    SubmissionOutcome,
};
pub use state::{AccountView, OrderView, PositionView};
