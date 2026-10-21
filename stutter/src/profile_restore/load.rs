use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::Context;

use super::model::ProfileRestoreState;

pub fn default_restore_path() -> PathBuf {
    let mut base = env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.push(".local");
    base.push("state");
    base.push("stutter");
    base.push("last_profile_restore.json");
    base
}

pub fn load_restore_state(path: &Path) -> anyhow::Result<ProfileRestoreState> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read profile restore file {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&data)
        .with_context(|| format!("failed to parse profile restore file {}", path.display()))?;
    anyhow::ensure!(
        value.get("records").is_none(),
        "affinity restore format is not a profile restore file"
    );
    let state: ProfileRestoreState = serde_json::from_value(value)?;
    anyhow::ensure!(
        state.schema_version == super::model::PROFILE_RESTORE_SCHEMA_VERSION,
        "unsupported profile restore schema {}",
        state.schema_version
    );
    Ok(state)
}
