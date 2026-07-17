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
    /// Relative spread, `(max - min) / mean * 100`. The number the noise floor
    /// judges: it is dispersion expressed on the same scale as the BEN-020
    /// margin, so the two can be compared directly. `0.0` when `mean` is 0.
    pub spread_pct: f64,
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
    let mean = sum / count as f64;
    Some(Dispersion {
        min,
        mean,
        max,
        spread_pct: if mean.abs() > f64::EPSILON {
            (max - min) / mean * 100.0
        } else {
            0.0
        },
    })
}

/// The noise floor a run must clear before any cell of it may be judged
/// (BEN-011 + BEN-020).
///
/// **Why this exists, and why it fails the run instead of annotating it.**
/// The harness reported dispersion from the start and nothing consumed it. It
/// was possible — and it happened — to publish a per-cell G5 verdict from a
/// matrix whose untouched peer lanes had moved +95%/+75%/+45% between runs and
/// whose worst cells swung 43 points. Every one of those numbers was in the
/// artifact; none of them stopped the verdict being written.
///
/// A number that is reported but cannot fail anything is decoration. BEN-020
/// asks whether Thunder leads by **≥10%**; a cell whose own repetitions
/// disagree by more than that margin cannot answer the question in either
/// direction, so the honest outcome is to refuse the run, not to round it off.
pub const DEFAULT_NOISE_FLOOR_PCT: f64 = 5.0;

/// The margin BEN-020 requires Thunder to lead every peer by, in percent.
/// [`DEFAULT_NOISE_FLOOR_PCT`] must stay below it — a run whose own noise is as
/// wide as the margin can manufacture the gate in either direction.
pub const BEN_020_MARGIN_PCT: f64 = 10.0;

/// One cell that failed the noise floor.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct NoisyCell {
    /// Scenario name.
    pub scenario: String,
    /// Lane key.
    pub lane: String,
    /// Pipeline depth.
    pub depth: usize,
    /// Connection count.
    pub connections: usize,
    /// The qps spread that busted the floor.
    pub spread_pct: f64,
}

/// The verdict on a whole run's stability.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct NoiseVerdict {
    /// The floor applied.
    pub floor_pct: f64,
    /// Cells whose qps spread exceeded the floor, worst first.
    pub offenders: Vec<NoisyCell>,
}

impl NoiseVerdict {
    /// Whether the run is quiet enough for its cells to mean anything.
    pub fn is_quiet(&self) -> bool {
        self.offenders.is_empty()
    }

    /// The worst spread seen, if any cell busted the floor.
    pub fn worst_spread_pct(&self) -> Option<f64> {
        self.offenders.first().map(|c| c.spread_pct)
    }
}

/// Judge a run: any cell whose qps spread across repetitions exceeds `floor_pct`
/// makes the run unfit to answer BEN-020. Offenders come back worst-first.
///
/// Cells with a single repetition have no spread to measure and are skipped —
/// they are not evidence of quiet, so `repetitions: 1` cannot pass this check
/// by vacuum; the runner refuses that separately.
pub fn noise_check<I>(cells: I, floor_pct: f64) -> NoiseVerdict
where
    I: IntoIterator<Item = NoisyCell>,
{
    let mut offenders: Vec<NoisyCell> = cells
        .into_iter()
        .filter(|c| c.spread_pct > floor_pct)
        .collect();
    offenders.sort_by(|a, b| {
        b.spread_pct
            .partial_cmp(&a.spread_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    NoiseVerdict {
        floor_pct,
        offenders,
    }
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
        // (6 - 2) / 4 = 100%.
        assert_eq!(d.spread_pct, 100.0);
    }

    #[test]
    fn dispersion_of_empty_is_none() {
        assert!(dispersion(std::iter::empty()).is_none());
    }

    #[test]
    fn identical_repetitions_have_no_spread() {
        let d = dispersion([50.0, 50.0, 50.0]).unwrap();
        assert_eq!(d.spread_pct, 0.0);
    }

    #[test]
    fn zero_mean_reports_zero_spread_rather_than_dividing_by_zero() {
        let d = dispersion([0.0, 0.0]).unwrap();
        assert_eq!(d.spread_pct, 0.0);
        assert!(d.spread_pct.is_finite());
    }

    fn cell(scenario: &str, spread_pct: f64) -> NoisyCell {
        NoisyCell {
            scenario: scenario.to_owned(),
            lane: "thunder".to_owned(),
            depth: 1,
            connections: 1,
            spread_pct,
        }
    }

    #[test]
    fn a_quiet_run_has_no_offenders() {
        let v = noise_check([cell("point-echo-64B", 1.2), cell("medium-4KiB", 4.9)], 5.0);
        assert!(v.is_quiet());
        assert_eq!(v.worst_spread_pct(), None);
    }

    #[test]
    fn a_cell_over_the_floor_fails_the_run_and_offenders_come_worst_first() {
        // The real numbers that motivated the floor: medium-4KiB swung 43 points
        // across runs while a quiet cell sat at 1.2%.
        let v = noise_check(
            [
                cell("point-echo-64B", 1.2),
                cell("medium-4KiB", 43.0),
                cell("pipelined-1k", 12.0),
            ],
            DEFAULT_NOISE_FLOOR_PCT,
        );
        assert!(!v.is_quiet());
        assert_eq!(v.offenders.len(), 2);
        assert_eq!(v.offenders[0].scenario, "medium-4KiB");
        assert_eq!(v.offenders[1].scenario, "pipelined-1k");
        assert_eq!(v.worst_spread_pct(), Some(43.0));
    }

    /// The floor only means something if a cell that clears it cannot be hiding
    /// a swing as large as the margin under test — otherwise noise could
    /// manufacture the gate. Asserted at compile time: it is a property of the
    /// constant, not of any run.
    const _: () = assert!(DEFAULT_NOISE_FLOOR_PCT < BEN_020_MARGIN_PCT);

    #[test]
    fn a_cell_exactly_at_the_floor_passes() {
        let v = noise_check([cell("point-echo-64B", 5.0)], 5.0);
        assert!(v.is_quiet());
    }
}
