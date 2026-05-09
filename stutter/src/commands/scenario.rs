use std::{path::PathBuf, sync::Arc};

use crate::{scenario, session::run_monitor};

pub fn run_create_command(
    name: String,
    force: bool,
    watch_process: Option<String>,
    duration: u64,
    preset: String,
    mangohud_log: Option<PathBuf>,
    notes: Option<String>,
) -> anyhow::Result<()> {
    let path = scenario::create_scenario(scenario::ScenarioCreateInput {
        name: name.clone(),
        force,
        watch_process,
        duration,
        preset,
        mangohud_log,
        notes,
    })?;
    println!("created scenario {} at {}", name, path.display());
    println!("edit notes/expected_classes before running if needed");
    Ok(())
}

pub async fn run_scenario_command(
    name: String,
    role: String,
    dry_run: bool,
    out_dir: Option<PathBuf>,
    mangohud_log_override: Option<PathBuf>,
) -> anyhow::Result<()> {
    let role = scenario::ScenarioRole::parse(&role)?;
    let prepared = scenario::prepare_scenario_run(scenario::ScenarioRunInput {
        name,
        role,
        dry_run,
        out_dir,
        mangohud_log_override,
    })?;

    if prepared.dry_run {
        print!("{}", prepared.dry_run_text);
        return Ok(());
    }

    println!("{}", prepared.start_text);
    let record = prepared.record.clone();
    let config = Arc::new(prepared.config);

    match run_monitor(config, None, None, None).await {
        Ok(_) => {
            scenario::append_run_record(&record)?;
            println!("scenario run complete: {}", record.run_dir.display());
            Ok(())
        }
        Err(err) => Err(err),
    }
}

pub fn run_compare_command(
    name: String,
    baseline: Option<PathBuf>,
    current: Option<PathBuf>,
    top: usize,
    json_summary: bool,
    validate: bool,
) -> anyhow::Result<()> {
    scenario::compare_scenario(scenario::ScenarioCompareInput {
        name,
        baseline,
        current,
        top,
        json_summary,
        validate,
    })
}

pub fn run_path_command(name: String) -> anyhow::Result<()> {
    let path = scenario::scenario_path(&name)?;
    println!("{}", path.display());
    Ok(())
}

pub fn run_list_command() -> anyhow::Result<()> {
    scenario::list_scenarios()
}
