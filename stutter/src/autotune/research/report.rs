//! Deterministic reconstruction over sealed attempts and limited FPS records. Reusing this same
//! calculation can detect byte drift but cannot independently establish game provenance, source
//! quality, restoration or final acceptance. Frames are never used as independent replicates.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    AttemptOutcome, AttemptRecord, AttemptSlot, ProtonSource, ResearchIntentV1,
    analysis::{NativeMetric, Rule, Verdict, analyse_pairs},
    driver::build_schedule,
    loaded_runtime::RuntimeBinding,
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
    /// Native file hashes and declared process identity; independent attribution is still required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_completion: Option<Box<super::cyberpunk::CyberpunkBenchmarkCompletion>>,
    /// A CP-LONG attempt's sealed session record, in place of a native benchmark result (never both): a
    /// long session is verified by `long_receipt::verify` against the intent's authorized unit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_session: Option<Box<super::long_receipt::LongSessionRecord>>,
    /// The CPU and GPU temperatures when the attempt launched and when its result arrived (absent for results
    /// from before it existed, or where the host reads none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal: Option<super::thermal::ThermalContext>,
}

impl AttemptEvidenceV1 {
    pub fn digest(&self) -> anyhow::Result<String> {
        Ok(stutter_core::hash::sha256_hex(&serde_json::to_vec(self)?))
    }

    pub(super) fn validate_for(
        &self,
        slot: &AttemptSlot,
        intent: &ResearchIntentV1,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == 1
                && self.slot_index == slot.index
                && self.average_fps.is_finite()
                && self.average_fps > 0.0,
            "invalid attempt evidence"
        );
        if let Some(native) = &self.native_completion {
            let tool = intent
                .tools
                .iter()
                .find(|tool| tool.source == slot.arm)
                .ok_or_else(|| anyhow::anyhow!("attempt tool is not authorized"))?;
            anyhow::ensure!(
                native.schema_version == 1
                    && native.app_id == intent.app_id
                    && native.build_id == intent.build_id
                    && native.process.uid == intent.uid
                    && native.process.app_id == intent.app_id
                    && native.process.build_id == intent.build_id
                    && native.process.start_ticks != 0
                    && native.process.pid.as_u32() > 1
                    && native.process.executable_sha256 == intent.game_executable_sha256
                    && native.proton_source == slot.arm
                    && native.proton_identity_sha256 == tool.identity_sha256
                    && native.metrics.average_fps.to_bits() == self.average_fps.to_bits()
                    && native.started_unix_nanos < native.completed_unix_nanos
                    && !native.files.is_empty()
                    && native.runtime_receipt_consistent(),
                "native completion does not match the frozen attempt"
            );
        }
        Ok(())
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
    /// Present only for a within-source study: the one arm result is that build measured with this
    /// factor or action on versus off, not a second Proton.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub treatment: Option<super::TreatmentSpec>,
    /// Present only when the comparison was judged on the 99th-percentile frame time; absent means the
    /// average FPS (results recorded before tail statistics existed, or without a known frame population).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_metric: Option<String>,
}

