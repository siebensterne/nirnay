//! Typed input contract for one source's five-level market-by-price book.
//!
//! The CSV is the same header-prefixed export consumed by
//! `analytics/kdb/mbp_metrics.q`. Fields are primitive scalars; quoting and
//! embedded commas sit outside the contract by design.

use std::str::Split;

use anyhow::{
    Context, //
    Result,
    bail,
};

/// Canonical header of a KDB-X analytics input file.
pub const OBSERVATION_HEADER: &str =
    "seq,recv_ns,bid1_p,bid1_q,ask1_p,ask1_q,spread,imbalance,levels";

/// One decoded five-level observation. Prices and spread are f64 in the
/// input unit; quantities and sequence/level counts are integers.
#[derive(Debug, Clone, PartialEq)]
pub struct MbpObservation {
    pub seq: i64,
    pub recv_ns: i64,
    pub bid1_p: f64,
    pub bid1_q: i64,
    pub ask1_p: f64,
    pub ask1_q: i64,
    pub spread: f64,
    pub imbalance: f64,
    pub levels: u32,
}

/// Split one line into non-empty trimmed fields. Quoting is not supported by
/// this contract; a field containing a comma is rejected by the arity check.
fn fields(line: &str) -> Split<'_, char> {
    line.split(',')
}

fn next_field(parts: &mut Split<'_, char>, name: &str, row: usize) -> Result<String> {
    parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .with_context(|| format!("row {row} missing {name}"))
}

/// Parse every observation in a header-prefixed CSV, failing closed on a
/// header mismatch, wrong arity, non-finite float or negative count.
pub fn parse_observations_csv(text: &str) -> Result<Vec<MbpObservation>> {
    let mut lines = text.lines();
    let header = lines
        .next()
        .context("observation CSV is empty; expected a header row")?
        .trim();
    if header != OBSERVATION_HEADER {
        bail!("observation CSV header {header:?} does not match {OBSERVATION_HEADER:?}");
    }
    let mut rows = Vec::new();
    for (index, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = fields(line);
        let row = index + 1;
        let seq = next_field(&mut parts, "seq", row)?
            .parse::<i64>()
            .context("invalid seq")?;
        let recv_ns = next_field(&mut parts, "recv_ns", row)?
            .parse::<i64>()
            .context("invalid recv_ns")?;
        let bid1_p = next_field(&mut parts, "bid1_p", row)?
            .parse::<f64>()
            .context("invalid bid1_p")?;
        let bid1_q = next_field(&mut parts, "bid1_q", row)?
            .parse::<i64>()
            .context("invalid bid1_q")?;
        let ask1_p = next_field(&mut parts, "ask1_p", row)?
            .parse::<f64>()
            .context("invalid ask1_p")?;
        let ask1_q = next_field(&mut parts, "ask1_q", row)?
            .parse::<i64>()
            .context("invalid ask1_q")?;
        let spread = next_field(&mut parts, "spread", row)?
            .parse::<f64>()
            .context("invalid spread")?;
        let imbalance = next_field(&mut parts, "imbalance", row)?
            .parse::<f64>()
            .context("invalid imbalance")?;
        let levels = next_field(&mut parts, "levels", row)?
            .parse::<u32>()
            .context("invalid levels")?;
        if parts.next().is_some() {
            bail!("row {row} has more than nine fields");
        }
        for (name, value) in [
            ("bid1_p", bid1_p),
            ("ask1_p", ask1_p),
            ("spread", spread),
            ("imbalance", imbalance),
        ] {
            if !value.is_finite() {
                bail!("row {row} has non-finite {name}");
            }
        }
        rows.push(MbpObservation {
            seq,
            recv_ns,
            bid1_p,
            bid1_q,
            ask1_p,
            ask1_q,
            spread,
            imbalance,
            levels,
        });
    }
    if rows.is_empty() {
        bail!("observation CSV has no data rows");
    }
    Ok(rows)
}
