use anyhow::{Context, Result};
use stutter_config::source::ConfigSource;

use super::*;
use crate::{
    autotune::workload_policy::{
        lint_workload_policy, parse_workload_policy_rule_configs, validate_workload_policy_lints,
    },
    config::schema::ConfigDiagnostic,
    daemon::{
        DaemonConfig,
        policy::{DaemonPolicyBuildInput, build_daemon_policy},
    },
    remote::AgentAutotuneLimits,
};

pub fn validate_agent_autotune_limits(limits: &AgentAutotuneLimits) -> Result<()> {
    if limits.max_active_controllers == 0 {
        anyhow::bail!("agent.autotune_limits.max_active_controllers must be greater than zero");
    }

    if limits.max_active_controllers > 1 {
        anyhow::bail!(
            "agent.autotune_limits.max_active_controllers greater than 1 is not supported yet"
        );
    }

    if limits.max_mode > crate::daemon_policy::DaemonMode::ApplyLowRisk {
        anyhow::bail!("remote autotune currently supports max_mode = apply-low-risk only");
    }

    if limits.max_safety_class > crate::actions::SafetyClass::ReversibleLowRisk {
        anyhow::bail!(
            "remote autotune currently supports max_safety_class = ReversibleLowRisk only"
        );
    }

    if limits.max_candidate_window_seconds == 0 {
        anyhow::bail!(
            "agent.autotune_limits.max_candidate_window_seconds must be greater than zero"
        );
    }

    if limits.max_candidate_window_seconds > 120 {
        anyhow::bail!("agent.autotune_limits.max_candidate_window_seconds must be <= 120");
    }

    if limits.max_targets == 0 {
        anyhow::bail!("agent.autotune_limits.max_targets must be greater than zero");
    }

    if limits.max_targets > 1 {
        anyhow::bail!("remote autotune currently supports max_targets = 1 only");
    }

    if limits.allow_system_wide_apply {
        anyhow::bail!("agent.autotune_limits.allow_system_wide_apply must be false");
    }

    Ok(())
}

pub(super) fn workload_policy_config_diagnostics(config: &UserConfigFile) -> Vec<ConfigDiagnostic> {
    let mut diagnostics = Vec::new();

    let Some((field, rules)) = (match workload_policy_rule_configs_from_user_config(config) {
        Ok(rules) => rules,
        Err(err) => {
            diagnostics.push(ConfigDiagnostic::error(
                ConfigSource::UserFile,
                Some("autotune.workload_policy".to_owned()),
                err.to_string(),
            ));
            return diagnostics;
        }
    }) else {
        return diagnostics;
    };

    for (idx, rule) in rules.iter().enumerate() {
        if rule.autonomous_families.is_empty() && !rule.allowed_families.is_empty() {
            diagnostics.push(ConfigDiagnostic::warning(
                ConfigSource::UserFile,
                Some(format!("{field}[{idx}].autonomous_families")),
                "empty autonomous_families disables autonomous apply for this workload situation; suggestions remain allowed by allowed_families",
            ));
        }
    }

    if let Err(err) = parse_workload_policy_rule_configs(rules) {
        diagnostics.push(ConfigDiagnostic::error(
            ConfigSource::UserFile,
            Some(field.to_owned()),
            format!("invalid workload policy rules: {err:#}"),
        ));
    }

    diagnostics
}

