//! Tests for eBPF attachment degradation behavior through `AttachOps`.

use std::{collections::BTreeSet, path::PathBuf};

use crate::{
    config::model::MonitorConfig,
    ebpf::{
        attach::{
            AttachOps, FaultPerfProbe, TracepointAttachError, attach_drm_fence_tracepoints,
            attach_kms_tracepoints,
        },
        load::{
            attach_optional_fault_perf_events, attach_optional_follow_exec_tracepoint,
            attach_optional_probe_tracepoints, attach_optional_scheduler_tracepoints,
            attach_required_scheduler_tracepoints,
        },
        preflight::TracepointAvailability,
    },
    probe_activation::ProbeActivationPlan,
};

#[derive(Default)]
struct FakeAttachOps {
    fail_programs: BTreeSet<&'static str>,
    tracepoint_calls: Vec<(&'static str, String, String)>,
    perf_calls: Vec<(&'static str, FaultPerfProbe)>,
}

impl FakeAttachOps {
    fn fail_program(program: &'static str) -> Self {
        Self::fail_programs([program])
    }

    fn fail_programs(programs: impl IntoIterator<Item = &'static str>) -> Self {
        Self {
            fail_programs: programs.into_iter().collect(),
            ..Self::default()
        }
    }
}

impl AttachOps for FakeAttachOps {
    fn attach_tracepoint(
        &mut self,
        program_name: &'static str,
        category: &str,
        tracepoint_name: &str,
    ) -> Result<(), TracepointAttachError> {
        self.tracepoint_calls.push((
            program_name,
            category.to_owned(),
            tracepoint_name.to_owned(),
        ));

        if self.fail_programs.contains(program_name) {
            return Err(TracepointAttachError::new(
                program_name,
                category,
                tracepoint_name,
                anyhow::anyhow!("{program_name} failed for test"),
            ));
        }

        Ok(())
    }

    fn attach_perf_event(
        &mut self,
        program_name: &'static str,
        probe: FaultPerfProbe,
    ) -> anyhow::Result<()> {
        self.perf_calls.push((program_name, probe));

        if self.fail_programs.contains(program_name) {
            anyhow::bail!("{program_name} failed for test");
        }

        Ok(())
    }
}

fn attach_test_tracepoints() -> TracepointAvailability {
    TracepointAvailability {
        sched_wakeup_new: true,
        sched_migrate_task: true,
        cpu_frequency: false,
        sched_stat_wait: false,
        irq_handler: false,
        block_rq: false,
        block_rq_has_rwbs: false,
        block_rq_key_offset: None,
        block_rq_issue_nr_sector_offset: None,
        block_rq_issue_rwbs_offset: None,
        block_rq_complete_nr_sector_offset: None,
        block_rq_complete_rwbs_offset: None,
        kms: crate::drm_tracepoints::KmsTracepointAvailability::unavailable(),
        drm_fence: None,
        sched_process_exit: true,
        sched_process_exec: true,
    }
}

fn config_with_all_optional_attach_probes() -> MonitorConfig {
    let mut config = MonitorConfig::default();
    config.probes.cpu_freq = true;
    config.probes.stat_wait = true;
    config.probes.irq_latency = true;
    config.probes.block_io = true;
    config.probes.faults = true;
    config.safety.follow_exec = true;
    config
}

fn tracepoints_with_all_optional_attach_points() -> TracepointAvailability {
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
        kms: crate::drm_tracepoints::KmsTracepointAvailability::unavailable(),
        drm_fence: None,
        sched_process_exit: true,
        sched_process_exec: true,
    }
}

fn fence_format(
    category: &str,
    name: &str,
) -> crate::drm_fence_tracepoints::DrmFenceTracepointFormat {
    crate::drm_fence_tracepoints::parse_tracepoint_format(
        category,
        name,
        "\
field:u64 context;\toffset:8;\tsize:8;\tsigned:0;
field:u64 seqno;\toffset:16;\tsize:8;\tsigned:0;
field:char timeline[32];\toffset:24;\tsize:32;\tsigned:0;
",
    )
}

fn fence_discovery() -> crate::drm_fence_tracepoints::DrmFenceTracepointDiscovery {
    crate::drm_fence_tracepoints::DrmFenceTracepointDiscovery {
        events_root: PathBuf::from("/test/events"),
        supported_profile: "test".to_owned(),
        categories: vec![crate::drm_fence_tracepoints::DrmFenceTracepointCategory {
            category: "dma_fence".to_owned(),
            status: "available".to_owned(),
            tracepoints: vec![
                fence_format("dma_fence", "dma_fence_wait_start"),
                fence_format("dma_fence", "dma_fence_wait_done"),
                fence_format("dma_fence", "dma_fence_signal"),
            ],
            warnings: Vec::new(),
        }],
    }
}

#[test]
fn scheduler_optional_tracepoint_attach_failures_degrade_through_activation_warnings() {
    let source = include_str!("../../ebpf/load.rs");

    for program in [
        "sched_wakeup_new",
        "sched_process_exit",
        "sched_migrate_task",
    ] {
        let marker = format!("\"{program}\"");
        let start = source
            .find(&marker)
            .unwrap_or_else(|| panic!("{program} attach block not found"));
        let end = source.len().min(start + 1_200);
        let body = &source[start..end];

        assert!(body.contains("activation_plan.push_tracepoint_attach_warning"));
        assert!(body.contains("ProbeKey::SchedulerRunnableLatency"));
        assert!(body.contains("optional_probe_attach_failed"));
        assert!(!body.contains("context(\"eBPF load failed: attach"));
    }
}

#[test]
fn tracepoint_attach_error_carries_program_category_and_tracepoint_name() {
    let mut fake = FakeAttachOps::fail_program("sched_wakeup");

    let err = fake
        .attach_tracepoint("sched_wakeup", "sched", "sched_wakeup")
        .unwrap_err();

    assert_eq!(err.program_name(), "sched_wakeup");
    assert_eq!(err.category(), "sched");
    assert_eq!(err.tracepoint_name(), "sched_wakeup");
    assert!(err.to_string().contains("sched/sched_wakeup"));
    assert!(err.source().to_string().contains("failed for test"));
}

#[test]
fn required_sched_wakeup_attach_failure_aborts_load_plan() {
    let mut fake = FakeAttachOps::fail_program("sched_wakeup");

    let err = attach_required_scheduler_tracepoints(&mut fake).unwrap_err();

    assert!(err.to_string().contains("sched_wakeup"));
    assert_eq!(fake.tracepoint_calls.len(), 1);
    assert_eq!(fake.tracepoint_calls[0].0, "sched_wakeup");
}

#[test]
fn optional_sched_wakeup_new_attach_failure_records_warning_and_continues() {
    let config = MonitorConfig::default();
    let mut plan = ProbeActivationPlan::from_config(&config, &attach_test_tracepoints()).unwrap();
    let mut fake = FakeAttachOps::fail_program("sched_wakeup_new");

    attach_optional_scheduler_tracepoints(&mut fake, &mut plan);

    assert!(
        fake.tracepoint_calls
            .iter()
            .any(|(program, _, _)| *program == "sched_process_exit"),
    );
    assert!(plan.warnings.iter().any(|warning| {
        warning.message.contains("sched/sched_wakeup_new")
            && warning.message.contains("sched_wakeup_new")
    }));
}

#[test]
fn optional_probe_tracepoint_failures_record_warnings_and_continue() {
    let config = config_with_all_optional_attach_probes();
    let tracepoints = tracepoints_with_all_optional_attach_points();
    let mut plan = ProbeActivationPlan::from_config(&config, &tracepoints).unwrap();
    let mut fake = FakeAttachOps::fail_programs([
        "cpu_frequency",
        "sched_stat_wait",
        "irq_handler_entry",
        "block_rq_issue",
    ]);

    attach_optional_probe_tracepoints(&mut fake, &mut plan, &tracepoints);

    for program in [
        "cpu_frequency",
        "sched_stat_wait",
        "irq_handler_entry",
        "irq_handler_exit",
        "block_rq_issue",
        "block_rq_complete",
    ] {
        assert!(
            fake.tracepoint_calls
                .iter()
                .any(|(called, _, _)| *called == program),
            "{program} was not attempted"
        );
    }

    for tracepoint in [
        "power/cpu_frequency",
        "sched/sched_stat_wait",
        "irq/irq_handler_entry",
        "block/block_rq_issue",
    ] {
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.message.contains(tracepoint)),
            "{tracepoint} warning was not recorded"
        );
    }
}

