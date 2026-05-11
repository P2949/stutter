use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::{
    config::{FocusSource, ForegroundSource},
    remote::AgentAutotuneLimits,
};

#[derive(Debug, Clone, Deserialize, Default)]
pub struct UserConfigFile {
    pub summary_ms: Option<u64>,
    pub spike_us: Option<u64>,
    pub hwmon: Option<bool>,
    pub cpu_freq: Option<bool>,
    pub no_cpu_freq: Option<bool>,
    pub include_comm: Option<Vec<String>>,
    pub exclude_comm: Option<Vec<String>>,
    pub max_tasks: Option<usize>,
    pub retain_intervals: Option<usize>,
    pub foreground_window: Option<bool>,
    pub focus_source: Option<String>,
    pub foreground_source: Option<String>,
    pub foreground_poll_ms: Option<u64>,
    pub foreground_max_stale_ms: Option<u64>,
    pub foreground_include_title: Option<bool>,
    #[allow(dead_code)]
    pub community_rules: Option<CommunityRulesConfigFile>,
    pub agent: Option<AgentConfigFile>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CommunityRulesConfigFile {
    pub enabled: Option<bool>,
    pub sources: Option<Vec<String>>,
    pub paths: Option<Vec<PathBuf>>,
}

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
    pub allow_system_wide_actions: Option<bool>,
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
            allow_system_wide_actions: self
                .allow_system_wide_actions
                .unwrap_or(defaults.allow_system_wide_actions),
        };

        validate_agent_autotune_limits(&limits)?;
        Ok(limits)
    }
}

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

    if limits.allow_system_wide_actions {
        anyhow::bail!("agent.autotune_limits.allow_system_wide_actions must be false");
    }

    Ok(())
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

pub fn load_user_config() -> Result<Option<UserConfigFile>> {
    let Some(path) = resolve_user_config_path() else {
        return Ok(None);
    };

    if !path.exists() {
        return Ok(None);
    }

    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;

    let config = parse_user_config_toml(&contents)
        .with_context(|| format!("failed to parse config file {}", path.display()))?;

    Ok(Some(config))
}

pub fn parse_focus_source_value(value: &str) -> Result<FocusSource> {
    match value.trim().to_ascii_lowercase().as_str() {
        "heuristic" => Ok(FocusSource::Heuristic),
        "foreground" => Ok(FocusSource::Foreground),
        "hybrid" => Ok(FocusSource::Hybrid),
        other => anyhow::bail!(
            "invalid focus_source {:?}; valid values are heuristic, foreground, hybrid",
            other
        ),
    }
}

pub fn parse_foreground_source_value(value: &str) -> Result<ForegroundSource> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(ForegroundSource::Auto),
        "sway" => Ok(ForegroundSource::Sway),
        "hyprland" => Ok(ForegroundSource::Hyprland),
        "x11" => Ok(ForegroundSource::X11),
        other => anyhow::bail!(
            "invalid foreground_source {:?}; valid values are auto, sway, hyprland, x11",
            other
        ),
    }
}

pub fn parse_user_config_toml(contents: &str) -> Result<UserConfigFile> {
    Ok(toml::from_str::<UserConfigFile>(contents)?)
}

