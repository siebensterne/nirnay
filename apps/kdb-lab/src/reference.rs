//! Independent Rust reference for the q analytics arithmetic.
//!
//! This is a differential check, not a replacement engine: KDB-X remains the
//! authority for the analytics result. The reference lets CI validate the
//! numeric golden contract when q is not installed, and catches a drifted q
//! script even if a golden file were regenerated incorrectly.

use anyhow::{
    Context, //
    Result,
    bail,
};

use crate::metrics::MbpMetrics;
use crate::observation::MbpObservation;

fn mean(values: &[f64]) -> Result<f64> {
    if values.is_empty() {
        bail!("cannot average an empty series");
    }
    Ok(values.iter().sum::<f64>() / values.len() as f64)
}

fn median(values: &[f64]) -> Result<f64> {
    if values.is_empty() {
        bail!("cannot take the median of an empty series");
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        Ok((sorted[middle - 1] + sorted[middle]) / 2.0)
    } else {
        Ok(sorted[middle])
    }
}

/// Compute every contract metric from typed observations without invoking q.
pub fn reference_metrics(rows: &[MbpObservation], source_id: &str) -> Result<MbpMetrics> {
    if rows.is_empty() {
        bail!("reference analytics require at least one observation");
    }
    let spreads: Vec<f64> = rows.iter().map(|row| row.spread).collect();
    let imbalances: Vec<f64> = rows.iter().map(|row| row.imbalance).collect();
    let bids: Vec<f64> = rows.iter().map(|row| row.bid1_p).collect();
    let asks: Vec<f64> = rows.iter().map(|row| row.ask1_p).collect();
    let gaps = rows
        .windows(2)
        .filter(|pair| pair[1].seq != pair[0].seq + 1)
        .count();
    let duration_ns = rows
        .last()
        .context("reference analytics require a last row")?
        .recv_ns
        - rows
            .first()
            .context("reference analytics require a first row")?
            .recv_ns;
    let levels_min = rows
        .iter()
        .map(|row| row.levels)
        .min()
        .context("no levels")?;
    let bid1_max_qty = rows
        .iter()
        .map(|row| row.bid1_q)
        .max()
        .context("no bid quantity")?;
    let ask1_max_qty = rows
        .iter()
        .map(|row| row.ask1_q)
        .max()
        .context("no ask quantity")?;

    Ok(MbpMetrics {
        schema_version: 1,
        source_id: source_id.to_owned(),
        rows: rows.len() as u64,
        duration_s: duration_ns as f64 / 1_000_000_000.0,
        gap_count: gaps as u64,
        spread_mean: mean(&spreads)?,
        spread_median: median(&spreads)?,
        spread_max: spreads.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        imbalance_mean: mean(&imbalances)?,
        imbalance_min: imbalances.iter().copied().fold(f64::INFINITY, f64::min),
        imbalance_max: imbalances.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        levels_min,
        bid1_mean: mean(&bids)?,
        ask1_mean: mean(&asks)?,
        bid1_max_qty,
        ask1_max_qty,
    })
}

#[cfg(all(test, feature = "market-shaped-tests"))]
mod tests {
    use super::reference_metrics;
    use crate::metrics::parse_metrics_csv;
    use crate::observation::parse_observations_csv;

    const INPUT: &str = include_str!("../tests/golden/mbp-observation-v1.csv");
    const EXPECTED: &str = include_str!("../tests/golden/mbp-metrics-v1.csv");

    fn close(actual: f64, expected: f64, label: &str) {
        assert!(
            (actual - expected).abs() <= 1e-6,
            "{label}: reference {actual} differs from q golden {expected}"
        );
    }

    #[test]
    fn reference_matches_recorded_q_golden() {
        let rows = parse_observations_csv(INPUT).expect("input must parse");
        let reference = reference_metrics(&rows, "synthetic-source").expect("metrics");
        let golden = parse_metrics_csv(EXPECTED).expect("golden must parse");
        assert_eq!(reference.rows, golden.rows, "row count");
        assert_eq!(reference.gap_count, golden.gap_count, "gap count");
        assert_eq!(reference.levels_min, golden.levels_min, "level floor");
        assert_eq!(reference.bid1_max_qty, golden.bid1_max_qty, "bid size");
        assert_eq!(reference.ask1_max_qty, golden.ask1_max_qty, "ask size");
        let pairs: [(f64, f64, &str); 8] = [
            (reference.duration_s, golden.duration_s, "duration_s"),
            (reference.spread_mean, golden.spread_mean, "spread_mean"),
            (
                reference.spread_median,
                golden.spread_median,
                "spread_median",
            ),
            (reference.spread_max, golden.spread_max, "spread_max"),
            (
                reference.imbalance_mean,
                golden.imbalance_mean,
                "imbalance_mean",
            ),
            (
                reference.imbalance_min,
                golden.imbalance_min,
                "imbalance_min",
            ),
            (
                reference.imbalance_max,
                golden.imbalance_max,
                "imbalance_max",
            ),
            (reference.bid1_mean, golden.bid1_mean, "bid1_mean"),
        ];
        for (actual, expected, label) in pairs {
            close(actual, expected, label);
        }
        close(reference.ask1_mean, golden.ask1_mean, "ask1_mean");
    }
}
