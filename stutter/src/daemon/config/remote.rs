use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonRemoteConfig {
    pub allow_remote_apply: bool,
    pub require_auth_for_apply: bool,
    pub allow_non_loopback_apply: bool,
}

impl Default for DaemonRemoteConfig {
    fn default() -> Self {
        Self {
            allow_remote_apply: false,
            require_auth_for_apply: true,
            allow_non_loopback_apply: false,
        }
    }
}
