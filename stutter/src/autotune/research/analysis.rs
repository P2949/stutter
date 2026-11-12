//! Fixed-size, whole-run paired analysis using a caller-supplied frozen study design.
//!
//! This numeric calculation does not attest the runs or their context. The caller must validate
//! source, scenario and block provenance before invoking it and must preserve the frozen rule.
use serde::{Deserialize, Serialize};

use crate::autotune::experiment_design::{
    EXPERIMENT_SCHEMA_VERSION, ExperimentDesign, StoppingRule,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NativeMetric {
    AverageFps,
    P99FrameMs,
}

impl NativeMetric {
    fn name_and_unit(self) -> (&'static str, &'static str) {
        match self {
            Self::AverageFps => ("native_average_fps", "fps"),
            Self::P99FrameMs => ("native_p99_frame_ms", "ms"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rule {
    /// Source- and scenario-bound design, including ordered block identities and fixed N.
    pub(crate) design: ExperimentDesign,
    /// Family-wise error rate and number of comparisons frozen before any observations.
    pub(crate) family_alpha: f64,
    pub(crate) comparisons: u32,
    pub(crate) metric: NativeMetric,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Improved,
    Regressed,
    Equivalent,
    NoUsefulEffect,
    Inconclusive,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    InvalidRule,
    ContextDrift,
    TooFewPairs,
    TooManyPairs,
    NonFinite,
    NonPositiveBaseline,
    NonPositiveCandidate,
    UnequalPairing,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Outcome {
    /// SHA-256 of the complete serialized rule, including design and multiplicity budget.
    pub(crate) rule_sha256: String,
    pub(crate) pairs: usize,
    /// Mean relative effect, sign-adjusted so positive always means better.
    pub(crate) mean_effect: f64,
    pub(crate) ci_low: f64,
    pub(crate) ci_high: f64,
    pub(crate) verdict: Verdict,
}

fn digest_ok(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Compare index-aligned native runs. `context_drifted` must come from independent provenance
/// checks; false does not itself establish source quality or authorize a mutation.
pub(crate) fn analyse_pairs(
    rule: &Rule,
    control: &[f64],
    candidate: &[f64],
    context_drifted: bool,
) -> Result<Outcome, Refused> {
    let design = &rule.design;
    let (blocks, confidence) = match design.stopping {
        StoppingRule::FixedN { blocks, confidence } => (blocks, confidence),
        _ => return Err(Refused::InvalidRule),
    };
    let corrected_alpha = rule.family_alpha / f64::from(rule.comparisons);
    let (metric, unit) = rule.metric.name_and_unit();
    if design.schema_version != EXPERIMENT_SCHEMA_VERSION
        || design.validate().is_err()
        || !(rule.family_alpha > 0.0 && rule.family_alpha < 1.0)
        || rule.comparisons == 0
        // Four or more pairs and this floor keep the shared t-quantile search bracket adequate.
        || !(1e-6..1.0).contains(&corrected_alpha)
        || confidence.to_bits() != (1.0 - corrected_alpha).to_bits()
        || !(4..=1000).contains(&blocks)
        || design.block_ids.len() != blocks as usize
        || design.block_ids.iter().any(|id| id.is_empty() || id.len() > 128 || id.chars().any(char::is_control))
        || design.objective != "proton_native_benchmark"
        || design.metric != metric || design.unit != unit
        || design.candidate.is_empty() || design.candidate.len() > 128 || design.candidate.chars().any(char::is_control)
        || !digest_ok(&design.context_fingerprint)
    {
        return Err(Refused::InvalidRule);
    }
    if context_drifted {
        return Err(Refused::ContextDrift);
    }
    if control.len() != candidate.len() {
        return Err(Refused::UnequalPairing);
    }
    if control.len() < blocks as usize {
        return Err(Refused::TooFewPairs);
    }
    if control.len() > blocks as usize {
        return Err(Refused::TooManyPairs);
    }
    if control.iter().chain(candidate).any(|v| !v.is_finite()) {
        return Err(Refused::NonFinite);
    }
    if control.iter().any(|v| *v <= 0.0) {
        return Err(Refused::NonPositiveBaseline);
    }
    if candidate.iter().any(|v| *v <= 0.0) {
        return Err(Refused::NonPositiveCandidate);
    }
    let sign = match rule.metric {
        NativeMetric::AverageFps => 1.0,
        NativeMetric::P99FrameMs => -1.0,
    };
    let effects: Vec<f64> = control
        .iter()
        .zip(candidate)
        .map(|(c, k)| 100.0 * sign * ((k - c) / c))
        .collect();
    if effects.iter().any(|value| !value.is_finite()) {
        return Err(Refused::NonFinite);
    }
    let analysis = design
        .analyse(&effects, 1, context_drifted)
        .map_err(|_| Refused::NonFinite)?;
    let (low, mean, high) = analysis.interval;
    if [low, mean, high].iter().any(|value| !value.is_finite()) {
        return Err(Refused::NonFinite);
    }
    let useful = design.min_effect_percent;
    let margin = design.equivalence_margin_percent;
    let verdict = if low >= useful {
        Verdict::Improved
    } else if high <= -useful {
        Verdict::Regressed
    } else if low >= -margin && high <= margin {
        Verdict::Equivalent
    } else if high < useful && low > -useful {
        Verdict::NoUsefulEffect
    } else {
        Verdict::Inconclusive
    };
    let encoded = serde_json::to_vec(rule).map_err(|_| Refused::InvalidRule)?;
    Ok(Outcome {
        rule_sha256: stutter_core::hash::sha256_hex(&encoded),
        pairs: effects.len(),
        mean_effect: mean / 100.0,
        ci_low: low / 100.0,
        ci_high: high / 100.0,
        verdict,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rule(pairs: u32) -> Rule {
        Rule {
            design: ExperimentDesign {
                schema_version: EXPERIMENT_SCHEMA_VERSION,
                objective: "proton_native_benchmark".into(),
                metric: "native_average_fps".into(),
                unit: "fps".into(),
                candidate: "test_candidate".into(),
                context_fingerprint: "a".repeat(64),
                min_effect_percent: 5.0,
                equivalence_margin_percent: 3.0,
                seed: 5,
                block_ids: (0..pairs).map(|index| format!("block-{index}")).collect(),
                stopping: StoppingRule::FixedN {
                    blocks: pairs,
                    confidence: 0.95,
                },
                warmup_seconds: 10,
                washout_seconds: 2,
            },
            family_alpha: 0.05,
            comparisons: 1,
            metric: NativeMetric::AverageFps,
        }
    }

    #[test]
    fn clear_gain_regression_and_equivalence_are_bound_to_the_rule() {
        let control = [100.0, 101.0, 99.0, 100.0, 100.5, 99.5];
        let better: Vec<_> = control.iter().map(|v| v * 1.15).collect();
        let worse: Vec<_> = control.iter().map(|v| v * 0.85).collect();
        let first = analyse_pairs(&rule(6), &control, &better, false).unwrap();
        assert_eq!(first.verdict, Verdict::Improved);
        assert_eq!(
            analyse_pairs(&rule(6), &control, &worse, false)
                .unwrap()
                .verdict,
            Verdict::Regressed
        );
        assert_eq!(
            analyse_pairs(&rule(6), &control, &control, false)
                .unwrap()
                .verdict,
            Verdict::Equivalent
        );
        let mut changed = rule(6);
        changed.design.context_fingerprint = "b".repeat(64);
        assert_ne!(
            first.rule_sha256,
            analyse_pairs(&changed, &control, &better, false)
                .unwrap()
                .rule_sha256
        );
    }

    #[test]
    fn refuses_early_extra_invalid_and_drifted_data() {
        assert_eq!(
            analyse_pairs(&rule(4), &[1.0; 4], &[1.0; 3], false),
            Err(Refused::UnequalPairing)
        );
        assert_eq!(
            analyse_pairs(&rule(5), &[100.0; 4], &[110.0; 4], false),
            Err(Refused::TooFewPairs)
        );
        assert_eq!(
            analyse_pairs(&rule(4), &[100.0; 5], &[110.0; 5], false),
            Err(Refused::TooManyPairs)
        );
        assert_eq!(
            analyse_pairs(&rule(4), &[10.0; 4], &[-1.0; 4], false),
            Err(Refused::NonPositiveCandidate)
        );
        assert_eq!(
            analyse_pairs(&rule(4), &[0.0; 4], &[1.0; 4], false),
            Err(Refused::NonPositiveBaseline)
        );
        assert_eq!(
            analyse_pairs(&rule(4), &[1.0; 4], &[f64::NAN; 4], false),
            Err(Refused::NonFinite)
        );
        assert_eq!(
            analyse_pairs(&rule(4), &[f64::MIN_POSITIVE; 4], &[f64::MAX; 4], false),
            Err(Refused::NonFinite)
        );
        assert_eq!(
            analyse_pairs(&rule(4), &[100.0; 4], &[110.0; 4], true),
            Err(Refused::ContextDrift)
        );
    }

    #[test]
    fn noisy_effect_is_inconclusive() {
        assert_eq!(
            analyse_pairs(&rule(4), &[100.0; 4], &[112.0, 90.0, 108.0, 96.0], false)
                .unwrap()
                .verdict,
            Verdict::Inconclusive
        );
    }

    #[test]
    fn multiplicity_and_metric_direction_are_frozen() {
        let control = [100.0; 5];
        let candidate = [106.0, 107.0, 105.0, 108.0, 106.5];
        let one = analyse_pairs(&rule(5), &control, &candidate, false).unwrap();
        let mut family = rule(5);
        family.comparisons = 15;
        family.design.stopping = StoppingRule::FixedN {
            blocks: 5,
            confidence: 1.0 - family.family_alpha / 15.0,
        };
        let many = analyse_pairs(&family, &control, &candidate, false).unwrap();
        assert!(many.ci_high - many.ci_low > one.ci_high - one.ci_low);
        let mut frame_time = rule(4);
        frame_time.metric = NativeMetric::P99FrameMs;
        frame_time.design.metric = "native_p99_frame_ms".into();
        frame_time.design.unit = "ms".into();
        assert_eq!(
            analyse_pairs(&frame_time, &[10.0; 4], &[8.0; 4], false)
                .unwrap()
                .verdict,
            Verdict::Improved
        );
    }

    #[test]
    fn rejects_wrong_units_context_and_unbudgeted_confidence() {
        let mut bad = rule(4);
        bad.design.unit = "ms".into();
        assert_eq!(
            analyse_pairs(&bad, &[1.0; 4], &[1.0; 4], false),
            Err(Refused::InvalidRule)
        );
        bad = rule(4);
        bad.design.context_fingerprint = "not-a-digest".into();
        assert_eq!(
            analyse_pairs(&bad, &[1.0; 4], &[1.0; 4], false),
            Err(Refused::InvalidRule)
        );
        bad = rule(4);
        bad.comparisons = 15;
        assert_eq!(
            analyse_pairs(&bad, &[1.0; 4], &[1.0; 4], false),
            Err(Refused::InvalidRule)
        );
        bad = rule(4);
        bad.comparisons = 1_000_000;
        assert_eq!(
            analyse_pairs(&bad, &[1.0; 4], &[1.0; 4], false),
            Err(Refused::InvalidRule)
        );
    }
}
