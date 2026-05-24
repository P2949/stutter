use std::{fs, path::Path};

use anyhow::Context;
use aya::{
    EbpfLoader,
    maps::{HashMap as AyaHashMap, PerCpuArray, RingBuf},
};
use tokio::io::unix::AsyncFd;

use crate::{
    config::TARGET_PIDS_MAX,
    drm_tracepoints::KmsTracepointProvider,
    ebpf::{
        attach::{
            AttachOps, AyaAttachOps, FaultPerfProbe, attach_drm_fence_tracepoints,
            attach_kms_tracepoints,
        },
        maps::map_sizing_for_config_after_memlock,
        memlock::{log_memlock_policy_report, raise_memlock_limit},
        memory::format_optional_bytes,
        model::{BlockIoCorrelationBasis, LoadedEbpf, NativeCgroupFilterStatus},
        object::ebpf_object_bytes,
        preflight::{TracepointAvailability, validate_tracepoint_formats},
        tracepoints::{
            drm_fence::drm_fence_tracepoint_offsets, kms::kms_provider_tracepoint_offsets,
        },
    },
    error::{EbpfError, ProbeError, TargetError},
    probe_activation::ProbeActivationPlan,
    probe_registry::ProbeKey,
    session::targeting::TargetPolicy,
};

#[cfg(unix)]
pub fn resolve_cgroup_id_best_effort(path: &Path) -> anyhow::Result<u64> {
    use std::os::unix::fs::MetadataExt;

    // Experimental best-effort cgroup id resolver. bpf_get_current_cgroup_id()
    // returns a kernel cgroup id; for cgroup v2 the directory inode is commonly
    // usable on supported kernels, but this is not a full replacement for PID
    // expansion. Keep scheduler wakeup targeting backed by TARGET_PIDS.
    let metadata = fs::metadata(path)?;
    Ok(metadata.ino())
}

#[cfg(not(unix))]
pub fn resolve_cgroup_id_best_effort(_path: &Path) -> anyhow::Result<u64> {
    anyhow::bail!("native cgroup filtering is only supported on Unix/Linux");
}

pub(crate) fn attach_required_scheduler_tracepoints(
    ops: &mut impl AttachOps,
) -> Result<(), EbpfError> {
    ops.attach_tracepoint("sched_wakeup", "sched", "sched_wakeup")
        .map_err(|source| EbpfError::Attach {
            program: "sched_wakeup",
            source,
        })?;
    ops.attach_tracepoint("sched_switch", "sched", "sched_switch")
        .map_err(|source| EbpfError::Attach {
            program: "sched_switch",
            source,
        })?;

    Ok(())
}

pub(crate) fn attach_optional_scheduler_tracepoints(
    ops: &mut impl AttachOps,
    activation_plan: &mut ProbeActivationPlan,
) {
    if activation_plan.should_attach_program("sched_wakeup_new") {
        if let Err(err) = ops.attach_tracepoint("sched_wakeup_new", "sched", "sched_wakeup_new") {
            activation_plan.push_tracepoint_attach_warning(
                ProbeKey::SchedulerRunnableLatency,
                "sched_wakeup_new",
                "sched",
                "sched_wakeup_new",
                &err,
            );
            log::warn!(
                "optional_probe_attach_failed key={:?} program=sched_wakeup_new tracepoint=sched/sched_wakeup_new err={err:#}",
                ProbeKey::SchedulerRunnableLatency
            );
        }
    } else {
        log::warn!(
            "optional_tracepoint_unavailable tracepoint=sched_wakeup_new coverage=reduced_new_task_wakeups message=\"sched_wakeup remains attached, but wakeups for newly created tasks may have reduced coverage\""
        );
    }

    if activation_plan.should_attach_program("sched_process_exit")
        && let Err(err) = ops.attach_tracepoint("sched_process_exit", "sched", "sched_process_exit")
    {
        activation_plan.push_tracepoint_attach_warning(
            ProbeKey::SchedulerRunnableLatency,
            "sched_process_exit",
            "sched",
            "sched_process_exit",
            &err,
        );
        log::warn!(
            "optional_probe_attach_failed key={:?} program=sched_process_exit tracepoint=sched/sched_process_exit err={err:#}",
            ProbeKey::SchedulerRunnableLatency
        );
    }

    if activation_plan.should_attach_program("sched_migrate_task")
        && let Err(err) = ops.attach_tracepoint("sched_migrate_task", "sched", "sched_migrate_task")
    {
        activation_plan.push_tracepoint_attach_warning(
            ProbeKey::SchedulerRunnableLatency,
            "sched_migrate_task",
            "sched",
            "sched_migrate_task",
            &err,
        );
        log::warn!(
            "optional_probe_attach_failed key={:?} program=sched_migrate_task tracepoint=sched/sched_migrate_task err={err:#}",
            ProbeKey::SchedulerRunnableLatency
        );
    }
}

