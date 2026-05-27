use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::process_tree::TaskClass;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioFile {
    pub name: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch_process: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_pid: Option<u32>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pid: Vec<u32>,

    pub duration: u64,

    #[serde(default = "default_scenario_preset")]
    pub preset: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mangohud_log: Option<PathBuf>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_classes: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,

    #[serde(default = "default_true")]
    pub persistent: bool,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_comm: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_comm: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_ms: Option<u64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spike_us: Option<u64>,

    #[serde(default)]
    pub irq_latency: bool,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub irqs: Vec<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hwmon: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_freq: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faults: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_io: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stat_wait: Option<bool>,
}

fn default_scenario_preset() -> String {
    "diagnosis".to_owned()
}

fn default_true() -> bool {
    true
}

pub fn validate_scenario_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("scenario name must not be empty");
    }
    if name.len() > 64 {
        anyhow::bail!("scenario name is too long (max 64 characters)");
    }
    for c in name.chars() {
        if !c.is_ascii_alphanumeric() && c != '_' && c != '-' {
            anyhow::bail!(
                "scenario name contains invalid characters: only ASCII letters, digits, '_' and '-' are allowed"
            );
        }
    }
    Ok(())
}

impl ScenarioFile {
    pub fn validate(&self) -> Result<()> {
        validate_scenario_name(&self.name)?;
        if self.duration == 0 {
            anyhow::bail!("scenario duration must be greater than zero");
        }

        let has_target =
            self.watch_process.is_some() || self.tree_pid.is_some() || !self.pid.is_empty();

        if !has_target {
            anyhow::bail!("scenario requires watch_process, tree_pid, or pid");
        }

        if self
            .watch_process
            .as_deref()
            .is_some_and(|s| s.trim().is_empty())
        {
            anyhow::bail!("watch_process must not be empty");
        }

        if self.irq_latency && self.irqs.is_empty() {
            anyhow::bail!("irq_latency requires at least one irq");
        }

        for class in &self.expected_classes {
            if TaskClass::from_str_opt(class).is_none() {
                anyhow::bail!("unknown expected task class: {class}");
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioRole {
    Baseline,
    Current,
}

impl ScenarioRole {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "baseline" => Ok(Self::Baseline),
            "current" => Ok(Self::Current),
            _ => anyhow::bail!("role must be baseline or current"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Current => "current",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScenarioRunsIndex {
    pub scenario: String,
    pub runs: Vec<ScenarioRunRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioRunRecord {
    pub role: ScenarioRole,
    pub run_dir: PathBuf,
    pub run_name: String,
    pub unix_nanos: u128,
    pub duration: u64,
    pub notes: Option<String>,
}

#[derive(Serialize)]
pub struct ScenarioCompareJson {
    pub scenario: String,
    pub baseline: PathBuf,
    pub current: PathBuf,
    pub diff: crate::report::diff::RunDiffSummary,
    pub expected_class_check: ExpectedClassCheck,
}

#[derive(Serialize)]
pub struct ExpectedClassCheck {
    pub baseline_missing: Vec<String>,
    pub current_missing: Vec<String>,
}
