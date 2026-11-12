//! Fixed-size, run-level paired analysis for a frozen Proton comparison.
//!
//! Whole runs are the independent observations. The existing experiment design and statistics
//! enforce the single fixed-N look and compute the multiplicity-adjusted confidence interval.
use crate::autotune::experiment_design::{
    EXPERIMENT_SCHEMA_VERSION, ExperimentDesign, StoppingRule,
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rule {
    /// Smallest relative effect that matters (for example 0.05).
    pub(crate) useful_effect: f64,
    /// Relative half-width inside which the arms are called equivalent.
    pub(crate) equivalence_margin: f64,
    /// Family-wise error rate before multiplicity correction.
    pub(crate) alpha: f64,
    /// Number of simultaneous comparisons in the family (Bonferroni).
    pub(crate) comparisons: usize,
    /// Number of independent pairs frozen before collecting runs; no early or extra look.
    pub(crate) planned_pairs: usize,
    /// True when larger metric values are better (FPS); false for frame time.
    pub(crate) higher_is_better: bool,
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
    TooFewPairs,
    TooManyPairs,
    NonFinite,
    NonPositiveBaseline,
    NonPositiveCandidate,
    UnequalPairing,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Outcome {
    pub(crate) pairs: usize,
    /// Mean relative effect, sign-adjusted so positive always means better.
    pub(crate) mean_effect: f64,
    pub(crate) ci_low: f64,
    pub(crate) ci_high: f64,
    pub(crate) verdict: Verdict,
}

/// Compare a predeclared number of index-aligned, native per-run values.
pub(crate) fn analyse_pairs(
    rule: &Rule,
    control: &[f64],
    candidate: &[f64],
) -> Result<Outcome, Refused> {
    let adjusted_alpha = rule.alpha / rule.comparisons as f64;
    if !(rule.alpha > 0.0 && rule.alpha < 1.0)
        || rule.comparisons == 0
        // The shared quantile routine has a finite search bracket; with at least four pairs,
        // this conservative floor keeps the requested critical value inside that bracket.
        || !(1e-6..1.0).contains(&adjusted_alpha)
        || !(rule.useful_effect > rule.equivalence_margin && rule.useful_effect.is_finite())
        || !(rule.equivalence_margin > 0.0 && rule.equivalence_margin.is_finite())
        || !(4..=1000).contains(&rule.planned_pairs)
    {
        return Err(Refused::InvalidRule);
    }
    if control.len() != candidate.len() {
        return Err(Refused::UnequalPairing);
    }
    if control.len() < rule.planned_pairs {
        return Err(Refused::TooFewPairs);
    }
    if control.len() > rule.planned_pairs {
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
    let sign = if rule.higher_is_better { 1.0 } else { -1.0 };
    let effects: Vec<f64> = control
        .iter()
        .zip(candidate)
        .map(|(c, k)| 100.0 * sign * ((k - c) / c))
        .collect();
    if effects.iter().any(|value| !value.is_finite()) {
        return Err(Refused::NonFinite);
    }
    let design = ExperimentDesign {
        schema_version: EXPERIMENT_SCHEMA_VERSION,
        objective: "native_run_comparison".into(),
        metric: "paired_relative_effect".into(),
        unit: "percent".into(),
        candidate: "frozen_candidate".into(),
        context_fingerprint: "numeric_adapter_only".into(),
        min_effect_percent: rule.useful_effect * 100.0,
        equivalence_margin_percent: rule.equivalence_margin * 100.0,
        seed: 0,
        block_ids: (0..rule.planned_pairs)
            .map(|index| index.to_string())
            .collect(),
        stopping: StoppingRule::FixedN {
            blocks: rule.planned_pairs as u32,
            confidence: 1.0 - adjusted_alpha,
        },
        warmup_seconds: 0,
        washout_seconds: 0,
    };
    design.validate().map_err(|_| Refused::InvalidRule)?;
    let analysis = design
        .analyse(&effects, 1, false)
        .map_err(|_| Refused::NonFinite)?;
    let (low, mean, high) = analysis.interval;
    if [low, mean, high].iter().any(|value| !value.is_finite()) {
        return Err(Refused::NonFinite);
    }
    let (low, mean, high) = (low / 100.0, mean / 100.0, high / 100.0);
    let verdict = if low >= rule.useful_effect {
        Verdict::Improved
    } else if high <= -rule.useful_effect {
        Verdict::Regressed
    } else if low >= -rule.equivalence_margin && high <= rule.equivalence_margin {
        Verdict::Equivalent
    } else if high < rule.useful_effect && low > -rule.useful_effect {
        Verdict::NoUsefulEffect
    } else {
        Verdict::Inconclusive
    };
    Ok(Outcome {
        pairs: effects.len(),
        mean_effect: mean,
        ci_low: low,
        ci_high: high,
        verdict,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rule(planned_pairs: usize) -> Rule {
        Rule {
            useful_effect: 0.05,
            equivalence_margin: 0.03,
            alpha: 0.05,
            comparisons: 1,
            planned_pairs,
            higher_is_better: true,
        }
    }

    #[test]
    fn clear_gain_regression_and_equivalence() {
        let control = [100.0, 101.0, 99.0, 100.0, 100.5, 99.5];
        let better: Vec<_> = control.iter().map(|v| v * 1.15).collect();
        let worse: Vec<_> = control.iter().map(|v| v * 0.85).collect();
        assert_eq!(
            analyse_pairs(&rule(6), &control, &better).unwrap().verdict,
            Verdict::Improved
        );
        assert_eq!(
            analyse_pairs(&rule(6), &control, &worse).unwrap().verdict,
            Verdict::Regressed
        );
        assert_eq!(
            analyse_pairs(&rule(6), &control, &control).unwrap().verdict,
            Verdict::Equivalent
        );
    }

    #[test]
    fn fixed_n_refuses_early_and_extra_looks_and_invalid_measurements() {
        assert_eq!(
            analyse_pairs(&rule(4), &[1.0; 4], &[1.0; 3]),
            Err(Refused::UnequalPairing)
        );
        assert_eq!(
            analyse_pairs(&rule(5), &[100.0; 4], &[110.0; 4]),
            Err(Refused::TooFewPairs)
        );
        assert_eq!(
            analyse_pairs(&rule(4), &[100.0; 5], &[110.0; 5]),
            Err(Refused::TooManyPairs)
        );
        assert_eq!(
            analyse_pairs(&rule(4), &[10.0; 4], &[-1.0; 4]),
            Err(Refused::NonPositiveCandidate)
        );
        assert_eq!(
            analyse_pairs(&rule(4), &[0.0; 4], &[1.0; 4]),
            Err(Refused::NonPositiveBaseline)
        );
        assert_eq!(
            analyse_pairs(&rule(4), &[1.0; 4], &[f64::NAN; 4]),
            Err(Refused::NonFinite)
        );
        assert_eq!(
            analyse_pairs(&rule(4), &[f64::MIN_POSITIVE; 4], &[f64::MAX; 4]),
            Err(Refused::NonFinite)
        );
    }

    #[test]
    fn noisy_effect_is_inconclusive() {
        assert_eq!(
            analyse_pairs(&rule(4), &[100.0; 4], &[112.0, 90.0, 108.0, 96.0])
                .unwrap()
                .verdict,
            Verdict::Inconclusive
        );
    }

    #[test]
    fn multiplicity_widens_interval_and_lower_is_better_uses_the_right_sign() {
        let control = [100.0; 5];
        let candidate = [106.0, 107.0, 105.0, 108.0, 106.5];
        let one = analyse_pairs(&rule(5), &control, &candidate).unwrap();
        let mut family = rule(5);
        family.comparisons = 15;
        let many = analyse_pairs(&family, &control, &candidate).unwrap();
        assert!(many.ci_high - many.ci_low > one.ci_high - one.ci_low);
        let mut frame_time = rule(4);
        frame_time.higher_is_better = false;
        assert_eq!(
            analyse_pairs(&frame_time, &[10.0; 4], &[8.0; 4])
                .unwrap()
                .verdict,
            Verdict::Improved
        );
    }

    #[test]
    fn invalid_rules_and_unrepresentable_confidence_are_refused() {
        let mut bad = rule(4);
        bad.alpha = 2.0;
        assert_eq!(
            analyse_pairs(&bad, &[1.0; 4], &[1.0; 4]),
            Err(Refused::InvalidRule)
        );
        bad.alpha = 0.05;
        bad.comparisons = 1_000_000;
        assert_eq!(
            analyse_pairs(&bad, &[1.0; 4], &[1.0; 4]),
            Err(Refused::InvalidRule)
        );
    }
}
