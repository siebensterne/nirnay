#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Checked-in KDB-X analytics contract for Nirnay.
//!
//! This crate does not reimplement market analytics in Rust. It owns the typed
//! input and output contracts around the checked-in q script
//! `analytics/kdb/mbp_metrics.q` and drives a locally installed KDB-X engine.
//! A differential reference computes the same arithmetic in Rust.
//! D11 retired the synthetic golden corpus; no checked-in golden is consumed.
//!
//! The KDB-X path is a research/statistics surface. Price and spread values
//! are f64 observations in the input unit (rupees as exported by
//! `upstox-decode-export`); they are not the authoritative money types used by
//! the trading domain. One book is analysed per source; sources are never
//! merged or summed.

mod metrics;
mod observation;
mod reference;
mod report;
mod runner;

pub use metrics::{METRIC_NAMES, METRICS_HEADER, MbpMetrics, parse_metrics_csv, to_json_value};
pub use observation::{MbpObservation, parse_observations_csv};
pub use reference::reference_metrics;
pub use report::{AnalysisReport, EngineInfo, REPORT_SCHEMA, build_report};
pub use runner::{Analysis, DEFAULT_Q_BIN, Q_BIN_ENV, analyze, find_q, q_script_path};
