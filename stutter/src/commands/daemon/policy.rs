use serde::Serialize;

use super::config::build_config_explain_output_from_user_config;
use crate::{
    config_file::{self, UserConfigFile},
    daemon::{DaemonConfig, DaemonPolicy, DaemonPolicyExplanation},
};

#[derive(Clone, Debug, Serialize)]
pub struct DaemonPolicyExplainOutput {
    pub config: DaemonConfig,
    pub policy: DaemonPolicy,
    pub explanation: DaemonPolicyExplanation,
    pub user_config_loaded: bool,
}

pub fn run_policy_explain_command(
    input: crate::commands::input::DaemonPolicyExplainCommandInput,
) -> anyhow::Result<()> {
    let user_config = config_file::load_user_config()?;
    let output = build_policy_explain_output_from_user_config(
        user_config.as_ref(),
        input.preset.as_deref(),
    )?;

    if input.json {
        println!("{}", render_policy_explain_json(&output)?);
    } else {
        print!("{}", render_policy_explain_text(&output));
    }

    Ok(())
}

pub fn build_policy_explain_output_from_user_config(
    user_config: Option<&UserConfigFile>,
    preset: Option<&str>,
) -> anyhow::Result<DaemonPolicyExplainOutput> {
    let config_output = build_config_explain_output_from_user_config(user_config, preset)?;
    let explanation = DaemonPolicyExplanation::from_policy(&config_output.policy);

    Ok(DaemonPolicyExplainOutput {
        config: config_output.config,
        policy: config_output.policy,
        explanation,
        user_config_loaded: config_output.user_config_loaded,
    })
}

pub fn render_policy_explain_json(output: &DaemonPolicyExplainOutput) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(output)?)
}

pub fn render_policy_explain_text(output: &DaemonPolicyExplainOutput) -> String {
    let mut text = String::new();

    text.push_str("Daemon policy explanation\n");
    text.push_str("=========================\n");
    text.push_str(&format!(
        "user_config_loaded: {}\n",
        output.user_config_loaded
    ));
    text.push_str(&format!("preset: {}\n", output.config.preset));
    text.push_str(&format!("mode: {}\n", output.policy.mode));
    text.push_str(&format!("source: {:?}\n", output.policy.source));
    text.push_str(&format!(
        "max_safety_class: {:?}\n",
        output.policy.max_safety_class
    ));
    text.push_str(&format!(
        "min_confidence: {:.3}\n",
        output.policy.min_confidence
    ));
    text.push_str("\nPolicy decisions\n");
    text.push_str("----------------\n");

    for line in &output.explanation.lines {
        text.push_str(&format!(
            "- {}: {} - {}\n",
            line.rule, line.outcome, line.reason
        ));
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_policy_explain_text_contains_action_level_decisions() {
        let output =
            build_policy_explain_output_from_user_config(None, Some("gaming-low-risk")).unwrap();

        let text = render_policy_explain_text(&output);

        assert!(text.contains("Daemon policy explanation"));
        assert!(text.contains("preset: gaming-low-risk"));
        assert!(text.contains("action:apply_low_risk_cpu_affinity: allowed"));
        assert!(text.contains("action:apply_without_rollback:rollback_available: failed"));
        assert!(!text.contains("later patch"));
    }

    #[test]
    fn daemon_policy_explain_json_contains_policy_lines() {
        let output = build_policy_explain_output_from_user_config(None, None).unwrap();

        let json = render_policy_explain_json(&output).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["config"]["preset"], "observe-only");
        assert_eq!(value["policy"]["mode"], "observe");
        let lines = value["explanation"]["lines"].as_array().unwrap();
        assert!(lines.iter().any(|line| {
            line["rule"] == "action:observe_status" && line["outcome"] == "allowed"
        }));
        assert!(lines.iter().any(|line| {
            line["rule"] == "action:apply_low_risk_cpu_affinity"
                && line["outcome"] == "rejected:intent_not_allowed"
        }));
    }
}
