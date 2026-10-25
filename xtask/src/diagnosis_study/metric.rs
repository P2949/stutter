//! Direct runnable-latency metric of the study's target. The production objective signal only
//! counts game-class tasks; the study's targets (a `vkmark` window, an `fio` job) are not
//! classified as games, so the target's own population is merged here instead: every interval
//! record of the target process, buckets summed, the conservative bucket bound reported.
use stutter::api::recorder::IntervalRecord;

/// 99th-percentile runnable latency (ns) over all records of `pid`, from merged histograms.
/// `None` when the target has no runnable samples or its histograms disagree on buckets.
pub fn merged_runnable_p99_ns(records: &[IntervalRecord], pid: u32) -> Option<f64> {
    let mut buckets: Vec<(Option<u64>, u64)> = Vec::new();
    let mut max_ns = 0_u64;
    for record in records
        .iter()
        .filter(|r| r.process_pid == Some(pid) && !r.histogram.is_empty())
    {
        if buckets.is_empty() {
            buckets = record
                .histogram
                .iter()
                .map(|b| (b.upper_bound_ns, 0))
                .collect();
        }
        if buckets.len() != record.histogram.len()
            || buckets
                .iter()
                .zip(&record.histogram)
                .any(|(a, b)| a.0 != b.upper_bound_ns)
        {
            return None;
        }
        for (merged, incoming) in buckets.iter_mut().zip(&record.histogram) {
            merged.1 = merged.1.checked_add(incoming.count)?;
        }
        max_ns = max_ns.max(record.max_ns);
    }
    let samples: u64 = buckets.iter().map(|(_, count)| count).sum();
    if samples == 0 {
        return None;
    }
    let rank = (u128::from(samples) * 990).div_ceil(1_000) as u64;
    let mut cumulative = 0_u64;
    for (upper, count) in &buckets {
        cumulative += count;
        if cumulative >= rank {
            return Some(upper.unwrap_or(max_ns).min(max_ns.max(1)) as f64);
        }
    }
    Some(max_ns as f64)
}

/// Share of the target's runnable samples above `threshold_ns`, from the merged histograms
/// (conservative: a bucket counts only if its lower bound is at or above the threshold).
pub fn merged_runnable_over_rate(
    records: &[IntervalRecord],
    pid: u32,
    threshold_ns: u64,
) -> Option<f64> {
    let (mut over, mut total) = (0_u64, 0_u64);
    for record in records
        .iter()
        .filter(|r| r.process_pid == Some(pid) && !r.histogram.is_empty())
    {
        let mut lower = 0_u64;
        for bucket in &record.histogram {
            total = total.checked_add(bucket.count)?;
            if lower >= threshold_ns {
                over = over.checked_add(bucket.count)?;
            }
            lower = bucket.upper_bound_ns.unwrap_or(u64::MAX);
        }
    }
    (total > 0).then(|| over as f64 / total as f64)
}

#[cfg(test)]
mod tests {
    use stutter::api::recorder::{IntervalRecord, LatencyHistogram};

    use super::*;

    fn record(pid: u32, counts: &[(usize, u64)], max_ns: u64) -> IntervalRecord {
        let mut histogram = LatencyHistogram::new().snapshot();
        let mut samples = 0;
        for (index, count) in counts {
            histogram[*index].count = *count;
            samples += count;
        }
        IntervalRecord {
            process_pid: Some(pid),
            samples,
            max_ns,
            histogram,
            ..Default::default()
        }
    }

    #[test]
    fn the_target_population_is_merged_across_classes_and_other_processes_are_ignored() {
        let bound = LatencyHistogram::new().snapshot();
        let low = bound[0].upper_bound_ns.unwrap();
        let high = bound[10].upper_bound_ns.unwrap();
        let records = [
            record(7, &[(0, 980)], low),
            record(7, &[(10, 20)], high),
            record(8, &[(10, 5_000)], high),
        ];
        // 20 of 1000 samples sit in the high bucket, so p99 falls in it.
        assert_eq!(merged_runnable_p99_ns(&records, 7), Some(high as f64));
        let quiet = [record(7, &[(0, 1_000)], low)];
        assert_eq!(merged_runnable_p99_ns(&quiet, 7), Some(low as f64));
        assert_eq!(merged_runnable_p99_ns(&records, 99), None);
    }
}
