use serde::{Deserialize, Serialize};

pub const DEFAULT_BOOTSTRAP_ITERATIONS: usize = 2_000;
pub const DEFAULT_CONFIDENCE_LEVEL: f64 = 0.95;
pub const MIN_FORMAL_SAMPLES_PER_SIDE: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapConfidenceInterval {
    pub confidence_level: f64,
    pub lower: f64,
    pub upper: f64,
    pub iterations: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormalMetricComparison {
    pub metric: String,
    pub unit: String,
    pub lower_is_better: bool,
    pub baseline_samples: usize,
    pub tuned_samples: usize,
    pub baseline_median: f64,
    pub tuned_median: f64,
    /// Positive values mean the tuned/profile-under-test side improved over the
    /// baseline/comparison side for lower-is-better metrics.
    pub improvement_delta: f64,
    pub effect_size: Option<f64>,
    pub bootstrap_ci95: Option<BootstrapConfidenceInterval>,
    pub enough_samples: bool,
    pub not_enough_samples_reason: Option<String>,
    pub statistically_significant: bool,
}

pub fn compare_lower_is_better_metric(
    metric: impl Into<String>,
    unit: impl Into<String>,
    baseline_values: &[f64],
    tuned_values: &[f64],
) -> FormalMetricComparison {
    let metric = metric.into();
    let unit = unit.into();
    let baseline_median = median_f64(baseline_values);
    let tuned_median = median_f64(tuned_values);
    let improvement_delta = baseline_median - tuned_median;
    let enough_samples = baseline_values.len() >= MIN_FORMAL_SAMPLES_PER_SIDE
        && tuned_values.len() >= MIN_FORMAL_SAMPLES_PER_SIDE;
    let not_enough_samples_reason = if enough_samples {
        None
    } else {
        Some(format!(
            "not enough samples for formal A/B comparison: baseline_runs={} tuned_runs={} required_each={}",
            baseline_values.len(),
            tuned_values.len(),
            MIN_FORMAL_SAMPLES_PER_SIDE
        ))
    };
    let pooled_stddev = pooled_sample_stddev(baseline_values, tuned_values);
    let effect_size = if pooled_stddev > f64::EPSILON {
        Some(improvement_delta / pooled_stddev)
    } else {
        None
    };
    let bootstrap_ci95 = enough_samples.then(|| {
        bootstrap_median_delta_ci(
            baseline_values,
            tuned_values,
            DEFAULT_BOOTSTRAP_ITERATIONS,
            DEFAULT_CONFIDENCE_LEVEL,
            metric_seed(&metric, baseline_values.len(), tuned_values.len()),
        )
    });
    let statistically_significant = bootstrap_ci95
        .as_ref()
        .is_some_and(|ci| ci.lower > 0.0 || ci.upper < 0.0);

    FormalMetricComparison {
        metric,
        unit,
        lower_is_better: true,
        baseline_samples: baseline_values.len(),
        tuned_samples: tuned_values.len(),
        baseline_median,
        tuned_median,
        improvement_delta,
        effect_size,
        bootstrap_ci95,
        enough_samples,
        not_enough_samples_reason,
        statistically_significant,
    }
}

pub fn median_f64(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut values = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

pub fn sample_stddev_f64(values: &[f64]) -> f64 {
    let values = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    if values.len() < 2 {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| {
            let diff = value - mean;
            diff * diff
        })
        .sum::<f64>()
        / (values.len() as f64 - 1.0);
    variance.sqrt()
}

pub fn pooled_sample_stddev(left: &[f64], right: &[f64]) -> f64 {
    if left.len() < 2 || right.len() < 2 {
        return 0.0;
    }
    let left_stddev = sample_stddev_f64(left);
    let right_stddev = sample_stddev_f64(right);
    let numerator = ((left.len() - 1) as f64 * left_stddev * left_stddev)
        + ((right.len() - 1) as f64 * right_stddev * right_stddev);
    let denominator = (left.len() + right.len() - 2) as f64;
    if denominator <= 0.0 {
        0.0
    } else {
        (numerator / denominator).sqrt()
    }
}

fn bootstrap_median_delta_ci(
    baseline_values: &[f64],
    tuned_values: &[f64],
    iterations: usize,
    confidence_level: f64,
    seed: u64,
) -> BootstrapConfidenceInterval {
    let mut rng = SplitMix64::new(seed);
    let mut deltas = Vec::with_capacity(iterations);

    for _ in 0..iterations {
        let baseline = resampled_median(baseline_values, &mut rng);
        let tuned = resampled_median(tuned_values, &mut rng);
        deltas.push(baseline - tuned);
    }

    deltas.sort_by(f64::total_cmp);
    let alpha = ((1.0 - confidence_level) / 2.0).clamp(0.0, 0.50);
    let lower_idx = ((alpha * iterations as f64).floor() as usize).min(iterations - 1);
    let upper_idx = (((1.0 - alpha) * iterations as f64).ceil() as usize)
        .saturating_sub(1)
        .min(iterations - 1);

    BootstrapConfidenceInterval {
        confidence_level,
        lower: deltas[lower_idx],
        upper: deltas[upper_idx],
        iterations,
    }
}

fn resampled_median(values: &[f64], rng: &mut SplitMix64) -> f64 {
    let mut sample = Vec::with_capacity(values.len());
    for _ in 0..values.len() {
        let idx = rng.next_index(values.len());
        sample.push(values[idx]);
    }
    median_f64(&sample)
}

fn metric_seed(metric: &str, baseline_len: usize, tuned_len: usize) -> u64 {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64 ^ ((baseline_len as u64) << 32) ^ tuned_len as u64;
    for byte in metric.as_bytes() {
        seed = seed.rotate_left(5) ^ u64::from(*byte);
        seed = seed.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    }
    seed
}

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_index(&mut self, len: usize) -> usize {
        if len <= 1 {
            return 0;
        }
        (self.next_u64() as usize) % len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_reports_not_enough_samples() {
        let comparison = compare_lower_is_better_metric("score", "points", &[100.0], &[80.0]);

        assert!(!comparison.enough_samples);
        assert!(comparison.bootstrap_ci95.is_none());
        assert!(comparison.not_enough_samples_reason.is_some());
    }

    #[test]
    fn bootstrap_ci_excludes_zero_for_clear_improvement() {
        let comparison = compare_lower_is_better_metric(
            "score",
            "points",
            &[110.0, 120.0, 130.0, 140.0, 150.0],
            &[70.0, 80.0, 90.0, 100.0, 110.0],
        );

        assert!(comparison.enough_samples);
        assert!(comparison.effect_size.is_some());
        assert!(comparison.statistically_significant);
        assert!(comparison.bootstrap_ci95.unwrap().lower > 0.0);
    }
}
