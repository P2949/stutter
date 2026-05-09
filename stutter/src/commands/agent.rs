use std::{net::SocketAddr, path::PathBuf};

use crate::{agent, config_file};

#[allow(clippy::too_many_arguments)]
pub async fn run_agent_command(
    bind: SocketAddr,
    runs_dir: Option<PathBuf>,
    allow_unsafe_bind: bool,
    bearer_token_env: String,
    bearer_token_file: Option<PathBuf>,
    max_duration_seconds: u64,
    max_targets: usize,
    max_concurrent_recordings: usize,
) -> anyhow::Result<()> {
    let runs_dir = runs_dir.unwrap_or_else(agent::default_runs_dir);
    let bearer_token = agent::load_bearer_token(&bearer_token_env, bearer_token_file.as_deref())?;

    let user_config = config_file::load_user_config()?;
    let autotune_limits =
        config_file::agent_autotune_limits_from_user_config(user_config.as_ref())?;

    agent::run_agent(agent::AgentConfig {
        bind,
        runs_dir,
        allow_unsafe_bind,
        bearer_token,
        max_duration_seconds,
        max_targets,
        max_concurrent_recordings,
        autotune_limits,
        rollback_on_crash_recovery: true,
    })
    .await
}
