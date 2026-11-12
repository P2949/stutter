//! Deterministic reconstruction over sealed attempts and limited FPS records. Reusing this same
//! calculation can detect byte drift but cannot independently establish game provenance, source
//! quality, restoration or final acceptance. Frames are never used as independent replicates.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    AttemptOutcome, AttemptRecord, AttemptSlot, ProtonSource, ResearchIntentV1,
    analysis::{NativeMetric, Rule, Verdict, analyse_pairs},
    driver::build_schedule,
};
use crate::autotune::experiment_design::{ExperimentDesign, StoppingRule};

const FAMILY_ALPHA: f64 = 0.05;
const MIN_EFFECT_PERCENT: f64 = 2.0;
const EQUIVALENCE_MARGIN_PERCENT: f64 = 1.0;

/// Raw native result for one sealed attempt; its serialized SHA-256 is the journal evidence digest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptEvidenceV1 {
    pub schema_version: u32,
    pub slot_index: u32,
    pub average_fps: f64,
}

impl AttemptEvidenceV1 {
    pub fn digest(&self) -> anyhow::Result<String> {
        Ok(stutter_core::hash::sha256_hex(&serde_json::to_vec(self)?))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArmResult {
    pub source: ProtonSource,
    /// Qualification status, or the typed reason the calculation was refused.
    pub status: String,
    /// Numeric result only: current evidence lacks real-game provenance and cannot establish a win.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numeric_verdict: Option<String>,
    pub pairs: usize,
    pub mean_effect_percent: Option<f64>,
    pub ci_low_percent: Option<f64>,
    pub ci_high_percent: Option<f64>,
    pub rule_sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResearchReportV1 {
    pub schema_version: u32,
    pub intent_sha256: String,
    pub attempts_scheduled: usize,
    pub attempts_sealed: usize,
    pub attempts_completed: usize,
    pub control: ProtonSource,
    pub arms: Vec<ArmResult>,
}

fn verdict_name(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Improved => "improved",
        Verdict::Regressed => "regressed",
        Verdict::Equivalent => "equivalent",
        Verdict::NoUsefulEffect => "no_useful_effect",
        Verdict::Inconclusive => "inconclusive",
    }
}

fn design(
    candidate: ProtonSource,
    slots: &[AttemptSlot],
    blocks: usize,
    confidence: f64,
    seed: u64,
) -> ExperimentDesign {
    ExperimentDesign {
        schema_version: crate::autotune::experiment_design::EXPERIMENT_SCHEMA_VERSION,
        objective: "proton_native_benchmark".into(),
        metric: "native_average_fps".into(),
        unit: "fps".into(),
        candidate: format!("{candidate:?}"),
        context_fingerprint: slots[0].context_sha256.clone(),
        min_effect_percent: MIN_EFFECT_PERCENT,
        equivalence_margin_percent: EQUIVALENCE_MARGIN_PERCENT,
        seed,
        block_ids: (0..blocks)
            .map(|block| format!("block-{block:04}"))
            .collect(),
        stopping: StoppingRule::FixedN {
            blocks: blocks as u32,
            confidence,
        },
        warmup_seconds: 0,
        washout_seconds: 0,
    }
}

pub fn build_report(
    intent: &ResearchIntentV1,
    records: &[AttemptRecord],
    evidence: &BTreeMap<u32, AttemptEvidenceV1>,
) -> anyhow::Result<ResearchReportV1> {
    let schedule = build_schedule(intent)?;
    let arms: Vec<ProtonSource> = intent.tools.iter().map(|tool| tool.source).collect();
    let control = if arms.contains(&ProtonSource::ValveStable) {
        ProtonSource::ValveStable
    } else {
        arms[0]
    };
    anyhow::ensure!(
        records.len() <= schedule.len(),
        "more records than scheduled attempts"
    );
    for (record, slot) in records.iter().zip(&schedule) {
        anyhow::ensure!(
            &record.slot == slot,
            "sealed record does not match the frozen schedule"
        );
    }
    // Completed attempts must carry exactly the evidence their seal committed to.
    let mut fps: BTreeMap<u32, f64> = BTreeMap::new();
    for record in records {
        if let AttemptOutcome::Completed { evidence_sha256 } = &record.outcome {
            let item = evidence
                .get(&record.slot.index)
                .ok_or_else(|| anyhow::anyhow!("missing evidence for a completed attempt"))?;
            anyhow::ensure!(
                item.schema_version == 1
                    && item.slot_index == record.slot.index
                    && item.average_fps.is_finite()
                    && item.average_fps > 0.0
                    && &item.digest()? == evidence_sha256,
                "evidence does not match its sealed digest"
            );
            fps.insert(record.slot.index, item.average_fps);
        }
    }
    anyhow::ensure!(
        evidence.len() == fps.len(),
        "unsealed or unexpected evidence exists"
    );
    let blocks = schedule.len() / arms.len();
    let comparisons = (arms.len() - 1) as u32;
    let confidence = 1.0 - FAMILY_ALPHA / f64::from(comparisons);
    let seed = u64::from_str_radix(&intent.digest()?[16..32], 16)?;
    let value = |block: usize, source: ProtonSource| -> Option<f64> {
        let slot = schedule[block * arms.len()..(block + 1) * arms.len()]
            .iter()
            .find(|slot| slot.arm == source)?;
        fps.get(&slot.index).copied()
    };
    let mut results = Vec::new();
    for &candidate in arms.iter().filter(|source| **source != control) {
        let (mut left, mut right) = (Vec::new(), Vec::new());
        for block in 0..blocks {
            // A block counts only when every arm in it completed: partial blocks are dropped.
            if arms.iter().all(|arm| value(block, *arm).is_some()) {
                left.push(value(block, control).unwrap_or(f64::NAN));
                right.push(value(block, candidate).unwrap_or(f64::NAN));
            }
        }
        let rule = Rule {
            design: design(candidate, &schedule, blocks, confidence, seed),
            family_alpha: FAMILY_ALPHA,
            comparisons,
            metric: NativeMetric::AverageFps,
        };
        results.push(match analyse_pairs(&rule, &left, &right, false) {
            Ok(outcome) => ArmResult {
                source: candidate,
                status: "unqualified_provenance".into(),
                numeric_verdict: Some(verdict_name(outcome.verdict).into()),
                pairs: outcome.pairs,
                mean_effect_percent: Some(outcome.mean_effect * 100.0),
                ci_low_percent: Some(outcome.ci_low * 100.0),
                ci_high_percent: Some(outcome.ci_high * 100.0),
                rule_sha256: Some(outcome.rule_sha256),
            },
            Err(refused) => ArmResult {
                source: candidate,
                status: format!("refused_{refused:?}"),
                numeric_verdict: None,
                pairs: left.len(),
                mean_effect_percent: None,
                ci_low_percent: None,
                ci_high_percent: None,
                rule_sha256: None,
            },
        });
    }
    Ok(ResearchReportV1 {
        schema_version: 2,
        intent_sha256: intent.digest()?,
        attempts_scheduled: schedule.len(),
        attempts_sealed: records.len(),
        attempts_completed: fps.len(),
        control,
        arms: results,
    })
}

/// Fixture-only reconstruction: rebuild from the same raw inputs and require identical bytes.
#[cfg(test)]
pub fn verify_report(
    intent: &ResearchIntentV1,
    records: &[AttemptRecord],
    evidence: &BTreeMap<u32, AttemptEvidenceV1>,
    report: &ResearchReportV1,
) -> anyhow::Result<()> {
    let rebuilt = build_report(intent, records, evidence)?;
    anyhow::ensure!(
        serde_json::to_vec(&rebuilt)? == serde_json::to_vec(report)?,
        "report does not match an independent rebuild"
    );
    Ok(())
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
