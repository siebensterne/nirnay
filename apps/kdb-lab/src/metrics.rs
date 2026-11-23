//! Typed output contract emitted by `analytics/kdb/mbp_metrics.q`.
//!
//! The q script writes deterministic `metric,value` lines. This module rejects
//! unknown, missing or duplicated names so a drifted engine cannot be mistaken
//! for a valid result.

use std::collections::BTreeMap;

use anyhow::{
    Context, //
    Result,
    bail,
};
use serde_json::{
    Map, //
    Value,
};

/// Header of every KDB-X metrics document.
pub const METRICS_HEADER: &str = "metric,value";

/// Canonical metric order. The q script emits exactly these names in this
/// order; the parser validates the set but stores values by name.
pub const METRIC_NAMES: &[&str] = &[
    "schema_version",
    "source_id",
    "rows",
    "duration_s",
    "gap_count",
    "spread_mean",
    "spread_median",
    "spread_max",
    "imbalance_mean",
    "imbalance_min",
    "imbalance_max",
    "levels_min",
    "bid1_mean",
    "ask1_mean",
    "bid1_max_qty",
    "ask1_max_qty",
];

/// Parsed metrics from one q run.
#[derive(Debug, Clone, PartialEq)]
pub struct MbpMetrics {
    pub schema_version: u32,
    pub source_id: String,
    pub rows: u64,
    pub duration_s: f64,
    pub gap_count: u64,
    pub spread_mean: f64,
    pub spread_median: f64,
    pub spread_max: f64,
    pub imbalance_mean: f64,
    pub imbalance_min: f64,
    pub imbalance_max: f64,
    pub levels_min: u32,
    pub bid1_mean: f64,
    pub ask1_mean: f64,
    pub bid1_max_qty: i64,
    pub ask1_max_qty: i64,
}

fn pull<'a>(values: &'a BTreeMap<String, String>, name: &str) -> Result<&'a str> {
    values
        .get(name)
        .map(String::as_str)
        .with_context(|| format!("q output is missing metric {name}"))
}

fn as_u64(values: &BTreeMap<String, String>, name: &str) -> Result<u64> {
    let raw = pull(values, name)?;
    raw.parse::<u64>()
        .with_context(|| format!("metric {name} value {raw:?} is not a non-negative integer"))
}

fn as_i64(values: &BTreeMap<String, String>, name: &str) -> Result<i64> {
    let raw = pull(values, name)?;
    raw.parse::<i64>()
        .with_context(|| format!("metric {name} value {raw:?} is not an integer"))
}

fn as_f64(values: &BTreeMap<String, String>, name: &str) -> Result<f64> {
    let raw = pull(values, name)?;
    let value = raw
        .parse::<f64>()
        .with_context(|| format!("metric {name} value {raw:?} is not a number"))?;
    if value.is_finite() {
        Ok(value)
    } else {
        bail!("metric {name} value {raw:?} is not finite")
    }
}

/// Parse the complete stdout of one q analytics invocation.
pub fn parse_metrics_csv(text: &str) -> Result<MbpMetrics> {
    let mut lines = text.lines();
    let header = lines
        .next()
        .context("q produced no output")?
        .trim_end_matches('\r');
    if header != METRICS_HEADER {
        bail!("q output header {header:?} does not match {METRICS_HEADER:?}");
    }
    let mut values: BTreeMap<String, String> = BTreeMap::new();
    for line in lines {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, ',');
        let name = parts
            .next()
            .context("q output line has no metric name")?
            .trim();
        let raw = parts
            .next()
            .with_context(|| format!("q metric {name} has no value"))?
            .trim();
        if !METRIC_NAMES.contains(&name) {
            bail!("q emitted unknown metric {name:?}");
        }
        if values.insert(name.to_owned(), raw.to_owned()).is_some() {
            bail!("q emitted metric {name} twice");
        }
    }
    for name in METRIC_NAMES {
        if !values.contains_key(*name) {
            bail!("q output is missing metric {name}");
        }
    }

    Ok(MbpMetrics {
        schema_version: as_u64(&values, "schema_version")? as u32,
        source_id: pull(&values, "source_id")?.to_owned(),
        rows: as_u64(&values, "rows")?,
        duration_s: as_f64(&values, "duration_s")?,
        gap_count: as_u64(&values, "gap_count")?,
        spread_mean: as_f64(&values, "spread_mean")?,
        spread_median: as_f64(&values, "spread_median")?,
        spread_max: as_f64(&values, "spread_max")?,
        imbalance_mean: as_f64(&values, "imbalance_mean")?,
        imbalance_min: as_f64(&values, "imbalance_min")?,
        imbalance_max: as_f64(&values, "imbalance_max")?,
        levels_min: as_u64(&values, "levels_min")? as u32,
        bid1_mean: as_f64(&values, "bid1_mean")?,
        ask1_mean: as_f64(&values, "ask1_mean")?,
        bid1_max_qty: as_i64(&values, "bid1_max_qty")?,
        ask1_max_qty: as_i64(&values, "ask1_max_qty")?,
    })
}

/// Convert typed metrics to a JSON object for the analysis report.
pub fn to_json_value(metrics: &MbpMetrics) -> Value {
    let mut object = Map::new();
    object.insert(
        "schema_version".to_owned(),
        Value::from(metrics.schema_version),
    );
    object.insert(
        "source_id".to_owned(),
        Value::String(metrics.source_id.clone()),
    );
    object.insert("rows".to_owned(), Value::from(metrics.rows));
    object.insert("duration_s".to_owned(), Value::from(metrics.duration_s));
    object.insert("gap_count".to_owned(), Value::from(metrics.gap_count));
    object.insert("spread_mean".to_owned(), Value::from(metrics.spread_mean));
    object.insert(
        "spread_median".to_owned(),
        Value::from(metrics.spread_median),
    );
    object.insert("spread_max".to_owned(), Value::from(metrics.spread_max));
    object.insert(
        "imbalance_mean".to_owned(),
        Value::from(metrics.imbalance_mean),
    );
    object.insert(
        "imbalance_min".to_owned(),
        Value::from(metrics.imbalance_min),
    );
    object.insert(
        "imbalance_max".to_owned(),
        Value::from(metrics.imbalance_max),
    );
    object.insert("levels_min".to_owned(), Value::from(metrics.levels_min));
    object.insert("bid1_mean".to_owned(), Value::from(metrics.bid1_mean));
    object.insert("ask1_mean".to_owned(), Value::from(metrics.ask1_mean));
    object.insert("bid1_max_qty".to_owned(), Value::from(metrics.bid1_max_qty));
    object.insert("ask1_max_qty".to_owned(), Value::from(metrics.ask1_max_qty));
    Value::Object(object)
}
