//! Threshold sweeps over a calibration dataset. The decision sweep sees only positives and
//! matched negative controls; mixed/confounded captures are evaluated in a separate section.
use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::{Value, json};
use stutter::api::report::CalibrationOutcome;

use super::{FamilySpec, Fixture, Prediction, corpus::CalibrationRole, score};

pub type Diagnose<'a> = dyn Fn(&Fixture, f64) -> Result<Option<CalibrationOutcome>> + 'a;

/// TP/FP/FN/TN, precision/recall, false-high counts and confidence distributions per
/// threshold for `(fixture, is_positive)` pairs. Agreement is against the current threshold,
/// computed up front so sweep order cannot matter.
pub fn sweep(
    spec: &FamilySpec,
    items: &[(&Fixture, bool)],
    diagnose: &Diagnose<'_>,
) -> Result<Vec<Value>> {
    let mut baseline: BTreeMap<String, String> = BTreeMap::new();
    for (fixture, _) in items {
        baseline.insert(
            fixture.name.clone(),
            diagnose(fixture, spec.current)?
                .map(|o| o.cause)
                .unwrap_or_default(),
        );
    }
    let mut results = Vec::new();
    for value in spec.sweep {
        let (mut labels, mut stable) = (Vec::new(), 0);
        let mut observation: BTreeMap<String, usize> = BTreeMap::new();
        let mut causal: BTreeMap<String, usize> = BTreeMap::new();
        for (fixture, positive) in items {
            let outcome = diagnose(fixture, *value)?;
            let cause = outcome
                .as_ref()
                .map(|o| o.cause.clone())
                .unwrap_or_default();
            if baseline.get(&fixture.name).is_some_and(|b| *b == cause) {
                stable += 1;
            }
            if let Some(o) = outcome
                .as_ref()
                .filter(|_| spec.positives.contains(&cause.as_str()))
            {
                *observation.entry(o.confidence.clone()).or_default() += 1;
                *causal.entry(o.causal_confidence.clone()).or_default() += 1;
            }
            labels.push((
                *positive,
                outcome.map(|o| Prediction {
                    cause: o.cause,
                    observation: o.confidence,
                    causal: o.causal_confidence,
                }),
            ));
        }
        let counts = score(spec, &labels);
        results.push(json!({"threshold":value,"unit":spec.unit,"tp":counts.tp,"fp":counts.fp,"fn":counts.fn_,"tn":counts.tn,
            "precision":counts.precision(),"recall":counts.recall(),"false_high_observation_confidence":counts.false_high_observation,
            "false_high_causal_confidence":counts.false_high_causal,
            "primary_cause_agreement_with_current":stable as f64 / items.len().max(1) as f64,
            "observation_confidence_distribution":observation,
            "causal_confidence_distribution":causal}));
    }
    Ok(results)
}

/// Per-threshold behavior on mixed/confounded captures: what Stutter called primary and how
/// confident it was. A mixed capture is never scored as TP/FP/FN/TN.
pub fn mixed_sweep(
    spec: &FamilySpec,
    mixed: &[&Fixture],
    diagnose: &Diagnose<'_>,
) -> Result<Vec<Value>> {
    let mut results = Vec::new();
    for value in spec.sweep {
        let mut causes: BTreeMap<String, usize> = BTreeMap::new();
        let mut observation: BTreeMap<String, usize> = BTreeMap::new();
        let mut causal: BTreeMap<String, usize> = BTreeMap::new();
        let mut false_high_causal = 0;
        for fixture in mixed {
            let outcome = diagnose(fixture, *value)?;
            let cause = outcome
                .as_ref()
                .map_or("none".to_owned(), |o| o.cause.clone());
            *causes.entry(cause).or_default() += 1;
            if let Some(o) = &outcome {
                *observation.entry(o.confidence.clone()).or_default() += 1;
                *causal.entry(o.causal_confidence.clone()).or_default() += 1;
                // Overlap among deliberately combined causes is not an established single cause.
                false_high_causal += usize::from(o.causal_confidence == "High");
            }
        }
        results.push(
            json!({"threshold":value,"unit":spec.unit,"mixed_case_count":mixed.len(),
            "mixed_primary_causes":causes,"mixed_observation_confidence":observation,
            "mixed_causal_confidence":causal,"mixed_false_high_causal_claims":false_high_causal}),
        );
    }
    Ok(results)
}

/// Splits a decision set into the binary dataset and the mixed list.
pub fn partition<'a>(
    set: &[(&'a Fixture, CalibrationRole)],
) -> (Vec<(&'a Fixture, bool)>, Vec<&'a Fixture>) {
    let mut binary = Vec::new();
    let mut mixed = Vec::new();
    for (fixture, role) in set {
        match role {
            CalibrationRole::Positive => binary.push((*fixture, true)),
            CalibrationRole::NegativeControl => binary.push((*fixture, false)),
            CalibrationRole::MixedConfounded => mixed.push(*fixture),
            CalibrationRole::Ineligible => {}
        }
    }
    (binary, mixed)
}
