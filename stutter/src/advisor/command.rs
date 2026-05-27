use std::{collections::BTreeSet, time::Duration};

use tokio::time::sleep;

use super::{
    engine::build_advisor_report,
    models::{AdvisorCommandInput, AdvisorReport},
    render::render_advisor_report,
    scanner::{completed_run_dirs, default_runs_dir},
};

pub async fn advisor_command(input: AdvisorCommandInput) -> anyhow::Result<()> {
    if input.watch_runs {
        return watch_runs(input).await;
    }
    let run = input
        .run
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("advisor requires --run unless --watch-runs is set"))?;
    let report = build_advisor_report(run, input.profiles.as_deref())?;
    print_report(&report, input.json)?;
    Ok(())
}
async fn watch_runs(input: AdvisorCommandInput) -> anyhow::Result<()> {
    let runs_dir = input.runs_dir.unwrap_or_else(default_runs_dir);
    let mut processed = BTreeSet::new();
    loop {
        let runs = completed_run_dirs(&runs_dir, &processed)?;
        for run in runs {
            match build_advisor_report(&run, input.profiles.as_deref()) {
                Ok(report) => {
                    print_report(&report, input.json)?;
                    processed.insert(run);
                }
                Err(err) => {
                    log::warn!(
                        "advisor_watch_run_load_failed run={} err={err:#}",
                        run.display()
                    );
                }
            }
        }
        if input.once {
            return Ok(());
        }
        sleep(Duration::from_secs(input.poll_seconds)).await;
    }
}
fn print_report(report: &AdvisorReport, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
    } else {
        print!("{}", render_advisor_report(report));
    }
    Ok(())
}
