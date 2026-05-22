use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::*;
use crate::{
    autotune::workload_policy::{DaemonWorkloadPolicyConfigFile, WorkloadPolicyRuleConfigFile},
    config::schema::ConfigDiagnostic,
    remote::AgentAutotuneLimits,
};

#[derive(Debug, Clone, Deserialize, Default)]
pub struct UserConfigFile {
    pub config_version: Option<u32>,

    pub experimental: Option<bool>,

    pub summary_ms: Option<u64>,
    pub summary_period_ms: Option<u64>,
    pub spike_us: Option<u64>,
    pub spike_threshold_ns: Option<u64>,

    pub hwmon: Option<bool>,
    pub cpu_freq: Option<bool>,
    pub no_cpu_freq: Option<bool>,
    pub include_comm: Option<Vec<String>>,
    pub exclude_comm: Option<Vec<String>>,
    pub max_tasks: Option<usize>,
    pub retain_intervals: Option<usize>,
    pub retention_max_run_count: Option<usize>,
    pub retention_max_total_bytes: Option<u64>,
    pub retention_max_age_seconds: Option<u64>,
    pub retention_min_free_bytes: Option<u64>,
    pub foreground_window: Option<bool>,
    pub focus_source: Option<String>,
    pub foreground_source: Option<String>,
    pub foreground_poll_ms: Option<u64>,
    pub foreground_max_stale_ms: Option<u64>,
    pub foreground_include_title: Option<bool>,
    pub dmabuf_tracking: Option<bool>,
    pub dmabuf_log: Option<PathBuf>,
    pub gpu_engine_sampling: Option<bool>,
    pub display_topology: Option<bool>,
    pub daemon_preset: Option<String>,
    pub daemon_enabled_action_families: Option<Vec<String>>,
    pub daemon_denied_action_families: Option<Vec<String>>,
    pub daemon_interactive_cgroup: Option<PathBuf>,
    pub daemon_background_cgroup: Option<PathBuf>,
    pub daemon_game_cgroup: Option<PathBuf>,
    pub daemon_compile_cgroup: Option<PathBuf>,
    pub daemon_min_confidence: Option<f32>,
    pub daemon_min_suggest_confidence: Option<f32>,
    pub daemon_min_apply_low_risk_confidence: Option<f32>,
    pub daemon_min_apply_medium_risk_confidence: Option<f32>,
    pub daemon_min_high_risk_suggestion_confidence: Option<f32>,
    pub daemon_max_cpu_temp_celsius: Option<u32>,
    pub daemon_max_gpu_temp_celsius: Option<u32>,
    pub daemon_min_disk_available_bytes: Option<u64>,
    pub daemon_max_memory_pressure_some_avg10_percent: Option<f32>,
    pub daemon_allow_system_wide_suggestions: Option<bool>,
    pub daemon_allow_system_wide_apply: Option<bool>,
    pub daemon_allow_high_risk: Option<bool>,
    pub daemon_allow_medium_risk_apply: Option<bool>,
    pub system_wide_allowlist: Option<crate::daemon::config::DaemonSystemWideAllowlistConfig>,
    pub autotune: Option<AutotuneConfigFile>,
    pub community_rules: Option<CommunityRulesConfigFile>,
    pub agent: Option<AgentConfigFile>,

    #[serde(skip)]
    pub diagnostics: Vec<ConfigDiagnostic>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AutotuneConfigFile {
    pub allow_medium_risk_apply: Option<bool>,
    pub allow_cpu_power_on_battery: Option<bool>,
    pub allow_gpu_power_in_autotune: Option<bool>,
    pub allow_vm_knobs_in_autotune: Option<bool>,
    pub privileged_worker_socket: Option<PathBuf>,
    pub unsafe_in_process_privileged_worker: Option<bool>,
    pub manage_privileged_worker: Option<bool>,
    pub privileged_worker_restart_limit: Option<u32>,
    pub external_mutation_policy:
        Option<crate::autotune::external_mutation::ExternalMutationPolicy>,
    pub high_risk_dry_run: Option<bool>,
    pub workload_policy: Option<DaemonWorkloadPolicyConfigFile>,
    pub workload_policy_rules: Option<Vec<WorkloadPolicyRuleConfigFile>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CommunityRulesConfigFile {
    pub enabled: Option<bool>,
    pub sources: Option<Vec<String>>,
    pub paths: Option<Vec<PathBuf>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AgentConfigFile {
    pub autotune_limits: Option<AgentAutotuneLimitsFile>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AgentAutotuneLimitsFile {
    pub max_active_controllers: Option<usize>,
    pub max_mode: Option<String>,
    pub max_safety_class: Option<String>,
    pub allow_high_risk: Option<bool>,
    pub max_candidate_window_seconds: Option<u64>,
    pub max_targets: Option<usize>,
    pub allow_system_wide_suggestions: Option<bool>,
    pub allow_system_wide_apply: Option<bool>,
}

impl AgentAutotuneLimitsFile {
    pub fn into_limits(self) -> Result<AgentAutotuneLimits> {
        let defaults = AgentAutotuneLimits::default();

        let max_safety_class =
            crate::remote::parse_legacy_safety_class(self.max_safety_class.as_deref())
                .map_err(|err| anyhow::anyhow!(err))
                .context("invalid agent.autotune_limits.max_safety_class")?;

        let max_mode = if let Some(raw) = self.max_mode.as_deref() {
            raw.parse().context("invalid max_mode")?
        } else if self.max_safety_class.is_some() {
            crate::remote::mode_for_safety_class(max_safety_class.clone())
        } else {
            defaults.max_mode
        };

        let limits = AgentAutotuneLimits {
            max_active_controllers: self
                .max_active_controllers
                .unwrap_or(defaults.max_active_controllers),
            max_mode,
            max_safety_class,
            allow_high_risk: self.allow_high_risk.unwrap_or(defaults.allow_high_risk),
            max_candidate_window_seconds: self
                .max_candidate_window_seconds
                .unwrap_or(defaults.max_candidate_window_seconds),
            max_targets: self.max_targets.unwrap_or(defaults.max_targets),
            allow_system_wide_suggestions: self
                .allow_system_wide_suggestions
                .unwrap_or(defaults.allow_system_wide_suggestions),
            allow_system_wide_apply: self
                .allow_system_wide_apply
                .unwrap_or(defaults.allow_system_wide_apply),
        };

        validate_agent_autotune_limits(&limits)?;
        Ok(limits)
    }
}
