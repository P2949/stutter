//! Evidence integration and the tier decision. The decision is derived from stage outcomes and
//! never forced: `Supported` needs every stage to have passed.
use std::fs;

use serde_json::json;

use super::{
    benefit_stage::read_pairs,
    graph::Stage,
    model::{Disposition, TierDecision},
    runner::{StageContext, StageOutcome},
};
use crate::validation_campaign::{
    evidence::{
        Accept, PipelineFailure, PublishRequest, publish_bundle, rebuild_combined_index,
        reverify_task,
    },
    model::{FailureClass, TaskStatus},
    state::{unix_ms, write_atomic},
};

/// Suite stages that publish bundles.
pub const BUNDLE_STAGES: [Stage; 3] = [Stage::Mechanical, Stage::FaultMatrix, Stage::SuspendResume];

pub fn integrate(context: &StageContext<'_>, attempt: u32) -> StageOutcome {
    let dir = context
        .dir
        .attempt_dir(Stage::EvidenceIntegrate.as_str(), attempt);
    let mut entries = Vec::new();
    for stage in BUNDLE_STAGES {
        if !context.state.tasks.contains_key(stage.as_str()) {
            continue;
        }
        match reverify_task(context.dir, stage.as_str()) {
            Ok(found) if !found.is_empty() => entries.extend(found),
            Ok(_) => {
                return StageOutcome::failed(
                    FailureClass::SoftwareDefect,
                    "integration",
                    format!("stage {} published no bundle", stage.as_str()),
                );
            }
            Err(error) => {
                return StageOutcome::failed(
                    FailureClass::SoftwareDefect,
                    "integration",
                    format!("{error:#}"),
                );
            }
        }
    }
    if let Some(entry) = entries
        .iter()
        .find(|entry| entry["source_commit"] != context.state.evidence_base.as_str())
    {
        return StageOutcome::failed(
            FailureClass::SoftwareDefect,
            "integration",
            format!(
                "bundle {} certifies {}, not {}",
                entry["capture_id"], entry["source_commit"], context.state.evidence_base
            ),
        );
    }
    match benefit_bundle(context, attempt) {
        Ok(entry) => entries.push(entry),
        Err(failure) => {
            return StageOutcome::Failed {
                class: failure_class(&failure.stage),
                stage: failure.stage,
                summary: failure.summary,
                artifacts: Some(context.dir.relative(&dir)),
            };
        }
    }
    let summary = json!({
        "schema_version": 1,
        "operation": context.state.operation,
        "objective": context.state.objective,
        "source_commit": context.state.evidence_base,
        "entries": entries,
    });
    let path = dir.join("integration.json");
    if let Err(error) = write_atomic(
        &path,
        &serde_json::to_vec_pretty(&summary).unwrap_or_default(),
    )
    .and_then(|()| rebuild_combined_index(context.dir, context.repository).map(|_| ()))
    {
        return StageOutcome::failed(FailureClass::Retryable, "receipt", format!("{error:#}"));
    }
    StageOutcome::passed(
        Some(context.dir.relative(&dir)),
        Some(context.dir.relative(&path)),
    )
}

fn failure_class(stage: &str) -> FailureClass {
    match stage {
        "fsync" | "publish" | "receipt" => FailureClass::Retryable,
        _ => FailureClass::SoftwareDefect,
    }
}

