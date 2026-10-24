//! Independent target-impact evidence per family: does the intervention measurably worsen the
//! *target's own* direct metric (never a diagnosis output)? The decision uses the production
//! A/B comparison: bootstrap CI must exclude zero in the worsening direction and the effect
//! size must be non-trivial.
use serde_json::{Value, json};
use stutter::api::statistics::{FormalMetricComparison, compare_lower_is_better_metric};

use super::{
    model::{FamilyDef, StudyConfig},
    store::{Outcome, Role, RunRecord},
};
use crate::evidence_bundle::sha256::sha256_hex;

/// Values of the primary metric from accepted runs of pairs `0..pairs`.
pub fn series(runs: &[RunRecord], family_scenario: &str, pairs: usize) -> Vec<(usize, f64)> {
    let mut values: Vec<(usize, f64)> = runs
        .iter()
        .filter(|r| r.accepted && r.scenario == family_scenario && r.pair < pairs)
        .filter_map(|r| Some((r.pair, r.metrics["primary"].as_f64()?)))
        .collect();
    values.sort_by_key(|(pair, _)| *pair);
    values
}

fn worsening_established(comparison: &FormalMetricComparison, min_effect: f64) -> bool {
    comparison.enough_samples
        && comparison
            .bootstrap_ci95
            .as_ref()
            .is_some_and(|ci| ci.upper < 0.0)
        && comparison.improvement_delta < 0.0
        && comparison.effect_size.is_some_and(|d| d <= -min_effect)
}

pub struct Evaluation {
    pub outcome: Outcome,
    pub report: Value,
    pub report_sha256: String,
}

/// Decides one family at `pairs` matched pairs. Controls are the baseline side and positives the
/// compared side, so a worsening intervention has a negative improvement delta.
pub fn evaluate(
    config: &StudyConfig,
    def: &FamilyDef,
    runs: &[RunRecord],
    pairs: usize,
) -> Evaluation {
    let controls = series(runs, &super::plan::control_id(def.family), pairs);
    let positives = series(runs, &super::plan::positive_id(def.family), pairs);
    let values = |s: &[(usize, f64)]| s.iter().map(|(_, v)| *v).collect::<Vec<_>>();
    let comparison = compare_lower_is_better_metric(
        def.metric.name(),
        def.metric.unit(),
        &values(&controls),
        &values(&positives),
    );
    let established = worsening_established(&comparison, config.sampling.min_effect_size);
    let outcome = if established {
        Outcome::Pass
    } else if pairs + config.sampling.extension_step <= config.sampling.max_pairs {
        Outcome::Extend
    } else {
        Outcome::Invalid
    };
    let order: Vec<&str> = (0..pairs)
        .map(|p| {
            if super::plan::pair_order(p)[0] == Role::Control {
                "AB"
            } else {
                "BA"
            }
        })
        .collect();
    let mut report = json!({
        "schema_version": 1,
        "family": def.family.as_str(),
        "metric": def.metric.name(),
        "unit": def.metric.unit(),
        "pairs": pairs,
        "order": order,
        "min_effect_size": config.sampling.min_effect_size,
        "worsening_established": established,
        "outcome": outcome,
        "comparison": comparison,
        "controls": values(&controls),
        "positives": values(&positives),
    });
    let sha256 = sha256_hex(report.to_string().as_bytes());
    report["report_sha256"] = json!(sha256);
    Evaluation {
        outcome,
        report,
        report_sha256: sha256,
    }
}