fn metric_label(metric: NativeMetric) -> Option<String> {
    (metric == NativeMetric::P99FrameMs).then(|| metric.name_and_unit().0.to_owned())
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

/// The design of one comparison. Warmup and washout are zero seconds by construction, not by
/// omission: a unit is one whole launch of the game. Prefix identity is checked separately and does
/// not prove shader-cache temperature. Any prior screen used as operational warmup is recorded in
/// its own child study, and
/// carry-over between launches is controlled by complete, seeded, order-balanced blocks
/// (`scheduler::balanced`) rather than by discarding time. A policy that discarded seconds would
/// have to be frozen in the intent before the data, and none is.
fn design(
    candidate: ProtonSource,
    slots: &[AttemptSlot],
    blocks: usize,
    confidence: f64,
    seed: u64,
    rule: Option<&super::pilot_rule::FrozenConfirmationRule>,
    metric: NativeMetric,
) -> ExperimentDesign {
    // A confirmation runs under the thresholds its pilot froze; anything else keeps the provisional
    // defaults. The frozen rule is never replaced by a threshold chosen after the data.
    let (min_effect, margin) =
        rule.map_or((MIN_EFFECT_PERCENT, EQUIVALENCE_MARGIN_PERCENT), |rule| {
            (
                f64::from(rule.min_effect_bp) / 100.0,
                f64::from(rule.equivalence_margin_bp) / 100.0,
            )
        });
    ExperimentDesign {
        schema_version: crate::autotune::experiment_design::EXPERIMENT_SCHEMA_VERSION,
        objective: "proton_native_benchmark".into(),
        metric: metric.name_and_unit().0.into(),
        unit: metric.name_and_unit().1.into(),
        candidate: format!("{candidate:?}"),
        context_fingerprint: slots[0].context_sha256.clone(),
        min_effect_percent: min_effect,
        equivalence_margin_percent: margin,
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
    let control = {
        let viable = |source: ProtonSource| arms.contains(&source);
        [ProtonSource::ValveStable, ProtonSource::ValveExperimental]
            .into_iter()
            .find(|s| viable(*s))
            .or_else(|| arms.first().copied())
            .ok_or_else(|| anyhow::anyhow!("no authorized arms"))?
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
            item.validate_for(&record.slot, intent)?;
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
    // An initialization study compares nothing: its report only accounts for the attempts.
    if intent.initialization {
        return Ok(ResearchReportV1 {
            schema_version: 2,
            intent_sha256: intent.digest()?,
            attempts_scheduled: schedule.len(),
            attempts_sealed: records.len(),
            attempts_completed: fps.len(),
            control,
            arms: Vec::new(),
            primary_metric: None,
            treatment: None,
        });
    }
    // The primary metric is chosen once, from the sealed evidence, before any comparison is read.
    let (metric, fps) = super::metric_plan::values(
        evidence,
        fps.keys().copied(),
        intent
            .confirmation_rule
            .as_ref()
            .map(|rule| rule.tail_metric),
    )?;
    if let Some(treatments) = super::treatment::treatments_of(intent)? {
        let result = treatment_result(intent, &schedule, &fps, evidence, &treatments, metric)?;
        return Ok(ResearchReportV1 {
            schema_version: 2,
            intent_sha256: intent.digest()?,
            attempts_scheduled: schedule.len(),
            attempts_sealed: records.len(),
            attempts_completed: fps.len(),
            control,
            arms: vec![result],
            primary_metric: metric_label(metric),
            treatment: intent.treatment,
        });
    }
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
        // Context is judged per comparison: only the control and this candidate must agree, so
        // unrelated arms or studies never mark this pair as drifted.
        let context = context_state(
            &evidence
                .iter()
                .filter(|(index, _)| {
                    schedule
                        .get(**index as usize)
                        .is_some_and(|slot| slot.arm == control || slot.arm == candidate)
                })
                .map(|(index, item)| (*index, item.clone()))
                .collect(),
        );
        let (mut left, mut right) = (Vec::new(), Vec::new());
        for block in 0..blocks {
            // A block counts for this comparison only when both its control and this candidate
            // completed. An unrelated arm failing never discards a complete pair, and a pair is
            // never assembled from different blocks.
            if value(block, control).is_some() && value(block, candidate).is_some() {
                left.push(value(block, control).unwrap_or(f64::NAN));
                right.push(value(block, candidate).unwrap_or(f64::NAN));
            }
        }
        let rule = Rule {
            design: design(
                candidate,
                &schedule,
                blocks,
                confidence,
                seed,
                intent.confirmation_rule.as_ref(),
                metric,
            ),
            family_alpha: FAMILY_ALPHA,
            comparisons,
            metric,
        };
        results.push(
            match analyse_pairs(&rule, &left, &right, context == ContextState::Drifted) {
                Ok(outcome) => ArmResult {
                    source: candidate,
                    status: match context {
                        ContextState::Consistent => "native_context_consistent_unqualified",
                        _ => "unqualified_provenance",
                    }
                    .into(),
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
            },
        );
    }
    Ok(ResearchReportV1 {
        schema_version: 2,
        intent_sha256: intent.digest()?,
        attempts_scheduled: schedule.len(),
        attempts_sealed: records.len(),
        attempts_completed: fps.len(),
        control,
        arms: results,
        primary_metric: metric_label(metric),
        treatment: None,
    })
}

/// The one result of a within-source study: this build with the switch on versus off, paired within
/// each complete block. Whole runs are the units, as everywhere else.
fn treatment_result(
    intent: &ResearchIntentV1,
    schedule: &[AttemptSlot],
    fps: &BTreeMap<u32, f64>,
    evidence: &BTreeMap<u32, AttemptEvidenceV1>,
    treatments: &[super::Treatment],
    metric: NativeMetric,
) -> anyhow::Result<ArmResult> {
    use super::Treatment::{Off, On};
    let source = intent.tools[0].source;
    // A long session seals a placeholder FPS only so the record has a valid number; it is not a measurement.
    // Comparing off and on over a constant is meaningless, so no numeric result is ever derived from it: the
    // runnable-latency measurement an action study needs is a separate, source-qualified one.
    if !evidence.is_empty()
        && evidence
            .values()
            .all(|item| item.long_session.is_some() && item.native_completion.is_none())
    {
        return Ok(ArmResult {
            source,
            status: "ActionMetricUnavailable".into(),
            numeric_verdict: None,
            pairs: 0,
            mean_effect_percent: None,
            ci_low_percent: None,
            ci_high_percent: None,
            rule_sha256: None,
        });
    }
    let seed = u64::from_str_radix(&intent.digest()?[16..32], 16)?;
    let blocks = schedule.len() / 2;
    let value = |block: usize, wanted: super::Treatment| -> Option<f64> {
        let index = (block * 2..block * 2 + 2).find(|index| treatments[*index] == wanted)?;
        fps.get(&schedule[index].index).copied()
    };
    // Every attempt is the same build, so the context must agree across all of them.
    let context = context_state_with_treatment(evidence, intent.treatment.as_ref());
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for block in 0..blocks {
        if let (Some(off), Some(on)) = (value(block, Off), value(block, On)) {
            left.push(off);
            right.push(on);
        }
    }
    let rule = Rule {
        design: design(
            source,
            schedule,
            blocks,
            1.0 - FAMILY_ALPHA,
            seed,
            None,
            metric,
        ),
        family_alpha: FAMILY_ALPHA,
        comparisons: 1,
        metric,
    };
    Ok(
        match analyse_pairs(&rule, &left, &right, context == ContextState::Drifted) {
            Ok(outcome) => ArmResult {
                source,
                status: match context {
                    ContextState::Consistent => "native_context_consistent_unqualified",
                    _ => "unqualified_provenance",
                }
                .into(),
                numeric_verdict: Some(verdict_name(outcome.verdict).into()),
                pairs: outcome.pairs,
                mean_effect_percent: Some(outcome.mean_effect * 100.0),
                ci_low_percent: Some(outcome.ci_low * 100.0),
                ci_high_percent: Some(outcome.ci_high * 100.0),
                rule_sha256: Some(outcome.rule_sha256),
            },
            Err(refused) => ArmResult {
                source,
                status: format!("refused_{refused:?}"),
                numeric_verdict: None,
                pairs: left.len(),
                mean_effect_percent: None,
                ci_low_percent: None,
                ci_high_percent: None,
                rule_sha256: None,
            },
        },
    )
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContextState {
    /// Some completed attempt carries no native completion, so stability cannot be shown.
    Unproven,
    /// Native completions disagree on build, scene, settings or game executable.
    Drifted,
    /// Every completed attempt agrees; this is necessary, not sufficient, for qualification.
    Consistent,
}

/// Cross-attempt context check from the sealed native completions themselves. Unknown is never
/// treated as stable, and an inconsistent context withholds the numeric comparison.
fn context_state(evidence: &BTreeMap<u32, AttemptEvidenceV1>) -> ContextState {
    context_state_with_treatment(evidence, None)
}

fn context_state_with_treatment(
    evidence: &BTreeMap<u32, AttemptEvidenceV1>,
    treatment: Option<&super::TreatmentSpec>,
) -> ContextState {
    /// Two reported values disagree only when both were reported; a hole is not a disagreement but
    /// it is not agreement either (see `is_complete`).
    fn differs<T: PartialEq>(a: &Option<T>, b: &Option<T>) -> bool {
        matches!((a, b), (Some(x), Some(y)) if x != y)
    }
    // For a factor study, the field that IS the factor is intentionally different between arms.
    // Differences in that field are legal; differences in any other field are Drifted.
    let factor = match treatment {
        Some(super::TreatmentSpec::Factor(f)) => Some(*f),
        _ => None,
    };
    let mut natives = evidence
        .values()
        .map(|item| item.native_completion.as_deref());
    let Some(Some(first)) = natives.next() else {
        return ContextState::Unproven;
    };
    let complete = |native: &super::cyberpunk::CyberpunkBenchmarkCompletion| {
        native
            .context
            .as_ref()
            .is_some_and(super::native_context::NativeContext::is_complete)
    };
    // Every completion must carry a fully reported graphics context, the first one included.
    let mut unproven = !complete(first);
    match first.runtime_binding {
        Some(RuntimeBinding::Mismatched) => return ContextState::Drifted,
        Some(RuntimeBinding::Matched) => {}
        Some(RuntimeBinding::Unknown) | None => unproven = true,
    }
    // The built-in benchmark is a fixed fly-through: every run lasts the same time. A run of another
    // length is a scene that did not repeat, and its numbers are not comparable with the others'.
    let seconds = |native: &super::cyberpunk::CyberpunkBenchmarkCompletion| {
        native
            .context
            .as_ref()
            .and_then(|context| context.benchmark_seconds(native.metrics.average_fps))
    };
    let first_seconds = seconds(first);
    for native in natives {
        let Some(native) = native else {
            unproven = true;
            continue;
        };
        if native.build_id != first.build_id
            || native.scene != first.scene
            || native.settings_sha256 != first.settings_sha256
            || native.process.executable_sha256 != first.process.executable_sha256
        {
            return ContextState::Drifted;
        }
        if let (Some(a), Some(b)) = (first_seconds, seconds(native))
            && !super::native_context::same_scene_length(a, b)
        {
            return ContextState::Drifted;
        }
        unproven |= !complete(native);
        match native.runtime_binding {
            Some(RuntimeBinding::Mismatched) => return ContextState::Drifted,
            Some(RuntimeBinding::Matched) => {}
            // Never captured, or captured with an unobserved path layout: not proven.
            Some(RuntimeBinding::Unknown) | None => unproven = true,
        }
        if let (Some(a), Some(b)) = (&first.context, &native.context)
            && (differs(&a.game_version, &b.game_version)
                || differs(&a.preset_name, &b.preset_name)
                || differs(&a.gpu_name, &b.gpu_name)
                || (factor != Some(super::ResearchFactor::Fsr4)
                    && differs(&a.upscaling_type, &b.upscaling_type))
                || differs(&a.ray_tracing_enabled, &b.ray_tracing_enabled)
                || (factor != Some(super::ResearchFactor::FrameGeneration)
                    && differs(&a.frame_generation_type, &b.frame_generation_type)))
        {
            return ContextState::Drifted;
        }
    }
    // The attempts must also share a thermal context: a hot start or a throttled end is not one comparison.
    match super::thermal::state(
        &evidence
            .values()
            .map(|item| item.thermal)
            .collect::<Vec<_>>(),
    ) {
        super::thermal::ThermalState::Drifted => return ContextState::Drifted,
        super::thermal::ThermalState::Unproven => unproven = true,
        super::thermal::ThermalState::Consistent | super::thermal::ThermalState::NotRecorded => {}
    }
    if unproven {
        ContextState::Unproven
    } else {
        ContextState::Consistent
    }
}

/// Whether the sealed native completions prove one consistent comparison context (identity,
/// settings, graphics context and matched runtime). Used by child studies to qualify their result.
pub(super) fn evidence_context_consistent(
    evidence: &BTreeMap<u32, AttemptEvidenceV1>,
    treatment: Option<&super::TreatmentSpec>,
) -> bool {
    // Long sessions carry no graphics context: they are consistent when every one played the same unit.
    if !evidence.is_empty() && evidence.values().all(|item| item.long_session.is_some()) {
        let mut units = evidence
            .values()
            .filter_map(|item| item.long_session.as_deref())
            .map(|s| (s.unit_id.as_str(), s.definition_sha256.as_str()));
        let first = units.next();
        return units.all(|unit| Some(unit) == first);
    }
    context_state_with_treatment(evidence, treatment) == ContextState::Consistent
}
