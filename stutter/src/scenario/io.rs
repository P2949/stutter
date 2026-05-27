use std::{fs, path::PathBuf};

use anyhow::{Context, Result};

use super::model::*;

pub fn default_scenario_dir() -> PathBuf {
    let mut path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    path.push(".config");
    path.push("stutter");
    path.push("scenarios");
    path
}

pub fn scenario_path(name: &str) -> Result<PathBuf> {
    validate_scenario_name(name)?;
    Ok(default_scenario_dir().join(format!("{name}.toml")))
}

pub fn default_scenario_state_dir(name: &str) -> Result<PathBuf> {
    validate_scenario_name(name)?;
    let mut path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    path.push(".local");
    path.push("state");
    path.push("stutter");
    path.push("scenarios");
    path.push(name);
    Ok(path)
}

pub fn load_scenario(name: &str) -> Result<ScenarioFile> {
    let path = scenario_path(name)?;
    let data = fs::read_to_string(&path)
        .with_context(|| format!("failed to read scenario {}", path.display()))?;
    let scenario: ScenarioFile = toml::from_str(&data)
        .with_context(|| format!("failed to parse scenario {}", path.display()))?;
    if scenario.name != name {
        anyhow::bail!(
            "scenario file name mismatch: requested {name}, file contains {}",
            scenario.name
        );
    }
    scenario.validate()?;
    Ok(scenario)
}

pub fn append_run_record(record: &ScenarioRunRecord) -> Result<()> {
    // run_dir is state_dir / role / run-nanos
    let state_dir = record
        .run_dir
        .parent()
        .context("run_dir has no parent")?
        .parent()
        .context("run_dir has no grandparent")?;

    let index_path = state_dir.join("runs.json");
    let mut index = if index_path.exists() {
        let data = fs::read_to_string(&index_path)?;
        serde_json::from_str(&data)?
    } else {
        ScenarioRunsIndex {
            scenario: state_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_owned(),
            runs: Vec::new(),
        }
    };

    index.runs.push(record.clone());
    let data = serde_json::to_string_pretty(&index)?;
    fs::create_dir_all(state_dir)?;
    fs::write(&index_path, data)?;

    Ok(())
}
