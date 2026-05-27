use std::{fs, path::PathBuf};

use anyhow::{Context, Result};

use super::{io::*, model::*};

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
