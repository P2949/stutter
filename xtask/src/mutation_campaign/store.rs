//! Layout and durable persistence of one mutation campaign under
//! `.llm-runs/mutation-validation/<campaign-id>/`. Atomic writes and the evidence pipeline are
//! the Core Validation ones: `CampaignDir` is only a directory layout.
use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail, ensure};

use super::model::{MutationState, SCHEMA_VERSION};
use crate::validation_campaign::state::{CampaignDir, write_atomic};

pub const ROOT: &str = ".llm-runs/mutation-validation";

pub fn dir_for(repository: &Path, campaign_id: &str) -> CampaignDir {
    CampaignDir(repository.join(ROOT).join(campaign_id))
}

pub fn state_path(dir: &CampaignDir) -> PathBuf {
    dir.0.join("mutation-campaign.json")
}

pub fn manifest_snapshot(dir: &CampaignDir) -> PathBuf {
    dir.0.join("manifest.snapshot.toml")
}

pub fn save(dir: &CampaignDir, state: &MutationState) -> anyhow::Result<()> {
    write_atomic(&state_path(dir), &serde_json::to_vec_pretty(state)?)
}

pub fn load(dir: &CampaignDir) -> anyhow::Result<MutationState> {
    let path = state_path(dir);
    let state: MutationState = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("read {}", path.display()))?,
    )
    .with_context(|| format!("mutation campaign state {} is corrupt", path.display()))?;
    ensure!(
        state.schema_version == SCHEMA_VERSION,
        "unsupported mutation campaign schema {}",
        state.schema_version
    );
    Ok(state)
}

/// The explicitly named campaign, or the newest one that has not finished.
pub fn locate(repository: &Path, explicit: Option<PathBuf>) -> anyhow::Result<CampaignDir> {
    if let Some(path) = explicit {
        return Ok(CampaignDir(if path.is_absolute() {
            path
        } else {
            repository.join(path)
        }));
    }
    let base = repository.join(ROOT);
    let mut found: Vec<(u64, PathBuf)> = fs::read_dir(&base)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter_map(|path| {
            let state = load(&CampaignDir(path.clone())).ok()?;
            Some((state.created_unix_ms, path))
        })
        .collect();
    found.sort();
    match found.pop() {
        Some((_, path)) => Ok(CampaignDir(path)),
        None => bail!(
            "no mutation campaign under {}; run `mutation-campaign init <operation>`",
            base.display()
        ),
    }
}
