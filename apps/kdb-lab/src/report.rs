//! Machine-readable analysis report with input/output digests.
//!
//! The report is validated by `schemas/analytics/kdb-metrics-report.v1.schema.json`
//! in the repository's contract gate. It never contains credentials or
//! provider account identifiers.

use serde::Serialize;
use serde_json::Value;

use crate::metrics::to_json_value;
use crate::runner::Analysis;

/// Schema reference carried by every report.
pub const REPORT_SCHEMA: &str = "schemas/analytics/kdb-metrics-report.v1.schema.json";

/// Identity of the analytics engine used for one run.
#[derive(Debug, Clone, Serialize)]
pub struct EngineInfo {
    pub name: &'static str,
    pub binary: String,
}

/// Input lineage recorded for one run.
#[derive(Debug, Clone, Serialize)]
pub struct InputInfo {
    pub path: String,
    pub sha256: String,
    pub rows: u64,
}

/// Output lineage recorded for one run.
#[derive(Debug, Clone, Serialize)]
pub struct OutputInfo {
    pub sha256: String,
    pub bytes: u64,
}

/// Complete report for one KDB-X analytics run.
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisReport {
    pub schema: &'static str,
    pub schema_version: u32,
    pub source_id: String,
    pub engine: EngineInfo,
    pub input: InputInfo,
    pub output: OutputInfo,
    pub metrics: Value,
}

/// Build a report from a successful analysis invocation.
pub fn build_report(analysis: &Analysis, input_path: &str) -> AnalysisReport {
    AnalysisReport {
        schema: REPORT_SCHEMA,
        schema_version: 1,
        source_id: analysis.metrics.source_id.clone(),
        engine: EngineInfo {
            name: "kdb-x",
            binary: analysis.binary.clone(),
        },
        input: InputInfo {
            path: input_path.to_owned(),
            sha256: analysis.input_sha256.clone(),
            rows: analysis.input_rows,
        },
        output: OutputInfo {
            sha256: analysis.output_sha256.clone(),
            bytes: analysis.output_bytes,
        },
        metrics: to_json_value(&analysis.metrics),
    }
}
