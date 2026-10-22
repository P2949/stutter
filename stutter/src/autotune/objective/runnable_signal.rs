//! Whole-window scheduler latency evidence. Percentiles are conservative merged
//! histogram bucket bounds, never averages of interval percentiles.
use super::{ObjectiveSignalQuality, ObjectiveSignals};
use crate::{
    metrics::{IntervalRecord, LatencyHistogram, LatencyHistogramBucket},
    process_tree::TaskClass,
};

pub(crate) const RUNNABLE_HISTOGRAM_SOURCE: &str = "target-game-scheduler-histogram-v1";

pub(crate) fn is_primary_game_class(class: TaskClass) -> bool {
    matches!(
        class,
        TaskClass::Game
            | TaskClass::GameRenderThread
            | TaskClass::GameWorkerThread
            | TaskClass::GameHelper
            | TaskClass::WineServer
    )
}

struct WindowHistogram {
    buckets: Vec<LatencyHistogramBucket>,
    samples: u64,
    max_ns: u64,
}

impl WindowHistogram {
    fn from_records<'a>(
        records: impl Iterator<Item = &'a IntervalRecord>,
    ) -> Result<Self, &'static str> {
        let mut window = Self {
            buckets: LatencyHistogram::new().snapshot(),
            samples: 0,
            max_ns: 0,
        };
        for record in records.filter(|record| is_primary_game_class(record.class)) {
            if record.samples == 0
                && record.over_1ms == 0
                && record.over_2ms == 0
                && record.over_5ms == 0
                && record.histogram.iter().all(|bucket| bucket.count == 0)
            {
                continue;
            }
            if record.histogram.len() != window.buckets.len()
                || record
                    .histogram
                    .iter()
                    .zip(&window.buckets)
                    .any(|(incoming, expected)| incoming.upper_bound_ns != expected.upper_bound_ns)
            {
                return Err("missing or incompatible runnable-latency histogram");
            }
            let count = record
                .histogram
                .iter()
                .try_fold(0u64, |total, bucket| total.checked_add(bucket.count))
                .ok_or("runnable-latency histogram count overflow")?;
            if count != record.samples {
                return Err("inconsistent runnable-latency histogram population");
            }
            for threshold in [1_000_000, 2_000_000, 5_000_000] {
                let tail = tail_count(&record.histogram, threshold);
                let expected = match threshold {
                    1_000_000 => record.over_1ms,
                    2_000_000 => record.over_2ms,
                    _ => record.over_5ms,
                };
                if tail != expected {
                    return Err("runnable-latency histogram disagrees with tail counters");
                }
            }
            // A populated bucket's lower bound must be below the observed max.
            let mut lower_bound = None;
            for bucket in &record.histogram {
                if bucket.count > 0 && lower_bound.is_some_and(|lower| record.max_ns <= lower) {
                    return Err("runnable-latency histogram disagrees with maximum");
                }
                lower_bound = bucket.upper_bound_ns;
            }
            if record
                .histogram
                .iter()
                .find(|bucket| {
                    bucket
                        .upper_bound_ns
                        .is_none_or(|upper| record.max_ns <= upper)
                })
                .is_none_or(|bucket| bucket.count == 0)
            {
                return Err("runnable-latency maximum has no histogram observation");
            }
            window.samples = window
                .samples
                .checked_add(count)
                .ok_or("runnable-latency window count overflow")?;
            window.max_ns = window.max_ns.max(record.max_ns);
            for (bucket, incoming) in window.buckets.iter_mut().zip(&record.histogram) {
                bucket.count = bucket
                    .count
                    .checked_add(incoming.count)
                    .ok_or("runnable-latency bucket count overflow")?;
            }
        }
        if window.samples == 0 {
            return Err("direct runnable-latency signal unavailable: no game scheduler samples");
        }
        Ok(window)
    }

    fn percentile(&self, per_thousand: u64) -> u64 {
        let rank = (u128::from(self.samples) * u128::from(per_thousand)).div_ceil(1_000) as u64;
        let mut cumulative = 0u64;
        for bucket in &self.buckets {
            cumulative += bucket.count;
            if cumulative >= rank {
                return bucket
                    .upper_bound_ns
                    .unwrap_or(self.max_ns)
                    .min(self.max_ns);
            }
        }
        // The validated population covers the rank. Retain a conservative bound
        // instead of a production panic if histogram representation changes.
        self.max_ns
    }
}