#[test]
fn optional_follow_exec_failure_records_tracepoint_warning() {
    let config = config_with_all_optional_attach_probes();
    let tracepoints = tracepoints_with_all_optional_attach_points();
    let mut plan = ProbeActivationPlan::from_config(&config, &tracepoints).unwrap();
    let mut fake = FakeAttachOps::fail_program("sched_process_exec");

    attach_optional_follow_exec_tracepoint(&mut fake, &mut plan);

    assert!(
        fake.tracepoint_calls
            .iter()
            .any(|(program, category, name)| {
                *program == "sched_process_exec"
                    && category == "sched"
                    && name == "sched_process_exec"
            })
    );
    assert!(
        plan.warnings
            .iter()
            .any(|warning| warning.message.contains("sched/sched_process_exec"))
    );
}

#[test]
fn optional_fault_perf_failures_record_warnings_and_continue() {
    let config = config_with_all_optional_attach_probes();
    let tracepoints = tracepoints_with_all_optional_attach_points();
    let mut plan = ProbeActivationPlan::from_config(&config, &tracepoints).unwrap();
    let mut fake = FakeAttachOps::fail_programs(["major_fault", "minor_fault"]);

    attach_optional_fault_perf_events(&mut fake, &mut plan);

    assert!(
        fake.perf_calls
            .iter()
            .any(|(program, probe)| *program == "major_fault" && *probe == FaultPerfProbe::Major)
    );
    assert!(
        fake.perf_calls
            .iter()
            .any(|(program, probe)| *program == "minor_fault" && *probe == FaultPerfProbe::Minor)
    );
    assert!(
        plan.warnings
            .iter()
            .any(|warning| warning.message.contains("major_fault"))
    );
    assert!(
        plan.warnings
            .iter()
            .any(|warning| warning.message.contains("minor_fault"))
    );
}

