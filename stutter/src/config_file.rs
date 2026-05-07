use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

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
