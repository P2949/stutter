use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{config::model::MonitorConfig, process_tree::TaskClass};

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

pub struct ScenarioCreateInput {
    pub name: String,
    pub force: bool,
    pub watch_process: Option<String>,
    pub duration: u64,
    pub preset: String,
    pub mangohud_log: Option<PathBuf>,
    pub notes: Option<String>,
}

pub fn create_scenario(input: ScenarioCreateInput) -> Result<PathBuf> {
    validate_scenario_name(&input.name)?;
    let path = scenario_path(&input.name)?;

    if path.exists() && !input.force {
        anyhow::bail!("scenario already exists; pass --force to overwrite");
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create scenario directory {}", parent.display()))?;
    }

    let scenario = ScenarioFile {
        name: input.name.clone(),
        watch_process: input.watch_process.or_else(|| Some("Game.exe".to_owned())),
        tree_pid: None,
        pid: Vec::new(),
        duration: input.duration,
        preset: input.preset,
        mangohud_log: input.mangohud_log,
        expected_classes: vec![
            "Game".to_owned(),
            "GameScope".to_owned(),
            "Compositor".to_owned(),
        ],
        notes: input.notes.or_else(|| {
            Some(
                "TODO: describe the route and edit watch_process/tree_pid/pid before running"
                    .to_owned(),
            )
        }),
        persistent: true,
        include_comm: Vec::new(),
        exclude_comm: Vec::new(),
        summary_ms: None,
        spike_us: None,
        irq_latency: false,
        irqs: Vec::new(),
        hwmon: None,
        cpu_freq: None,
        faults: None,
        block_io: None,
        stat_wait: None,
    };

    let toml = toml::to_string_pretty(&scenario).context("failed to serialize scenario to TOML")?;
    fs::write(&path, toml)
        .with_context(|| format!("failed to write scenario file {}", path.display()))?;

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

pub struct ScenarioRunInput {
    pub name: String,
    pub role: ScenarioRole,
    pub dry_run: bool,
    pub out_dir: Option<PathBuf>,
    pub mangohud_log_override: Option<PathBuf>,
}

pub struct PreparedScenarioRun {
    pub config: MonitorConfig,
    pub record: ScenarioRunRecord,
    pub dry_run: bool,
    pub dry_run_text: String,
    pub start_text: String,
}

pub fn prepare_scenario_run(input: ScenarioRunInput) -> Result<PreparedScenarioRun> {
    let scenario = load_scenario(&input.name)?;
    let role = input.role;

    let state_dir = default_scenario_state_dir(&input.name)?;
    let timestamp = crate::audit::unix_nanos_now();
    let run_id = format!("run-{}", timestamp);

    let out_dir = if let Some(dir) = input.out_dir {
        dir
    } else {
        state_dir.join(role.as_str()).join(&run_id)
    };

    let run_name = format!("scenario-{}-{}", role.as_str(), scenario.name);

    // Build Config
    let preset = scenario.preset.parse::<crate::presets::Preset>()?;
    let preset_defaults = preset.defaults();

    fn merge_opt_bool(preset_val: Option<bool>, scenario_val: Option<bool>) -> bool {
        scenario_val.or(preset_val).unwrap_or(false)
    }

    let hwmon = merge_opt_bool(preset_defaults.hwmon, scenario.hwmon);
    let faults = merge_opt_bool(preset_defaults.faults, scenario.faults);
    let stat_wait = merge_opt_bool(preset_defaults.stat_wait, scenario.stat_wait);
    let block_io = merge_opt_bool(preset_defaults.block_io, scenario.block_io);
    let runtime_slices = preset_defaults.runtime_slices.unwrap_or(false);
    let irq_latency = scenario.irq_latency;

    let cpu_freq_config = scenario
        .cpu_freq
        .or(preset_defaults.cpu_freq)
        .unwrap_or(false);
    let cpu_freq = (cpu_freq_config || true) && scenario.cpu_freq.unwrap_or(true);

    let config = MonitorConfig {
        target: crate::config::model::TargetConfig {
            target_pids: scenario.pid.clone(),
            tree_pids: scenario.tree_pid.map(|p| vec![p]).unwrap_or_default(),
            include_comm: scenario.include_comm.clone(),
            exclude_comm: scenario.exclude_comm.clone(),
            watch_process: scenario.watch_process.clone(),
            persistent: scenario.persistent,
            max_tasks: 1024,
            ..Default::default()
        },
        timing: crate::config::model::TimingConfig {
            summary_period_ms: scenario.summary_ms.unwrap_or(1000),
            epoch_period_ms: scenario.summary_ms,
            spike_threshold_ns: scenario.spike_us.unwrap_or(1000) * 1000,
            max_duration: Some(Duration::from_secs(scenario.duration)),
        },
        probes: crate::config::model::ProbeConfig {
            irq_latency,
            irqs: scenario.irqs.clone(),
            hwmon,
            cpu_freq,
            faults,
            block_io,
            stat_wait,
            runtime_slices,
            ..Default::default()
        },
        recording: crate::config::model::RecordingConfig {
            run_name: Some(run_name.clone()),
            output_dir: Some(out_dir.clone()),
            ..Default::default()
        },
        mangohud: crate::config::model::MangoHudConfig {
            log: input
                .mangohud_log_override
                .or(scenario.mangohud_log.clone()),
            ..Default::default()
        },
        watch: crate::config::model::WatchConfig {
            poll_ms: 2000,
            ..Default::default()
        },
        ..Default::default()
    };

    let dry_run_text = format!(
        "scenario: {}\n\
         role: {}\n\
         duration: {}s\n\
         watch_process: {:?}\n\
         tree_pid: {:?}\n\
         pid: {:?}\n\
         preset: {}\n\
         output: {}\n\
         mangohud_log: {:?}\n\
         expected_classes: {:?}\n\
         notes: {}\n\
         effective collectors:\n\
           hwmon: {}\n\
           cpu_freq: {}\n\
           faults: {}\n\
           stat_wait: {}\n\
           block_io: {}\n\
           irq_latency: {} (irqs: {:?})\n\
         dry run: no recording started\n",
        scenario.name,
        role.as_str(),
        scenario.duration,
        scenario.watch_process,
        scenario.tree_pid,
        scenario.pid,
        scenario.preset,
        out_dir.display(),
        config.mangohud.log,
        scenario.expected_classes,
        scenario.notes.as_deref().unwrap_or(""),
        config.probes.hwmon,
        config.probes.cpu_freq,
        config.probes.faults,
        config.probes.stat_wait,
        config.probes.block_io,
        config.probes.irq_latency,
        config.probes.irqs,
    );

    let start_text = format!(
        "scenario: {}\n\
         role: {}\n\
         notes: {}\n\
         duration: {}s\n\
         output: {}\n\
         Start the route now; recording will follow watch_process {}.\n",
        scenario.name,
        role.as_str(),
        scenario.notes.as_deref().unwrap_or(""),
        scenario.duration,
        out_dir.display(),
        scenario.watch_process.as_deref().unwrap_or("None"),
    );

    let record = ScenarioRunRecord {
        role,
        run_dir: out_dir,
        run_name,
        unix_nanos: timestamp,
        duration: scenario.duration,
        notes: scenario.notes,
    };

    Ok(PreparedScenarioRun {
        config,
        record,
        dry_run: input.dry_run,
        dry_run_text,
        start_text,
    })
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

pub struct ScenarioCompareInput {
    pub name: String,
    pub baseline: Option<PathBuf>,
    pub current: Option<PathBuf>,
    pub top: usize,
    pub json_summary: bool,
    pub validate: bool,
}

#[derive(Serialize)]
pub struct ScenarioCompareJson {
    pub scenario: String,
    pub baseline: PathBuf,
    pub current: PathBuf,
    pub diff: crate::summary::RunDiffSummary,
    pub expected_class_check: ExpectedClassCheck,
}

#[derive(Serialize)]
pub struct ExpectedClassCheck {
    pub baseline_missing: Vec<String>,
    pub current_missing: Vec<String>,
}

pub fn compare_scenario(input: ScenarioCompareInput) -> Result<()> {
    let scenario = load_scenario(&input.name)?;
    let state_dir = default_scenario_state_dir(&input.name)?;
    let index_path = state_dir.join("runs.json");

    let index: ScenarioRunsIndex = if index_path.exists() {
        let data = fs::read_to_string(&index_path)?;
        serde_json::from_str(&data)?
    } else {
        ScenarioRunsIndex::default()
    };

    let baseline_path = if let Some(p) = input.baseline {
        p
    } else {
        index.runs.iter()
            .rfind(|r| r.role == ScenarioRole::Baseline)
            .map(|r| r.run_dir.clone())
            .ok_or_else(|| anyhow::anyhow!("no baseline run found for scenario {}; run `stutter scenario run {} --role baseline` first", input.name, input.name))?
    };

    let current_path = if let Some(p) = input.current {
        p
    } else {
        index.runs.iter()
            .rfind(|r| r.role == ScenarioRole::Current)
            .map(|r| r.run_dir.clone())
            .ok_or_else(|| anyhow::anyhow!("no current run found for scenario {}; run `stutter scenario run {} --role current` first", input.name, input.name))?
    };

    if input.validate {
        if !crate::validate::validate_run_for_command(&baseline_path, false).passed {
            anyhow::bail!(
                "baseline run validation failed for {}",
                baseline_path.display()
            );
        }
        if !crate::validate::validate_run_for_command(&current_path, false).passed {
            anyhow::bail!(
                "current run validation failed for {}",
                current_path.display()
            );
        }
    }

    let baseline_missing = missing_expected_classes(&baseline_path, &scenario.expected_classes)?;
    let current_missing = missing_expected_classes(&current_path, &scenario.expected_classes)?;

    if input.json_summary {
        let diff = crate::summary::build_run_diff_summary(&baseline_path, &current_path, None)?
            .limited(input.top);
        let output = ScenarioCompareJson {
            scenario: scenario.name,
            baseline: baseline_path,
            current: current_path,
            diff,
            expected_class_check: ExpectedClassCheck {
                baseline_missing,
                current_missing,
            },
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("stutter scenario compare");
        println!("========================");
        println!("scenario: {}", scenario.name);
        if let Some(notes) = &scenario.notes {
            println!("notes: {}", notes);
        }
        println!("baseline: {}", baseline_path.display());
        println!("current:  {}", current_path.display());
        println!();

        for class in &baseline_missing {
            println!("WARNING: baseline missing expected class {}", class);
        }
        for class in &current_missing {
            println!("WARNING: current missing expected class {}", class);
        }
        if !baseline_missing.is_empty() || !current_missing.is_empty() {
            println!();
        }

        crate::report::print_diff_report(&baseline_path, &current_path, input.top, None)?;
    }

    Ok(())
}

pub fn missing_expected_classes(run_dir: &Path, expected: &[String]) -> Result<Vec<String>> {
    let session = crate::session_io::load_session(run_dir)?;
    let mut present = BTreeSet::new();
    for task in &session.tasks {
        present.insert(format!("{:?}", task.class));
    }

    let mut missing = Vec::new();
    for class in expected {
        if !present.contains(class) {
            missing.push(class.clone());
        }
    }
    Ok(missing)
}

pub fn list_scenarios() -> Result<()> {
    let dir = default_scenario_dir();
    if !dir.exists() {
        return Ok(());
    }

    println!("Scenarios:");
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let extension = path.extension().and_then(|s| s.to_str());
        let stem = path.file_stem().and_then(|s| s.to_str());
        if let (Some("toml"), Some(name)) = (extension, stem) {
            println!("  - {}", name);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_name_rejects_path_traversal() {
        assert!(validate_scenario_name("../test").is_err());
        assert!(validate_scenario_name("test/foo").is_err());
        assert!(validate_scenario_name("test\\foo").is_err());
    }

    #[test]
    fn scenario_name_accepts_slug() {
        assert!(validate_scenario_name("kcd-route").is_ok());
        assert!(validate_scenario_name("kcd_route_v2").is_ok());
    }

    #[test]
    fn scenario_requires_positive_duration() {
        let mut s = ScenarioFile {
            name: "test".to_owned(),
            watch_process: Some("game".to_owned()),
            tree_pid: None,
            pid: Vec::new(),
            duration: 0,
            preset: "diagnosis".to_owned(),
            mangohud_log: None,
            expected_classes: Vec::new(),
            notes: None,
            persistent: true,
            include_comm: Vec::new(),
            exclude_comm: Vec::new(),
            summary_ms: None,
            spike_us: None,
            irq_latency: false,
            irqs: Vec::new(),
            hwmon: None,
            cpu_freq: None,
            faults: None,
            block_io: None,
            stat_wait: None,
        };
        assert!(s.validate().is_err());
        s.duration = 1;
        assert!(s.validate().is_ok());
    }

    #[test]
    fn scenario_requires_some_target() {
        let s = ScenarioFile {
            name: "test".to_owned(),
            watch_process: None,
            tree_pid: None,
            pid: Vec::new(),
            duration: 10,
            preset: "diagnosis".to_owned(),
            mangohud_log: None,
            expected_classes: Vec::new(),
            notes: None,
            persistent: true,
            include_comm: Vec::new(),
            exclude_comm: Vec::new(),
            summary_ms: None,
            spike_us: None,
            irq_latency: false,
            irqs: Vec::new(),
            hwmon: None,
            cpu_freq: None,
            faults: None,
            block_io: None,
            stat_wait: None,
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn scenario_rejects_unknown_expected_class() {
        let s = ScenarioFile {
            name: "test".to_owned(),
            watch_process: Some("game".to_owned()),
            tree_pid: None,
            pid: Vec::new(),
            duration: 10,
            preset: "diagnosis".to_owned(),
            mangohud_log: None,
            expected_classes: vec!["InvalidClass".to_owned()],
            notes: None,
            persistent: true,
            include_comm: Vec::new(),
            exclude_comm: Vec::new(),
            summary_ms: None,
            spike_us: None,
            irq_latency: false,
            irqs: Vec::new(),
            hwmon: None,
            cpu_freq: None,
            faults: None,
            block_io: None,
            stat_wait: None,
        };
        assert!(s.validate().is_err());
    }
}