pub fn resolve_user_config_path() -> Option<PathBuf> {
    #[allow(clippy::collapsible_if)]
    if let Ok(path) = std::env::var("STUTTER_CONFIG") {
        if !path.trim().is_empty() {
            return Some(PathBuf::from(path));
        }
    }

    #[allow(clippy::collapsible_if)]
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.trim().is_empty() {
            return Some(PathBuf::from(xdg).join("stutter").join("config.toml"));
        }
    }

    #[allow(clippy::collapsible_if)]
    if let Ok(home) = std::env::var("HOME") {
        if !home.trim().is_empty() {
            return Some(
                PathBuf::from(home)
                    .join(".config")
                    .join("stutter")
                    .join("config.toml"),
            );
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_toml() {
        let toml = r#"
            summary_ms = 500
            spike_us = 1000
            hwmon = true
            cpu_freq = true
            include_comm = ["Game", "Render"]
        "#;
        let config = parse_user_config_toml(toml).unwrap();
        assert_eq!(config.summary_ms, Some(500));
        assert_eq!(config.spike_us, Some(1000));
        assert_eq!(config.hwmon, Some(true));
        assert_eq!(config.cpu_freq, Some(true));
        assert_eq!(config.include_comm.unwrap(), vec!["Game", "Render"]);
    }

    #[test]
    fn test_parse_focus_source_value() {
        assert_eq!(
            parse_focus_source_value("heuristic").unwrap(),
            FocusSource::Heuristic
        );
        assert_eq!(
            parse_focus_source_value("foreground").unwrap(),
            FocusSource::Foreground
        );
        assert_eq!(
            parse_focus_source_value("hybrid").unwrap(),
            FocusSource::Hybrid
        );
        assert!(parse_focus_source_value("invalid").is_err());
    }

    #[test]
    fn test_parse_foreground_source_value() {
        assert_eq!(
            parse_foreground_source_value("auto").unwrap(),
            ForegroundSource::Auto
        );
        assert_eq!(
            parse_foreground_source_value("sway").unwrap(),
            ForegroundSource::Sway
        );
        assert_eq!(
            parse_foreground_source_value("hyprland").unwrap(),
            ForegroundSource::Hyprland
        );
        assert_eq!(
            parse_foreground_source_value("x11").unwrap(),
            ForegroundSource::X11
        );
        assert!(parse_foreground_source_value("invalid").is_err());
    }

    #[test]
    fn test_parse_foreground_config_fields() {
        let toml = r#"
            foreground_window = true
            focus_source = "hybrid"
            foreground_source = "sway"
            foreground_poll_ms = 750
            foreground_max_stale_ms = 3000
            foreground_include_title = true
        "#;

        let config = parse_user_config_toml(toml).unwrap();

        assert_eq!(config.foreground_window, Some(true));
        assert_eq!(config.focus_source.as_deref(), Some("hybrid"));
        assert_eq!(config.foreground_source.as_deref(), Some("sway"));
        assert_eq!(config.foreground_poll_ms, Some(750));
        assert_eq!(config.foreground_max_stale_ms, Some(3000));
        assert_eq!(config.foreground_include_title, Some(true));
    }

    #[test]
    fn test_parse_community_rules_config_fields() {
        let toml = r#"
            [community_rules]
            enabled = true
            sources = ["user"]
            paths = ["/tmp/stutter/rules/custom.generated.json"]
        "#;

        let config = parse_user_config_toml(toml).unwrap();
        let community_rules = config.community_rules.unwrap();

        assert_eq!(community_rules.enabled, Some(true));
        assert_eq!(community_rules.sources.unwrap(), vec!["user"]);
        assert_eq!(
            community_rules.paths.unwrap(),
            vec![PathBuf::from("/tmp/stutter/rules/custom.generated.json")]
        );
    }

    #[test]
    fn test_community_rules_config_from_user_config_uses_parsed_section() {
        let toml = r#"
            [community_rules]
            enabled = false
            sources = []
            paths = ["/tmp/stutter/rules/custom.generated.json"]
        "#;

        let user_config = parse_user_config_toml(toml).unwrap();
        let community_rules = community_rules_config_from_user_config(Some(&user_config));

        assert!(!community_rules.enabled);
        assert_eq!(
            community_rules.explicit_rules_files,
            vec![PathBuf::from("/tmp/stutter/rules/custom.generated.json")]
        );
        assert!(community_rules.user_rules_dir.is_none());
        assert!(!community_rules.load_builtin_fixture);
    }

    #[test]
    fn test_parse_agent_autotune_limits() {
        let toml = r#"
            [agent.autotune_limits]
            max_active_controllers = 1
            max_safety_class = "ReversibleLowRisk"
            max_candidate_window_seconds = 120
            max_targets = 1
            allow_system_wide_actions = false
        "#;

        let config = parse_user_config_toml(toml).unwrap();
        let limits = agent_autotune_limits_from_user_config(Some(&config)).unwrap();

        assert_eq!(limits.max_active_controllers, 1);
        assert_eq!(
            limits.max_mode,
            crate::daemon_policy::DaemonMode::ApplyLowRisk
        );
        assert_eq!(
            limits.max_safety_class,
            crate::actions::SafetyClass::ReversibleLowRisk
        );
        assert_eq!(limits.max_candidate_window_seconds, 120);
        assert_eq!(limits.max_targets, 1);
        assert!(!limits.allow_system_wide_actions);
    }

    #[test]
    fn test_missing_agent_autotune_limits_uses_defaults() {
        let toml = r#"
            summary_ms = 500
        "#;

        let config = parse_user_config_toml(toml).unwrap();
        let limits = agent_autotune_limits_from_user_config(Some(&config)).unwrap();

        assert_eq!(limits, AgentAutotuneLimits::default());
    }

    #[test]
    fn test_agent_autotune_limits_reject_system_wide_actions() {
        let toml = r#"
            [agent.autotune_limits]
            allow_system_wide_actions = true
        "#;

        let config = parse_user_config_toml(toml).unwrap();
        let err = agent_autotune_limits_from_user_config(Some(&config))
            .unwrap_err()
            .to_string();

        assert!(err.contains("allow_system_wide_actions must be false"));
    }

    #[test]
    fn test_agent_autotune_limits_reject_too_many_targets() {
        let toml = r#"
            [agent.autotune_limits]
            max_targets = 2
        "#;

        let config = parse_user_config_toml(toml).unwrap();
        let err = agent_autotune_limits_from_user_config(Some(&config))
            .unwrap_err()
            .to_string();

        assert!(err.contains("max_targets = 1"));
    }

    #[test]
    fn test_agent_autotune_limits_reject_too_long_candidate_window() {
        let toml = r#"
            [agent.autotune_limits]
            max_candidate_window_seconds = 121
        "#;

        let config = parse_user_config_toml(toml).unwrap();
        let err = agent_autotune_limits_from_user_config(Some(&config))
            .unwrap_err()
            .to_string();

        assert!(err.contains("max_candidate_window_seconds must be <= 120"));
    }

    #[test]
    fn test_agent_autotune_limits_reject_high_risk_ceiling() {
        let toml = r#"
            [agent.autotune_limits]
            max_safety_class = "HighRisk"
        "#;

        let config = parse_user_config_toml(toml).unwrap();
        let err = agent_autotune_limits_from_user_config(Some(&config))
            .unwrap_err()
            .to_string();

        assert!(err.contains("apply-low-risk only") || err.contains("ReversibleLowRisk only"));
    }

    #[test]
    fn test_agent_autotune_limits_reject_high_mode_ceiling() {
        let toml = r#"
            [agent.autotune_limits]
            max_mode = "apply-medium-risk"
        "#;

        let config = parse_user_config_toml(toml).unwrap();
        let err = agent_autotune_limits_from_user_config(Some(&config))
            .unwrap_err()
            .to_string();

        assert!(err.contains("apply-low-risk only"));
    }

    #[test]
    fn test_agent_autotune_limits_reject_invalid_safety_class() {
        let toml = r#"
            [agent.autotune_limits]
            max_safety_class = "Invalid"
        "#;

        let config = parse_user_config_toml(toml).unwrap();
        let err = agent_autotune_limits_from_user_config(Some(&config)).unwrap_err();

        let err_str = err.to_string();
        assert!(err_str.contains("max_safety_class"));
    }

    #[test]
    fn test_parse_invalid_toml() {
        let toml = r#"
            summary_ms = "not a number"
        "#;
        let err = parse_user_config_toml(toml).unwrap_err();
        println!("Actual error: {}", err);
        assert!(
            err.to_string().to_lowercase().contains("integer")
                || err.to_string().to_lowercase().contains("invalid type")
        );
    }

    struct EnvGuard {
        key: &'static str,
        old: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let old = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(old) = &self.old {
                unsafe {
                    std::env::set_var(self.key, old);
                }
            } else {
                unsafe {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    #[test]
    fn test_stutter_config_env_honored() {
        let _guard = EnvGuard::set("STUTTER_CONFIG", "/tmp/stutter.toml");
        let path = resolve_user_config_path().unwrap();
        assert_eq!(path, PathBuf::from("/tmp/stutter.toml"));
    }
}
