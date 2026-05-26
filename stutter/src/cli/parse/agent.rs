use std::time::Duration;

use super::super::agent::{AgentArgs, PrivilegedWorkerArgs, agent_listen_args};
use crate::commands::input::{AgentCommandInput, AppCommand, PrivilegedWorkerCommandInput};

pub(super) fn parse_agent_command(args: AgentArgs) -> anyhow::Result<AppCommand> {
    if args.max_duration_seconds == 0 {
        anyhow::bail!("--max-duration-seconds must be greater than zero");
    }
    if args.max_targets == 0 {
        anyhow::bail!("--max-targets must be greater than zero");
    }
    if args.max_concurrent_recordings == 0 {
        anyhow::bail!("--max-concurrent-recordings must be greater than zero");
    }
    if args.max_concurrent_recordings > 1 {
        anyhow::bail!("agent currently supports at most 1 concurrent recording");
    }
    if args.max_unix_connections == 0 {
        anyhow::bail!("--max-unix-connections must be greater than zero");
    }
    if args.unix_connection_timeout_ms == 0 {
        anyhow::bail!("--unix-connection-timeout-ms must be greater than zero");
    }
    let (bind, unix_socket) = agent_listen_args(args.bind, args.port, args.unix_socket)?;
    Ok(AppCommand::Agent(AgentCommandInput {
        bind,
        unix_socket,
        runs_dir: args.runs_dir,
        allow_unsafe_bind: args.allow_unsafe_bind,
        bearer_token_env: args.bearer_token_env,
        bearer_token_file: args.bearer_token_file,
        read_token_env: args.read_token_env,
        read_token_file: args.read_token_file,
        apply_token_env: args.apply_token_env,
        apply_token_file: args.apply_token_file,
        max_duration_seconds: args.max_duration_seconds,
        max_targets: args.max_targets,
        max_concurrent_recordings: args.max_concurrent_recordings,
        max_unix_connections: args.max_unix_connections,
        unix_connection_timeout: Duration::from_millis(args.unix_connection_timeout_ms),
    }))
}

pub(super) fn parse_privileged_worker_command(
    args: PrivilegedWorkerArgs,
) -> anyhow::Result<AppCommand> {
    let socket = match args.socket {
        Some(socket) => socket,
        None => crate::daemon::privilege::default_privileged_worker_socket_path()?,
    };
    Ok(AppCommand::PrivilegedWorker(PrivilegedWorkerCommandInput {
        socket,
    }))
}
