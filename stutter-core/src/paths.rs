use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Centralized filesystem paths used by stutter components.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StutterPaths {
    pub state_dir: PathBuf,
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub runs_dir: PathBuf,
    pub audit_log: PathBuf,
    pub daemon_state: PathBuf,
    pub agent_socket: PathBuf,
}

impl StutterPaths {
    pub fn new(
        state_dir: impl Into<PathBuf>,
        config_dir: impl Into<PathBuf>,
        cache_dir: impl Into<PathBuf>,
        runs_dir: impl Into<PathBuf>,
        audit_log: impl Into<PathBuf>,
        daemon_state: impl Into<PathBuf>,
        agent_socket: impl Into<PathBuf>,
    ) -> Self {
        Self {
            state_dir: state_dir.into(),
            config_dir: config_dir.into(),
            cache_dir: cache_dir.into(),
            runs_dir: runs_dir.into(),
            audit_log: audit_log.into(),
            daemon_state: daemon_state.into(),
            agent_socket: agent_socket.into(),
        }
    }
}

/// Logical path string shared across crates without binding to a filesystem root.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LogicalPath(String);

impl LogicalPath {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{LogicalPath, StutterPaths};

    #[test]
    fn stutter_paths_constructs_from_explicit_paths() {
        let paths = StutterPaths::new(
            "/var/lib/stutter",
            "/etc/stutter",
            "/var/cache/stutter",
            "/var/lib/stutter/runs",
            "/var/log/stutter/audit.jsonl",
            "/var/lib/stutter/daemon-state.json",
            "/run/stutter/agent.sock",
        );

        assert_eq!(paths.state_dir, PathBuf::from("/var/lib/stutter"));
        assert_eq!(paths.config_dir, PathBuf::from("/etc/stutter"));
        assert_eq!(paths.cache_dir, PathBuf::from("/var/cache/stutter"));
        assert_eq!(paths.runs_dir, PathBuf::from("/var/lib/stutter/runs"));
        assert_eq!(
            paths.audit_log,
            PathBuf::from("/var/log/stutter/audit.jsonl")
        );
        assert_eq!(
            paths.daemon_state,
            PathBuf::from("/var/lib/stutter/daemon-state.json")
        );
        assert_eq!(paths.agent_socket, PathBuf::from("/run/stutter/agent.sock"));
    }

    #[test]
    fn logical_path_keeps_string_value() {
        let path = LogicalPath::new("runs/latest");
        assert_eq!(path.as_str(), "runs/latest");
        assert_eq!(path.into_string(), "runs/latest");
    }
}
