use serde::Serialize;

use crate::{
    actions::{ActionId, SafetyClass},
    config_file::{self, UserConfigFile},
    daemon::{
        ActionDescriptor, ActionSource, DaemonConfig, DaemonPolicy, DaemonPolicyBuildInput,
        PolicyExplanation, PolicyIntent, RollbackRequirement, build_daemon_policy,
    },
    remote::AgentAutotuneLimits,
};

#[derive(Clone, Debug, Serialize)]
pub struct DaemonConfigExplainOutput {
    pub config: DaemonConfig,
    pub policy: DaemonPolicy,
    pub explanation: PolicyExplanation,
    pub agent_autotune_limits: AgentAutotuneLimits,
    pub user_config_loaded: bool,
}

pub fn run_config_explain_command(
    input: crate::commands::input::DaemonConfigExplainCommandInput,
) -> anyhow::Result<()> {
    let user_config = config_file::load_user_config()?;
    let output = build_config_explain_output_from_user_config(
        user_config.as_ref(),
        input.preset.as_deref(),
    )?;

    if input.json {
        println!("{}", render_config_explain_json(&output)?);
    } else {
        print!("{}", render_config_explain_text(&output));
    }

    Ok(())
}

pub fn build_config_explain_output_from_user_config(
    user_config: Option<&UserConfigFile>,
    preset: Option<&str>,
) -> anyhow::Result<DaemonConfigExplainOutput> {
    let config =
        config_file::daemon_config_from_user_config(user_config, preset, ActionSource::Cli)?;
    let agent_autotune_limits = config_file::agent_autotune_limits_from_user_config(user_config)?;
    let policy = build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    });
    let descriptor = daemon_config_explain_descriptor();
    let explanation = policy.explain_action(PolicyIntent::Observe, &descriptor);

    Ok(DaemonConfigExplainOutput {
        config,
        policy,
        explanation,
        agent_autotune_limits,
        user_config_loaded: user_config.is_some(),
    })
}

pub fn daemon_config_explain_descriptor() -> ActionDescriptor {
    ActionDescriptor {
        action_id: ActionId("daemon-config-explain".to_owned()),
        action_kind: "daemon-config-explain".to_owned(),
        safety_class: SafetyClass::ObserveOnly,
        effect_scope: crate::daemon::ActionEffectScope::ObserveOnly,
        rollback: RollbackRequirement::NotRequiredForDryRun,
        persistent_effect: false,
        touches_system_wide_state: false,
        requires_explicit_target: false,
        confidence: None,
    }
}

pub fn render_config_explain_json(output: &DaemonConfigExplainOutput) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(output)?)
}

