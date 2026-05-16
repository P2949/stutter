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
mod tests {
    use std::path::PathBuf;

    use crate::commands::input::AppCommand;

    fn parse_agent_command<const N: usize>(args: [&str; N]) -> anyhow::Result<AppCommand> {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        crate::cli::parse_app_command_from(args)
    }

    #[test]
    fn agent_accepts_allow_unsafe_bind() {
        let command = parse_agent_command(["stutter", "agent", "--allow-unsafe-bind"]).unwrap();

        let AppCommand::Agent(input) = command else {
            panic!("expected agent command");
        };

        assert!(input.allow_unsafe_bind);
        assert!(input.unix_socket.is_some());
    }

    #[test]
    fn agent_defaults_to_unix_socket() {
        let command = parse_agent_command(["stutter", "agent"]).unwrap();

        let AppCommand::Agent(input) = command else {
            panic!("expected agent command");
        };

        assert!(input.unix_socket.is_some());
        assert_eq!(input.bind.port(), 0);
        assert_eq!(input.runs_dir, None);
        assert_eq!(input.bearer_token_env, "STUTTER_AGENT_TOKEN");
        assert_eq!(input.read_token_env, "STUTTER_AGENT_READ_TOKEN");
        assert_eq!(input.apply_token_env, "STUTTER_AGENT_APPLY_TOKEN");
    }

    #[test]
    fn agent_accepts_tcp_bind_and_port_overrides() {
        let bind_command =
            parse_agent_command(["stutter", "agent", "--bind", "127.0.0.1:9999"]).unwrap();

        let AppCommand::Agent(bind_input) = bind_command else {
            panic!("expected agent command");
        };

        assert_eq!(bind_input.bind, "127.0.0.1:9999".parse().unwrap());
        assert!(bind_input.unix_socket.is_none());

        let port_command = parse_agent_command(["stutter", "agent", "--port", "9998"]).unwrap();

        let AppCommand::Agent(port_input) = port_command else {
            panic!("expected agent command");
        };

        assert_eq!(port_input.bind, "127.0.0.1:9998".parse().unwrap());
        assert!(port_input.unix_socket.is_none());
    }

    #[test]
    fn agent_accepts_explicit_unix_socket() {
        let command = parse_agent_command([
            "stutter",
            "agent",
            "--unix-socket",
            "/tmp/stutter-agent.sock",
        ])
        .unwrap();

        let AppCommand::Agent(input) = command else {
            panic!("expected agent command");
        };

        assert_eq!(
            input.unix_socket,
            Some(PathBuf::from("/tmp/stutter-agent.sock"))
        );
        assert_eq!(input.bind.port(), 0);
    }

    #[test]
    fn agent_rejects_ambiguous_listen_overrides() {
        assert!(
            parse_agent_command([
                "stutter",
                "agent",
                "--bind",
                "127.0.0.1:9999",
                "--port",
                "9998",
            ])
            .is_err()
        );

        assert!(
            parse_agent_command([
                "stutter",
                "agent",
                "--unix-socket",
                "/tmp/stutter-agent.sock",
                "--port",
                "9998",
            ])
            .is_err()
        );

        assert!(
            parse_agent_command([
                "stutter",
                "agent",
                "--bind",
                "127.0.0.1:9999",
                "--unix-socket",
                "/tmp/stutter-agent.sock",
            ])
            .is_err()
        );
    }

    #[test]
    fn agent_accepts_bearer_token_file() {
        let command =
            parse_agent_command(["stutter", "agent", "--bearer-token-file", "/tmp/token"]).unwrap();

        let AppCommand::Agent(input) = command else {
            panic!("expected agent command");
        };

        assert_eq!(input.bearer_token_file, Some(PathBuf::from("/tmp/token")));
    }

    #[test]
    fn agent_accepts_bearer_token_env() {
        let command =
            parse_agent_command(["stutter", "agent", "--bearer-token-env", "MY_TOKEN"]).unwrap();

        let AppCommand::Agent(input) = command else {
            panic!("expected agent command");
        };

        assert_eq!(input.bearer_token_env, "MY_TOKEN");
    }

    #[test]
    fn agent_accepts_split_read_and_apply_tokens() {
        let command = parse_agent_command([
            "stutter",
            "agent",
            "--read-token-env",
            "READ_TOKEN",
            "--read-token-file",
            "/tmp/read-token",
            "--apply-token-env",
            "APPLY_TOKEN",
            "--apply-token-file",
            "/tmp/apply-token",
        ])
        .unwrap();

        let AppCommand::Agent(input) = command else {
            panic!("expected agent command");
        };

        assert_eq!(input.read_token_env, "READ_TOKEN");
        assert_eq!(
            input.read_token_file,
            Some(PathBuf::from("/tmp/read-token"))
        );
        assert_eq!(input.apply_token_env, "APPLY_TOKEN");
        assert_eq!(
            input.apply_token_file,
            Some(PathBuf::from("/tmp/apply-token"))
        );
    }

    #[test]
    fn agent_accepts_runs_dir_and_limit_overrides() {
        let command = parse_agent_command([
            "stutter",
            "agent",
            "--runs-dir",
            "/tmp/stutter-runs",
            "--max-duration-seconds",
            "300",
            "--max-targets",
            "9",
            "--max-concurrent-recordings",
            "1",
        ])
        .unwrap();

        let AppCommand::Agent(input) = command else {
            panic!("expected agent command");
        };

        assert_eq!(input.runs_dir, Some(PathBuf::from("/tmp/stutter-runs")));
        assert_eq!(input.max_duration_seconds, 300);
        assert_eq!(input.max_targets, 9);
        assert_eq!(input.max_concurrent_recordings, 1);
    }

    #[test]
    fn agent_rejects_zero_limits() {
        for (args, expected) in [
            (
                ["stutter", "agent", "--max-duration-seconds", "0"],
                "--max-duration-seconds must be greater than zero",
            ),
            (
                ["stutter", "agent", "--max-targets", "0"],
                "--max-targets must be greater than zero",
            ),
            (
                ["stutter", "agent", "--max-concurrent-recordings", "0"],
                "--max-concurrent-recordings must be greater than zero",
            ),
        ] {
            let err = parse_agent_command(args).unwrap_err();

            assert!(
                err.to_string().contains(expected),
                "expected error containing {expected:?}, got {err:#}"
            );
        }
    }

    #[test]
    fn agent_rejects_max_concurrent_recordings_above_one() {
        let err = parse_agent_command(["stutter", "agent", "--max-concurrent-recordings", "2"])
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("agent currently supports at most 1 concurrent recording")
        );
    }
}