#[test]
fn kms_optional_attach_warning_names_actual_tracepoint() {
    let mut plan =
        ProbeActivationPlan::from_config(&MonitorConfig::default(), &attach_test_tracepoints())
            .unwrap();
    let kms = crate::drm_tracepoints::KmsTracepointAvailability {
        pageflip_request: None,
        pageflip_done: None,
        vblank_event: Some(crate::drm_tracepoints::parse_drm_tracepoint_format(
            "drm",
            "drm_vblank_event",
            "field:unsigned int crtc_id;\toffset:8;\tsize:4;\tsigned:0;\n",
        )),
        atomic_commit: None,
        provider: crate::drm_tracepoints::KmsTracepointProvider::GenericDrm,
        generic_drm: Vec::new(),
        i915: Vec::new(),
        amdgpu: Vec::new(),
        warnings: Vec::new(),
    };
    let mut fake = FakeAttachOps::fail_program("drm_vblank_event");

    attach_kms_tracepoints(&mut fake, &mut plan, &kms);

    assert!(plan.warnings.iter().any(|warning| {
        warning.message.contains("drm/drm_vblank_event")
            && warning.message.contains("drm_vblank_event")
    }));
}

#[test]
fn drm_fence_optional_attach_warning_names_selected_tracepoints() {
    let discovery = fence_discovery();
    let offsets =
        crate::ebpf::tracepoints::drm_fence::drm_fence_tracepoint_offsets(&discovery).unwrap();
    let mut plan =
        ProbeActivationPlan::from_config(&MonitorConfig::default(), &attach_test_tracepoints())
            .unwrap();
    let mut fake = FakeAttachOps::fail_programs([
        "drm_fence_wait_start",
        "drm_fence_wait_done",
        "drm_fence_signal",
    ]);

    attach_drm_fence_tracepoints(&mut fake, &mut plan, &discovery, offsets);

    for tracepoint in [
        "dma_fence/dma_fence_wait_start",
        "dma_fence/dma_fence_wait_done",
        "dma_fence/dma_fence_signal",
    ] {
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.message.contains(tracepoint)),
            "{tracepoint} warning was not recorded"
        );
    }
}
