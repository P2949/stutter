use crate::{
    commands::input::{BenchCommandInput, MonitorCommandInput},
    remote,
    session::run_monitor,
};

pub async fn run_monitor_command(input: MonitorCommandInput) -> anyhow::Result<()> {
    let config = crate::config::effective::resolve_arc_monitor_config(input.config)?;

    if let Some(remote) = config.remote.as_deref() {
        let request = remote::request_from_monitor_config(&config)?;
        remote::run_remote_monitor(remote, request).await?;
        Ok(())
    } else {
        run_monitor(config, None, None, None).await.map(|_| ())
    }
}

pub async fn run_bench_command(input: BenchCommandInput) -> anyhow::Result<()> {
    let config = crate::config::effective::resolve_arc_monitor_config(input.config)?;

    run_monitor(config, None, None, None).await?;
    if input.role == "baseline" {
        println!(
            "bench complete role=baseline run_name={} next=\"run tune, then stutter recommend --baseline <run-dir> --tune <tune-dir>\"",
            input.run_name
        );
    } else {
        println!(
            "bench complete role=current run_name={} next=\"use stutter report --diff <baseline-run-dir> <current-run-dir>\"",
            input.run_name
        );
    }
    Ok(())
}
