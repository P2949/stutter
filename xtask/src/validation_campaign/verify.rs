//! `verify --strict`: campaign truth rebuilt from retained attempts, receipts and bundles,
//! never from the mutable cached task statuses alone.
use std::fs;

use super::{
    evidence::reverify_task,
    graph::{Graph, TaskKind},
    manifest::CampaignManifest,
    model::{AttemptOutcome, CampaignMode, CampaignState, TaskAttempt, TaskStatus},
    state::CampaignDir,
};
use crate::evidence_bundle::sha256::sha256_hex;

/// Tasks whose pass must be backed by verified portable bundles.
pub fn produces_bundles(kind: TaskKind) -> bool {
    matches!(
        kind,
        TaskKind::LiveObservation
            | TaskKind::GameRunnable
            | TaskKind::OverheadControlled
            | TaskKind::OverheadReal
            | TaskKind::ScoreRunnable
            | TaskKind::KernelMatrix
            | TaskKind::RcLiveObservation
            | TaskKind::RcGameRunnable
            | TaskKind::RcAffinity
            | TaskKind::RcRecovery
    )
}

/// Returns every inconsistency found; empty means the campaign verifies.
pub fn verify_state(
    dir: &CampaignDir,
    state: &CampaignState,
    graph: &Graph,
    manifest: &CampaignManifest,
) -> Vec<String> {
    let mut problems = Vec::new();
    match fs::read(dir.manifest_snapshot()) {
        Ok(bytes) if sha256_hex(&bytes) == state.manifest_sha256 => {
            if CampaignManifest::parse(&String::from_utf8_lossy(&bytes)).is_err() {
                problems.push("manifest snapshot no longer parses".to_owned());
            }
        }
        Ok(_) => problems.push("manifest snapshot differs from the recorded digest".to_owned()),
        Err(error) => problems.push(format!("manifest snapshot unreadable: {error}")),
    }
    for id in state.tasks.keys() {
        if graph.get(id).is_none() {
            problems.push(format!("state has unknown task {id}"));
        }
    }
    for spec in graph.specs() {
        let Some(task) = state.tasks.get(spec.id) else {
            problems.push(format!("task {} is missing from state", spec.id));
            continue;
        };
        for attempt in &task.attempts {
            let path = dir
                .attempt_dir(spec.id, attempt.number)
                .join("attempt.json");
            match fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<TaskAttempt>(&bytes).ok())
            {
                Some(on_disk) if &on_disk == attempt => {}
                Some(_) => problems.push(format!(
                    "{}: attempt {} differs from its immutable record",
                    spec.id, attempt.number
                )),
                None => problems.push(format!(
                    "{}: attempt {} has no readable immutable record",
                    spec.id, attempt.number
                )),
            }
        }
        if task.status != TaskStatus::Passed {
            continue;
        }
        if !task
            .attempts
            .iter()
            .any(|attempt| attempt.outcome == AttemptOutcome::Passed)
        {
            problems.push(format!("{} is Passed but has no passing attempt", spec.id));
        }
        if !state.effective_status(spec).satisfies_dependents() {
            problems.push(format!(
                "{} passed for a different fingerprint (stale)",
                spec.id
            ));
        }
        if produces_bundles(spec.kind) {
            match reverify_task(dir, spec.id) {
                Ok(entries) if entries.is_empty() => {
                    problems.push(format!("{} is Passed but has no verified bundle", spec.id));
                }
                Ok(entries) => {
                    let expected = state.expected_fingerprint(spec).evidence_commit;
                    let mut kernels = std::collections::BTreeSet::new();
                    for entry in &entries {
                        // A kernel observed as incompatible is a FAIL result that is still a
                        // valid, kept observation.
                        let observed_incompatible = spec.kind == TaskKind::KernelMatrix
                            && entry["status"] == "FAIL"
                            && entry["compatibility"] == "ObservedIncompatible";
                        if (entry["status"] != "PASS" && !observed_incompatible)
                            || entry["source_commit"] != expected
                        {
                            problems.push(format!(
                                "{}: bundle {} does not certify {expected}",
                                spec.id, entry["capture_id"]
                            ));
                        }
                        if let Some(kernel) = entry["kernel_release"].as_str() {
                            kernels.insert(kernel.to_owned());
                        }
                    }
                    if spec.kind == TaskKind::KernelMatrix
                        && kernels.len() < manifest.kernels.required_distinct
                    {
                        problems.push(format!(
                            "{} is Passed with {} distinct kernels; {} are required",
                            spec.id,
                            kernels.len(),
                            manifest.kernels.required_distinct
                        ));
                    }
                }
                Err(error) => problems.push(format!("{}: {error:#}", spec.id)),
            }
        }
    }
    if state.mode == CampaignMode::Complete {
        for spec in graph.specs() {
            let skipped =
                state.tasks.get(spec.id).map(|t| t.status) == Some(TaskStatus::SkippedByPolicy);
            if !skipped && !state.effective_status(spec).satisfies_dependents() {
                problems.push(format!(
                    "campaign is Complete but {} has not passed",
                    spec.id
                ));
            }
        }
    }
    problems
}
