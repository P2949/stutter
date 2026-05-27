use std::path::{Path, PathBuf};

use anyhow::Context;

use super::{
    daemon::status_from_daemon_state,
    history::status_from_history_events,
    model::{AutotuneStatus, AutotuneStatusCommandInput},
    render::render_autotune_status_text,
};
use crate::{
    autotune::history::{default_autotune_history_path, read_autotune_history_events},
    daemon::state::{default_daemon_state_snapshot_path, load_daemon_state},
};

pub fn daemon_state_path_for_history_path(history_path: &Path) -> PathBuf {
    history_path
        .parent()
        .map(|parent| parent.join("daemon_state.json"))
        .unwrap_or_else(default_daemon_state_snapshot_path)
}

pub fn autotune_status_command(input: AutotuneStatusCommandInput) -> anyhow::Result<()> {
    let history_path = input
        .history_path
        .unwrap_or_else(default_autotune_history_path);
    let status = load_autotune_status(&history_path)?;

    if input.json {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        print!("{}", render_autotune_status_text(&status));
    }

    Ok(())
}

pub fn load_autotune_status(history_path: &Path) -> anyhow::Result<AutotuneStatus> {
    let daemon_state_path = daemon_state_path_for_history_path(history_path);
    if daemon_state_path.exists() {
        let state = load_daemon_state(&daemon_state_path).with_context(|| {
            format!(
                "failed to load daemon state snapshot {}",
                daemon_state_path.display()
            )
        })?;
        return Ok(status_from_daemon_state(daemon_state_path, &state));
    }

    let events = read_autotune_history_events(history_path)?;
    Ok(status_from_history_events(
        history_path.to_path_buf(),
        &events,
    ))
}
