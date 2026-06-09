use super::*;

#[derive(Args, Debug, Clone)]
pub(super) struct AgentArgs {
    #[arg(
        long = "bind",
        conflicts_with = "unix_socket",
        help = "Bind the agent to a TCP address instead of the default Unix socket"
    )]
    pub(super) bind: Option<std::net::SocketAddr>,

    #[arg(
        long = "port",
        value_name = "PORT",
        conflicts_with_all = ["bind", "unix_socket"]
    )]
    pub(super) port: Option<u16>,

    #[arg(
        long = "unix-socket",
        value_name = "PATH",
        help = "Bind the agent to a Unix domain socket; defaults to the platform runtime path"
    )]
    pub(super) unix_socket: Option<PathBuf>,

    #[arg(long = "runs-dir", value_name = "PATH")]
    pub(super) runs_dir: Option<std::path::PathBuf>,

    #[arg(
        long = "allow-unsafe-bind",
        help = "Allow binding the agent to a non-loopback address. Dangerous unless the network is trusted."
    )]
    pub(super) allow_unsafe_bind: bool,

    #[arg(
        long = "bearer-token-env",
        value_name = "ENV",
        default_value = "STUTTER_AGENT_TOKEN",
        help = "Environment variable containing bearer token for agent HTTP API"
    )]
    pub(super) bearer_token_env: String,

    #[arg(
        long = "bearer-token-file",
        value_name = "PATH",
        help = "Read bearer token for agent HTTP API from this file"
    )]
    pub(super) bearer_token_file: Option<PathBuf>,

    #[arg(
        long = "read-token-env",
        value_name = "ENV",
        default_value = "STUTTER_AGENT_READ_TOKEN",
        help = "Environment variable containing read-only bearer token for agent API"
    )]
    pub(super) read_token_env: String,

    #[arg(
        long = "read-token-file",
        value_name = "PATH",
        help = "Read read-only bearer token for agent API from this file"
    )]
    pub(super) read_token_file: Option<PathBuf>,

    #[arg(
        long = "apply-token-env",
        value_name = "ENV",
        default_value = "STUTTER_AGENT_APPLY_TOKEN",
        help = "Environment variable containing state-changing bearer token for agent API"
    )]
    pub(super) apply_token_env: String,

    #[arg(
        long = "apply-token-file",
        value_name = "PATH",
        help = "Read state-changing bearer token for agent API from this file"
    )]
    pub(super) apply_token_file: Option<PathBuf>,

    #[arg(
        long = "max-duration-seconds",
        default_value_t = crate::agent::DEFAULT_AGENT_MAX_DURATION_SECONDS,
        value_name = "SECONDS"
    )]
    pub(super) max_duration_seconds: u64,

    #[arg(
        long = "max-targets",
        default_value_t = crate::agent::DEFAULT_AGENT_MAX_TARGETS,
        value_name = "N"
    )]
    pub(super) max_targets: usize,

    #[arg(
        long = "max-concurrent-recordings",
        default_value_t = crate::agent::DEFAULT_AGENT_MAX_CONCURRENT_RECORDINGS,
        value_name = "N"
    )]
    pub(super) max_concurrent_recordings: usize,

    #[arg(
        long = "max-unix-connections",
        default_value_t = crate::agent::DEFAULT_AGENT_UNIX_CONNECTION_LIMIT,
        value_name = "N"
    )]
    pub(super) max_unix_connections: usize,

    #[arg(
        long = "unix-connection-timeout-ms",
        default_value_t = crate::agent::DEFAULT_AGENT_UNIX_CONNECTION_TIMEOUT_MS,
        value_name = "MS"
    )]
    pub(super) unix_connection_timeout_ms: u64,
}

#[derive(Args, Debug, Clone)]
pub(super) struct PrivilegedWorkerArgs {
    #[arg(
        long = "socket",
        value_name = "PATH",
        help = "Unix domain socket used by the local control plane"
    )]
    pub(super) socket: Option<PathBuf>,
}

pub(super) fn agent_listen_args(
    bind: Option<std::net::SocketAddr>,
    port: Option<u16>,
    unix_socket: Option<PathBuf>,
) -> anyhow::Result<(std::net::SocketAddr, Option<PathBuf>)> {
    if let Some(path) = unix_socket {
        return Ok((std::net::SocketAddr::from(([127, 0, 0, 1], 0)), Some(path)));
    }
    if let Some(port) = port {
        return Ok((std::net::SocketAddr::from(([127, 0, 0, 1], port)), None));
    }
    if let Some(bind) = bind {
        return Ok((bind, None));
    }

    Ok((
        std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
        Some(crate::agent::default_agent_unix_socket_path()?),
    ))
}

#[cfg(test)]
#[path = "tests/agent.rs"]
mod tests;
