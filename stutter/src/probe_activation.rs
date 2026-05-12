use std::collections::BTreeSet;

use serde::Serialize;

use crate::{
    artifacts::{ArtifactKind, artifact_is_ndjson_stream},
    config::{FocusSource, model::MonitorConfig},
    ebpf_loader::TracepointAvailability,
    probe_catalog::ProbeStatus,
    probe_registry::{
        DataQualityRule, EbpfProgramSpec, PROBE_REGISTRY, PerfEventSpec, ProbeKey, ProbeSpec,
        TracepointSpec, probe_spec,
    },
};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProbeDisabledReason {
    pub key: ProbeKey,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProbeActivationWarning {
    pub key: Option<ProbeKey>,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ProbeActivationPlan {
    pub enabled: Vec<&'static ProbeSpec>,
    pub disabled: Vec<ProbeDisabledReason>,
    pub warnings: Vec<ProbeActivationWarning>,
    attach_programs: BTreeSet<&'static str>,
    follow_exec: bool,
    faults: bool,
    stat_wait: bool,
}

impl ProbeActivationPlan {
    pub fn from_config(
        config: &MonitorConfig,
        tracepoints: &TracepointAvailability,
    ) -> anyhow::Result<Self> {
        let mut enabled = Vec::new();
        let mut disabled = Vec::new();
        let mut warnings = Vec::new();

        for spec in PROBE_REGISTRY {
            if spec.status == ProbeStatus::Planned {
                disabled.push(ProbeDisabledReason {
                    key: spec.key,
                    reason: "planned probe is not implemented".to_owned(),
                });
                continue;
            }

            if !probe_requested(spec.key, config) {
                disabled.push(ProbeDisabledReason {
                    key: spec.key,
                    reason: "probe was not requested by resolved monitor configuration".to_owned(),
                });
                continue;
            }

            if let Some(reason) = unavailable_reason(spec.key, config, tracepoints) {
                warnings.push(ProbeActivationWarning {
                    key: Some(spec.key),
                    message: reason.clone(),
                });
                disabled.push(ProbeDisabledReason {
                    key: spec.key,
                    reason,
                });
                continue;
            }

            enabled.push(spec);
        }

        if !tracepoints.sched_wakeup_new {
            warnings.push(ProbeActivationWarning {
                key: Some(ProbeKey::SchedulerRunnableLatency),
                message:
                    "sched_wakeup_new tracepoint unavailable; wakeup-new attribution is degraded"
                        .to_owned(),
            });
        }

        if !tracepoints.sched_process_exit {
            warnings.push(ProbeActivationWarning {
                key: Some(ProbeKey::SchedulerRunnableLatency),
                message: "sched_process_exit tracepoint unavailable; stale wakeup/fault cleanup on task exit is disabled"
                    .to_owned(),
            });
        }

        if !tracepoints.sched_migrate_task {
            warnings.push(ProbeActivationWarning {
                key: Some(ProbeKey::SchedulerRunnableLatency),
                message: "sched_migrate_task tracepoint unavailable; migration_events.json will not be populated"
                    .to_owned(),
            });
        }

        if config.safety.follow_exec && !tracepoints.sched_process_exec {
            warnings.push(ProbeActivationWarning {
                key: Some(ProbeKey::SchedulerRunnableLatency),
                message:
                    "sched_process_exec tracepoint unavailable; follow-exec cleanup may be degraded"
                        .to_owned(),
            });
        }

        if config.probes.stat_wait && !tracepoints.sched_stat_wait {
            warnings.push(ProbeActivationWarning {
                key: Some(ProbeKey::Faults),
                message: "sched_stat_wait tracepoint unavailable; stat-wait interval evidence is disabled"
                    .to_owned(),
            });
        }

        let attach_programs = attach_programs_for(&enabled, config, tracepoints);

        if !enabled
            .iter()
            .any(|spec| spec.key == ProbeKey::SchedulerRunnableLatency)
        {
            anyhow::bail!("mandatory scheduler runnable latency probe was not activated");
        }

        Ok(Self {
            enabled,
            disabled,
            warnings,
            attach_programs,
            follow_exec: config.safety.follow_exec,
            faults: config.probes.faults,
            stat_wait: config.probes.stat_wait,
        })
    }

    pub fn has_probe(&self, key: ProbeKey) -> bool {
        self.enabled.iter().any(|spec| spec.key == key)
    }

    pub fn should_attach_program(&self, program_name: &str) -> bool {
        self.attach_programs.contains(program_name)
    }

    pub fn should_attach_follow_exec(&self) -> bool {
        self.follow_exec && self.should_attach_program("sched_process_exec")
    }

    pub fn should_attach_fault_perf(&self) -> bool {
        self.faults && self.has_probe(ProbeKey::Faults)
    }

    pub fn should_attach_stat_wait(&self) -> bool {
        self.stat_wait && self.should_attach_program("sched_stat_wait")
    }

    pub fn required_artifacts(&self) -> BTreeSet<ArtifactKind> {
        self.enabled
            .iter()
            .flat_map(|spec| spec.artifacts.iter().copied())
            .collect()
    }

    pub fn required_stream_artifacts(&self) -> BTreeSet<ArtifactKind> {
        self.required_artifacts()
            .into_iter()
            .filter(|kind| artifact_is_ndjson_stream(*kind))
            .collect()
    }

    pub fn ebpf_programs(&self) -> Vec<EbpfProgramSpec> {
        self.enabled
            .iter()
            .flat_map(|spec| spec.ebpf_programs.iter().copied())
            .collect()
    }

    pub fn tracepoints(&self) -> Vec<TracepointSpec> {
        self.enabled
            .iter()
            .flat_map(|spec| spec.tracepoints.iter().copied())
            .collect()
    }

    pub fn perf_events(&self) -> Vec<PerfEventSpec> {
        self.enabled
            .iter()
            .flat_map(|spec| spec.perf_events.iter().copied())
            .collect()
    }

    pub fn data_quality_rules(&self) -> Vec<DataQualityRule> {
        self.enabled
            .iter()
            .flat_map(|spec| spec.data_quality_rules.iter().copied())
            .collect()
    }

    pub fn push_attach_warning(
        &mut self,
        key: ProbeKey,
        program_name: &'static str,
        error: impl std::fmt::Display,
    ) {
        self.warnings.push(ProbeActivationWarning {
            key: Some(key),
            message: format!(
                "optional probe program {program_name} failed to attach; probe evidence is degraded: {error}"
            ),
        });
    }
}

fn probe_requested(key: ProbeKey, config: &MonitorConfig) -> bool {
    match key {
        ProbeKey::SchedulerRunnableLatency => true,
        ProbeKey::CpuFreq => config.probes.cpu_freq,
        ProbeKey::PsiTimeline => true,
        ProbeKey::PressureStallTimelineOverlay => true,
        ProbeKey::IrqLatency => config.probes.irq_latency,
        ProbeKey::GpuHwmon => config.probes.hwmon,
        ProbeKey::FrameLog => config.mangohud.log.is_some(),
        ProbeKey::ForegroundWindow => {
            config.focus.foreground_window
                || (config.focus.auto_focus && config.focus.focus_source != FocusSource::Heuristic)
        }
        ProbeKey::BlockIo => config.probes.block_io,
        ProbeKey::Faults => config.probes.faults || config.probes.stat_wait,
        ProbeKey::CpuPerf => config.probes.cpu_perf,
        ProbeKey::RuntimeSlices => config.probes.runtime_slices,
        ProbeKey::DrmFenceLatency
        | ProbeKey::PerfCounterPresets
        | ProbeKey::CompositorFramePacingViews => false,
    }
}

fn unavailable_reason(
    key: ProbeKey,
    config: &MonitorConfig,
    tracepoints: &TracepointAvailability,
) -> Option<String> {
    match key {
        ProbeKey::SchedulerRunnableLatency => None,
        ProbeKey::CpuFreq if !tracepoints.cpu_frequency => {
            Some("power/cpu_frequency tracepoint unavailable".to_owned())
        }
        ProbeKey::IrqLatency if !tracepoints.irq_handler => {
            Some("IRQ handler tracepoints unavailable or incompatible".to_owned())
        }
        ProbeKey::BlockIo if !tracepoints.block_rq => {
            Some("block_rq tracepoints unavailable or incompatible".to_owned())
        }
        ProbeKey::Faults
            if config.probes.stat_wait && !tracepoints.sched_stat_wait && !config.probes.faults =>
        {
            Some(
                "sched_stat_wait tracepoint unavailable and fault perf probes were not requested"
                    .to_owned(),
            )
        }
        _ => None,
    }
}

fn attach_programs_for(
    enabled: &[&'static ProbeSpec],
    config: &MonitorConfig,
    tracepoints: &TracepointAvailability,
) -> BTreeSet<&'static str> {
    let mut programs = BTreeSet::new();

    for spec in enabled {
        for program in spec.ebpf_programs {
            if program_available(program.name, config, tracepoints) {
                programs.insert(program.name);
            }
        }
    }

    programs
}

fn program_available(
    program_name: &'static str,
    config: &MonitorConfig,
    tracepoints: &TracepointAvailability,
) -> bool {
    match program_name {
        "sched_wakeup" | "sched_switch" => true,
        "sched_wakeup_new" => tracepoints.sched_wakeup_new,
        "sched_process_exit" => tracepoints.sched_process_exit,
        "sched_migrate_task" => tracepoints.sched_migrate_task,
        "sched_process_exec" => config.safety.follow_exec && tracepoints.sched_process_exec,
        "cpu_frequency" => tracepoints.cpu_frequency,
        "sched_stat_wait" => config.probes.stat_wait && tracepoints.sched_stat_wait,
        "irq_handler_entry" | "irq_handler_exit" => tracepoints.irq_handler,
        "block_rq_issue" | "block_rq_complete" => tracepoints.block_rq,
        "major_fault" | "minor_fault" => config.probes.faults,
        _ => false,
    }
}

pub fn registry_spec_for_key(key: ProbeKey) -> &'static ProbeSpec {
    probe_spec(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> MonitorConfig {
        let mut c = MonitorConfig::default();
        c.probes.cpu_freq = true;
        c.safety.follow_exec = true;
        c
    }

    fn tracepoints() -> TracepointAvailability {
        TracepointAvailability {
            sched_wakeup_new: true,
            sched_migrate_task: true,
            cpu_frequency: true,
            sched_stat_wait: true,
            irq_handler: true,
            block_rq: true,
            block_rq_has_rwbs: true,
            block_rq_key_offset: Some(16),
            block_rq_issue_nr_sector_offset: Some(24),
            block_rq_issue_rwbs_offset: Some(32),
            block_rq_complete_nr_sector_offset: Some(24),
            block_rq_complete_rwbs_offset: Some(32),
            sched_process_exit: true,
            sched_process_exec: true,
        }
    }

    #[test]
    fn default_plan_enables_core_cpu_freq_and_psi() {
        let plan = ProbeActivationPlan::from_config(&config(), &tracepoints()).unwrap();

        assert!(plan.has_probe(ProbeKey::SchedulerRunnableLatency));
        assert!(plan.has_probe(ProbeKey::CpuFreq));
        assert!(plan.has_probe(ProbeKey::PsiTimeline));
        assert!(plan.required_artifacts().contains(&ArtifactKind::Interval));
        assert!(
            plan.required_artifacts()
                .contains(&ArtifactKind::SpikeEvents)
        );
        assert!(plan.should_attach_program("sched_wakeup"));
        assert!(plan.should_attach_program("sched_switch"));
        assert!(plan.should_attach_program("cpu_frequency"));
    }

    #[test]
    fn optional_tracepoint_missing_disables_requested_probe_with_warning() {
        let mut config = config();
        config.probes.block_io = true;
        let mut tracepoints = tracepoints();
        tracepoints.block_rq = false;

        let plan = ProbeActivationPlan::from_config(&config, &tracepoints).unwrap();

        assert!(!plan.has_probe(ProbeKey::BlockIo));
        assert!(
            plan.disabled
                .iter()
                .any(|reason| reason.key == ProbeKey::BlockIo)
        );
        assert!(plan.warnings.iter().any(|warning| {
            warning.key == Some(ProbeKey::BlockIo)
                && warning.message.contains("block_rq tracepoints")
        }));
        assert!(!plan.should_attach_program("block_rq_issue"));
    }

    #[test]
    fn foreground_activation_accepts_auto_focus_foreground_source() {
        let mut config = config();
        config.focus.auto_focus = true;
        config.focus.focus_source = FocusSource::Foreground;

        let plan = ProbeActivationPlan::from_config(&config, &tracepoints()).unwrap();

        assert!(plan.has_probe(ProbeKey::ForegroundWindow));
        assert!(
            plan.required_artifacts()
                .contains(&ArtifactKind::ForegroundEvents)
        );
        assert!(
            plan.required_artifacts()
                .contains(&ArtifactKind::FocusEvents)
        );
    }

    #[test]
    fn required_stream_artifacts_filters_json_objects() {
        let plan = ProbeActivationPlan::from_config(&config(), &tracepoints()).unwrap();

        assert!(
            !plan
                .required_stream_artifacts()
                .contains(&ArtifactKind::Session)
        );
        assert!(
            plan.required_stream_artifacts()
                .contains(&ArtifactKind::Interval)
        );
    }

    #[test]
    fn attach_failure_warning_is_structured() {
        let mut plan = ProbeActivationPlan::from_config(&config(), &tracepoints()).unwrap();

        plan.push_attach_warning(
            ProbeKey::CpuFreq,
            "cpu_frequency",
            "permission denied for test",
        );

        assert!(plan.warnings.iter().any(|warning| {
            warning.key == Some(ProbeKey::CpuFreq)
                && warning.message.contains("cpu_frequency")
                && warning.message.contains("permission denied")
        }));
    }
}
