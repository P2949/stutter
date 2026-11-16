//! Pilot-informed confirmation rule (PLAN section 6.3). The effect size a confirmation must be able
//! to resolve and the margin inside which two arms count as equivalent are set from the observed
//! run-to-run variability of a pilot, then frozen with a digest before any confirmation run starts,
//! so thresholds are never tuned after seeing the confirmation data. The rule is expressed in
//! integer basis points (1/100 of a percent): it is embedded in a confirmation study's frozen
//! intent, which must stay exactly comparable, and a float would not be. The constants below are
//! provisional policy (named, tested and digest-bound), not a measured result.

use serde::{Deserialize, Serialize};

/// Fewer pilot runs per arm cannot estimate a spread.
pub const MIN_PILOT_RUNS: usize = 3;
/// A confirmation must resolve at least this many times the pilot's coefficient of variation.
const EFFECT_OVER_CV: u32 = 2;
/// But never claim sensitivity finer than this (1.00 %), whatever the pilot says.
const EFFECT_FLOOR_BP: u32 = 100;
/// A pilot noisier than this (10.00 %) cannot support a confirmation at the planned size.
const MAX_CV_BP: u32 = 1_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenConfirmationRule {
    pub pilot_runs: u32,
    /// The pilot's coefficient of variation, rounded up, in basis points.
    pub pilot_cv_bp: u32,
    pub min_effect_bp: u32,
    pub equivalence_margin_bp: u32,
    /// SHA-256 of the rule's other fields; the confirmation study binds it.
    pub rule_sha256: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PilotRefusal {
    TooFewRuns,
    InvalidMeasurement,
    TooNoisy,
}

impl FrozenConfirmationRule {
    fn digest(&self) -> String {
        stutter_core::hash::sha256_hex(
            format!(
                "pilot-rule-v1:{}:{}:{}:{}",
                self.pilot_runs, self.pilot_cv_bp, self.min_effect_bp, self.equivalence_margin_bp
            )
            .as_bytes(),
        )
    }

    /// The rule is internally consistent: its digest matches and its thresholds follow from its
    /// pilot spread by the frozen policy, so a hand-edited threshold is detected.
    pub fn is_consistent(&self) -> bool {
        let min_effect = self
            .pilot_cv_bp
            .saturating_mul(EFFECT_OVER_CV)
            .max(EFFECT_FLOOR_BP);
        self.pilot_runs as usize >= MIN_PILOT_RUNS
            && self.pilot_cv_bp <= MAX_CV_BP
            && self.min_effect_bp == min_effect
            && self.equivalence_margin_bp == min_effect / 2
            && self.rule_sha256 == self.digest()
    }
}

/// Freeze the confirmation rule from one arm's per-run average FPS (one value per whole run).
pub fn freeze(pilot_fps: &[f64]) -> Result<FrozenConfirmationRule, PilotRefusal> {
    if pilot_fps.len() < MIN_PILOT_RUNS {
        return Err(PilotRefusal::TooFewRuns);
    }
    if pilot_fps.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return Err(PilotRefusal::InvalidMeasurement);
    }
    let n = pilot_fps.len() as f64;
    let mean = pilot_fps.iter().sum::<f64>() / n;
    let variance = pilot_fps.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
    // Rounded up so the rule is never finer than the pilot supports.
    let cv_bp = (variance.sqrt() / mean * 10_000.0).ceil() as u32;
    if cv_bp > MAX_CV_BP {
        return Err(PilotRefusal::TooNoisy);
    }
    let min_effect = cv_bp.saturating_mul(EFFECT_OVER_CV).max(EFFECT_FLOOR_BP);
    let mut rule = FrozenConfirmationRule {
        pilot_runs: pilot_fps.len() as u32,
        pilot_cv_bp: cv_bp,
        min_effect_bp: min_effect,
        equivalence_margin_bp: min_effect / 2,
        rule_sha256: String::new(),
    };
    rule.rule_sha256 = rule.digest();
    Ok(rule)
}

/// The rule a pilot freezes from its runs (one value per whole run, with its arm): each of the two arms
/// needs enough runs, and the rule follows the noisier arm so it is never finer than either supports.
/// The executor and the independent verifier both use this one derivation.
pub fn rule_from_runs(
    runs: &[(super::ProtonSource, f64)],
) -> Result<FrozenConfirmationRule, &'static str> {
    let mut arms: std::collections::BTreeMap<super::ProtonSource, Vec<f64>> =
        std::collections::BTreeMap::new();
    for (arm, fps) in runs {
        arms.entry(*arm).or_default().push(*fps);
    }
    if arms.len() != 2 {
        return Err("PilotArms");
    }
    let mut chosen: Option<FrozenConfirmationRule> = None;
    for runs in arms.values() {
        let rule = freeze(runs).map_err(|refusal| match refusal {
            PilotRefusal::TooFewRuns => "PilotTooThin",
            PilotRefusal::TooNoisy => "PilotTooNoisy",
            PilotRefusal::InvalidMeasurement => "PilotInvalid",
        })?;
        if chosen
            .as_ref()
            .is_none_or(|best| rule.pilot_cv_bp > best.pilot_cv_bp)
        {
            chosen = Some(rule);
        }
    }
    chosen.ok_or("PilotArms")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_follows_the_pilots_spread_and_is_frozen_by_digest() {
        let calm = freeze(&[70.0, 70.1, 69.9]).unwrap();
        let noisy = freeze(&[70.0, 73.0, 67.0]).unwrap();
        assert!(calm.min_effect_bp >= EFFECT_FLOOR_BP);
        assert!(noisy.min_effect_bp > calm.min_effect_bp);
        assert_eq!(noisy.equivalence_margin_bp, noisy.min_effect_bp / 2);
        assert_eq!(calm, freeze(&[70.0, 70.1, 69.9]).unwrap());
        assert_ne!(calm.rule_sha256, noisy.rule_sha256);
        assert!(calm.is_consistent() && noisy.is_consistent());
    }

    #[test]
    fn a_pilot_that_cannot_support_a_confirmation_is_refused() {
        assert_eq!(freeze(&[70.0, 71.0]), Err(PilotRefusal::TooFewRuns));
        assert_eq!(
            freeze(&[70.0, f64::NAN, 71.0]),
            Err(PilotRefusal::InvalidMeasurement)
        );
        assert_eq!(
            freeze(&[70.0, 0.0, 71.0]),
            Err(PilotRefusal::InvalidMeasurement)
        );
        assert_eq!(freeze(&[40.0, 70.0, 100.0]), Err(PilotRefusal::TooNoisy));
    }

    #[test]
    fn an_edited_or_forged_rule_is_not_consistent() {
        let rule = freeze(&[70.0, 73.0, 67.0]).unwrap();
        let mut looser = rule.clone();
        looser.min_effect_bp *= 3;
        assert!(
            !looser.is_consistent(),
            "a loosened threshold with the old digest"
        );
        let mut redigested = looser.clone();
        redigested.rule_sha256 = redigested.digest();
        assert!(
            !redigested.is_consistent(),
            "thresholds must follow from the pilot spread, not just match their digest"
        );
        let mut thin = rule.clone();
        thin.pilot_runs = 1;
        assert!(!thin.is_consistent());
        let mut wrong_digest = rule;
        wrong_digest.rule_sha256 = "0".repeat(64);
        assert!(!wrong_digest.is_consistent());
    }
}
