//! An explicitly configured, exact-process KPI observer. Its output goes only to diagnostics.
use std::{io::Read, os::unix::fs::OpenOptionsExt, path::Path};

use super::{AutotuneRuntime, AutotuneRuntimeConfig};
use crate::autotune::objective::source::{KpiCollector, KpiContract, KpiObservation};

pub(super) fn load(
    config: &AutotuneRuntimeConfig,
) -> (Option<KpiCollector>, Option<KpiObservation>) {
    let options = &config.daemon_config.autotune;
    if options.kpi_contract.is_none() && options.kpi_socket_dir.is_none() {
        return (None, None);
    }
    let result = (|| -> anyhow::Result<KpiCollector> {
        let contract_path = options
            .kpi_contract
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing contract"))?;
        let dir = options
            .kpi_socket_dir
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing socket directory"))?;
        let mut bytes = Vec::new();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(contract_path)?;
        anyhow::ensure!(
            file.metadata()?.is_file(),
            "KPI contract must be a regular file"
        );
        file.take(65_537).read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() <= 65_536, "KPI contract exceeds 64 KiB");
        let contract: KpiContract = serde_json::from_slice(&bytes)?;
        KpiCollector::bind(contract, dir)
    })();
    match result {
        Ok(collector) => (Some(collector), None),
        // Failure only withdraws this optional source. Runnable-only behavior still works.
        Err(error) => {
            log::warn!("KPI observer unavailable: {error}");
            let denial = error
                .downcast_ref::<crate::telemetry::proc_view::ClockNamespaceDenial>()
                .map_or("invalid_configuration", |denial| denial.code());
            (
                None,
                Some(KpiObservation {
                    contract_digest: String::new(),
                    source_session: String::new(),
                    binding: crate::autotune::objective::source::SourceBinding {
                        uid: crate::syscall::geteuid(),
                        pid: 0,
                        start_ticks: 0,
                    },
                    clock_namespace: None,
                    status: "unavailable_contract_or_socket".into(),
                    accepted_samples: 0,
                    denied_samples: 0,
                    last_denial: Some(denial.into()),
                    metrics: Vec::new(),
                    authorizes_mutation: false,
                }),
            )
        }
    }
}

impl AutotuneRuntime {
    pub fn begin_kpi_observation_study(
        &mut self,
        config: crate::autotune::live_experiment::KpiObservationStudyConfig,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.config.mode() == crate::daemon::policy::DaemonMode::Observe,
            "KPI studies require observation-only runtime mode"
        );
        let collector = self
            .kpi_collector
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no configured KPI collector"))?;
        anyhow::ensure!(
            collector.contract() == &config.contract,
            "study contract differs from the runtime source"
        );
        self.live_experiments.begin_kpi_observation_study(config)
    }

    pub fn capture_kpi_block(
        &self,
        request: crate::autotune::objective::source::KpiBlockRequest,
    ) -> anyhow::Result<crate::autotune::objective::source::ObservedKpiBlock> {
        self.kpi_collector
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no configured KPI collector"))?
            .capture_block(request)
    }

    pub fn observe_kpi_pair(
        &mut self,
        baseline: &crate::autotune::objective::source::ObservedKpiBlock,
        treatment: &crate::autotune::objective::source::ObservedKpiBlock,
    ) -> anyhow::Result<()> {
        self.live_experiments.observe_kpi_pair(baseline, treatment)
    }

    pub fn analyse_kpi_observation_study(
        &mut self,
        look: u32,
    ) -> anyhow::Result<crate::autotune::live_experiment::KpiObservationStudyEvidence> {
        self.live_experiments.analyse_kpi_observation_study(look)
    }

    pub(super) fn poll_kpi_source(
        &mut self,
        identity: Option<&crate::autotune::observation::WorkloadIdentity>,
    ) {
        let Some(collector) = self.kpi_collector.as_mut() else {
            return;
        };
        let target = identity
            .and_then(|identity| Some((identity.root_pid, identity.process_starttime_ticks?)));
        let now_ns = crate::syscall::clock_gettime_ns(libc::CLOCK_MONOTONIC).unwrap_or(0);
        self.kpi_observation = Some(collector.poll(
            target,
            Path::new("/proc"),
            now_ns,
            self.config.window_seconds.saturating_mul(1_000_000_000),
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::Write,
        os::unix::{fs::DirBuilderExt, net::UnixStream},
    };

    use super::*;

    #[test]
    fn runtime_tick_reports_kpis_but_preserves_the_observe_decision_and_empty_action_state() {
        let dir = std::env::temp_dir().join(format!("stutter-runtime-kpi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let pid = std::process::id();
        let live = crate::telemetry::proc_view::live_process(Path::new("/proc"), pid).unwrap();
        let contract = crate::autotune::objective::source::audio_xrun(
            crate::autotune::objective::source::SourceBinding {
                pid,
                uid: live.uid,
                start_ticks: live.start_ticks,
            },
        )
        .contract;
        let contract_path = dir.join("contract.json");
        std::fs::write(&contract_path, serde_json::to_vec(&contract).unwrap()).unwrap();
        let mut config = AutotuneRuntimeConfig::observe(None, Some(pid), None);
        config.history_log = None;
        config.daemon_config.autotune.kpi_contract = Some(contract_path);
        config.daemon_config.autotune.kpi_socket_dir = Some(dir.clone());
        let mut runtime = AutotuneRuntime::new(config);
        runtime
            .on_event(crate::session_events::MonitorEvent::Interval {
                elapsed_ms: 0,
                records: Vec::new(),
                drop_counters: Default::default(),
            })
            .unwrap();
        let mut stream = UnixStream::connect(dir.join("kpi.sock")).unwrap();
        let at = crate::syscall::clock_gettime_ns(libc::CLOCK_MONOTONIC).unwrap();
        for metric in ["deadline_miss_count", "xrun_count"] {
            writeln!(stream, "{{\"metric\":\"{metric}\",\"value\":0,\"monotonic_ns\":{at},\"work_unit\":\"fixed-work\",\"population\":1000,\"kind\":\"batch_sum\"}}").unwrap();
        }
        let mut observed = false;
        for tick in 0..20 {
            let entry = runtime
                .on_event(crate::session_events::MonitorEvent::Interval {
                    elapsed_ms: tick * 500,
                    records: Vec::new(),
                    drop_counters: Default::default(),
                })
                .unwrap()
                .unwrap();
            assert_eq!(entry.decision, "observed");
            let kpi = entry.kpi_source.unwrap();
            assert!(!kpi.authorizes_mutation);
            if kpi.accepted_samples == 2 {
                assert!(kpi.metrics[0].aggregate.is_some());
                assert!(kpi.metrics[1].aggregate.is_some());
                observed = true;
                break;
            }
        }
        assert!(observed);
        assert!(!runtime.has_active_experiment());
        assert!(!runtime.has_kept_actions());
        drop(runtime);
        assert!(!dir.join("kpi.sock").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unavailable_kpi_config_is_diagnostic_and_the_source_off_default_is_empty() {
        let config = AutotuneRuntimeConfig::observe(None, Some(std::process::id()), None);
        assert!(load(&config).0.is_none());
        assert!(load(&config).1.is_none());
        let mut invalid = config;
        invalid.daemon_config.autotune.kpi_contract = Some("/no/such/contract.json".into());
        invalid.daemon_config.autotune.kpi_socket_dir = Some("/no/such/directory".into());
        let (collector, report) = load(&invalid);
        assert!(collector.is_none());
        assert_eq!(report.unwrap().status, "unavailable_contract_or_socket");
    }
}
