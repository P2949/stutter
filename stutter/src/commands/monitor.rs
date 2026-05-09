use std::sync::Arc;

use crate::{cli::Config, remote, session::run_monitor};

pub async fn run_monitor_command(config: Arc<Config>) -> anyhow::Result<()> {
    if let Some(remote) = config.remote.as_deref() {
        let request = remote::request_from_monitor_config(&config)?;
        remote::run_remote_monitor(remote, request).await?;
        Ok(())
    } else {
        run_monitor(config, None, None, None).await.map(|_| ())
    }
}

pub async fn run_bench_command(
    config: Arc<Config>,
    role: String,
    run_name: String,
) -> anyhow::Result<()> {
    run_monitor(config, None, None, None).await?;
    if role == "baseline" {
        println!(
            "bench complete role=baseline run_name={} next=\"run tune, then stutter recommend --baseline <run-dir> --tune <tune-dir>\"",
            run_name
        );
    } else {
        println!(
            "bench complete role=current run_name={} next=\"use stutter report --diff <baseline-run-dir> <current-run-dir>\"",
            run_name
        );
    }
    Ok(())
}
