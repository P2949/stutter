//! `stutter kpi-serve`: observe an application KPI over a confined local socket.
use std::{path::PathBuf, time::Duration};

#[derive(Debug)]
pub enum ContractSource {
    /// A JSON `KpiContract` file.
    File(PathBuf),
    /// An experimental template bound to one live process.
    Template { name: String, pid: u32 },
}

#[derive(Debug)]
pub struct KpiServeCommandInput {
    pub dir: PathBuf,
    pub name: String,
    pub source: ContractSource,
    pub window: Duration,
    pub connections: Option<usize>,
}

fn load_contract(
    source: &ContractSource,
) -> anyhow::Result<crate::autotune::objective::source::KpiContract> {
    use crate::autotune::objective::source::{
        KpiContract, SourceBinding, audio_xrun, browser_response,
    };
    match source {
        ContractSource::File(path) => serde_json::from_slice(&std::fs::read(path)?)
            .map_err(|error| anyhow::anyhow!("not a KPI contract: {error}")),
        ContractSource::Template { name, pid } => {
            let live =
                crate::telemetry::proc_view::live_process(std::path::Path::new("/proc"), *pid)
                    .ok_or_else(|| anyhow::anyhow!("process {pid} is not running"))?;
            anyhow::ensure!(
                live.uid == crate::syscall::geteuid(),
                "process {pid} belongs to another user"
            );
            let binding = SourceBinding {
                uid: live.uid,
                pid: *pid,
                start_ticks: live.start_ticks,
            };
            let template = match name.as_str() {
                "audio-xrun" => audio_xrun(binding),
                "browser-response" => browser_response(binding),
                other => {
                    anyhow::bail!("unknown template {other:?}; use audio-xrun or browser-response")
                }
            };
            Ok::<KpiContract, anyhow::Error>(template.contract)
        }
    }
}

pub fn run_kpi_serve_command(input: KpiServeCommandInput) -> anyhow::Result<()> {
    use crate::autotune::objective::source::serve_observer;
    let contract = load_contract(&input.source)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_binds_to_a_live_process_and_unknown_or_foreign_requests_are_refused() {
        let pid = std::process::id();
        let contract = load_contract(&ContractSource::Template {
            name: "audio-xrun".into(),
            pid,
        })
        .unwrap();
        assert_eq!(contract.binding.pid, pid);
        assert_eq!(contract.primary.id, "deadline_miss_count");
        assert!(
            load_contract(&ContractSource::Template {
                name: "nonsense".into(),
                pid
            })
            .is_err()
        );
        assert!(
            load_contract(&ContractSource::Template {
                name: "audio-xrun".into(),
                pid: u32::MAX - 1
            })
            .is_err()
        );
        assert!(load_contract(&ContractSource::File("/nonexistent".into())).is_err());
    }
}