pub fn render_config_explain_text(output: &DaemonConfigExplainOutput) -> String {
    let mut text = String::new();

    text.push_str("Daemon config explanation\n");
    text.push_str("=========================\n");
    text.push_str(&format!(
        "user_config_loaded: {}\n",
        output.user_config_loaded
    ));
    text.push_str(&format!("preset: {}\n", output.config.preset));
    text.push_str(&format!("mode: {}\n", output.config.mode));
    text.push_str(&format!("source: {:?}\n", output.config.source));
    text.push_str(&format!(
        "target_pids: {:?}\n",
        output.config.target.target_pids
    ));
    text.push_str(&format!(
        "tree_pids: {:?}\n",
        output.config.target.tree_pids
    ));
    text.push_str(&format!(
        "watch_process: {}\n",
        output
            .config
            .target
            .watch_process
            .as_deref()
            .unwrap_or("<none>")
    ));
    text.push_str(&format!(
        "require_explicit_target: {}\n",
        output.config.target.require_explicit_target
    ));
    text.push_str("\nEffective policy\n");
    text.push_str("----------------\n");
    text.push_str(&format!(
        "max_safety_class: {:?}\n",
        output.policy.max_safety_class
    ));
    text.push_str(&format!(
        "rollback_required_before_apply: {}\n",
        output.policy.rollback_required_before_apply
    ));
    text.push_str(&format!(
        "allow_system_wide_suggestions: {}\n",
        output.policy.allow_system_wide_suggestions
    ));
    text.push_str(&format!(
        "allow_system_wide_apply: {}\n",
        output.policy.allow_system_wide_apply
    ));
    text.push_str(&format!(
        "allow_high_risk: {}\n",
        output.policy.allow_high_risk
    ));
    text.push_str(&format!(
        "allow_persistent_effects: {}\n",
        output.policy.allow_persistent_effects
    ));
    text.push_str(&format!(
        "allow_cpu_power_on_battery: {}\n",
        output.policy.allow_cpu_power_on_battery
    ));
    text.push_str(&format!(
        "enabled_action_families: {}\n",
        if output.policy.enabled_action_families.is_empty() {
            "none".to_owned()
        } else {
            output
                .policy
                .enabled_action_families
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        }
    ));
    text.push_str(&format!(
        "denied_action_families: {}\n",
        if output.policy.denied_action_families.is_empty() {
            "none".to_owned()
        } else {
            output
                .policy
                .denied_action_families
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        }
    ));
    text.push_str(&format!(
        "min_confidence: {:.3}\n",
        output.policy.min_confidence
    ));
    text.push_str("\nHealth guardrails\n");
    text.push_str("-----------------\n");
    text.push_str(&format!(
        "max_cpu_temp_celsius: {}\n",
        output.config.health.max_cpu_temp_celsius
    ));
    text.push_str(&format!(
        "max_gpu_temp_celsius: {}\n",
        output.config.health.max_gpu_temp_celsius
    ));
    text.push_str(&format!(
        "min_disk_available_bytes: {}\n",
        output.config.health.min_disk_available_bytes
    ));
    text.push_str(&format!(
        "max_memory_pressure_some_avg10_percent: {:.3}\n",
        output.config.health.max_memory_pressure_some_avg10_percent
    ));
    text.push_str(&format!(
        "remote_apply_allowed: {}\n",
        output.policy.remote_apply.allow_remote_apply
    ));
    text.push_str(&format!(
        "agent_limits.max_mode: {}\n",
        output.agent_autotune_limits.max_mode
    ));
    text.push_str(&format!(
        "agent_limits.max_safety_class: {:?}\n",
        output.agent_autotune_limits.max_safety_class
    ));
    text.push_str(&format!(
        "agent_limits.max_targets: {}\n",
        output.agent_autotune_limits.max_targets
    ));
    text.push_str("\nExplanation\n");
    text.push_str("-----------\n");
    text.push_str(&format!(
        "verdict: {}\n",
        output.explanation.verdict.as_str()
    ));
    text.push_str(&format!("decision: {:?}\n", output.explanation.decision));
    text.push_str(&format!("intent: {:?}\n", output.explanation.intent));
    text.push_str(&format!(
        "final_reason: {}\n",
        output.explanation.final_reason
    ));
    text.push_str("evaluated_rules:\n");

    for rule in &output.explanation.evaluated_rules {
        let status = if rule.passed { "passed" } else { "failed" };
        text.push_str(&format!(
            "  - {}: {} - {}\n",
            rule.rule, status, rule.reason
        ));
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::DaemonPreset;

    #[test]
    fn daemon_config_explain_text_contains_effective_policy_and_rules() {
        let output = build_config_explain_output_from_user_config(None, None).unwrap();

        let text = render_config_explain_text(&output);

        assert!(text.contains("Daemon config explanation"));
        assert!(text.contains("Effective policy"));
        assert!(text.contains("preset: observe-only"));
        assert!(text.contains("mode: observe"));
        assert!(text.contains("max_safety_class: ObserveOnly"));
        assert!(text.contains("verdict: allow"));
        assert!(text.contains("final_reason: action is allowed by daemon policy"));
        assert!(text.contains("intent_allowed"));
    }

    #[test]
    fn daemon_config_explain_json_contains_config_policy_and_explanation() {
        let output = build_config_explain_output_from_user_config(None, None).unwrap();

        let json = render_config_explain_json(&output).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["config"]["mode"], "observe");
        assert_eq!(value["config"]["preset"], "observe-only");
        assert_eq!(value["config"]["source"], "cli");
        assert_eq!(value["policy"]["mode"], "observe");
        assert_eq!(value["explanation"]["action_kind"], "daemon-config-explain");
        assert_eq!(value["explanation"]["verdict"], "allow");
        assert_eq!(
            value["explanation"]["final_reason"],
            "action is allowed by daemon policy"
        );
        assert_eq!(value["agent_autotune_limits"]["max_mode"], "apply-low-risk");
    }

    #[test]
    fn daemon_config_explain_loads_agent_limits_from_user_config() {
        let user_config = UserConfigFile {
            daemon_preset: Some("gaming-laptop-safe".to_owned()),
            agent: Some(crate::config_file::AgentConfigFile {
                autotune_limits: Some(crate::config_file::AgentAutotuneLimitsFile {
                    max_candidate_window_seconds: Some(60),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };

        let output =
            build_config_explain_output_from_user_config(Some(&user_config), None).unwrap();

        assert!(output.user_config_loaded);
        assert_eq!(output.config.preset, DaemonPreset::GamingLaptopSafe);
        assert_eq!(
            output.agent_autotune_limits.max_candidate_window_seconds,
            60
        );
    }

    #[test]
    fn daemon_config_explain_applies_safe_user_policy_overrides() {
        let user_config = UserConfigFile {
            daemon_preset: Some("gaming-low-risk".to_owned()),
            daemon_enabled_action_families: Some(vec!["cpu_affinity_profile".to_owned()]),
            daemon_denied_action_families: Some(vec!["ionice".to_owned()]),
            daemon_min_confidence: Some(0.93),
            daemon_max_cpu_temp_celsius: Some(81),
            daemon_max_gpu_temp_celsius: Some(82),
            daemon_min_disk_available_bytes: Some(2_000_000_000),
            daemon_max_memory_pressure_some_avg10_percent: Some(15.5),
            ..Default::default()
        };

        let output =
            build_config_explain_output_from_user_config(Some(&user_config), None).unwrap();

        assert!(
            output
                .policy
                .enabled_action_families
                .contains("cpu_affinity_profile")
        );
        assert!(output.policy.denied_action_families.contains("ionice"));
        assert_eq!(output.policy.min_confidence, 0.93);
        assert_eq!(output.config.health.max_cpu_temp_celsius, 81);
        assert_eq!(output.config.health.max_gpu_temp_celsius, 82);
        assert_eq!(output.config.health.min_disk_available_bytes, 2_000_000_000);
        assert_eq!(
            output
                .config
                .health
                .thresholds()
                .max_memory_pressure_some_avg10_millipercent,
            15_500
        );

        let text = render_config_explain_text(&output);
        assert!(text.contains("denied_action_families: ionice"));
        assert!(text.contains("min_confidence: 0.930"));
        assert!(text.contains("max_cpu_temp_celsius: 81"));
        assert!(text.contains("min_disk_available_bytes: 2000000000"));
    }

    #[test]
    fn daemon_config_explain_rejects_unguarded_experimental_policy_overrides() {
        let user_config = UserConfigFile {
            daemon_preset: Some("gaming-low-risk".to_owned()),
            daemon_allow_system_wide_suggestions: Some(true),
            daemon_allow_system_wide_apply: Some(true),
            ..Default::default()
        };

        let err = build_config_explain_output_from_user_config(Some(&user_config), None)
            .unwrap_err()
            .to_string();

        assert!(err.contains("experimental = true"));
    }

    #[test]
    fn daemon_config_explain_cli_preset_overrides_user_file_preset() {
        let user_config = UserConfigFile {
            daemon_preset: Some("gaming-laptop-safe".to_owned()),
            ..Default::default()
        };

        let output =
            build_config_explain_output_from_user_config(Some(&user_config), Some("observe-only"))
                .unwrap();

        assert_eq!(output.config.preset, DaemonPreset::ObserveOnly);
        assert_eq!(output.config.mode, crate::daemon::DaemonMode::Observe);
    }

    #[test]
    fn daemon_config_explain_can_render_low_risk_preset() {
        let output =
            build_config_explain_output_from_user_config(None, Some("gaming-low-risk")).unwrap();

        assert_eq!(output.config.preset, DaemonPreset::GamingLowRisk);
        assert_eq!(output.config.mode, crate::daemon::DaemonMode::ApplyLowRisk);
        assert!(
            output
                .policy
                .enabled_action_families
                .contains("cpu_affinity_profile")
        );
        assert!(output.policy.min_confidence >= 0.85);

        let text = render_config_explain_text(&output);
        assert!(text.contains("preset: gaming-low-risk"));
        assert!(text.contains("enabled_action_families: cpu_affinity_profile"));
    }
}
