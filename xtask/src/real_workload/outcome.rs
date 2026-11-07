//! Paired statistics and the preregistered classification of a real-workload comparison.
use serde::{Deserialize, Serialize};

use super::manifest::{Direction, MIN_PAIRS, Manifest};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    PerformancePass,
    NoBenefit,
    Inconclusive,
    BlockedResource,
    SafetyAbort,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub n: usize,
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
}

/// One baseline/treatment pair: the primary metric and each guardrail metric, in the manifest's
/// guardrail order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PairResult {
    pub baseline_primary: f64,
    pub treatment_primary: f64,
    pub baseline_guardrails: Vec<f64>,
    pub treatment_guardrails: Vec<f64>,
}

/// What the run established independently of performance.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RunFacts {
    /// Data, physical action and recovery were all correct.
    pub engineering_correct: bool,
    pub safety_fault: Option<String>,
    pub blocked: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub engineering_pass: bool,
    pub outcome: Outcome,
    pub pairs: usize,
    /// Mean improvement of the primary metric in percent (positive is better) and its 95% bounds.
    pub effect_percent: f64,
    pub ci_low: f64,
    pub ci_high: f64,
    pub reason: String,
    /// Distribution of the primary metric per arm (mean, p50, p95, p99).
    pub baseline: Option<Summary>,
    pub treatment: Option<Summary>,
}

pub fn summarize(values: &[f64]) -> Option<Summary> {
    if values.is_empty() || values.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = |q: f64| {
        let index = ((q * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
        sorted[index - 1]
    };
    Some(Summary {
        n: sorted.len(),
        mean: sorted.iter().sum::<f64>() / sorted.len() as f64,
        p50: rank(0.50),
        p95: rank(0.95),
        p99: rank(0.99),
    })
}

/// Two-sided 95% Student-t critical values for 1..=30 degrees of freedom.
const T95: [f64; 30] = [
    12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.160,
    2.145, 2.131, 2.120, 2.110, 2.101, 2.093, 2.086, 2.080, 2.074, 2.069, 2.064, 2.060, 2.056,
    2.052, 2.048, 2.045, 2.042,
];

/// Mean and 95% interval of per-pair improvement percentages.
fn interval(improvements: &[f64]) -> Option<(f64, f64, f64)> {
    let n = improvements.len();
    if n < 2 {
        return None;
    }
    let mean = improvements.iter().sum::<f64>() / n as f64;
    let variance = improvements
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (n as f64 - 1.0);
    let t = T95[(n - 2).min(T95.len() - 1)];
    let half = t * (variance / n as f64).sqrt();
    Some((mean, mean - half, mean + half))
}

fn improvement(direction: Direction, baseline: f64, treatment: f64) -> Option<f64> {
    if !baseline.is_finite() || !treatment.is_finite() || baseline <= 0.0 {
        return None;
    }
    let change = (baseline - treatment) / baseline * 100.0;
    Some(match direction {
        Direction::LowerIsBetter => change,
        Direction::HigherIsBetter => -change,
    })
}

fn verdict(facts: &RunFacts, outcome: Outcome, reason: &str) -> Verdict {
    Verdict {
        engineering_pass: facts.engineering_correct,
        outcome,
        pairs: 0,
        effect_percent: 0.0,
        ci_low: 0.0,
        ci_high: 0.0,
        reason: reason.to_owned(),
        baseline: None,
        treatment: None,
    }
}

/// Classifies the pairs by the manifest's frozen thresholds. A safety fault or a blocked resource
/// comes first; too few pairs or a damaged engineering path is `Inconclusive`; a gain counts only
/// when its whole 95% interval clears the minimum effect and every guardrail's regression interval
/// stays within its limit; an interval wholly inside the equivalence margin is `NoBenefit`.
pub fn classify(manifest: &Manifest, facts: &RunFacts, pairs: &[PairResult]) -> Verdict {
    if let Some(fault) = &facts.safety_fault {
        return verdict(facts, Outcome::SafetyAbort, fault);
    }
    if let Some(blocked) = &facts.blocked {
        return verdict(facts, Outcome::BlockedResource, blocked);
    }
    if !facts.engineering_correct {
        return verdict(
            facts,
            Outcome::Inconclusive,
            "data, action or recovery was not correct",
        );
    }
    if pairs.len() < MIN_PAIRS as usize {
        return verdict(
            facts,
            Outcome::Inconclusive,
            "too few pairs to judge an effect",
        );
    }
    let primary: Option<Vec<f64>> = pairs
        .iter()
        .map(|pair| {
            improvement(
                manifest.primary.direction,
                pair.baseline_primary,
                pair.treatment_primary,
            )
        })
        .collect();
    let Some((effect, low, high)) = primary.as_deref().and_then(interval) else {
        return verdict(
            facts,
            Outcome::Inconclusive,
            "primary metric invalid in some pair",
        );
    };
    let mut result = Verdict {
        engineering_pass: true,
        outcome: Outcome::Inconclusive,
        pairs: pairs.len(),
        effect_percent: effect,
        ci_low: low,
        ci_high: high,
        reason: String::new(),
        baseline: summarize(
            &pairs
                .iter()
                .map(|pair| pair.baseline_primary)
                .collect::<Vec<_>>(),
        ),
        treatment: summarize(
            &pairs
                .iter()
                .map(|pair| pair.treatment_primary)
                .collect::<Vec<_>>(),
        ),
    };
    for (index, guardrail) in manifest.guardrails.iter().enumerate() {
        let regressions: Option<Vec<f64>> = pairs
            .iter()
            .map(|pair| {
                let baseline = *pair.baseline_guardrails.get(index)?;
                let treatment = *pair.treatment_guardrails.get(index)?;
                improvement(guardrail.metric.direction, baseline, treatment).map(|gain| -gain)
            })
            .collect();
        let Some((_, _, worst)) = regressions.as_deref().and_then(interval) else {
            result.reason = format!("guardrail {} invalid in some pair", guardrail.metric.name);
            return result;
        };
        if worst > guardrail.max_regression_percent {
            result.outcome = Outcome::NoBenefit;
            result.reason = format!(
                "guardrail {} may have regressed by {worst:.1}% (limit {:.1}%)",
                guardrail.metric.name, guardrail.max_regression_percent
            );
            return result;
        }
    }
    let margin = manifest.equivalence_margin_percent;
    if low >= manifest.min_effect_percent {
        result.outcome = Outcome::PerformancePass;
        result.reason =
            "the whole interval clears the minimum effect and all guardrails hold".into();
    } else if high < manifest.min_effect_percent {
        result.outcome = Outcome::NoBenefit;
        result.reason = if low > -margin && high < margin {
            "equivalent: the whole interval lies inside the equivalence margin".into()
        } else {
            "no meaningful gain: the upper bound is below the minimum effect".into()
        };
    } else {
        result.reason = "the interval spans the minimum effect: more pairs would be needed".into();
    }
    result
}
