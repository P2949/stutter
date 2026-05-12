use std::sync::Arc;

use crate::{commands::input, scenario, session::run_monitor};

pub fn run_create_command(input: input::ScenarioCreateCommandInput) -> anyhow::Result<()> {
    let path = scenario::create_scenario(scenario::ScenarioCreateInput {
        name: input.name.clone(),
        force: input.force,
        watch_process: input.watch_process,
        duration: input.duration,
        preset: input.preset,
        mangohud_log: input.mangohud_log,
        notes: input.notes,
    })?;
    println!("created scenario {} at {}", input.name, path.display());
    println!("edit notes/expected_classes before running if needed");
    Ok(())
}

pub async fn run_scenario_command(input: input::ScenarioRunCommandInput) -> anyhow::Result<()> {
    let role = scenario::ScenarioRole::parse(&input.role)?;
    let prepared = scenario::prepare_scenario_run(scenario::ScenarioRunInput {
        name: input.name,
        role,
        dry_run: input.dry_run,
        out_dir: input.out_dir,
        mangohud_log_override: input.mangohud_log_override,
    })?;

    if prepared.dry_run {
        print!("{}", prepared.dry_run_text);
        return Ok(());
    }

    println!("{}", prepared.start_text);
    let record = prepared.record.clone();
    let config = Arc::new(crate::config::effective::resolve_monitor_config(
        &prepared.config,
    )?);

    match run_monitor(config, None, None, None).await {
        Ok(_) => {
            scenario::append_run_record(&record)?;
            println!("scenario run complete: {}", record.run_dir.display());
            Ok(())
        }
        Err(err) => Err(err),
    }
}

pub fn run_compare_command(input: input::ScenarioCompareCommandInput) -> anyhow::Result<()> {
    scenario::compare_scenario(scenario::ScenarioCompareInput {
        name: input.name,
        baseline: input.baseline,
        current: input.current,
        top: input.top,
        json_summary: input.json_summary,
        validate: input.validate,
    })
}

pub fn run_path_command(input: input::ScenarioPathCommandInput) -> anyhow::Result<()> {
    let path = scenario::scenario_path(&input.name)?;
    println!("{}", path.display());
    Ok(())
}

pub fn run_list_command(_input: input::ScenarioListCommandInput) -> anyhow::Result<()> {
    scenario::list_scenarios()
}