fn attach_optional_probe_tracepoints(
    ops: &mut impl AttachOps,
    activation_plan: &mut ProbeActivationPlan,
    tracepoints: &TracepointAvailability,
) {
    if activation_plan.should_attach_program("cpu_frequency")
        && let Err(err) = ops.attach_tracepoint("cpu_frequency", "power", "cpu_frequency")
    {
        activation_plan.push_tracepoint_attach_warning(
            ProbeKey::CpuFreq,
            "cpu_frequency",
            "power",
            "cpu_frequency",
            &err,
        );
        log::warn!(
            "optional_probe_attach_failed key={:?} program=cpu_frequency tracepoint=power/cpu_frequency err={err:#}",
            ProbeKey::CpuFreq
        );
    }

    if activation_plan.should_attach_stat_wait()
        && let Err(err) = ops.attach_tracepoint("sched_stat_wait", "sched", "sched_stat_wait")
    {
        activation_plan.push_tracepoint_attach_warning(
            ProbeKey::Faults,
            "sched_stat_wait",
            "sched",
            "sched_stat_wait",
            &err,
        );
        log::warn!(
            "optional_probe_attach_failed key={:?} program=sched_stat_wait tracepoint=sched/sched_stat_wait err={err:#}",
            ProbeKey::Faults
        );
    }

    if activation_plan.has_probe(ProbeKey::IrqLatency) {
        if let Err(err) = ops.attach_tracepoint("irq_handler_entry", "irq", "irq_handler_entry") {
            activation_plan.push_tracepoint_attach_warning(
                ProbeKey::IrqLatency,
                "irq_handler_entry",
                "irq",
                "irq_handler_entry",
                &err,
            );
            log::warn!(
                "optional_probe_attach_failed key={:?} program=irq_handler_entry tracepoint=irq/irq_handler_entry err={err:#}",
                ProbeKey::IrqLatency
            );
        }
        if let Err(err) = ops.attach_tracepoint("irq_handler_exit", "irq", "irq_handler_exit") {
            activation_plan.push_tracepoint_attach_warning(
                ProbeKey::IrqLatency,
                "irq_handler_exit",
                "irq",
                "irq_handler_exit",
                &err,
            );
            log::warn!(
                "optional_probe_attach_failed key={:?} program=irq_handler_exit tracepoint=irq/irq_handler_exit err={err:#}",
                ProbeKey::IrqLatency
            );
        }
    }

    if activation_plan.has_probe(ProbeKey::BlockIo) {
        if let Err(err) = ops.attach_tracepoint("block_rq_issue", "block", "block_rq_issue") {
            activation_plan.push_tracepoint_attach_warning(
                ProbeKey::BlockIo,
                "block_rq_issue",
                "block",
                "block_rq_issue",
                &err,
            );
            log::warn!(
                "optional_probe_attach_failed key={:?} program=block_rq_issue tracepoint=block/block_rq_issue err={err:#}",
                ProbeKey::BlockIo
            );
        }
        if let Err(err) = ops.attach_tracepoint("block_rq_complete", "block", "block_rq_complete") {
            activation_plan.push_tracepoint_attach_warning(
                ProbeKey::BlockIo,
                "block_rq_complete",
                "block",
                "block_rq_complete",
                &err,
            );
            log::warn!(
                "optional_probe_attach_failed key={:?} program=block_rq_complete tracepoint=block/block_rq_complete err={err:#}",
                ProbeKey::BlockIo
            );
        }

        if let Some(offset) = tracepoints.block_rq_key_offset {
            log::info!("Block I/O correlation using request pointer identity at offset {offset}");
        } else {
            log::warn!(
                "Block I/O correlation is approximate: using dev+sector hashing instead of request pointers. Concurrent same-sector requests may collide; stutter drops ambiguous fallback samples, so block I/O latency coverage may be incomplete."
            );
        }

        if !tracepoints.block_rq_has_rwbs {
            log::warn!(
                "block_rq tracepoints missing `rwbs`; block I/O correlation will continue but read/write flags are unavailable"
            );
        }
    }
}

