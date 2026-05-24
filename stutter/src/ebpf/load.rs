use std::{fs, path::Path};

use anyhow::Context;
use aya::EbpfLoader;

use crate::{
    config::TARGET_PIDS_MAX,
    ebpf::{
        attach::{
            AttachOps, AyaAttachOps, FaultPerfProbe, attach_drm_fence_tracepoints,
            attach_kms_tracepoints,
        },
        load_plan::{apply_loader_plan, build_loader_plan},
        map_init::{AyaMapInitOps, InitializedEbpfMaps, initialize_ebpf_maps},
        maps::map_sizing_for_config_after_memlock,
        memlock::{log_memlock_policy_report, raise_memlock_limit},
        memory::format_optional_bytes,
        model::{LoadedEbpf, NativeCgroupFilterStatus},
        object::ebpf_object_bytes,
        preflight::{TracepointAvailability, validate_tracepoint_formats},
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

pub(crate) fn attach_optional_probe_tracepoints(
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

pub(crate) fn attach_optional_follow_exec_tracepoint(
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

pub(crate) fn attach_optional_fault_perf_events(
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

#[cfg(test)]
pub(crate) use crate::ebpf::map_init::{map_init_context, missing_map_context};

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

    let loader_plan = build_loader_plan(&tracepoints, map_sizing);
    let block_io_correlation_basis = loader_plan.block_io_correlation_basis;
    let drm_fence_offsets = loader_plan.drm_fence_offsets;
    let mut loader = EbpfLoader::new();
    apply_loader_plan(&mut loader, &loader_plan);

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

    let InitializedEbpfMaps {
        mut target_pid_map,
        target_irq_map,
        drop_counters,
        events,
        prev_faults_map,
    } = {
        let mut map_ops = AyaMapInitOps::new(&mut ebpf);
        initialize_ebpf_maps(&mut map_ops)?
    };

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