/// The aggregate benefit evidence as one portable bundle: the stored pairs, the frozen thresholds,
/// the single holdout verdict and, per pair, the typed experiment evidence the controller recorded.
/// Nothing in it is hand-written; every figure is a stored measurement.
fn benefit_bundle(
    context: &StageContext<'_>,
    attempt: u32,
) -> Result<serde_json::Value, PipelineFailure> {
    let at = |stage: &str, error: &dyn std::fmt::Display| PipelineFailure {
        stage: stage.to_owned(),
        summary: error.to_string(),
    };
    let base = context.dir.0.join("benefit");
    let raw = context
        .dir
        .attempt_dir(Stage::EvidenceIntegrate.as_str(), attempt)
        .join("benefit-raw");
    let _ = fs::remove_dir_all(&raw);
    fs::create_dir_all(raw.join("pairs")).map_err(|e| at("benefit_raw", &e))?;
    for name in ["pairs.jsonl", "calibration.json", "holdout.json"] {
        fs::copy(base.join(name), raw.join(name))
            .map_err(|e| at("benefit_raw", &format!("{name}: {e}")))?;
    }
    let pairs =
        read_pairs(&base.join("pairs.jsonl")).map_err(|e| at("benefit_raw", &format!("{e:#}")))?;
    for pair in &pairs {
        let attempt_raw = context
            .dir
            .attempt_dir(Stage::BenefitCalibration.as_str(), pair.attempt)
            .join("raw");
        let name = format!("{:?}-{:04}", pair.scenario, pair.pair).to_lowercase();
        fs::copy(
            attempt_raw.join("pair.json"),
            raw.join("pairs").join(format!("{name}.pair.json")),
        )
        .map_err(|e| at("benefit_raw", &format!("{name}: {e}")))?;
        let experiment = fs::read_to_string(attempt_raw.join("history.jsonl"))
            .map_err(|e| at("benefit_raw", &format!("{name} history: {e}")))?
            .lines()
            .find(|line| line.contains("\"objective_evidence\":{"))
            .map(str::to_owned)
            .ok_or_else(|| at("benefit_raw", &format!("{name}: no experiment evidence")))?;
        fs::write(
            raw.join("pairs").join(format!("{name}.experiment.json")),
            experiment,
        )
        .map_err(|e| at("benefit_raw", &e))?;
    }
    let holdout: serde_json::Value = serde_json::from_slice(
        &fs::read(base.join("holdout.json")).map_err(|e| at("benefit_raw", &e))?,
    )
    .map_err(|e| at("benefit_raw", &e))?;
    let calibration: serde_json::Value = serde_json::from_slice(
        &fs::read(base.join("calibration.json")).map_err(|e| at("benefit_raw", &e))?,
    )
    .map_err(|e| at("benefit_raw", &e))?;
    let passed = holdout["verdict"]["passed"] == true;
    crate::live_acceptance::write_benefit_summary(
        context.repository,
        &raw,
        &context.binaries.stutter,
        if passed { "PASS" } else { "FAIL" },
        json!({
            "operation": context.state.operation,
            "objective": context.state.objective,
            "pairs_per_scenario": calibration["pairs_per_scenario"],
            "frozen_thresholds": calibration["thresholds"],
            "calibration_tally": calibration["calibration_tally"],
            "holdout": holdout["verdict"],
            "scope": "matched pairs of one CPU-affinity candidate against controlled scheduler contention, through the production live controller; thresholds frozen on the calibration part and the holdout evaluated once",
            "not_claimed": ["any other operation, objective or workload", "kernels other than the one recorded", "frame pacing or a real game"],
        }),
    )
    .map_err(|e| at("benefit_summary", &format!("{e:#}")))?;
    let published = publish_bundle(&PublishRequest {
        dir: context.dir,
        repository: context.repository,
        task: Stage::EvidenceIntegrate.as_str(),
        attempt,
        raw: &raw,
        expected_commit: &context.state.evidence_base,
        allowed_kinds: &["controlled_real"],
        accept: Accept::PassOnly,
    })?;
    Ok(published.index_entry)
}

/// Derives the disposition from the recorded stage outcomes.
pub fn derive(context: &StageContext<'_>) -> TierDecision {
    let mut reasons = Vec::new();
    let mut deferred = false;
    for (id, task) in &context.state.tasks {
        if id == Stage::TierDecision.as_str() {
            continue;
        }
        match task.status {
            TaskStatus::Passed => {}
            TaskStatus::Failed
                if task.attempts.last().is_some_and(|attempt| {
                    matches!(
                        attempt.outcome,
                        crate::validation_campaign::model::AttemptOutcome::Failed {
                            class: FailureClass::ValidationFailure,
                            ..
                        }
                    )
                }) =>
            {
                deferred = true;
                reasons.push(format!(
                    "{id}: the experiment did not meet the graduation requirements"
                ));
            }
            other => {
                deferred = true;
                reasons.push(format!("{id}: {other:?}"));
            }
        }
    }
    let disposition = if deferred {
        Disposition::DeferredEvidence
    } else {
        reasons.push("every stage passed".to_owned());
        Disposition::Supported
    };
    TierDecision {
        disposition,
        reasons,
        decided_unix_ms: unix_ms(),
    }
}

pub fn tier(context: &StageContext<'_>, attempt: u32) -> StageOutcome {
    let dir = context
        .dir
        .attempt_dir(Stage::TierDecision.as_str(), attempt);
    let decision = derive(context);
    let path = dir.join("decision.json");
    if let Err(error) = fs::create_dir_all(&dir)
        .map_err(anyhow::Error::from)
        .and_then(|()| write_atomic(&path, &serde_json::to_vec_pretty(&decision)?))
    {
        return StageOutcome::failed(FailureClass::Retryable, "receipt", format!("{error:#}"));
    }
    StageOutcome::passed(
        Some(context.dir.relative(&dir)),
        Some(context.dir.relative(&path)),
    )
}