fn attach_optional_follow_exec_tracepoint(
    ops: &mut impl AttachOps,
    activation_plan: &mut ProbeActivationPlan,
) {
    if activation_plan.should_attach_follow_exec()
        && let Err(err) = ops.attach_tracepoint("sched_process_exec", "sched", "sched_process_exec")
    {
        activation_plan.push_tracepoint_attach_warning(
            ProbeKey::SchedulerRunnableLatency,
            "sched_process_exec",
            "sched",
            "sched_process_exec",
            &err,
        );
        log::warn!(
            "optional_probe_attach_failed key={:?} program=sched_process_exec tracepoint=sched/sched_process_exec err={err:#}",
            ProbeKey::SchedulerRunnableLatency
        );
    }
}

fn attach_optional_fault_perf_events(
    ops: &mut impl AttachOps,
    activation_plan: &mut ProbeActivationPlan,
) {
    if !activation_plan.should_attach_fault_perf() {
        return;
    }

    // Fault perf events are optional correlation probes. If perf_event_open is
    // blocked by policy or capabilities, log a warning and continue rather than
    // aborting the whole profiler startup.
    if let Err(e) = ops.attach_perf_event("major_fault", FaultPerfProbe::Major) {
        let err = ProbeError::Attach {
            probe: "faults".to_owned(),
            program: "major_fault",
            source: e,
        };
        activation_plan.push_attach_warning(ProbeKey::Faults, "major_fault", &err);
        log::warn!(
            "optional_probe_attach_failed key={:?} program=major_fault err={err:#}",
            ProbeKey::Faults
        );
    }
    if let Err(e) = ops.attach_perf_event("minor_fault", FaultPerfProbe::Minor) {
        let err = ProbeError::Attach {
            probe: "faults".to_owned(),
            program: "minor_fault",
            source: e,
        };
        activation_plan.push_attach_warning(ProbeKey::Faults, "minor_fault", &err);
        log::warn!(
            "optional_probe_attach_failed key={:?} program=minor_fault err={err:#}",
            ProbeKey::Faults
        );
    }
}

pub(crate) fn missing_map_context(map: &'static str) -> String {
    format!("eBPF load failed: {map} map not found")
}

pub(crate) fn map_init_context(map: &'static str) -> String {
    format!("eBPF load failed: {map} map init")
}

