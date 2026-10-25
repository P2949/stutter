//! Read-only views of a campaign: `status`, `verify` and `report`. None of them mutates state.
use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde_json::{Value, json};

use super::{
    benefit_stage::{Calibration, HoldoutRecord, pairs_digest, read_pairs},
    driver::{Next, integrity, next_stage, ordered_stages},
    model::{Disposition, MutationMode, MutationState, TierDecision},
};
use crate::validation_campaign::{
    evidence::reverify_task,
    model::{AttemptOutcome, TaskStatus},
    state::{CampaignDir, write_atomic},
};

fn manifest_attempts(dir: &CampaignDir) -> u32 {
    super::manifest::MutationManifest::load(&super::store::manifest_snapshot(dir))
        .map_or(1, |(manifest, _)| manifest.retry.default_attempts)
}

pub fn status(dir: &CampaignDir, state: &MutationState) -> anyhow::Result<Value> {
    let attempts = manifest_attempts(dir);
    let next = if state.mode == MutationMode::Active {
        match next_stage(state, attempts) {
            Next::Stage(stage) => json!(stage.as_str()),
            Next::Complete => json!("complete"),
            Next::Blocked(_) => Value::Null,
        }
    } else {
        Value::Null
    };
    let blocked = state
        .tasks
        .values()
        .find_map(|task| task.block.as_ref())
        .map(|reason| json!({"class": format!("{:?}", reason.class), "detail": reason.detail}));
    let tasks: Vec<Value> = ordered_stages(state)
        .into_iter()
        .map(|stage| {
            let task = &state.tasks[stage.as_str()];
            json!({
                "stage": stage.as_str(),
                "status": format!("{:?}", task.status),
                "attempts": task.attempts.len(),
                "last": task.attempts.last().map(|a| match &a.outcome {
                    AttemptOutcome::Passed => "passed".to_owned(),
                    AttemptOutcome::Failed { class, summary, .. } => format!("failed {class:?}: {summary}"),
                    AttemptOutcome::Blocked(reason) => format!("blocked: {}", reason.detail),
                    AttemptOutcome::WaitingForReboot => "waiting for reboot".to_owned(),
                }),
            })
        })
        .collect();
    let pairs = read_pairs(&benefit_dir_of(dir).join("pairs.jsonl")).unwrap_or_default();
    Ok(json!({
        "campaign": state.campaign_id,
        "source": state.evidence_base,
        "operation": state.operation,
        "objective": state.objective,
        "mode": format!("{:?}", state.mode),
        "next": next,
        "blocked": blocked,
        "stages": tasks,
        "skipped": state.skipped,
        "benefit": {"pairs": pairs.len()},
        "decision": state.decision,
    }))
}

fn benefit_dir_of(dir: &CampaignDir) -> PathBuf {
    dir.0.join("benefit")
}

/// Everything recomputed from disk; the recorded status is not trusted.
pub fn verify(root: &Path, dir: &CampaignDir, state: &MutationState) -> Vec<String> {
    let mut problems = Vec::new();
    if state.mode == MutationMode::Active || state.mode == MutationMode::Complete {
        if let Err(reason) = integrity(root, dir, state) {
            problems.push(format!("source integrity: {reason}"));
        }
    } else {
        problems.push(format!("campaign mode is {:?}", state.mode));
    }
    for stage in ordered_stages(state) {
        let task = &state.tasks[stage.as_str()];
        if task.status == TaskStatus::Passed {
            match &task.fingerprint {
                Some(fingerprint)
                    if fingerprint.evidence_commit == state.evidence_base
                        && fingerprint.manifest_sha256 == state.manifest_sha256 => {}
                _ => problems.push(format!(
                    "stage {} passed with a fingerprint that is not the campaign's",
                    stage.as_str()
                )),
            }
        }
        if matches!(
            stage,
            super::graph::Stage::Mechanical
                | super::graph::Stage::FaultMatrix
                | super::graph::Stage::SuspendResume
        ) && task.status == TaskStatus::Passed
        {
            match reverify_task(dir, stage.as_str()) {
                Ok(entries) if entries.is_empty() => {
                    problems.push(format!("stage {} published no bundle", stage.as_str()));
                }
                Ok(entries) => {
                    for entry in entries {
                        if entry["source_commit"] != state.evidence_base.as_str() {
                            problems.push(format!(
                                "bundle {} certifies {}",
                                entry["capture_id"], entry["source_commit"]
                            ));
                        }
                    }
                }
                Err(error) => problems.push(format!("{error:#}")),
            }
        }
    }
    problems.extend(verify_benefit(dir));
    if let Some(decision) = &state.decision {
        let all_passed = ordered_stages(state)
            .iter()
            .all(|stage| state.tasks[stage.as_str()].status == TaskStatus::Passed);
        if decision.disposition == Disposition::Supported && !all_passed {
            problems.push("Supported was decided while a stage did not pass".to_owned());
        }
    } else if state.mode == MutationMode::Complete {
        problems.push("campaign is complete without a decision".to_owned());
    }
    problems
}

fn verify_benefit(dir: &CampaignDir) -> Vec<String> {
    let mut problems = Vec::new();
    let base = benefit_dir_of(dir);
    let pairs_file = base.join("pairs.jsonl");
    for (file, name) in [
        ("calibration.json", "calibration"),
        ("holdout.json", "holdout"),
    ] {
        let path = base.join(file);
        if !path.is_file() {
            continue;
        }
        let digest = pairs_digest(&pairs_file);
        let recorded = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|value| value["pairs_sha256"].as_str().map(str::to_owned));
        if name == "holdout" && recorded.as_deref() != Some(digest.as_str()) {
            problems.push("holdout was evaluated on different pair data than is stored".to_owned());
        }
    }
    problems
}

pub fn report(dir: &CampaignDir, state: &MutationState) -> anyhow::Result<PathBuf> {
    let status = status(dir, state)?;
    let calibration: Option<Calibration> = fs::read(benefit_dir_of(dir).join("calibration.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let holdout: Option<HoldoutRecord> = fs::read(benefit_dir_of(dir).join("holdout.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let path = dir.0.join("report.json");
    write_atomic(
        &path,
        &serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "status": status,
            "calibration": calibration,
            "holdout": holdout,
        }))?,
    )?;
    Ok(path)
}

/// The decision the tier stage wrote.
pub fn read_decision(dir: &CampaignDir, state: &MutationState) -> anyhow::Result<TierDecision> {
    let task = state
        .tasks
        .get("tier_decision")
        .context("no tier_decision stage")?;
    let receipt = task
        .attempts
        .last()
        .and_then(|attempt| attempt.receipt.clone())
        .context("the tier stage left no receipt")?;
    Ok(serde_json::from_slice(&fs::read(dir.0.join(receipt))?)?)
}
