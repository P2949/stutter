//! Pilot-informed confirmation rule (PLAN section 6.3). The effect size a confirmation must be able
//! to resolve and the margin inside which two arms count as equivalent are set from the observed
//! run-to-run variability of a pilot, then frozen with a digest before any confirmation run starts,
//! so thresholds are never tuned after seeing the confirmation data. The constants below are
//! provisional policy (named, tested and digest-bound), not a measured result.

use serde::Serialize;

/// Fewer pilot runs cannot estimate a spread.
const MIN_PILOT_RUNS: usize = 3;
/// A confirmation must resolve at least this many times the pilot's coefficient of variation.
const EFFECT_OVER_CV: f64 = 2.0;
/// But never claim sensitivity finer than this, whatever the pilot says.
const EFFECT_FLOOR_PERCENT: f64 = 1.0;
/// A pilot noisier than this cannot support a confirmation at the planned size.
const MAX_CV_PERCENT: f64 = 10.0;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FrozenConfirmationRule {
    pub pilot_runs: u32,
    pub pilot_cv_percent: f64,
    pub min_effect_percent: f64,
    pub equivalence_margin_percent: f64,
    /// SHA-256 of the rule's canonical JSON; the confirmation study binds it.
    pub rule_sha256: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PilotRefusal {
    TooFewRuns,
    InvalidMeasurement,
    TooNoisy,
}

/// Freeze the confirmation rule from the pilot's per-run average FPS (one value per whole run).
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
    let cv = variance.sqrt() / mean * 100.0;
    if cv > MAX_CV_PERCENT {
        return Err(PilotRefusal::TooNoisy);
    }
    let min_effect = (cv * EFFECT_OVER_CV).max(EFFECT_FLOOR_PERCENT);
    let mut rule = FrozenConfirmationRule {
        pilot_runs: pilot_fps.len() as u32,
        pilot_cv_percent: cv,
        min_effect_percent: min_effect,
        equivalence_margin_percent: min_effect / 2.0,
        rule_sha256: String::new(),
    };
    rule.rule_sha256 = stutter_core::hash::sha256_hex(
        serde_json::to_string(&rule)
            .map_err(|_| PilotRefusal::InvalidMeasurement)?
            .as_bytes(),
    );
    Ok(rule)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_follows_the_pilots_spread_and_is_frozen_by_digest() {
        let calm = freeze(&[70.0, 70.1, 69.9]).unwrap();
        let noisy = freeze(&[70.0, 73.0, 67.0]).unwrap();
        assert!(calm.min_effect_percent >= EFFECT_FLOOR_PERCENT);
        assert!(noisy.min_effect_percent > calm.min_effect_percent);
        assert_eq!(
            noisy.equivalence_margin_percent,
            noisy.min_effect_percent / 2.0
        );
        // Deterministic, and any different pilot yields a different digest.
        assert_eq!(calm, freeze(&[70.0, 70.1, 69.9]).unwrap());
        assert_ne!(calm.rule_sha256, noisy.rule_sha256);
        assert_eq!(calm.rule_sha256.len(), 64);
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
}
