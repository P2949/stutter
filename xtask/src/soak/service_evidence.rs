//! The last stage of a service campaign: strict re-verification of the counted soak, a portable
//! evidence bundle that also carries the other stages' receipts, and its verified index entry.
use std::fs;

use anyhow::{Context, ensure};
use serde_json::{Value, json};

use crate::{
    evidence_bundle,
    service_campaign::model::{Stage, State},
    validation_campaign::{
        model::{AttemptOutcome, TaskStatus},
        state::{CampaignDir, write_atomic},
    },
};

fn last_result(dir: &CampaignDir, state: &State, stage: Stage) -> Option<Value> {
    let attempts = &state.tasks.get(stage.id())?.attempts;
    let last = attempts.last()?;
    serde_json::from_slice(
        &fs::read(dir.attempt_dir(stage.id(), last.number).join("result.json")).ok()?,
    )
    .ok()
}

/// Compact, non-sensitive receipts of every stage that is not the soak itself.
fn stage_summary(dir: &CampaignDir, state: &State) -> Value {
    let names = |value: Option<Value>, list: &str, key: &str| -> Vec<Value> {
        value
            .and_then(|value| value[list].as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .map(|entry| entry[key].clone())
            .collect()
    };
    let reboots: Vec<Value> = (1..=state.required_reboots as usize)
        .filter_map(|hop| {
            let read = |kind: &str| -> Option<Value> {
                serde_json::from_slice(
                    &fs::read(dir.0.join("reboots").join(format!("{kind}-{hop}.json"))).ok()?,
                )
                .ok()
            };
            let (pre, post) = (read("pre")?, read("post")?);
            Some(json!({
                "hop": hop,
                "boot_id_before": post["boot_id_before"],
                "boot_id_after": post["boot_id_after"],
                "tokens_outstanding_before": pre["outstanding"],
                "tokens_outstanding_after": post["outstanding_after"],
                "recovery_audit_before": pre["recovery_audit_completed"],
                "recovery_audit_after": post["recovery_audit_after"],
            }))
        })
        .collect();
    json!({
        "stages": state.tasks.iter().map(|(name, task)| json!({
            "stage": name,
            "status": format!("{:?}", task.status),
            "attempts": task.attempts.iter().map(|attempt| format!("{:?}", attempt.outcome)).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "retained_records": retained_records(dir, state),
        "discovery_cases": names(last_result(dir, state, Stage::Discovery), "cases", "scenario"),
        "recovery_legs": names(last_result(dir, state, Stage::Recovery), "legs", "leg"),
        "real_reboots": reboots,
        "boot_ids": state.boot_ids,
    })
}

pub(crate) fn run(state: &State, dir: &CampaignDir, attempt: u32) -> anyhow::Result<Value> {
    for stage in Stage::ALL
        .into_iter()
        .filter(|stage| *stage != Stage::Evidence)
    {
        let task = state
            .tasks
            .get(stage.id())
            .with_context(|| format!("stage {} missing", stage.id()))?;
        ensure!(
            task.status == TaskStatus::Passed
                && task
                    .attempts
                    .last()
                    .is_some_and(|attempt| attempt.outcome == AttemptOutcome::Passed),
            "stage {} has no PASS evidence to export",
            stage.id()
        );
    }
    let soak = dir.0.join("service-soak");
    let report = super::verify::verify(&soak)?;
    ensure!(report.passed, "the counted service soak does not verify");
    let raw = super::evidence::write_raw(&state.repository, &soak, &report)?
        .context("the counted service soak is not publishable")?;
    write_atomic(
        &raw.join("artifacts/service-stages.json"),
        &serde_json::to_vec_pretty(&stage_summary(dir, state))?,
    )?;
    copy_small_records(dir, state, &raw)?;
    let bundle = dir.0.join(format!("evidence-bundle-{attempt}"));
    evidence_bundle::export_bundle(&state.repository, &raw, &bundle)?;
    evidence_bundle::verify_bundle(&bundle)?;
    let entry = evidence_bundle::verified_index_entry(&bundle)?;
    write_atomic(
        &dir.0.join("evidence-index-entry.json"),
        &serde_json::to_vec_pretty(&entry)?,
    )?;
    Ok(
        json!({"passed": true, "bundle": bundle.file_name().and_then(|name| name.to_str()), "index_entry": entry}),
    )
}

/// Hashes of the immutable per-attempt records and reboot receipts, so the portable bundle names
/// exactly the files a reviewer can check against the retained campaign.
fn retained_records(dir: &CampaignDir, state: &State) -> Value {
    let hash = |path: std::path::PathBuf| -> Value {
        fs::read(path).map_or(Value::Null, |bytes| {
            json!(stutter_core::hash::sha256_hex(&bytes))
        })
    };
    let mut records = Vec::new();
    for (name, task) in &state.tasks {
        for attempt in &task.attempts {
            let base = dir.attempt_dir(name, attempt.number);
            records.push(json!({
                "stage": name,
                "attempt": attempt.number,
                "attempt_json_sha256": hash(base.join("attempt.json")),
                "result_json_sha256": hash(base.join("result.json")),
            }));
        }
    }
    for hop in 1..=state.required_reboots {
        for kind in ["pre", "post"] {
            records.push(json!({
                "receipt": format!("reboots/{kind}-{hop}.json"),
                "sha256": hash(dir.0.join("reboots").join(format!("{kind}-{hop}.json"))),
            }));
        }
    }
    json!(records)
}

/// The small immutable records themselves (attempt records and reboot receipts), beside their
/// hashes, so the portable bundle can be read without the retained campaign. Result files can be
/// large histories, so they are represented by their hashes and the compact stage summary.
fn copy_small_records(
    dir: &CampaignDir,
    state: &State,
    raw: &std::path::Path,
) -> anyhow::Result<()> {
    let out = raw.join("artifacts/service-campaign");
    for (name, task) in &state.tasks {
        for attempt in &task.attempts {
            let from = dir.attempt_dir(name, attempt.number).join("attempt.json");
            let to = out.join(format!("{name}-attempt-{}.json", attempt.number));
            write_atomic(
                &to,
                &fs::read(&from).with_context(|| from.display().to_string())?,
            )?;
        }
    }
    for hop in 1..=state.required_reboots {
        for kind in ["pre", "post"] {
            let from = dir.0.join("reboots").join(format!("{kind}-{hop}.json"));
            let to = out.join(format!("reboot-{kind}-{hop}.json"));
            write_atomic(
                &to,
                &fs::read(&from).with_context(|| from.display().to_string())?,
            )?;
        }
    }
    Ok(())
}
