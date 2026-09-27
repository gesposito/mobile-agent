//! Order statistics over latency samples in whole microseconds. Percentiles use
//! the nearest-rank method, as the spikes did, so every reported value is a
//! measured sample.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Percentiles and maximum of one series' measured samples, in microseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    /// Number of samples the statistics cover.
    pub n: u32,
    pub p50_us: u64,
    pub p95_us: u64,
    pub max_us: u64,
}

/// Nearest-rank percentile of `sorted`, which is in ascending order: the
/// smallest sample with at least `pct` percent of all samples at or below it.
/// `None` for no samples or a percentage outside 1..=100.
pub fn percentile(sorted: &[u64], pct: u32) -> Option<u64> {
    if sorted.is_empty() || pct == 0 || pct > 100 {
        return None;
    }
    // Rank ceil(pct * n / 100) in integers, so p95 of 100 samples is rank 95.
    let rank = (pct as usize * sorted.len()).div_ceil(100);
    Some(sorted[rank - 1])
}

/// Summarizes samples given in any order; `None` when there are none.
pub fn summarize(samples: &[u64]) -> Option<Summary> {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    Some(Summary {
        n: u32::try_from(sorted.len()).unwrap_or(u32::MAX),
        p50_us: percentile(&sorted, 50)?,
        p95_us: percentile(&sorted, 95)?,
        max_us: *sorted.last()?,
    })
}

/// `part` as a percentage of `whole`, rounded half up to a tenth; 0 when
/// `whole` is 0.
pub fn percent(part: u32, whole: u32) -> f64 {
    todo!("{part} of {whole} as a percentage")
}

/// Whole microseconds in `d`, saturating at `u64::MAX`.
pub fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// Milliseconds with three decimals, exact for whole microseconds; the unit of
/// every duration in the markdown summary.
pub fn ms(us: u64) -> String {
    format!("{}.{:03}", us / 1000, us % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn millis(values: impl IntoIterator<Item = u64>) -> Vec<u64> {
        values.into_iter().map(|v| v * 1000).collect()
    }

    #[test]
    fn nearest_rank_on_one_to_a_hundred() {
        let sorted = millis(1..=100);
        assert_eq!(percentile(&sorted, 50), Some(50_000));
        assert_eq!(percentile(&sorted, 95), Some(95_000));
        assert_eq!(percentile(&sorted, 100), Some(100_000));
        assert_eq!(percentile(&sorted, 1), Some(1_000));
    }

    #[test]
    fn the_rank_rounds_up_and_never_interpolates() {
        // 95 % of 30 samples is rank 28.5, so the 29th value.
        assert_eq!(percentile(&millis(1..=30), 95), Some(29_000));
        // An even count's median is the lower middle value, not an average.
        assert_eq!(percentile(&millis([1, 3, 5, 7]), 50), Some(3_000));
        assert_eq!(percentile(&millis([1, 3, 5]), 50), Some(3_000));
    }

    #[test]
    fn a_single_sample_is_every_percentile() {
        for pct in [1, 50, 95, 100] {
            assert_eq!(percentile(&[7], pct), Some(7));
        }
    }

    #[test]
    fn no_samples_or_an_out_of_range_percentage_give_none() {
        assert_eq!(percentile(&[], 50), None);
        assert_eq!(percentile(&[1, 2, 3], 0), None);
        assert_eq!(percentile(&[1, 2, 3], 101), None);
        assert_eq!(summarize(&[]), None);
    }

    #[test]
    fn the_summary_sorts_its_input() {
        let mut samples = millis(1..=20);
        samples.reverse();
        let summary = summarize(&samples).unwrap();
        assert_eq!(
            summary,
            Summary {
                n: 20,
                p50_us: 10_000,
                p95_us: 19_000,
                max_us: 20_000,
            }
        );
    }

    #[test]
    fn one_outlier_in_a_hundred_moves_max_but_not_p95() {
        let mut samples = vec![10_000; 99];
        samples.push(900_000);
        let summary = summarize(&samples).unwrap();
        assert_eq!(
            (summary.p50_us, summary.p95_us, summary.max_us),
            (10_000, 10_000, 900_000)
        );
    }

    #[test]
    fn milliseconds_have_three_exact_decimals() {
        assert_eq!(ms(126_438), "126.438");
        assert_eq!(ms(30), "0.030");
        assert_eq!(ms(150_000), "150.000");
        assert_eq!(ms(0), "0.000");
        assert_eq!(ms(2_000_000), "2000.000");
    }

    #[test]
    fn percentages_round_half_up_to_a_tenth() {
        assert_eq!(percent(2, 50), 4.0);
        assert_eq!(percent(1, 3), 33.3);
        assert_eq!(percent(2, 3), 66.7);
        assert_eq!(percent(1, 8), 12.5);
        assert_eq!(percent(1, 1600), 0.1);
        assert_eq!(percent(0, 50), 0.0);
        assert_eq!(percent(50, 50), 100.0);
        assert_eq!(percent(0, 0), 0.0);
    }

    #[test]
    fn micros_truncate_and_saturate() {
        assert_eq!(micros(Duration::from_nanos(1_999)), 1);
        assert_eq!(micros(Duration::from_millis(3)), 3_000);
        assert_eq!(micros(Duration::MAX), u64::MAX);
    }
}
