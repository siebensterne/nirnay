#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Pure numeric helper retained after D11.
//!
//! The opt-in authorization benchmark constructed invented quotes, books and
//! seeded intents; that workload and its builders were retired. Only the
//! scalar percentile helper and its numeric test remain.

fn percentile(sorted: &[u128], fraction: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

#[cfg(test)]
mod percentile_tests {
    use super::percentile;

    #[test]
    fn percentile_selects_known_quantiles() {
        let samples = [10_u128, 20, 30, 40, 50];
        assert_eq!(percentile(&samples, 0.5), 30, "median sample");
        assert_eq!(percentile(&samples, 0.999), 50, "upper sample");
        assert_eq!(percentile(&[], 0.5), 0, "empty input has no rank");
    }
}
