//! `stutter kpi-serve`: observe an application KPI over a confined local socket.
use std::{path::PathBuf, time::Duration};

#[derive(Debug)]
pub struct KpiServeCommandInput {
    pub dir: PathBuf,
    pub name: String,
    pub contract: PathBuf,
    pub window: Duration,
    pub connections: Option<usize>,
}

pub fn run_kpi_serve_command(input: KpiServeCommandInput) -> anyhow::Result<()> {
    use crate::autotune::objective::source::{KpiContract, serve_observer};
    let contract: KpiContract = serde_json::from_slice(&std::fs::read(&input.contract)?)
        .map_err(|error| anyhow::anyhow!("not a KPI contract: {error}"))?;
    contract
        .validate()
        .map_err(|error| anyhow::anyhow!("invalid KPI contract: {error:?}"))?;
    let listener =
        crate::telemetry::registration::bind(&input.dir, &input.name, crate::syscall::geteuid())
            .map_err(|error| anyhow::anyhow!("cannot bind the KPI socket: {error:?}"))?;
    let window_ns = u64::try_from(input.window.as_nanos()).unwrap_or(u64::MAX);
    let mut stdout = std::io::stdout().lock();
    serve_observer(
        contract,
        &listener,
        std::path::Path::new("/proc"),
        window_ns,
        input.connections,
        &mut stdout,
    )?;
    Ok(())
}