fn tail_count(buckets: &[LatencyHistogramBucket], threshold: u64) -> u64 {
    // Boundaries are validated against the canonical histogram and include each
    // threshold. Counts therefore describe strict >threshold events exactly.
    buckets
        .iter()
        .filter(|bucket| bucket.upper_bound_ns.is_none_or(|upper| upper > threshold))
        .map(|bucket| bucket.count)
        .sum()
}

impl ObjectiveSignals {
    pub(crate) fn set_runnable_latency_from_records<'a>(
        &mut self,
        records: impl Iterator<Item = &'a IntervalRecord>,
    ) {
        let result = WindowHistogram::from_records(records);
        let window = result.as_ref().ok();
        self.runnable_latency_p50_ns = window.map(|window| window.percentile(500));
        self.runnable_latency_p95_ns = window.map(|window| window.percentile(950));
        self.runnable_latency_p99_ns = window.map(|window| window.percentile(990));
        self.runnable_latency_p999_ns = window
            .filter(|window| window.samples >= 10_000)
            .map(|window| window.percentile(999));
        self.runnable_latency_max_ns = window.map(|window| window.max_ns);
        self.runnable_samples = window.map(|window| window.samples);
        self.runnable_over_1ms = window.map(|window| tail_count(&window.buckets, 1_000_000));
        self.runnable_over_2ms = window.map(|window| tail_count(&window.buckets, 2_000_000));
        self.runnable_over_5ms = window.map(|window| tail_count(&window.buckets, 5_000_000));
        self.runnable_latency_source = window.map(|_| RUNNABLE_HISTOGRAM_SOURCE.to_owned());
        self.runnable_latency_unavailable_reason =
            result.as_ref().err().map(|reason| (*reason).to_owned());
        self.signal_quality.runnable_latency = if window.is_some() {
            ObjectiveSignalQuality::Direct
        } else {
            ObjectiveSignalQuality::Missing
        };
    }
}

/// Production runnable-latency signal for one identified target process, computed from
/// recorded intervals with the same merged-histogram logic the live controller uses.
/// Used by live acceptance so it never injects an already-computed objective signal.
pub fn runnable_signals_for_target(
    records: &[IntervalRecord],
    root_pid: u32,
    starttime_ticks: u64,
) -> ObjectiveSignals {
    let mut signals = ObjectiveSignals::default();
    signals.set_runnable_latency_from_records(
        records
            .iter()
            .filter(|record| record.process_pid == Some(root_pid)),
    );
    signals.runnable_target_root_pid = Some(root_pid);
    signals.runnable_target_starttime_ticks = Some(starttime_ticks);
    signals
}

#[cfg(test)]
mod target_signal_tests {
    use super::*;
    use crate::metrics::LatencyHistogram;

    fn record(pid: u32, class: TaskClass, samples: u64) -> IntervalRecord {
        let mut buckets = LatencyHistogram::new().snapshot();
        buckets[0].count = samples;
        IntervalRecord {
            process_pid: Some(pid),
            class,
            samples,
            max_ns: 1,
            histogram: buckets,
            ..Default::default()
        }
    }

    #[test]
    fn only_the_target_games_scheduler_population_is_counted() {
        let records = [
            record(10, TaskClass::Game, 600),
            record(10, TaskClass::GameWorkerThread, 700),
            record(10, TaskClass::Compiler, 5_000),
            record(11, TaskClass::Game, 9_000),
        ];
        let signals = runnable_signals_for_target(&records, 10, 77);
        assert_eq!(signals.runnable_samples, Some(1_300));
        assert_eq!(
            signals.signal_quality.runnable_latency,
            ObjectiveSignalQuality::Direct
        );
        assert_eq!(signals.runnable_target_root_pid, Some(10));
        assert_eq!(signals.runnable_target_starttime_ticks, Some(77));
        let other = runnable_signals_for_target(&records, 99, 1);
        assert_eq!(
            other.signal_quality.runnable_latency,
            ObjectiveSignalQuality::Missing
        );
        assert!(other.runnable_samples.is_none());
    }
}
