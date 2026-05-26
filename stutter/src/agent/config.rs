//! Agent configuration contracts.

use super::*;

#[derive(Clone, Debug)]
pub struct AgentConfig {
    pub bind: SocketAddr,
    pub unix_socket: Option<PathBuf>,
    pub runs_dir: PathBuf,
    pub allow_unsafe_bind: bool,
    pub bearer_token: Option<String>,
    pub read_token: Option<String>,
    pub apply_token: Option<String>,
    pub max_duration_seconds: u64,
    pub max_targets: usize,
    pub max_concurrent_recordings: usize,
    pub max_unix_connections: usize,
    pub unix_connection_timeout: Duration,
    pub autotune_limits: AgentAutotuneLimits,
    pub health_thresholds: SystemHealthThresholds,
    pub rollback_on_crash_recovery: bool,
}

#[derive(Clone, Debug)]
pub struct AgentLimits {
    pub max_duration_seconds: u64,
    pub max_targets: usize,
    pub max_concurrent_recordings: usize,
}

pub fn default_runs_dir() -> PathBuf {
    let mut p = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    p.push("stutter-runs");
    p
}

pub fn default_agent_unix_socket_path() -> anyhow::Result<PathBuf> {
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(runtime_dir).join("stutter-agent.sock"));
    }

    let Some(home) = std::env::var_os("HOME") else {
        anyhow::bail!(
            "cannot choose default agent unix socket path without XDG_RUNTIME_DIR or HOME"
        );
    };

    Ok(PathBuf::from(home)
        .join(".local")
        .join("state")
        .join("stutter")
        .join("agent.sock"))
}

pub fn load_bearer_token(
    bearer_token_env: &str,
    bearer_token_file: Option<&StdPath>,
) -> anyhow::Result<Option<String>> {
    if let Some(path) = bearer_token_file {
        let token =
            std::fs::read_to_string(path).map_err(|source| AgentError::BearerTokenFile {
                path: path.to_path_buf(),
                source,
            })?;
        return normalize_bearer_token(token).map_err(anyhow::Error::new);
    }

    if let Some(value) = std::env::var_os(bearer_token_env) {
        return normalize_bearer_token(value.to_string_lossy().into_owned())
            .map_err(anyhow::Error::new);
    }

    Ok(None)
}

pub(crate) fn normalize_bearer_token(raw: String) -> Result<Option<String>, AgentError> {
    let token = raw.trim().to_owned();
    if token.is_empty() {
        return Err(AgentError::EmptyBearerToken);
    }
    Ok(Some(token))
}
