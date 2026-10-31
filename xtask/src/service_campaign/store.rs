//! Reuse the campaign directory layout and durable atomic writes; retain immutable attempts.
use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, ensure};

use super::model::State;
use crate::validation_campaign::state::{CampaignDir, write_atomic};
pub const ROOT: &str = ".llm-runs/service-validation";
pub fn path(dir: &CampaignDir) -> PathBuf {
    dir.0.join("service-campaign.json")
}
pub fn save(dir: &CampaignDir, state: &State) -> anyhow::Result<()> {
    write_atomic(&path(dir), &serde_json::to_vec_pretty(state)?)
}
pub fn load(dir: &CampaignDir) -> anyhow::Result<State> {
    let mut state: State = serde_json::from_slice(&fs::read(path(dir))?)?;
    ensure!(
        state.schema_version == 1,
        "unsupported service campaign state"
    );
    ensure!(
        state.tasks.len() == super::model::Stage::ALL.len(),
        "service campaign stage set is corrupt"
    );
    let mut reconciled = false;
    for stage in super::model::Stage::ALL {
        let entry = state
            .tasks
            .get_mut(stage.id())
            .context("required service stage missing")?;
        ensure!(
            entry.id == stage.id(),
            "service stage identity differs from its key"
        );
        loop {
            let retained = dir
                .attempt_dir(stage.id(), entry.attempts.len() as u32 + 1)
                .join("attempt.json");
            if !retained.exists() {
                break;
            }
            let attempt: crate::validation_campaign::model::TaskAttempt =
                serde_json::from_slice(&fs::read(retained)?)?;
            ensure!(
                attempt.fingerprint.evidence_commit == state.source_commit
                    && attempt.fingerprint.manifest_sha256 == state.manifest_sha256
                    && attempt.fingerprint.task == stage.id(),
                "orphaned service attempt has a different source fingerprint"
            );
            crate::validation_campaign::state::append_task_attempt(dir, entry, attempt)?;
            reconciled = true;
        }
    }
    if reconciled {
        save(dir, &state)?;
    }
    Ok(state)
}
pub fn locate(root: &Path, explicit: Option<PathBuf>) -> anyhow::Result<CampaignDir> {
    if let Some(path) = explicit {
        return Ok(CampaignDir(if path.is_absolute() {
            path
        } else {
            root.join(path)
        }));
    }
    let mut found: Vec<_> = fs::read_dir(root.join(ROOT))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let dir = CampaignDir(entry.path());
            let state = load(&dir).ok()?;
            Some((state.created_unix_ms, dir.0))
        })
        .collect();
    found.sort();
    Ok(CampaignDir(
        found
            .pop()
            .context("no service campaign; run service-campaign init")?
            .1,
    ))
}
