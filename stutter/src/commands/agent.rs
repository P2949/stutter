use crate::{agent, commands::input::AgentCommandInput, config_file};

pub async fn run_agent_command(input: AgentCommandInput) -> anyhow::Result<()> {
    let runs_dir = input.runs_dir.unwrap_or_else(agent::default_runs_dir);
    let bearer_token =
        agent::load_bearer_token(&input.bearer_token_env, input.bearer_token_file.as_deref())?;
    let read_token =
        agent::load_bearer_token(&input.read_token_env, input.read_token_file.as_deref())?;
    let apply_token =
        agent::load_bearer_token(&input.apply_token_env, input.apply_token_file.as_deref())?;

    let user_config = config_file::load_user_config()?;
    let autotune_limits =
        config_file::agent_autotune_limits_from_user_config(user_config.as_ref())?;
    let health_thresholds = config_file::daemon_health_thresholds_from_user_config(
        user_config.as_ref(),
        None,
        crate::daemon::policy::ActionSource::RemoteAgent,
    )?;

    agent::run_agent(agent::AgentConfig {
        bind: input.bind,
        unix_socket: input.unix_socket,
        runs_dir,
        allow_unsafe_bind: input.allow_unsafe_bind,
        bearer_token,
        read_token,
        apply_token,
        max_duration_seconds: input.max_duration_seconds,
        max_targets: input.max_targets,
        max_concurrent_recordings: input.max_concurrent_recordings,
        autotune_limits,
        health_thresholds,
        rollback_on_crash_recovery: true,
    })
    .await
}
