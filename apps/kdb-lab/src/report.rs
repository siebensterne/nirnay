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

#[cfg(all(test, feature = "market-shaped-tests"))]
mod tests {
    use serde_json::Value;

    use super::build_report;
    use crate::metrics::parse_metrics_csv;
    use crate::runner::Analysis;

    const GOLDEN_METRICS: &str = include_str!("../tests/golden/mbp-metrics-v1.csv");
    const GOLDEN_REPORT: &str = include_str!("../tests/golden/mbp-report-v1.json");

    #[test]
    fn report_builder_matches_golden() {
        let stdout = GOLDEN_METRICS.to_owned();
        let metrics = parse_metrics_csv(GOLDEN_METRICS).expect("golden metrics must parse");
        let analysis = Analysis {
            metrics,
            stdout,
            input_sha256: "40811f73f7d3e9a4b1a73920b0a794db332585d040be891dba297f185f6178a5"
                .to_owned(),
            output_sha256: "b3b6566e24097a14432e3dadd19ca7e8f81ac43a9b31bec5106c92c690d36a61"
                .to_owned(),
            input_rows: 6,
            output_bytes: 296,
            binary: "q".to_owned(),
        };
        let report = build_report(&analysis, "mbp-observation-v1.csv");
        let actual = serde_json::to_value(report).expect("report must serialise");
        let expected: Value = serde_json::from_str(GOLDEN_REPORT).expect("golden must parse");
        assert_eq!(actual, expected, "report builder must match the golden");
    }
}
