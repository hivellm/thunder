//! Per-cell metrics (BEN-010) and run-discipline helpers (BEN-011):
//! nearest-rank percentiles, qps, and min/mean/max dispersion across
//! repetitions.

use std::time::Duration;

/// Metrics for one repetition of one cell (BEN-010: p50, p99, qps; ops and
/// wall time kept so the artifact is self-checking).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct CellStats {
    /// Measured operations in this repetition (warmup already discarded).
    pub ops: u64,
    /// Wall time of the measured window, seconds.
    pub elapsed_secs: f64,
    /// Median latency, microseconds.
    pub p50_us: f64,
    /// 99th-percentile latency, microseconds.
    pub p99_us: f64,
    /// Throughput: `ops / elapsed`.
    pub qps: f64,
}

/// Sort the latency sample and compute one repetition's [`CellStats`].
pub fn compute(latencies: &mut [Duration], elapsed: Duration) -> CellStats {
    latencies.sort_unstable();
    let ops = latencies.len() as u64;
    let elapsed_secs = elapsed.as_secs_f64();
    CellStats {
        ops,
        elapsed_secs,
        p50_us: percentile_us(latencies, 50.0),
        p99_us: percentile_us(latencies, 99.0),
        qps: if elapsed_secs > 0.0 {
            ops as f64 / elapsed_secs
        } else {
            0.0
        },
    }
}

/// Nearest-rank percentile over an ascending-sorted sample, in
/// microseconds. Empty samples report `0.0`.
pub fn percentile_us(sorted: &[Duration], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let n = sorted.len();
    let rank = ((pct / 100.0) * n as f64).ceil() as usize;
    let idx = rank.clamp(1, n) - 1;
    sorted[idx].as_secs_f64() * 1e6
}

/// Min / mean / max across repetitions — the dispersion BEN-011 requires
/// next to every reported number.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Dispersion {
    /// Smallest repetition value.
    pub min: f64,
    /// Mean across repetitions.
    pub mean: f64,
    /// Largest repetition value.
    pub max: f64,
}

/// Compute [`Dispersion`] over one metric across repetitions. Returns
/// `None` for an empty input (pending cells).
pub fn dispersion<I: IntoIterator<Item = f64>>(values: I) -> Option<Dispersion> {
    let mut count = 0u64;
    let mut sum = 0.0f64;
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for v in values {
        count += 1;
        sum += v;
        min = min.min(v);
        max = max.max(v);
    }
    if count == 0 {
        return None;
    }
    Some(Dispersion {
        min,
        mean: sum / count as f64,
        max,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn us(values: &[u64]) -> Vec<Duration> {
        values.iter().map(|&v| Duration::from_micros(v)).collect()
    }

    #[test]
    fn percentile_nearest_rank_on_small_sample() {
        let sorted = us(&[10, 20, 30, 40]);
        // nearest-rank: p50 of 4 samples -> rank ceil(2) = 2nd value.
        assert_eq!(percentile_us(&sorted, 50.0), 20.0);
        // p99 of 4 samples -> rank ceil(3.96) = 4th value.
        assert_eq!(percentile_us(&sorted, 99.0), 40.0);
        // p100 stays in bounds.
        assert_eq!(percentile_us(&sorted, 100.0), 40.0);
    }

    #[test]
    fn percentile_on_100_samples_hits_the_textbook_ranks() {
        let sorted: Vec<Duration> = (1..=100).map(Duration::from_micros).collect();
        assert_eq!(percentile_us(&sorted, 50.0), 50.0);
        assert_eq!(percentile_us(&sorted, 99.0), 99.0);
    }

    #[test]
    fn percentile_of_single_sample_is_that_sample() {
        let sorted = us(&[7]);
        assert_eq!(percentile_us(&sorted, 50.0), 7.0);
        assert_eq!(percentile_us(&sorted, 99.0), 7.0);
    }

    #[test]
    fn percentile_of_empty_sample_is_zero() {
        assert_eq!(percentile_us(&[], 50.0), 0.0);
    }

    #[test]
    fn compute_sorts_and_reports_qps() {
        // Deliberately unsorted input: compute() must sort before ranking.
        let mut lats = us(&[30, 10, 20, 40]);
        let stats = compute(&mut lats, Duration::from_secs(2));
        assert_eq!(stats.ops, 4);
        assert_eq!(stats.p50_us, 20.0);
        assert_eq!(stats.p99_us, 40.0);
        assert_eq!(stats.qps, 2.0);
        assert_eq!(stats.elapsed_secs, 2.0);
    }

    #[test]
    fn compute_zero_elapsed_reports_zero_qps() {
        let mut lats = us(&[5]);
        let stats = compute(&mut lats, Duration::ZERO);
        assert_eq!(stats.qps, 0.0);
    }

    #[test]
    fn dispersion_reports_min_mean_max() {
        let d = dispersion([2.0, 4.0, 6.0]).unwrap();
        assert_eq!(d.min, 2.0);
        assert_eq!(d.mean, 4.0);
        assert_eq!(d.max, 6.0);
    }

    #[test]
    fn dispersion_of_empty_is_none() {
        assert!(dispersion(std::iter::empty()).is_none());
    }
}
