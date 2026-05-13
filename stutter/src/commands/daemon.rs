use serde::Serialize;

use crate::{
    actions::{ActionId, SafetyClass},
    commands::input,
    config_file::{self, UserConfigFile},
    daemon::{
        ActionDescriptor, ActionEffectScope, DaemonConfig, DaemonPolicy, DaemonPolicyBuildInput,
        PolicyExplanation, PolicyIntent, RollbackRequirement, build_daemon_policy,
    },
    remote::AgentAutotuneLimits,
};

#[derive(Clone, Debug, Serialize)]
struct DaemonConfigExplainOutput {
    config: DaemonConfig,
    policy: DaemonPolicy,
    explanation: PolicyExplanation,
    agent_autotune_limits: AgentAutotuneLimits,
    user_config_loaded: bool,
}

pub fn run_config_explain_command(
    input: input::DaemonConfigExplainCommandInput,
) -> anyhow::Result<()> {
    let user_config = config_file::load_user_config()?;
    let output = build_config_explain_output_from_user_config(user_config.as_ref())?;

    if input.json {
        println!("{}", render_config_explain_json(&output)?);
    } else {
        print!("{}", render_config_explain_text(&output));
    }

    Ok(())
}

fn build_config_explain_output_from_user_config(
    user_config: Option<&UserConfigFile>,
) -> anyhow::Result<DaemonConfigExplainOutput> {
    let config = DaemonConfig::default();
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

fn daemon_config_explain_descriptor() -> ActionDescriptor {
    ActionDescriptor {
        action_id: ActionId("daemon-config-explain".to_owned()),
        action_kind: "daemon-config-explain".to_owned(),
        safety_class: SafetyClass::ObserveOnly,
        effect_scope: ActionEffectScope::ObserveOnly,
        rollback: RollbackRequirement::NotRequiredForDryRun,
        persistent_effect: false,
        touches_system_wide_state: false,
        requires_explicit_target: false,
        confidence: None,
    }
}

fn render_config_explain_json(output: &DaemonConfigExplainOutput) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(output)?)
}

fn render_config_explain_text(output: &DaemonConfigExplainOutput) -> String {
    let mut text = String::new();

    text.push_str("Daemon config explanation\n");
    text.push_str("=========================\n");
    text.push_str(&format!(
        "user_config_loaded: {}\n",
        output.user_config_loaded
    ));
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
        "allow_system_wide_actions: {}\n",
        output.policy.allow_system_wide_actions
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
        "min_confidence: {:.3}\n",
        output.policy.min_confidence
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

    #[test]
    fn daemon_config_explain_text_contains_effective_policy_and_rules() {
        let output = build_config_explain_output_from_user_config(None).unwrap();

        let text = render_config_explain_text(&output);

        assert!(text.contains("Daemon config explanation"));
        assert!(text.contains("Effective policy"));
        assert!(text.contains("mode: observe"));
        assert!(text.contains("max_safety_class: ObserveOnly"));
        assert!(text.contains("final_reason: action is allowed by daemon policy"));
        assert!(text.contains("intent_allowed"));
    }

    #[test]
    fn daemon_config_explain_json_contains_config_policy_and_explanation() {
        let output = build_config_explain_output_from_user_config(None).unwrap();

        let json = render_config_explain_json(&output).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["config"]["mode"], "observe");
        assert_eq!(value["config"]["source"], "cli");
        assert_eq!(value["policy"]["mode"], "observe");
        assert_eq!(value["explanation"]["action_kind"], "daemon-config-explain");
        assert_eq!(
            value["explanation"]["final_reason"],
            "action is allowed by daemon policy"
        );
        assert_eq!(value["agent_autotune_limits"]["max_mode"], "apply-low-risk");
    }

    #[test]
    fn daemon_config_explain_loads_agent_limits_from_user_config() {
        let user_config = UserConfigFile {
            agent: Some(crate::config_file::AgentConfigFile {
                autotune_limits: Some(crate::config_file::AgentAutotuneLimitsFile {
                    max_candidate_window_seconds: Some(60),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };

        let output = build_config_explain_output_from_user_config(Some(&user_config)).unwrap();

        assert!(output.user_config_loaded);
        assert_eq!(
            output.agent_autotune_limits.max_candidate_window_seconds,
            60
        );
    }
}
