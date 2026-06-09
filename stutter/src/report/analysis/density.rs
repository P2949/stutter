//! Spike density and numeric percentile helpers.
//!
//! Owns spike density bucket construction and shared millisecond/percentile conversions. Does not
//! own correlation text, runtime summaries, frame pacing, or report orchestration.

use std::collections::BTreeMap;

use super::*;

pub(crate) fn ms_to_ns_i64(value: f64) -> i64 {
    (value * 1_000_000.0).ceil() as i64
}

pub(crate) fn ns_to_ms(ns: u64) -> f64 {
    ns as f64 / 1_000_000.0
}

pub fn build_spike_density(spikes: &[SpikeEvent], bucket_ms: u64) -> Vec<SpikeDensityBucket> {
    if spikes.is_empty() {
        return Vec::new();
    }

    let bucket_ms = bucket_ms.max(1);

    #[derive(Default)]
    struct BucketAccum {
        start_ms: u64,
        end_ms: u64,
        count: u64,
        max_latency_ms: f64,
        latencies_ms: Vec<f64>,
    }

    let mut buckets: BTreeMap<u64, BucketAccum> = BTreeMap::new();

    for spike in spikes {
        let elapsed_ms = spike.elapsed_ms.unwrap_or(0);
        let latency_ms = spike.latency_ns as f64 / 1_000_000.0;

        let bucket_idx = elapsed_ms / bucket_ms;
        let start_ms = bucket_idx * bucket_ms;
        let end_ms = start_ms + bucket_ms;

        let bucket = buckets.entry(bucket_idx).or_insert_with(|| BucketAccum {
            start_ms,
            end_ms,
            count: 0,
            max_latency_ms: 0.0,
            latencies_ms: Vec::new(),
        });

        bucket.count += 1;
        if latency_ms.is_finite() {
            bucket.max_latency_ms = bucket.max_latency_ms.max(latency_ms);
            bucket.latencies_ms.push(latency_ms);
        }
    }

    buckets
        .into_values()
        .map(|mut bucket| {
            let p99_latency_ms = percentile_f64(&mut bucket.latencies_ms, 0.99);
            SpikeDensityBucket {
                start_ms: bucket.start_ms,
                end_ms: bucket.end_ms,
                count: bucket.count,
                max_latency_ms: bucket.max_latency_ms,
                p99_latency_ms,
            }
        })
        .collect()
}

pub(crate) fn percentile_f64(values: &mut [f64], percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    values.sort_by(|a, b| a.total_cmp(b));

    let len = values.len();
    let rank = ((len as f64 - 1.0) * percentile).round() as usize;
    values[rank.min(len - 1)]
}

pub(crate) fn median_f64(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    values.sort_by(|a, b| a.total_cmp(b));
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}