pub fn validate_daemon_user_config(config: &UserConfigFile) -> Result<()> {
    if let Some(preset) = config.daemon_preset.as_deref() {
        preset
            .parse::<crate::daemon::DaemonPreset>()
            .context("invalid daemon_preset")?;
    }

    validate_action_families(
        "daemon_enabled_action_families",
        config.daemon_enabled_action_families.as_deref(),
    )?;
    validate_action_families(
        "daemon_denied_action_families",
        config.daemon_denied_action_families.as_deref(),
    )?;
    crate::daemon::DaemonCgroupTargetsConfig {
        interactive_cgroup: config.daemon_interactive_cgroup.clone(),
        background_cgroup: config.daemon_background_cgroup.clone(),
        game_cgroup: config.daemon_game_cgroup.clone(),
        compile_cgroup: config.daemon_compile_cgroup.clone(),
    }
    .validate()?;

    validate_optional_confidence("daemon_min_confidence", config.daemon_min_confidence)?;
    validate_optional_confidence(
        "daemon_min_suggest_confidence",
        config.daemon_min_suggest_confidence,
    )?;
    validate_optional_confidence(
        "daemon_min_apply_low_risk_confidence",
        config.daemon_min_apply_low_risk_confidence,
    )?;
    validate_optional_confidence(
        "daemon_min_apply_medium_risk_confidence",
        config.daemon_min_apply_medium_risk_confidence,
    )?;
    validate_optional_confidence(
        "daemon_min_high_risk_suggestion_confidence",
        config.daemon_min_high_risk_suggestion_confidence,
    )?;
    if let Some(cpu_temp) = config.daemon_max_cpu_temp_celsius
        && !(40..=120).contains(&cpu_temp)
    {
        anyhow::bail!("daemon_max_cpu_temp_celsius must be between 40 and 120");
    }
    if let Some(gpu_temp) = config.daemon_max_gpu_temp_celsius
        && !(40..=125).contains(&gpu_temp)
    {
        anyhow::bail!("daemon_max_gpu_temp_celsius must be between 40 and 125");
    }
    if let Some(bytes) = config.daemon_min_disk_available_bytes
        && bytes == 0
    {
        anyhow::bail!("daemon_min_disk_available_bytes must be greater than zero");
    }
    if let Some(memory_pressure) = config.daemon_max_memory_pressure_some_avg10_percent
        && (!memory_pressure.is_finite() || !(0.0..=100.0).contains(&memory_pressure))
    {
        anyhow::bail!(
            "daemon_max_memory_pressure_some_avg10_percent must be a finite value between 0.0 and 100.0"
        );
    }
    if let Some((field, rules)) = workload_policy_rule_configs_from_user_config(config)? {
        parse_workload_policy_rule_configs(rules).with_context(|| format!("invalid {field}"))?;
    }

    let experimental = config.experimental.unwrap_or(false);
    if config.daemon_allow_system_wide_apply == Some(true) && !experimental {
        anyhow::bail!(
            "daemon_allow_system_wide_apply requires experimental = true in the user config"
        );
    }
    if config.daemon_allow_high_risk == Some(true) && !experimental {
        anyhow::bail!("daemon_allow_high_risk requires experimental = true in the user config");
    }
    if config
        .autotune
        .as_ref()
        .and_then(|autotune| autotune.unsafe_in_process_privileged_worker)
        == Some(true)
        && !experimental
    {
        anyhow::bail!(
            "autotune.unsafe_in_process_privileged_worker requires experimental = true in the user config"
        );
    }
    if config
        .autotune
        .as_ref()
        .and_then(|autotune| autotune.privileged_worker_restart_limit)
        == Some(0)
    {
        anyhow::bail!("autotune.privileged_worker_restart_limit must be greater than zero");
    }
    if let Some(autotune) = config.autotune.as_ref() {
        validate_optional_timing_ms(
            "autotune.privileged_worker_socket_ready_timeout_ms",
            autotune.privileged_worker_socket_ready_timeout_ms,
            60_000,
        )?;
        validate_optional_timing_ms(
            "autotune.privileged_worker_socket_ready_retry_ms",
            autotune.privileged_worker_socket_ready_retry_ms,
            10_000,
        )?;
        validate_optional_timing_ms(
            "autotune.privileged_worker_shutdown_poll_ms",
            autotune.privileged_worker_shutdown_poll_ms,
            10_000,
        )?;
    }
    Ok(())
}

pub(super) fn validate_resolved_workload_policy(daemon_config: &DaemonConfig) -> Result<()> {
    let policy = build_daemon_policy(DaemonPolicyBuildInput {
        config: daemon_config,
        remote_context: None,
    });
    let matrix = daemon_config.autotune.workload_policy.resolved_matrix()?;
    let lints = lint_workload_policy(&matrix, &policy);
    validate_workload_policy_lints(&lints)
}

pub(super) fn validate_optional_confidence(field: &str, confidence: Option<f32>) -> Result<()> {
    if let Some(confidence) = confidence
        && (!confidence.is_finite() || !(0.0..=1.0).contains(&confidence))
    {
        anyhow::bail!("{field} must be a finite value between 0.0 and 1.0");
    }

    Ok(())
}

fn validate_optional_timing_ms(field: &str, value: Option<u64>, max: u64) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };

    if value == 0 {
        anyhow::bail!("{field} must be greater than zero");
    }
    if value > max {
        anyhow::bail!("{field} must be <= {max}");
    }

    Ok(())
}

pub(super) fn validate_action_families(field: &str, families: Option<&[String]>) -> Result<()> {
    let Some(families) = families else {
        return Ok(());
    };

    if families.is_empty() {
        anyhow::bail!("{field} must not be empty when present");
    }

    for family in families {
        let trimmed = family.trim();
        if trimmed.is_empty() {
            anyhow::bail!("{field} contains an empty action family");
        }
        if trimmed != family {
            anyhow::bail!("{field} entries must not contain leading or trailing whitespace");
        }
        if !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            anyhow::bail!(
                "{field} entry {family:?} may contain only ASCII letters, numbers, '_' or '-'"
            );
        }
    }

    Ok(())
}