pub fn load_and_attach(
    config: &crate::config::model::MonitorConfig,
    target_policy: &TargetPolicy,
) -> anyhow::Result<LoadedEbpf> {
    let memlock_report = raise_memlock_limit();
    log_memlock_policy_report(&memlock_report);
    let map_sizing = map_sizing_for_config_after_memlock(config, &memlock_report);
    log::info!(
        "ebpf_map_sizing locked_memory_limit={} available_memory={} events_ringbuf_bytes={} wakeup_data_entries={}",
        format_optional_bytes(map_sizing.locked_memory_limit_bytes),
        format_optional_bytes(map_sizing.available_memory_bytes),
        map_sizing.events_ringbuf_bytes,
        map_sizing.wakeup_data_entries,
    );
    let tracepoints = validate_tracepoint_formats(Path::new("/sys/kernel/tracing/events"), config)
        .context("tracepoint offset mismatch")?;

    let mut loader = EbpfLoader::new();
    loader
        .map_max_entries("EVENTS", map_sizing.events_ringbuf_bytes)
        .map_max_entries("WAKEUP_DATA", map_sizing.wakeup_data_entries)
        .map_max_entries("WAKEUP_CONSUMED", map_sizing.wakeup_data_entries);

    if let Some(ref offset) = tracepoints.block_rq_key_offset {
        loader.override_global("BLOCK_RQ_KEY_OFFSET", offset, true);
    }
    if let Some(ref offset) = tracepoints.block_rq_issue_nr_sector_offset {
        loader.override_global("BLOCK_RQ_ISSUE_NR_SECTOR_OFFSET", offset, true);
    }
    if let Some(ref offset) = tracepoints.block_rq_issue_rwbs_offset {
        loader.override_global("BLOCK_RQ_ISSUE_RWBS_OFFSET", offset, true);
    }
    if let Some(ref offset) = tracepoints.block_rq_complete_nr_sector_offset {
        loader.override_global("BLOCK_RQ_COMPLETE_NR_SECTOR_OFFSET", offset, true);
    }
    if let Some(ref offset) = tracepoints.block_rq_complete_rwbs_offset {
        loader.override_global("BLOCK_RQ_COMPLETE_RWBS_OFFSET", offset, true);
    }

    let kms_offsets = kms_provider_tracepoint_offsets(&tracepoints.kms);
    if let Some(ref offsets) = kms_offsets {
        match tracepoints.kms.provider {
            KmsTracepointProvider::I915 => {
                loader.override_global(
                    "I915_FLIP_REQUEST_CRTC_OFFSET",
                    &offsets.request_crtc_offset,
                    true,
                );
                loader.override_global(
                    "I915_FLIP_REQUEST_PIPE_OFFSET",
                    &offsets.request_pipe_offset,
                    true,
                );
                loader.override_global(
                    "I915_FLIP_DONE_CRTC_OFFSET",
                    &offsets.done_crtc_offset,
                    true,
                );
                loader.override_global(
                    "I915_FLIP_DONE_PIPE_OFFSET",
                    &offsets.done_pipe_offset,
                    true,
                );
                loader.override_global(
                    "I915_FLIP_DONE_SEQUENCE_OFFSET",
                    &offsets.done_sequence_offset,
                    true,
                );
                loader.override_global(
                    "I915_FLIP_DONE_SEQUENCE_SIZE",
                    &offsets.done_sequence_size,
                    true,
                );
            }
            KmsTracepointProvider::GenericDrm => {
                loader.override_global(
                    "DRM_FLIP_REQUEST_CRTC_OFFSET",
                    &offsets.request_crtc_offset,
                    true,
                );
                loader.override_global(
                    "DRM_FLIP_REQUEST_PIPE_OFFSET",
                    &offsets.request_pipe_offset,
                    true,
                );
                loader.override_global(
                    "DRM_FLIP_DONE_CRTC_OFFSET",
                    &offsets.done_crtc_offset,
                    true,
                );
                loader.override_global(
                    "DRM_FLIP_DONE_PIPE_OFFSET",
                    &offsets.done_pipe_offset,
                    true,
                );
                loader.override_global(
                    "DRM_FLIP_DONE_SEQUENCE_OFFSET",
                    &offsets.done_sequence_offset,
                    true,
                );
                loader.override_global(
                    "DRM_FLIP_DONE_SEQUENCE_SIZE",
                    &offsets.done_sequence_size,
                    true,
                );
                loader.override_global("DRM_VBLANK_CRTC_OFFSET", &offsets.vblank_crtc_offset, true);
                loader.override_global("DRM_VBLANK_PIPE_OFFSET", &offsets.vblank_pipe_offset, true);
                loader.override_global(
                    "DRM_VBLANK_SEQUENCE_OFFSET",
                    &offsets.vblank_sequence_offset,
                    true,
                );
                loader.override_global(
                    "DRM_VBLANK_SEQUENCE_SIZE",
                    &offsets.vblank_sequence_size,
                    true,
                );
            }
            KmsTracepointProvider::Amdgpu => {
                loader.override_global(
                    "AMDGPU_FLIP_REQUEST_CRTC_OFFSET",
                    &offsets.request_crtc_offset,
                    true,
                );
                loader.override_global(
                    "AMDGPU_FLIP_REQUEST_PIPE_OFFSET",
                    &offsets.request_pipe_offset,
                    true,
                );
                loader.override_global(
                    "AMDGPU_FLIP_DONE_CRTC_OFFSET",
                    &offsets.done_crtc_offset,
                    true,
                );
                loader.override_global(
                    "AMDGPU_FLIP_DONE_PIPE_OFFSET",
                    &offsets.done_pipe_offset,
                    true,
                );
                loader.override_global(
                    "AMDGPU_FLIP_DONE_SEQUENCE_OFFSET",
                    &offsets.done_sequence_offset,
                    true,
                );
                loader.override_global(
                    "AMDGPU_FLIP_DONE_SEQUENCE_SIZE",
                    &offsets.done_sequence_size,
                    true,
                );
                loader.override_global(
                    "AMDGPU_VBLANK_CRTC_OFFSET",
                    &offsets.vblank_crtc_offset,
                    true,
                );
                loader.override_global(
                    "AMDGPU_VBLANK_PIPE_OFFSET",
                    &offsets.vblank_pipe_offset,
                    true,
                );
                loader.override_global(
                    "AMDGPU_VBLANK_SEQUENCE_OFFSET",
                    &offsets.vblank_sequence_offset,
                    true,
                );
                loader.override_global(
                    "AMDGPU_VBLANK_SEQUENCE_SIZE",
                    &offsets.vblank_sequence_size,
                    true,
                );
            }
            KmsTracepointProvider::Mixed | KmsTracepointProvider::Unavailable => {}
        }
    }

    let drm_fence_offsets = tracepoints
        .drm_fence
        .as_ref()
        .and_then(drm_fence_tracepoint_offsets);
    if let Some(ref offsets) = drm_fence_offsets {
        loader.override_global(
            "DRM_FENCE_WAIT_START_CONTEXT_OFFSET",
            &offsets.wait_start_context_offset,
            true,
        );
        loader.override_global(
            "DRM_FENCE_WAIT_START_SEQNO_OFFSET",
            &offsets.wait_start_seqno_offset,
            true,
        );
        loader.override_global(
            "DRM_FENCE_WAIT_START_TIMELINE_OFFSET",
            &offsets.wait_start_timeline_offset,
            true,
        );
        loader.override_global(
            "DRM_FENCE_WAIT_DONE_CONTEXT_OFFSET",
            &offsets.wait_done_context_offset,
            true,
        );
        loader.override_global(
            "DRM_FENCE_WAIT_DONE_SEQNO_OFFSET",
            &offsets.wait_done_seqno_offset,
            true,
        );
        loader.override_global(
            "DRM_FENCE_WAIT_DONE_TIMELINE_OFFSET",
            &offsets.wait_done_timeline_offset,
            true,
        );
        loader.override_global(
            "DRM_FENCE_SIGNAL_CONTEXT_OFFSET",
            &offsets.signal_context_offset,
            true,
        );
        loader.override_global(
            "DRM_FENCE_SIGNAL_SEQNO_OFFSET",
            &offsets.signal_seqno_offset,
            true,
        );
        loader.override_global(
            "DRM_FENCE_SIGNAL_TIMELINE_OFFSET",
            &offsets.signal_timeline_offset,
            true,
        );
        loader.override_global(
            "DRM_FENCE_WAIT_START_PROVIDER",
            &offsets.wait_start_provider,
            true,
        );
        loader.override_global(
            "DRM_FENCE_WAIT_START_GPU_ROLE",
            &offsets.wait_start_gpu_role,
            true,
        );
        loader.override_global(
            "DRM_FENCE_WAIT_DONE_PROVIDER",
            &offsets.wait_done_provider,
            true,
        );
        loader.override_global(
            "DRM_FENCE_WAIT_DONE_GPU_ROLE",
            &offsets.wait_done_gpu_role,
            true,
        );
        loader.override_global("DRM_FENCE_SIGNAL_PROVIDER", &offsets.signal_provider, true);
        loader.override_global("DRM_FENCE_SIGNAL_GPU_ROLE", &offsets.signal_gpu_role, true);
    }

    let block_io_correlation_basis = if !tracepoints.block_rq {
        BlockIoCorrelationBasis::Disabled
    } else if tracepoints.block_rq_key_offset.is_some() {
        BlockIoCorrelationBasis::RequestPointer
    } else {
        BlockIoCorrelationBasis::DevSector
    };

    let object = ebpf_object_bytes()?;
    let mut ebpf = loader
        .load(object.as_ref())
        .map_err(|source| EbpfError::ObjectLoad {
            source: source.into(),
        })?;

    let mut activation_plan = ProbeActivationPlan::from_config(config, &tracepoints)?;
    for warning in &activation_plan.warnings {
        log::warn!(
            "probe_activation_warning key={:?} message={}",
            warning.key,
            warning.message
        );
    }

    {
        let mut attach_ops = AyaAttachOps::new(&mut ebpf);

        attach_required_scheduler_tracepoints(&mut attach_ops)?;
        attach_optional_scheduler_tracepoints(&mut attach_ops, &mut activation_plan);
        attach_optional_probe_tracepoints(&mut attach_ops, &mut activation_plan, &tracepoints);

        if activation_plan.has_probe(ProbeKey::KmsPageflipTiming) {
            attach_kms_tracepoints(&mut attach_ops, &mut activation_plan, &tracepoints.kms);
        }

        if activation_plan.has_probe(ProbeKey::DrmFenceLatency)
            && let (Some(discovery), Some(offsets)) = (&tracepoints.drm_fence, drm_fence_offsets)
        {
            attach_drm_fence_tracepoints(&mut attach_ops, &mut activation_plan, discovery, offsets);
        }

        attach_optional_follow_exec_tracepoint(&mut attach_ops, &mut activation_plan);
        attach_optional_fault_perf_events(&mut attach_ops, &mut activation_plan);
    }

    let mut target_pid_map = AyaHashMap::try_from(
        ebpf.take_map("TARGET_PIDS")
            .context(missing_map_context("TARGET_PIDS"))?,
    )
    .context(map_init_context("TARGET_PIDS"))?;

    let target_irq_map = ebpf
        .take_map("TARGET_IRQS")
        .map(AyaHashMap::try_from)
        .transpose()
        .context(map_init_context("TARGET_IRQS"))?;

    let drop_counters = PerCpuArray::try_from(
        ebpf.take_map("DROP_COUNTERS")
            .context(missing_map_context("DROP_COUNTERS"))?,
    )
    .context(map_init_context("DROP_COUNTERS"))?;

    let events = RingBuf::try_from(
        ebpf.take_map("EVENTS")
            .context(missing_map_context("EVENTS"))?,
    )
    .context(map_init_context("EVENTS"))?;

    let events = AsyncFd::new(events).context("eBPF load failed: events ringbuf async fd")?;

    let prev_faults_map = ebpf
        .take_map("PREV_FAULTS")
        .map(AyaHashMap::try_from)
        .transpose()
        .context(map_init_context("PREV_FAULTS"))?;

    let native_cgroup_filter = NativeCgroupFilterStatus::disabled();
    let target_cgroup_map = None;
    if config.safety.native_cgroup_filter {
        let cgroup_path =
            config.target.cgroupv2.as_ref().ok_or_else(|| {
                anyhow::anyhow!("native cgroup filtering requires --cgroupv2 PATH")
            })?;
        let cgroup_id = resolve_cgroup_id_best_effort(cgroup_path).map_err(|source| {
            TargetError::InvalidCgroupPath {
                path: cgroup_path.to_path_buf(),
                source,
            }
        })?;

        // Refuse to start a requested-but-inactive native cgroup mode. Directory
        // inode resolution is useful diagnostic context, but it is not a
        // runtime-verified proof that TARGET_CGROUP_IDS will match
        // bpf_get_current_cgroup_id() on this kernel. Keep PID expansion as the
        // only supported cgroup target path until a real verifier exists.
        return Err(TargetError::NativeCgroupFilterUnsupported {
            path: cgroup_path.to_path_buf(),
            cgroup_id,
        }
        .into());
    }

    if let Some(cgroup_path) = &target_policy.cgroupv2 {
        // Pre-populate TARGET_PIDS from the cgroup hierarchy to avoid races
        // where a task appears in sched events before the eBPF-side target
        // maps are populated. Native cgroup filtering only applies to
        // current-task probes; scheduler wakeup target filtering still needs
        // TARGET_PIDS because bpf_get_current_cgroup_id() reports the
        // waker/current task, not the wakee pid in sched_wakeup. Use a filtered
        // snapshot to ensure that we respect user-provided filters and do not
        // exceed crate::config::TARGET_PIDS_MAX due to unrelated tasks in the same
        // cgroup.
        let mut cache = crate::process_tree::ProcessCache::default();
        let snapshot = crate::process_tree::target_snapshot(
            crate::process_tree::TargetSnapshotInput::default()
                .cgroup_path(Some(cgroup_path))
                .exclude_tree_pids(&target_policy.exclude_tree_pids)
                .filters(&target_policy.compiled_filters)
                .keep_missing_pid(target_policy.keep_missing_pid)
                .cache(&mut cache),
        );
        let pids: Vec<_> = snapshot.tasks.keys().copied().collect();

        if pids.len() > TARGET_PIDS_MAX {
            anyhow::bail!(
                "cgroup target prepopulation failed: {} tasks in cgroup match filters, but target_pids_max is {}",
                pids.len(),
                crate::config::TARGET_PIDS_MAX
            );
        }

        // Also respect the user-defined --max-tasks limit during prepopulation.
        if pids.len() > target_policy.max_tasks {
            anyhow::bail!(
                "cgroup target prepopulation failed: {} tasks in cgroup match filters, but --max-tasks is {}",
                pids.len(),
                target_policy.max_tasks
            );
        }

        let mut failed_inserts = 0usize;
        for pid in pids.iter() {
            if target_pid_map.insert(*pid, 1, 0).is_err() {
                failed_inserts += 1;
            }
        }

        if failed_inserts > 0 {
            anyhow::bail!(
                "cgroup target prepopulation failed: {} tasks failed to insert (target_pids_max={}); use narrower filters or a smaller cgroup",
                failed_inserts,
                crate::config::TARGET_PIDS_MAX
            );
        }
    }

    Ok(LoadedEbpf {
        _ebpf: ebpf,
        events,
        target_pid_map,
        target_irq_map,
        target_cgroup_map,
        prev_faults_map,
        block_io_correlation_basis,
        native_cgroup_filter,
        activation_plan,
        drop_counters,
    })
}
