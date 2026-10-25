//! Evidence integration and the tier decision. The decision is derived from stage outcomes and
//! never forced: `Supported` needs every stage to have passed.
use std::fs;

use serde_json::json;

use super::{
    graph::Stage,
    model::{Disposition, TierDecision},
    runner::{StageContext, StageOutcome},
};
use crate::validation_campaign::{
    evidence::{rebuild_combined_index, reverify_task},
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
