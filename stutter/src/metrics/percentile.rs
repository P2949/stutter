use serde::{Deserialize, Serialize};

pub const MAX_EXACT_SAMPLES: usize = 1_024;
pub const LATENCY_HISTOGRAM_BUCKETS_NS: [u64; 15] = [
    1_000, 2_000, 5_000, 10_000, 20_000, 50_000, 100_000, 200_000, 500_000, 1_000_000, 2_000_000,
    5_000_000, 10_000_000, 20_000_000, 50_000_000,
];
pub const LATENCY_HISTOGRAM_BUCKET_COUNT: usize = LATENCY_HISTOGRAM_BUCKETS_NS.len() + 1;

#[derive(Clone, Debug, Default)]
pub struct LatencyStats {
    pub count: u64,
    pub min_ns: u64,
    pub max_ns: u64,
    pub sum_ns: u128,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
    pub samples_ns: Vec<u64>,
    pub samples_truncated: u64,
    pub histogram: LatencyHistogram,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LatencyHistogramBucket {
    pub upper_bound_ns: Option<u64>,
    pub count: u64,
}

#[derive(Clone, Debug)]
pub struct LatencyHistogram {
    buckets: [u64; LATENCY_HISTOGRAM_BUCKET_COUNT],
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct LatencySnapshot {
    pub count: u64,
    pub stored_samples: u64,
    pub min_ns: u64,
    pub avg_ns: u64,
    pub max_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
    pub samples_truncated: u64,
    pub percentile_scope: String,
    pub histogram: Vec<LatencyHistogramBucket>,
}

impl LatencyHistogram {
    pub fn new() -> Self {
        Self {
            buckets: [0; LATENCY_HISTOGRAM_BUCKET_COUNT],
        }
    }

    pub fn record(&mut self, latency_ns: u64) {
        let bucket_idx = LATENCY_HISTOGRAM_BUCKETS_NS
            .iter()
            .position(|upper_bound_ns| latency_ns <= *upper_bound_ns)
            .unwrap_or(LATENCY_HISTOGRAM_BUCKET_COUNT - 1);

        self.buckets[bucket_idx] += 1;
    }

    pub fn percentile_upper_bound(&self, total_count: u64, percentile: f64) -> Option<u64> {
        if total_count == 0 {
            return Some(0);
        }

        let rank = ((total_count as f64 * percentile).ceil() as u64).max(1);
        let mut cumulative = 0;

        for (idx, count) in self.buckets.iter().copied().enumerate() {
            cumulative += count;
            if cumulative >= rank {
                return LATENCY_HISTOGRAM_BUCKETS_NS.get(idx).copied();
            }
        }

        None
    }

    pub fn snapshot(&self) -> Vec<LatencyHistogramBucket> {
        let mut buckets = Vec::with_capacity(LATENCY_HISTOGRAM_BUCKET_COUNT);

        for (idx, count) in self.buckets.iter().copied().enumerate() {
            buckets.push(LatencyHistogramBucket {
                upper_bound_ns: LATENCY_HISTOGRAM_BUCKETS_NS.get(idx).copied(),
                count,
            });
        }

        buckets
    }
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}

impl LatencyStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, latency_ns: u64) {
        self.histogram.record(latency_ns);

        if self.count == 0 {
            self.min_ns = latency_ns;
            self.max_ns = latency_ns;
        } else {
            self.min_ns = self.min_ns.min(latency_ns);
            self.max_ns = self.max_ns.max(latency_ns);
        }

        self.count += 1;
        self.sum_ns += latency_ns as u128;

        if latency_ns >= 1_000_000 {
            self.over_1ms += 1;
        }
        if latency_ns >= 2_000_000 {
            self.over_2ms += 1;
        }
        if latency_ns >= 5_000_000 {
            self.over_5ms += 1;
        }

        if self.samples_ns.len() < MAX_EXACT_SAMPLES {
            self.samples_ns.push(latency_ns);
        } else {
            self.samples_truncated += 1;
        }
    }

    pub fn snapshot(&mut self) -> Option<LatencySnapshot> {
        if self.count == 0 {
            return None;
        }

        self.samples_ns.sort_unstable();
        let percentile_scope = if self.samples_truncated > 0 {
            "histogram"
        } else {
            "exact"
        };

        let (p95_ns, p99_ns) = if self.samples_truncated > 0 {
            (
                self.histogram
                    .percentile_upper_bound(self.count, 0.95)
                    // The final histogram bucket is an overflow bucket with no finite
                    // upper bound. If a percentile lands there, use the exact observed
                    // max as the conservative fallback.
                    .unwrap_or(self.max_ns),
                self.histogram
                    .percentile_upper_bound(self.count, 0.99)
                    // The final histogram bucket is an overflow bucket with no finite
                    // upper bound. If a percentile lands there, use the exact observed
                    // max as the conservative fallback.
                    .unwrap_or(self.max_ns),
            )
        } else {
            (
                percentile_from_sorted(&self.samples_ns, 0.95),
                percentile_from_sorted(&self.samples_ns, 0.99),
            )
        };

        Some(LatencySnapshot {
            count: self.count,
            stored_samples: self.stored_samples(),
            min_ns: self.min_ns,
            avg_ns: (self.sum_ns / self.count as u128) as u64,
            max_ns: self.max_ns,
            p95_ns,
            p99_ns,
            over_1ms: self.over_1ms,
            over_2ms: self.over_2ms,
            over_5ms: self.over_5ms,
            samples_truncated: self.samples_truncated,
            percentile_scope: percentile_scope.to_owned(),
            histogram: self.histogram.snapshot(),
        })
    }

    pub fn snapshot_and_reset(&mut self) -> Option<LatencySnapshot> {
        let snapshot = self.snapshot()?;
        *self = Self::new();
        Some(snapshot)
    }

    pub fn stored_samples(&self) -> u64 {
        self.count.min(MAX_EXACT_SAMPLES as u64)
    }
}

fn percentile_from_sorted(samples: &[u64], percentile: f64) -> u64 {
    if samples.is_empty() {
        return 0;
    }

    let rank = ((samples.len() as f64 * percentile).ceil() as usize).saturating_sub(1);
    let idx = rank.min(samples.len() - 1);

    samples[idx]
}
