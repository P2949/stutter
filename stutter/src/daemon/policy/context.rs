//! Daemon policy context DTOs; this module owns external evaluation/build inputs, not rules.

use serde::{Deserialize, Serialize};

use crate::{
    daemon::{
        capabilities::DaemonCapabilities, config::DaemonConfig, health::SystemHealthSnapshot,
    },
    remote::AgentAutotuneLimits,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonPolicyContext {
    pub data_quality_ok: bool,
    pub data_quality_reason_code: Option<String>,
    pub system_health_ok: bool,
    pub system_health_reason_code: Option<String>,
    pub workload_stable: bool,
    pub cooldown_active: bool,
    pub rollback_pending: bool,
    pub capabilities: Option<DaemonCapabilities>,
}

impl Default for DaemonPolicyContext {
    fn default() -> Self {
        Self {
            data_quality_ok: true,
            data_quality_reason_code: None,
            system_health_ok: true,
            system_health_reason_code: None,
            workload_stable: true,
            cooldown_active: false,
            rollback_pending: false,
            capabilities: None,
        }
    }
}

impl DaemonPolicyContext {
    pub fn with_system_health(mut self, health: &SystemHealthSnapshot) -> Self {
        self.system_health_ok = health.ok_for_apply;
        self.system_health_reason_code = health.reason_code.clone();
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemotePolicyContext {
    pub bind_is_loopback: bool,
    pub auth_configured: bool,
    pub request_authorized: bool,
    pub limits: AgentAutotuneLimits,
}

#[derive(Clone, Debug)]
pub struct DaemonPolicyBuildInput<'a> {
    pub config: &'a DaemonConfig,
    pub remote_context: Option<RemotePolicyContext>,
}
