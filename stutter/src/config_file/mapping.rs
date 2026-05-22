use anyhow::Result;

use super::*;
use crate::{
    autotune::workload_policy::{WorkloadPolicyRuleConfigFile, parse_workload_policy_rule_configs},
    daemon::{DaemonConfig, DaemonPreset, health::SystemHealthThresholds, policy::ActionSource},
    remote::AgentAutotuneLimits,
};

pub fn community_rules_config_from_user_config(
    config: Option<&UserConfigFile>,
) -> crate::community_rules::CommunityRulesConfig {
    let Some(config) = config else {
        return crate::community_rules::CommunityRulesConfig::default();
    };

    let Some(community_rules) = config.community_rules.clone() else {
        return crate::community_rules::CommunityRulesConfig::default();
    };

    crate::community_rules::CommunityRulesConfig::from_config_file(community_rules)
}

pub fn agent_autotune_limits_from_user_config(
    config: Option<&UserConfigFile>,
) -> Result<AgentAutotuneLimits> {
    let Some(config) = config else {
        return Ok(AgentAutotuneLimits::default());
    };

    let Some(agent) = config.agent.as_ref() else {
        return Ok(AgentAutotuneLimits::default());
    };

    let Some(autotune_limits) = agent.autotune_limits.clone() else {
        return Ok(AgentAutotuneLimits::default());
    };

    autotune_limits.into_limits()
}

pub fn daemon_config_from_user_config(
    config: Option<&UserConfigFile>,
    preset_override: Option<&str>,
    source: ActionSource,
) -> Result<DaemonConfig> {
    let preset = preset_override
        .or_else(|| config.and_then(|config| config.daemon_preset.as_deref()))
        .map(str::parse::<DaemonPreset>)
        .transpose()?
        .unwrap_or(DaemonPreset::ObserveOnly);

    if let Some(config) = config {
        validate_daemon_user_config(config)?;
    }

    let mut daemon_config = DaemonConfig::from_preset(preset, source);
    apply_daemon_user_config_overrides(&mut daemon_config, config);
    validate_resolved_workload_policy(&daemon_config)?;
    Ok(daemon_config)
}

pub fn daemon_health_thresholds_from_user_config(
    config: Option<&UserConfigFile>,
    preset_override: Option<&str>,
    source: ActionSource,
) -> Result<SystemHealthThresholds> {
    Ok(
        daemon_config_from_user_config(config, preset_override, source)?
            .health
            .thresholds(),
    )
}

pub fn apply_daemon_user_config_overrides(
    daemon_config: &mut DaemonConfig,
    user_config: Option<&UserConfigFile>,
) {
    let Some(user_config) = user_config else {
        return;
    };

    if let Some(families) = user_config.daemon_enabled_action_families.as_ref() {
        daemon_config.safety.enabled_action_families = families.iter().cloned().collect();
    }
    if let Some(families) = user_config.daemon_denied_action_families.as_ref() {
        daemon_config.safety.denied_action_families = families.iter().cloned().collect();
    }
    if let Some(path) = user_config.daemon_interactive_cgroup.as_ref() {
        daemon_config.safety.cgroup_targets.interactive_cgroup = Some(path.clone());
    }
    if let Some(path) = user_config.daemon_background_cgroup.as_ref() {
        daemon_config.safety.cgroup_targets.background_cgroup = Some(path.clone());
    }
    if let Some(path) = user_config.daemon_game_cgroup.as_ref() {
        daemon_config.safety.cgroup_targets.game_cgroup = Some(path.clone());
    }
    if let Some(path) = user_config.daemon_compile_cgroup.as_ref() {
        daemon_config.safety.cgroup_targets.compile_cgroup = Some(path.clone());
    }
    if let Some(min_confidence) = user_config.daemon_min_confidence {
        daemon_config.safety.min_confidence = min_confidence;
    }
    if let Some(min_confidence) = user_config.daemon_min_suggest_confidence {
        daemon_config.autotune.confidence.min_suggest_confidence = min_confidence;
    }
    if let Some(min_confidence) = user_config.daemon_min_apply_low_risk_confidence {
        daemon_config
            .autotune
            .confidence
            .min_apply_low_risk_confidence = min_confidence;
    }
    if let Some(min_confidence) = user_config.daemon_min_apply_medium_risk_confidence {
        daemon_config
            .autotune
            .confidence
            .min_apply_medium_risk_confidence = min_confidence;
    }
    if let Some(min_confidence) = user_config.daemon_min_high_risk_suggestion_confidence {
        daemon_config
            .autotune
            .confidence
            .min_high_risk_suggestion_confidence = min_confidence;
    }
    if let Some(cpu_temp) = user_config.daemon_max_cpu_temp_celsius {
        daemon_config.health.max_cpu_temp_celsius = cpu_temp;
    }
    if let Some(gpu_temp) = user_config.daemon_max_gpu_temp_celsius {
        daemon_config.health.max_gpu_temp_celsius = gpu_temp;
    }
    if let Some(bytes) = user_config.daemon_min_disk_available_bytes {
        daemon_config.health.min_disk_available_bytes = bytes;
    }
    if let Some(memory_pressure) = user_config.daemon_max_memory_pressure_some_avg10_percent {
        daemon_config.health.max_memory_pressure_some_avg10_percent = memory_pressure;
    }
    if let Some(allow) = user_config.daemon_allow_system_wide_suggestions {
        daemon_config.safety.allow_system_wide_suggestions = allow;
    }
    if let Some(allow) = user_config.daemon_allow_system_wide_apply {
        daemon_config.safety.allow_system_wide_apply = allow;
    }
    if let Some(allow_high_risk) = user_config.daemon_allow_high_risk {
        daemon_config.safety.allow_high_risk = allow_high_risk;
    }
    if let Some(allowlist) = user_config.system_wide_allowlist.clone() {
        daemon_config.safety.system_wide_allowlist = allowlist;
    }
    if let Some(allow_medium_risk_apply) = user_config
        .autotune
        .as_ref()
        .and_then(|autotune| autotune.allow_medium_risk_apply)
        .or(user_config.daemon_allow_medium_risk_apply)
    {
        daemon_config.autotune.allow_medium_risk_apply = allow_medium_risk_apply;
    }
    if let Some(allow_cpu_power_on_battery) = user_config
        .autotune
        .as_ref()
        .and_then(|autotune| autotune.allow_cpu_power_on_battery)
    {
        daemon_config.autotune.allow_cpu_power_on_battery = allow_cpu_power_on_battery;
    }
    if let Some(allow_gpu_power_in_autotune) = user_config
        .autotune
        .as_ref()
        .and_then(|autotune| autotune.allow_gpu_power_in_autotune)
    {
        daemon_config.autotune.allow_gpu_power_in_autotune = allow_gpu_power_in_autotune;
    }
    if let Some(allow_vm_knobs_in_autotune) = user_config
        .autotune
        .as_ref()
        .and_then(|autotune| autotune.allow_vm_knobs_in_autotune)
    {
        daemon_config.autotune.allow_vm_knobs_in_autotune = allow_vm_knobs_in_autotune;
    }
    if let Some(socket) = user_config
        .autotune
        .as_ref()
        .and_then(|autotune| autotune.privileged_worker_socket.clone())
    {
        daemon_config.autotune.privileged_worker_socket = Some(socket);
    }
    if let Some(unsafe_in_process) = user_config
        .autotune
        .as_ref()
        .and_then(|autotune| autotune.unsafe_in_process_privileged_worker)
    {
        daemon_config.autotune.unsafe_in_process_privileged_worker = unsafe_in_process;
    }
    if let Some(manage_worker) = user_config
        .autotune
        .as_ref()
        .and_then(|autotune| autotune.manage_privileged_worker)
    {
        daemon_config.autotune.manage_privileged_worker = manage_worker;
    }
    if let Some(restart_limit) = user_config
        .autotune
        .as_ref()
        .and_then(|autotune| autotune.privileged_worker_restart_limit)
    {
        daemon_config.autotune.privileged_worker_restart_limit = restart_limit;
    }
    if let Some(policy) = user_config
        .autotune
        .as_ref()
        .and_then(|autotune| autotune.external_mutation_policy)
    {
        daemon_config.autotune.external_mutation_policy = policy;
    }
    if let Some(high_risk_dry_run) = user_config
        .autotune
        .as_ref()
        .and_then(|autotune| autotune.high_risk_dry_run)
    {
        daemon_config.autotune.high_risk_dry_run = high_risk_dry_run;
    }
    if let Ok(Some((_field, rules))) = workload_policy_rule_configs_from_user_config(user_config)
        && let Ok(workload_policy) = parse_workload_policy_rule_configs(rules)
    {
        daemon_config.autotune.workload_policy = workload_policy;
    }
}

const WORKLOAD_POLICY_RULES_FIELD: &str = "autotune.workload_policy.rules";
const LEGACY_WORKLOAD_POLICY_RULES_FIELD: &str = "autotune.workload_policy_rules";

pub(super) fn workload_policy_rule_configs_from_user_config(
    config: &UserConfigFile,
) -> Result<Option<(&'static str, &[WorkloadPolicyRuleConfigFile])>> {
    let Some(autotune) = config.autotune.as_ref() else {
        return Ok(None);
    };

    let canonical = autotune
        .workload_policy
        .as_ref()
        .map(|workload_policy| workload_policy.rules.as_slice());
    let legacy = autotune.workload_policy_rules.as_deref();

    match (canonical, legacy) {
        (Some(_), Some(_)) => anyhow::bail!(
            "configure either {WORKLOAD_POLICY_RULES_FIELD} or {LEGACY_WORKLOAD_POLICY_RULES_FIELD}, not both"
        ),
        (Some(rules), None) => Ok(Some((WORKLOAD_POLICY_RULES_FIELD, rules))),
        (None, Some(rules)) => Ok(Some((LEGACY_WORKLOAD_POLICY_RULES_FIELD, rules))),
        (None, None) => Ok(None),
    }
}
