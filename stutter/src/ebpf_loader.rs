use std::{
    borrow::Cow,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use aya::{
    Ebpf, EbpfLoader,
    maps::{HashMap as AyaHashMap, MapData, PerCpuArray, RingBuf},
    programs::{PerfEvent, TracePoint},
    util::online_cpus,
};
use serde::{Deserialize, Serialize};
use stutter_common::{
    DROP_BLOCK_START_INSERT_FAILED, DROP_IRQ_START_TIMES_INSERT_FAILED,
    DROP_RINGBUF_RESERVE_FAILED, DROP_WAKEUP_DATA_INSERT_FAILED, DROP_WAKEUP_DATA_STALE_ENTRY,
};
use tokio::io::unix::AsyncFd;

use crate::{
    config::TARGET_PIDS_MAX, probe_activation::ProbeActivationPlan, probe_registry::ProbeKey,
    session::targeting::TargetPolicy,
};

const DEFAULT_AVAILABLE_MEMORY_BYTES: u64 = 1 << 30;
const AVAILABLE_MEMORY_BUDGET_DIVISOR: u64 = 64;
const MEMLOCK_BUDGET_NUMERATOR: u64 = 3;
const MEMLOCK_BUDGET_DENOMINATOR: u64 = 4;
// Conservative userspace budgeting estimate for one WAKEUP_DATA kernel hash-map
// entry. This is not the raw eBPF-private WakeupData struct size; it reserves
// room for kernel map metadata, alignment, hash storage overhead, and safety
// margin when splitting the available map-memory budget.
const WAKEUP_DATA_MAP_ENTRY_BUDGET_BYTES: u64 = 64;
const MIN_WAKEUP_DATA_ENTRIES: u32 = 4_096;
const MAX_WAKEUP_DATA_ENTRIES: u32 = 1_048_576;
const MIN_EVENTS_RINGBUF_BYTES: u32 = 64 * 1024;
const MAX_EVENTS_RINGBUF_BYTES: u32 = 16 * 1024 * 1024;
const EVENTS_BUDGET_NUMERATOR: u64 = 2;
const EVENTS_BUDGET_DENOMINATOR: u64 = 5;

pub struct LoadedEbpf {
    _ebpf: Ebpf,
    pub events: AsyncFd<RingBuf<MapData>>,
    pub target_pid_map: AyaHashMap<MapData, u32, u8>,
    pub target_irq_map: Option<AyaHashMap<MapData, u32, u8>>,
    // Stored to keep the optional native cgroup map FD alive for the session.
    pub target_cgroup_map: Option<AyaHashMap<MapData, u64, u8>>,
    pub prev_faults_map: Option<AyaHashMap<MapData, u32, [u64; 2]>>, // (tid) -> (maj, min)
    pub block_io_correlation_basis: BlockIoCorrelationBasis,
    pub activation_plan: ProbeActivationPlan,
    drop_counters: PerCpuArray<MapData, u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockIoCorrelationBasis {
    DevSector,
    RequestPointer,
}

impl BlockIoCorrelationBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DevSector => "dev+sector",
            Self::RequestPointer => "request-pointer",
        }
    }

    pub fn confidence(self) -> &'static str {
        match self {
            Self::RequestPointer => "high",
            Self::DevSector => "medium",
        }
    }

    pub fn warning(self) -> Option<&'static str> {
        match self {
            Self::RequestPointer => None,
            Self::DevSector => Some(
                "Block I/O correlation is approximate (dev+sector fallback); concurrent same-sector requests may collide.",
            ),
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "request-pointer" => Self::RequestPointer,
            _ => Self::DevSector,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DropCountersSnapshot {
    #[serde(
        rename = "lost_wakeup_timestamp_inserts",
        alias = "wakeup_data_insert_failed"
    )]
    pub wakeup_data_insert_failed: u64,
    #[serde(default)]
    pub wakeup_data_stale_entries: u64,
    pub ringbuf_reserve_failed: u64,
    #[serde(default)]
    pub irq_start_times_insert_failed: u64,
    #[serde(default)]
    pub block_start_insert_failed: u64,
}

impl DropCountersSnapshot {
    pub fn total(&self) -> u64 {
        self.wakeup_data_insert_failed
            .saturating_add(self.wakeup_data_stale_entries)
            .saturating_add(self.ringbuf_reserve_failed)
            .saturating_add(self.irq_start_times_insert_failed)
            .saturating_add(self.block_start_insert_failed)
    }

    pub fn total_excluding_block_io(&self) -> u64 {
        self.wakeup_data_insert_failed
            .saturating_add(self.wakeup_data_stale_entries)
            .saturating_add(self.ringbuf_reserve_failed)
            .saturating_add(self.irq_start_times_insert_failed)
    }
}

impl LoadedEbpf {
    pub fn snapshot_drop_counters(&self) -> DropCountersSnapshot {
        DropCountersSnapshot {
            wakeup_data_insert_failed: drop_counter_value(
                &self.drop_counters,
                DROP_WAKEUP_DATA_INSERT_FAILED,
            ),
            wakeup_data_stale_entries: drop_counter_value(
                &self.drop_counters,
                DROP_WAKEUP_DATA_STALE_ENTRY,
            ),
            ringbuf_reserve_failed: drop_counter_value(
                &self.drop_counters,
                DROP_RINGBUF_RESERVE_FAILED,
            ),
            irq_start_times_insert_failed: drop_counter_value(
                &self.drop_counters,
                DROP_IRQ_START_TIMES_INSERT_FAILED,
            ),
            block_start_insert_failed: drop_counter_value(
                &self.drop_counters,
                DROP_BLOCK_START_INSERT_FAILED,
            ),
        }
    }
}

#[cfg(unix)]
pub fn resolve_cgroup_id_best_effort(path: &Path) -> anyhow::Result<u64> {
    use std::os::unix::fs::MetadataExt;

    // Experimental best-effort cgroup id resolver. bpf_get_current_cgroup_id()
    // returns a kernel cgroup id; for cgroup v2 the directory inode is commonly
    // usable, but this is not a full replacement for PID expansion.
    let metadata = fs::metadata(path)?;
    Ok(metadata.ino())
}

#[cfg(not(unix))]
pub fn resolve_cgroup_id_best_effort(_path: &Path) -> anyhow::Result<u64> {
    anyhow::bail!("native cgroup filtering is only supported on Unix/Linux");
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
        .map_max_entries("WAKEUP_DATA", map_sizing.wakeup_data_entries);

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

    let block_io_correlation_basis = if tracepoints.block_rq_key_offset.is_some() {
        BlockIoCorrelationBasis::RequestPointer
    } else {
        BlockIoCorrelationBasis::DevSector
    };

    let object = ebpf_object_bytes()?;
    let mut ebpf = loader.load(object.as_ref()).context("eBPF load failed")?;

    let mut activation_plan = ProbeActivationPlan::from_config(config, &tracepoints)?;
    for warning in &activation_plan.warnings {
        log::warn!(
            "probe_activation_warning key={:?} message={}",
            warning.key,
            warning.message
        );
    }

    attach_tracepoint(&mut ebpf, "sched_wakeup", "sched", "sched_wakeup")
        .context("eBPF load failed: attach sched_wakeup")?;
    attach_tracepoint(&mut ebpf, "sched_switch", "sched", "sched_switch")
        .context("eBPF load failed: attach sched_switch")?;

    if activation_plan.should_attach_program("sched_wakeup_new") {
        attach_tracepoint(&mut ebpf, "sched_wakeup_new", "sched", "sched_wakeup_new")
            .context("eBPF load failed: attach sched_wakeup_new")?;
    } else {
        log::warn!(
            "optional_tracepoint_unavailable tracepoint=sched_wakeup_new coverage=reduced_new_task_wakeups message=\"sched_wakeup remains attached, but wakeups for newly created tasks may have reduced coverage\""
        );
    }

    if activation_plan.should_attach_program("sched_process_exit") {
        attach_tracepoint(
            &mut ebpf,
            "sched_process_exit",
            "sched",
            "sched_process_exit",
        )
        .context("eBPF load failed: attach sched_process_exit")?;
    }

    if activation_plan.should_attach_program("sched_migrate_task") {
        attach_tracepoint(
            &mut ebpf,
            "sched_migrate_task",
            "sched",
            "sched_migrate_task",
        )
        .context("eBPF load failed: attach sched_migrate_task")?;
    }

    if activation_plan.should_attach_program("cpu_frequency")
        && let Err(err) = attach_tracepoint(&mut ebpf, "cpu_frequency", "power", "cpu_frequency")
    {
        activation_plan.push_attach_warning(ProbeKey::CpuFreq, "cpu_frequency", &err);
        log::warn!(
            "optional_probe_attach_failed key={:?} program=cpu_frequency err={err:#}",
            ProbeKey::CpuFreq
        );
    }

    if activation_plan.should_attach_stat_wait()
        && let Err(err) =
            attach_tracepoint(&mut ebpf, "sched_stat_wait", "sched", "sched_stat_wait")
    {
        activation_plan.push_attach_warning(ProbeKey::Faults, "sched_stat_wait", &err);
        log::warn!(
            "optional_probe_attach_failed key={:?} program=sched_stat_wait err={err:#}",
            ProbeKey::Faults
        );
    }

    if activation_plan.has_probe(ProbeKey::IrqLatency) {
        if let Err(err) =
            attach_tracepoint(&mut ebpf, "irq_handler_entry", "irq", "irq_handler_entry")
        {
            activation_plan.push_attach_warning(ProbeKey::IrqLatency, "irq_handler_entry", &err);
            log::warn!(
                "optional_probe_attach_failed key={:?} program=irq_handler_entry err={err:#}",
                ProbeKey::IrqLatency
            );
        }
        if let Err(err) =
            attach_tracepoint(&mut ebpf, "irq_handler_exit", "irq", "irq_handler_exit")
        {
            activation_plan.push_attach_warning(ProbeKey::IrqLatency, "irq_handler_exit", &err);
            log::warn!(
                "optional_probe_attach_failed key={:?} program=irq_handler_exit err={err:#}",
                ProbeKey::IrqLatency
            );
        }
    }

    if activation_plan.has_probe(ProbeKey::BlockIo) {
        if let Err(err) = attach_tracepoint(&mut ebpf, "block_rq_issue", "block", "block_rq_issue")
        {
            activation_plan.push_attach_warning(ProbeKey::BlockIo, "block_rq_issue", &err);
            log::warn!(
                "optional_probe_attach_failed key={:?} program=block_rq_issue err={err:#}",
                ProbeKey::BlockIo
            );
        }
        if let Err(err) =
            attach_tracepoint(&mut ebpf, "block_rq_complete", "block", "block_rq_complete")
        {
            activation_plan.push_attach_warning(ProbeKey::BlockIo, "block_rq_complete", &err);
            log::warn!(
                "optional_probe_attach_failed key={:?} program=block_rq_complete err={err:#}",
                ProbeKey::BlockIo
            );
        }

        if let Some(offset) = tracepoints.block_rq_key_offset {
            log::info!("Block I/O correlation using request pointer identity at offset {offset}");
        } else {
            log::warn!(
                "Block I/O correlation is approximate: using dev+sector hashing instead of request pointers. Concurrent same-sector requests may collide and cause misattribution."
            );
        }

        if !tracepoints.block_rq_has_rwbs {
            log::warn!(
                "block_rq tracepoints missing `rwbs`; block I/O correlation will continue but read/write flags are unavailable"
            );
        }
    }

    if activation_plan.should_attach_follow_exec()
        && let Err(err) = attach_tracepoint(
            &mut ebpf,
            "sched_process_exec",
            "sched",
            "sched_process_exec",
        )
    {
        activation_plan.push_attach_warning(
            ProbeKey::SchedulerRunnableLatency,
            "sched_process_exec",
            &err,
        );
        log::warn!(
            "optional_probe_attach_failed key={:?} program=sched_process_exec err={err:#}",
            ProbeKey::SchedulerRunnableLatency
        );
    }

    if activation_plan.should_attach_fault_perf() {
        // Fault perf events are optional correlation probes. If perf_event_open
        // is blocked by policy or capabilities, log a warning and continue rather
        // than aborting the whole profiler startup.
        if let Err(e) = attach_software_perf_event(&mut ebpf, "major_fault", FaultPerfProbe::Major)
        {
            log::warn!(
                "failed to attach major_fault perf event; continuing without fault probes: {}",
                e
            );
        }
        if let Err(e) = attach_software_perf_event(&mut ebpf, "minor_fault", FaultPerfProbe::Minor)
        {
            log::warn!(
                "failed to attach minor_fault perf event; continuing without fault probes: {}",
                e
            );
        }
    }

    let mut target_pid_map = AyaHashMap::try_from(
        ebpf.take_map("TARGET_PIDS")
            .context("eBPF load failed: TARGET_PIDS map not found")?,
    )
    .context("eBPF load failed: TARGET_PIDS map init")?;

    let target_irq_map = ebpf
        .take_map("TARGET_IRQS")
        .map(AyaHashMap::try_from)
        .transpose()
        .context("eBPF load failed: TARGET_IRQS map init")?;

    let drop_counters = PerCpuArray::try_from(
        ebpf.take_map("DROP_COUNTERS")
            .context("eBPF load failed: DROP_COUNTERS map not found")?,
    )
    .context("eBPF load failed: DROP_COUNTERS map init")?;

    let events = RingBuf::try_from(
        ebpf.take_map("EVENTS")
            .context("eBPF load failed: EVENTS map not found")?,
    )
    .context("eBPF load failed: EVENTS map init")?;

    let events = AsyncFd::new(events).context("eBPF load failed: events ringbuf async fd")?;

    let prev_faults_map = ebpf
        .take_map("PREV_FAULTS")
        .map(AyaHashMap::try_from)
        .transpose()
        .context("eBPF load failed: PREV_FAULTS map init")?;

    let target_cgroup_map = if config.safety.native_cgroup_filter {
        let cgroup_path =
            config.target.cgroupv2.as_ref().ok_or_else(|| {
                anyhow::anyhow!("native cgroup filtering requires --cgroupv2 PATH")
            })?;
        let cgroup_id = resolve_cgroup_id_best_effort(cgroup_path).with_context(|| {
            format!(
                "failed to resolve native cgroup id for {}",
                cgroup_path.display()
            )
        })?;

        let mut map = AyaHashMap::<_, u64, u8>::try_from(
            ebpf.take_map("TARGET_CGROUP_IDS")
                .context("eBPF load failed: TARGET_CGROUP_IDS map not found")?,
        )
        .context("eBPF load failed: TARGET_CGROUP_IDS map init")?;
        map.insert(cgroup_id, 1, 0)
            .context("failed to insert TARGET_CGROUP_IDS entry")?;
        log::info!(
            "native_cgroup_filter enabled cgroup_path={} cgroup_id={}",
            cgroup_path.display(),
            cgroup_id
        );
        Some(map)
    } else {
        None
    };

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
        activation_plan,
        drop_counters,
    })
}

fn attach_tracepoint(
    ebpf: &mut Ebpf,
    program_name: &str,
    category: &str,
    tracepoint_name: &str,
) -> anyhow::Result<()> {
    let program: &mut TracePoint = ebpf
        .program_mut(program_name)
        .ok_or_else(|| anyhow::anyhow!("{program_name} program not found"))?
        .try_into()?;

    program.load()?;
    program.attach(category, tracepoint_name)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FaultPerfProbe {
    Minor,
    Major,
}

impl FaultPerfProbe {
    fn software_event(self) -> aya::programs::perf_event::SoftwareEvent {
        match self {
            Self::Minor => aya::programs::perf_event::SoftwareEvent::PageFaultsMin,
            Self::Major => aya::programs::perf_event::SoftwareEvent::PageFaultsMaj,
        }
    }
}

fn attach_software_perf_event(
    ebpf: &mut Ebpf,
    program_name: &str,
    probe: FaultPerfProbe,
) -> anyhow::Result<()> {
    let program: &mut PerfEvent = ebpf
        .program_mut(program_name)
        .ok_or_else(|| anyhow::anyhow!("{program_name} program not found"))?
        .try_into()?;

    program.load()?;

    for cpu in online_cpus().map_err(|e| anyhow::anyhow!("{}: {}", e.0, e.1))? {
        let sw_event = probe.software_event();
        program.attach(
            aya::programs::perf_event::PerfEventConfig::Software(sw_event),
            aya::programs::perf_event::PerfEventScope::AllProcessesOneCpu { cpu },
            aya::programs::perf_event::SamplePolicy::Period(1),
            true, // inherit
        )?;
    }

    Ok(())
}

fn drop_counter_value(counters: &PerCpuArray<MapData, u64>, key: u32) -> u64 {
    counters
        .get(&key, 0)
        .map(|values| values.iter().copied().sum())
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EbpfMapSizing {
    events_ringbuf_bytes: u32,
    wakeup_data_entries: u32,
    locked_memory_limit_bytes: Option<u64>,
    available_memory_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MemlockPolicyReport {
    pub before_limit_bytes: Option<u64>,
    pub after_limit_bytes: Option<u64>,
    pub raise_attempted: bool,
    pub raise_succeeded: bool,
    pub raise_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EbpfMapSizingReport {
    pub locked_memory_limit_bytes: Option<u64>,
    pub available_memory_bytes: Option<u64>,
    pub events_ringbuf_bytes: u32,
    pub target_pids_max: usize,
    pub wakeup_data_entries: u32,
    pub wakeup_data_map_entry_budget_bytes: u64,
    pub min_wakeup_data_entries: u32,
    pub max_wakeup_data_entries: u32,
}

fn wakeup_data_entries_for_config(
    computed_entries: u32,
    max_tasks: usize,
    wakeup_map_factor: Option<u32>,
) -> u32 {
    if let Some(factor) = wakeup_map_factor {
        return u32::try_from(max_tasks)
            .unwrap_or(u32::MAX)
            .saturating_mul(factor)
            .clamp(MIN_WAKEUP_DATA_ENTRIES, MAX_WAKEUP_DATA_ENTRIES);
    }

    computed_entries
        .max(wakeup_data_entries_floor_for_max_tasks(max_tasks))
        .clamp(MIN_WAKEUP_DATA_ENTRIES, MAX_WAKEUP_DATA_ENTRIES)
}

fn wakeup_data_entries_floor_for_max_tasks(max_tasks: usize) -> u32 {
    u32::try_from(max_tasks)
        .unwrap_or(u32::MAX)
        .min(MAX_WAKEUP_DATA_ENTRIES)
}

#[cfg(test)]
pub(crate) fn map_sizing_for_config(config: &crate::config::model::MonitorConfig) -> EbpfMapSizing {
    map_sizing_for_config_from_memory(config, current_memory_snapshot())
}

fn map_sizing_for_config_after_memlock(
    config: &crate::config::model::MonitorConfig,
    memlock_report: &MemlockPolicyReport,
) -> EbpfMapSizing {
    map_sizing_for_config_from_memory(
        config,
        MemorySnapshot {
            locked_memory_limit_bytes: memlock_report.after_limit_bytes,
            available_memory_bytes: available_memory_bytes(),
            page_size: system_page_size(),
        },
    )
}

fn map_sizing_for_config_from_memory(
    config: &crate::config::model::MonitorConfig,
    snapshot: MemorySnapshot,
) -> EbpfMapSizing {
    let mut sizing = map_sizing_from_memory(snapshot);

    if let Some(kb) = config.ebpf_sizing.ringbuf_size_kb {
        let bytes = u64::from(kb).saturating_mul(1024);
        let page_size = system_page_size();
        // RingBuf requires power-of-two and page-alignment
        let rounded =
            next_power_of_two(bytes).max(next_power_of_two(u64::from(MIN_EVENTS_RINGBUF_BYTES)));
        let rounded =
            round_up_to_multiple(rounded, page_size).min(u64::from(MAX_EVENTS_RINGBUF_BYTES));
        sizing.events_ringbuf_bytes = rounded as u32;
    }

    sizing.wakeup_data_entries = wakeup_data_entries_for_config(
        sizing.wakeup_data_entries,
        config.target.max_tasks,
        config.ebpf_sizing.wakeup_map_factor,
    );

    sizing
}

pub fn ebpf_map_sizing_report() -> EbpfMapSizingReport {
    let sizing = dynamic_map_sizing();
    EbpfMapSizingReport {
        locked_memory_limit_bytes: sizing.locked_memory_limit_bytes,
        available_memory_bytes: sizing.available_memory_bytes,
        events_ringbuf_bytes: sizing.events_ringbuf_bytes,
        target_pids_max: TARGET_PIDS_MAX,
        wakeup_data_entries: sizing.wakeup_data_entries,
        wakeup_data_map_entry_budget_bytes: WAKEUP_DATA_MAP_ENTRY_BUDGET_BYTES,
        min_wakeup_data_entries: MIN_WAKEUP_DATA_ENTRIES,
        max_wakeup_data_entries: MAX_WAKEUP_DATA_ENTRIES,
    }
}

fn dynamic_map_sizing() -> EbpfMapSizing {
    map_sizing_from_memory(current_memory_snapshot())
}

fn current_memory_snapshot() -> MemorySnapshot {
    MemorySnapshot {
        locked_memory_limit_bytes: locked_memory_limit_bytes(),
        available_memory_bytes: available_memory_bytes(),
        page_size: system_page_size(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MemorySnapshot {
    locked_memory_limit_bytes: Option<u64>,
    available_memory_bytes: Option<u64>,
    page_size: u64,
}

fn map_sizing_from_memory(snapshot: MemorySnapshot) -> EbpfMapSizing {
    let available_memory = snapshot
        .available_memory_bytes
        .unwrap_or(DEFAULT_AVAILABLE_MEMORY_BYTES);
    let available_budget = available_memory / AVAILABLE_MEMORY_BUDGET_DIVISOR;
    let memlock_budget = snapshot
        .locked_memory_limit_bytes
        .map(|bytes| bytes.saturating_mul(MEMLOCK_BUDGET_NUMERATOR) / MEMLOCK_BUDGET_DENOMINATOR)
        .unwrap_or(u64::MAX);
    let budget = available_budget.min(memlock_budget);
    let events_budget = budget.saturating_mul(EVENTS_BUDGET_NUMERATOR) / EVENTS_BUDGET_DENOMINATOR;
    let page_size = snapshot.page_size.max(1);
    let min_events = u64::from(MIN_EVENTS_RINGBUF_BYTES).max(page_size);
    let max_events = u64::from(MAX_EVENTS_RINGBUF_BYTES).max(min_events);
    let events_ringbuf_bytes =
        ring_buffer_size_from_budget(events_budget, min_events, max_events, page_size);
    let wakeup_budget = budget.saturating_sub(u64::from(events_ringbuf_bytes));
    let wakeup_data_entries = wakeup_budget
        .checked_div(WAKEUP_DATA_MAP_ENTRY_BUDGET_BYTES)
        .unwrap_or(0)
        .clamp(
            u64::from(MIN_WAKEUP_DATA_ENTRIES),
            u64::from(MAX_WAKEUP_DATA_ENTRIES),
        ) as u32;

    EbpfMapSizing {
        events_ringbuf_bytes,
        wakeup_data_entries,
        locked_memory_limit_bytes: snapshot.locked_memory_limit_bytes,
        available_memory_bytes: snapshot.available_memory_bytes,
    }
}

fn ring_buffer_size_from_budget(budget: u64, min_size: u64, max_size: u64, page_size: u64) -> u32 {
    let requested = budget.clamp(min_size, max_size);
    let rounded = floor_power_of_two(requested).max(next_power_of_two(min_size));
    let rounded = round_up_to_multiple(rounded, page_size).min(max_size);
    rounded.min(u64::from(u32::MAX)) as u32
}

fn floor_power_of_two(value: u64) -> u64 {
    if value <= 1 {
        return 1;
    }
    1u64 << (u64::BITS - 1 - value.leading_zeros())
}

fn next_power_of_two(value: u64) -> u64 {
    if value <= 1 {
        return 1;
    }
    value.next_power_of_two()
}

fn round_up_to_multiple(value: u64, multiple: u64) -> u64 {
    if multiple <= 1 {
        return value;
    }
    value.div_ceil(multiple).saturating_mul(multiple)
}

fn locked_memory_limit_bytes() -> Option<u64> {
    read_memlock_rlimit()
        .ok()
        .and_then(|rlim| memlock_limit_bytes_from_rlim(rlim.rlim_cur))
}

fn memlock_limit_bytes_from_rlim(value: libc::rlim_t) -> Option<u64> {
    if value == libc::RLIM_INFINITY {
        None
    } else {
        Some(value)
    }
}

fn read_memlock_rlimit() -> std::io::Result<libc::rlimit> {
    let mut rlim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };

    let ret = unsafe { libc::getrlimit(libc::RLIMIT_MEMLOCK, &mut rlim) };
    if ret == 0 {
        Ok(rlim)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn raise_memlock_limit() -> MemlockPolicyReport {
    let before = match read_memlock_rlimit() {
        Ok(rlim) => rlim,
        Err(err) => {
            return MemlockPolicyReport {
                before_limit_bytes: None,
                after_limit_bytes: locked_memory_limit_bytes(),
                raise_attempted: false,
                raise_succeeded: false,
                raise_error: Some(format!("failed to read RLIMIT_MEMLOCK before raise: {err}")),
            };
        }
    };

    let before_limit_bytes = memlock_limit_bytes_from_rlim(before.rlim_cur);
    if before.rlim_cur == libc::RLIM_INFINITY {
        return MemlockPolicyReport {
            before_limit_bytes,
            after_limit_bytes: before_limit_bytes,
            raise_attempted: false,
            raise_succeeded: false,
            raise_error: None,
        };
    }

    // Existing policy: try to make memlock unlimited for eBPF loading. If this
    // fails, continue startup and size maps from the effective post-attempt
    // limit so low-memlock systems remain conservative instead of aborting here.
    let unlimited = libc::rlimit {
        rlim_cur: libc::RLIM_INFINITY,
        rlim_max: libc::RLIM_INFINITY,
    };
    let ret = unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &unlimited) };
    let raise_succeeded = ret == 0;
    let raise_error = if raise_succeeded {
        None
    } else {
        Some(format!(
            "failed to raise RLIMIT_MEMLOCK to unlimited: {}",
            std::io::Error::last_os_error()
        ))
    };

    MemlockPolicyReport {
        before_limit_bytes,
        after_limit_bytes: locked_memory_limit_bytes(),
        raise_attempted: true,
        raise_succeeded,
        raise_error,
    }
}

fn log_memlock_policy_report(report: &MemlockPolicyReport) {
    let raise_error = report.raise_error.as_deref().unwrap_or("none");

    if report.raise_error.is_some() {
        log::warn!(
            "memlock_policy before_limit={} after_limit={} raise_attempted={} raise_succeeded={} raise_error={}",
            format_optional_bytes(report.before_limit_bytes),
            format_optional_bytes(report.after_limit_bytes),
            report.raise_attempted,
            report.raise_succeeded,
            raise_error,
        );
    } else {
        log::info!(
            "memlock_policy before_limit={} after_limit={} raise_attempted={} raise_succeeded={} raise_error={}",
            format_optional_bytes(report.before_limit_bytes),
            format_optional_bytes(report.after_limit_bytes),
            report.raise_attempted,
            report.raise_succeeded,
            raise_error,
        );
    }
}

fn available_memory_bytes() -> Option<u64> {
    mem_available_bytes_at(Path::new("/proc/meminfo")).or_else(available_memory_bytes_from_sysconf)
}

fn mem_available_bytes_at(path: &Path) -> Option<u64> {
    let meminfo = fs::read_to_string(path).ok()?;
    parse_mem_available_bytes(&meminfo)
}

fn parse_mem_available_bytes(meminfo: &str) -> Option<u64> {
    meminfo.lines().find_map(|line| {
        let value = line.strip_prefix("MemAvailable:")?;
        let kib = value.split_whitespace().next()?.parse::<u64>().ok()?;
        kib.checked_mul(1024)
    })
}

fn available_memory_bytes_from_sysconf() -> Option<u64> {
    let pages = unsafe { libc::sysconf(libc::_SC_AVPHYS_PAGES) };
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if pages <= 0 || page_size <= 0 {
        return None;
    }

    (pages as u64).checked_mul(page_size as u64)
}

fn system_page_size() -> u64 {
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size > 0 {
        page_size as u64
    } else {
        4096
    }
}

fn format_optional_bytes(value: Option<u64>) -> String {
    value
        .map(|bytes| bytes.to_string())
        .unwrap_or_else(|| "unknown_or_unlimited".to_owned())
}

#[cfg(test)]
mod map_sizing_tests {
    use super::*;

    const TEST_PAGE_SIZE: u64 = 4096;

    fn memory_snapshot(
        locked_memory_limit_bytes: Option<u64>,
        available_memory_bytes: Option<u64>,
    ) -> MemorySnapshot {
        MemorySnapshot {
            locked_memory_limit_bytes,
            available_memory_bytes,
            page_size: TEST_PAGE_SIZE,
        }
    }

    #[test]
    fn low_memlock_budget_clamps_wakeup_entries_to_minimum() {
        let sizing = map_sizing_from_memory(memory_snapshot(Some(128 * 1024), Some(1 << 30)));

        assert_eq!(sizing.events_ringbuf_bytes, MIN_EVENTS_RINGBUF_BYTES);
        assert_eq!(sizing.wakeup_data_entries, MIN_WAKEUP_DATA_ENTRIES);
        assert_eq!(sizing.locked_memory_limit_bytes, Some(128 * 1024));
        assert_eq!(sizing.available_memory_bytes, Some(1 << 30));
    }

    #[test]
    fn unknown_or_unlimited_memory_uses_default_available_memory_budget() {
        let sizing = map_sizing_from_memory(memory_snapshot(None, None));

        assert_eq!(sizing.events_ringbuf_bytes, 4 * 1024 * 1024);
        assert_eq!(sizing.wakeup_data_entries, 196_608);
        assert_eq!(sizing.locked_memory_limit_bytes, None);
        assert_eq!(sizing.available_memory_bytes, None);
    }

    #[test]
    fn memlock_limit_bytes_treats_rlim_infinity_as_unknown_or_unlimited() {
        assert_eq!(memlock_limit_bytes_from_rlim(libc::RLIM_INFINITY), None);
    }

    #[test]
    fn map_sizing_report_includes_target_and_wakeup_capacities() {
        let report = ebpf_map_sizing_report();
        let value = serde_json::to_value(&report).unwrap();

        assert_eq!(
            value
                .get("target_pids_max")
                .and_then(serde_json::Value::as_u64),
            Some(TARGET_PIDS_MAX as u64)
        );
        assert!(
            value
                .get("wakeup_data_entries")
                .and_then(serde_json::Value::as_u64)
                .is_some()
        );
    }

    #[test]
    fn drop_counter_serializes_wakeup_failures_as_lost_wakeup_timestamps() {
        let snapshot = DropCountersSnapshot {
            wakeup_data_insert_failed: 7,
            wakeup_data_stale_entries: 0,
            ringbuf_reserve_failed: 0,
            irq_start_times_insert_failed: 0,
            block_start_insert_failed: 0,
        };

        let value = serde_json::to_value(&snapshot).unwrap();

        assert_eq!(
            value
                .get("lost_wakeup_timestamp_inserts")
                .and_then(serde_json::Value::as_u64),
            Some(7)
        );
        assert!(value.get("wakeup_data_insert_failed").is_none());
    }

    #[test]
    fn drop_counter_reads_legacy_wakeup_data_insert_failed_name() {
        let snapshot: DropCountersSnapshot = serde_json::from_value(serde_json::json!({
            "wakeup_data_insert_failed": 9,
            "ringbuf_reserve_failed": 0,
            "irq_start_times_insert_failed": 0,
            "block_start_insert_failed": 0
        }))
        .unwrap();

        assert_eq!(snapshot.wakeup_data_insert_failed, 9);
        assert_eq!(snapshot.wakeup_data_stale_entries, 0);
    }

    #[test]
    fn drop_counter_serializes_stale_wakeup_entries() {
        let snapshot = DropCountersSnapshot {
            wakeup_data_insert_failed: 0,
            wakeup_data_stale_entries: 11,
            ringbuf_reserve_failed: 0,
            irq_start_times_insert_failed: 0,
            block_start_insert_failed: 0,
        };

        let value = serde_json::to_value(&snapshot).unwrap();

        assert_eq!(
            value
                .get("wakeup_data_stale_entries")
                .and_then(serde_json::Value::as_u64),
            Some(11)
        );
    }

    #[test]
    fn drop_counter_totals_include_stale_wakeup_entries() {
        let snapshot = DropCountersSnapshot {
            wakeup_data_insert_failed: 1,
            wakeup_data_stale_entries: 2,
            ringbuf_reserve_failed: 4,
            irq_start_times_insert_failed: 8,
            block_start_insert_failed: 16,
        };

        assert_eq!(snapshot.total(), 31);
        assert_eq!(snapshot.total_excluding_block_io(), 15);
    }

    #[test]
    fn map_sizing_after_failed_memlock_raise_uses_after_limit() {
        let report = MemlockPolicyReport {
            before_limit_bytes: Some(128 * 1024),
            after_limit_bytes: Some(128 * 1024),
            raise_attempted: true,
            raise_succeeded: false,
            raise_error: Some("operation not permitted".to_owned()),
        };

        let sizing =
            map_sizing_from_memory(memory_snapshot(report.after_limit_bytes, Some(1 << 30)));

        assert_eq!(sizing.events_ringbuf_bytes, MIN_EVENTS_RINGBUF_BYTES);
        assert_eq!(sizing.wakeup_data_entries, MIN_WAKEUP_DATA_ENTRIES);
        assert_eq!(sizing.locked_memory_limit_bytes, Some(128 * 1024));
    }

    #[test]
    fn map_sizing_after_unlimited_memlock_uses_available_memory_budget() {
        let report = MemlockPolicyReport {
            before_limit_bytes: None,
            after_limit_bytes: None,
            raise_attempted: false,
            raise_succeeded: false,
            raise_error: None,
        };

        let sizing =
            map_sizing_from_memory(memory_snapshot(report.after_limit_bytes, Some(1 << 30)));

        assert_eq!(sizing.events_ringbuf_bytes, 4 * 1024 * 1024);
        assert_eq!(sizing.wakeup_data_entries, 196_608);
        assert_eq!(sizing.locked_memory_limit_bytes, None);
    }

    #[test]
    fn map_sizing_after_unknown_memlock_uses_available_memory_budget() {
        let report = MemlockPolicyReport {
            before_limit_bytes: None,
            after_limit_bytes: None,
            raise_attempted: false,
            raise_succeeded: false,
            raise_error: Some("failed to read RLIMIT_MEMLOCK before raise".to_owned()),
        };

        let sizing =
            map_sizing_from_memory(memory_snapshot(report.after_limit_bytes, Some(1 << 30)));

        assert_eq!(sizing.events_ringbuf_bytes, 4 * 1024 * 1024);
        assert_eq!(sizing.wakeup_data_entries, 196_608);
        assert_eq!(sizing.locked_memory_limit_bytes, None);
    }

    #[test]
    fn very_high_available_memory_clamps_wakeup_entries_to_maximum() {
        let sizing = map_sizing_from_memory(memory_snapshot(None, Some(1u64 << 40)));

        assert_eq!(sizing.events_ringbuf_bytes, MAX_EVENTS_RINGBUF_BYTES);
        assert_eq!(sizing.wakeup_data_entries, MAX_WAKEUP_DATA_ENTRIES);
    }

    #[test]
    fn explicit_wakeup_map_factor_uses_max_tasks_times_factor() {
        let entries = wakeup_data_entries_for_config(MIN_WAKEUP_DATA_ENTRIES, 10_000, Some(4));

        assert_eq!(entries, 40_000);
    }

    #[test]
    fn explicit_wakeup_map_factor_is_clamped_to_minimum() {
        let entries = wakeup_data_entries_for_config(1, 1, Some(0));

        assert_eq!(entries, MIN_WAKEUP_DATA_ENTRIES);
    }

    #[test]
    fn explicit_wakeup_map_factor_is_clamped_to_maximum() {
        let entries = wakeup_data_entries_for_config(
            MIN_WAKEUP_DATA_ENTRIES,
            MAX_WAKEUP_DATA_ENTRIES as usize,
            Some(2),
        );

        assert_eq!(entries, MAX_WAKEUP_DATA_ENTRIES);
    }

    #[test]
    fn automatic_sizing_uses_at_least_configured_max_tasks() {
        let entries = wakeup_data_entries_for_config(MIN_WAKEUP_DATA_ENTRIES, 200_000, None);

        assert_eq!(entries, 200_000);
    }

    #[test]
    fn automatic_sizing_clamps_configured_max_tasks_to_maximum() {
        let entries = wakeup_data_entries_for_config(
            MIN_WAKEUP_DATA_ENTRIES,
            (MAX_WAKEUP_DATA_ENTRIES as usize).saturating_add(1),
            None,
        );

        assert_eq!(entries, MAX_WAKEUP_DATA_ENTRIES);
    }

    #[test]
    fn automatic_sizing_still_clamps_tiny_results_to_minimum() {
        let entries = wakeup_data_entries_for_config(1, 1, None);

        assert_eq!(entries, MIN_WAKEUP_DATA_ENTRIES);
    }
}

#[derive(Debug, Clone)]
pub struct TracepointAvailability {
    pub sched_wakeup_new: bool,
    pub sched_migrate_task: bool,
    pub cpu_frequency: bool,
    pub sched_stat_wait: bool,
    pub irq_handler: bool,
    pub block_rq: bool,
    pub block_rq_has_rwbs: bool,
    pub block_rq_key_offset: Option<u32>,
    pub block_rq_issue_nr_sector_offset: Option<u32>,
    pub block_rq_issue_rwbs_offset: Option<u32>,
    pub block_rq_complete_nr_sector_offset: Option<u32>,
    pub block_rq_complete_rwbs_offset: Option<u32>,
    pub sched_process_exit: bool,
    pub sched_process_exec: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TracepointField {
    name: String,
    offset: u32,
    size: u32,
    signed: bool,
    declaration: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TracepointFormat {
    path: PathBuf,
    fields: BTreeMap<String, TracepointField>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BlockIoTracepointOffsets {
    block_rq: bool,
    block_rq_has_rwbs: bool,
    block_rq_key_offset: Option<u32>,
    block_rq_issue_nr_sector_offset: Option<u32>,
    block_rq_issue_rwbs_offset: Option<u32>,
    block_rq_complete_nr_sector_offset: Option<u32>,
    block_rq_complete_rwbs_offset: Option<u32>,
}

fn validate_block_io_tracepoint_offsets(events_root: &Path) -> BlockIoTracepointOffsets {
    let issue_path = events_root.join("block/block_rq_issue/format");
    let complete_path = events_root.join("block/block_rq_complete/format");

    let issue = match parse_tracepoint_format_at(&issue_path) {
        Ok(format) => format,
        Err(err) => {
            log::warn!(
                "block_io_tracepoint_format_unavailable tracepoint=block_rq_issue path={} err={err:#}",
                issue_path.display()
            );
            return BlockIoTracepointOffsets::default();
        }
    };

    let complete = match parse_tracepoint_format_at(&complete_path) {
        Ok(format) => format,
        Err(err) => {
            log::warn!(
                "block_io_tracepoint_format_unavailable tracepoint=block_rq_complete path={} err={err:#}",
                complete_path.display()
            );
            return BlockIoTracepointOffsets::default();
        }
    };

    let issue_has_required_metadata = tracepoint_field_has_offset_and_size(&issue, "dev", 8, 4)
        && tracepoint_field_has_offset_and_size(&issue, "sector", 16, 8);
    let complete_has_required_metadata =
        tracepoint_field_has_offset_and_size(&complete, "dev", 8, 4)
            && tracepoint_field_has_offset_and_size(&complete, "sector", 16, 8);

    if !issue_has_required_metadata || !complete_has_required_metadata {
        log::warn!(
            "block_io_required_metadata_invalid issue_ok={} complete_ok={} fallback=disabled",
            issue_has_required_metadata,
            complete_has_required_metadata
        );
        return BlockIoTracepointOffsets::default();
    }

    let issue_rq_offset = validated_request_pointer_offset(&issue);
    let complete_rq_offset = validated_request_pointer_offset(&complete);

    let block_rq_key_offset = match (issue_rq_offset, complete_rq_offset) {
        (Some(issue_offset), Some(complete_offset)) if issue_offset == complete_offset => {
            Some(issue_offset)
        }
        (Some(issue_offset), Some(complete_offset)) => {
            log::warn!(
                "block_io_request_pointer_offset_mismatch issue_offset={} complete_offset={} fallback=dev_sector",
                issue_offset,
                complete_offset
            );
            None
        }
        _ => None,
    };

    let block_rq_issue_nr_sector_offset =
        validated_tracepoint_field_offset(&issue, "nr_sector", 4, "u32 nr_sector");
    let block_rq_complete_nr_sector_offset =
        validated_tracepoint_field_offset(&complete, "nr_sector", 4, "u32 nr_sector");

    let block_rq_issue_rwbs_offset =
        validated_tracepoint_field_offset(&issue, "rwbs", 8, "u64 rwbs bytes");
    let block_rq_complete_rwbs_offset =
        validated_tracepoint_field_offset(&complete, "rwbs", 8, "u64 rwbs bytes");

    BlockIoTracepointOffsets {
        block_rq: true,
        block_rq_has_rwbs: block_rq_issue_rwbs_offset.is_some()
            && block_rq_complete_rwbs_offset.is_some(),
        block_rq_key_offset,
        block_rq_issue_nr_sector_offset,
        block_rq_issue_rwbs_offset,
        block_rq_complete_nr_sector_offset,
        block_rq_complete_rwbs_offset,
    }
}

fn tracepoint_field_has_offset_and_size(
    format: &TracepointFormat,
    field_name: &str,
    expected_offset: u32,
    min_size: u32,
) -> bool {
    let Some(field) = format.fields.get(field_name) else {
        log::warn!(
            "tracepoint_required_field_missing path={} field={} expected_offset={} required_size={}",
            format.path.display(),
            field_name,
            expected_offset,
            min_size
        );
        return false;
    };

    if field.offset != expected_offset || field.size < min_size {
        log::warn!(
            "tracepoint_required_field_invalid path={} field={} offset={} expected_offset={} size={} required_size={}",
            format.path.display(),
            field.name,
            field.offset,
            expected_offset,
            field.size,
            min_size
        );
        return false;
    }

    true
}

fn validated_request_pointer_offset(format: &TracepointFormat) -> Option<u32> {
    for field_name in ["rq", "req", "request"] {
        if let Some(offset) =
            validated_tracepoint_field_offset(format, field_name, 8, "u64 request pointer")
        {
            return Some(offset);
        }
    }

    None
}

fn validated_tracepoint_field_offset(
    format: &TracepointFormat,
    field_name: &str,
    min_size: u32,
    read_type: &str,
) -> Option<u32> {
    let Some(field) = format.fields.get(field_name) else {
        log::warn!(
            "tracepoint_field_missing path={} field={} read_type={}",
            format.path.display(),
            field_name,
            read_type
        );
        return None;
    };

    if field.size < min_size {
        log::warn!(
            "tracepoint_field_too_small path={} field={} offset={} size={} required_size={} read_type={}",
            format.path.display(),
            field.name,
            field.offset,
            field.size,
            min_size,
            read_type
        );
        return None;
    }

    Some(field.offset)
}

fn parse_tracepoint_format_at(path: &Path) -> anyhow::Result<TracepointFormat> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read tracepoint format {}", path.display()))?;
    Ok(parse_tracepoint_format(path.to_path_buf(), &contents))
}

fn parse_tracepoint_format(path: PathBuf, contents: &str) -> TracepointFormat {
    let fields = contents
        .lines()
        .filter_map(parse_tracepoint_field_line)
        .map(|field| (field.name.clone(), field))
        .collect();

    TracepointFormat { path, fields }
}

fn parse_tracepoint_field_line(line: &str) -> Option<TracepointField> {
    let mut name = None;
    let mut offset = None;
    let mut size = None;
    let mut signed = None;

    for part in line.split(';') {
        let part = part.trim();
        if let Some(declaration) = part.strip_prefix("field:") {
            name = parse_tracepoint_field_name(declaration);
        } else if let Some(value) = part.strip_prefix("offset:") {
            offset = value.trim().parse::<u32>().ok();
        } else if let Some(value) = part.strip_prefix("size:") {
            size = value.trim().parse::<u32>().ok();
        } else if let Some(value) = part.strip_prefix("signed:") {
            signed = Some(value.trim() != "0");
        }
    }

    Some(TracepointField {
        name: name?,
        offset: offset?,
        size: size?,
        signed: signed.unwrap_or(false),
        declaration: line.trim().to_owned(),
    })
}

fn parse_tracepoint_field_name(declaration: &str) -> Option<String> {
    let token = declaration.split_whitespace().last()?;
    let token = token.trim_start_matches('*');
    let token = token.split('[').next().unwrap_or(token);
    let token = token.trim();

    if token.is_empty() {
        None
    } else {
        Some(token.to_owned())
    }
}

#[cfg(test)]
mod block_io_tracepoint_validation_tests {
    use super::*;

    fn write_block_format(events_root: &Path, tracepoint: &str, contents: &str) {
        let dir = events_root.join("block").join(tracepoint);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("format"), contents).unwrap();
    }

    fn block_rq_format(
        rq: Option<(u32, u32)>,
        nr_sector: Option<(u32, u32)>,
        rwbs: Option<(u32, u32)>,
    ) -> String {
        let mut format = String::from(
            "name: block_rq\nID: 1\nformat:\n\
             \tfield:unsigned short common_type;\toffset:0;\tsize:2;\tsigned:0;\n\
             \tfield:dev_t dev;\toffset:8;\tsize:4;\tsigned:0;\n\
             \tfield:sector_t sector;\toffset:16;\tsize:8;\tsigned:0;\n",
        );

        if let Some((offset, size)) = rq {
            format.push_str(&format!(
                "\tfield:void *rq;\toffset:{offset};\tsize:{size};\tsigned:0;\n"
            ));
        }

        if let Some((offset, size)) = nr_sector {
            format.push_str(&format!(
                "\tfield:unsigned int nr_sector;\toffset:{offset};\tsize:{size};\tsigned:0;\n"
            ));
        }

        if let Some((offset, size)) = rwbs {
            format.push_str(&format!(
                "\tfield:char rwbs[8];\toffset:{offset};\tsize:{size};\tsigned:0;\n"
            ));
        }

        format
    }

    #[test]
    fn block_io_request_pointer_available_and_valid() {
        let temp = tempfile::tempdir().unwrap();
        let events_root = temp.path();

        let format = block_rq_format(Some((40, 8)), Some((48, 4)), Some((56, 8)));
        write_block_format(events_root, "block_rq_issue", &format);
        write_block_format(events_root, "block_rq_complete", &format);

        let offsets = validate_block_io_tracepoint_offsets(events_root);

        assert!(offsets.block_rq);
        assert!(offsets.block_rq_has_rwbs);
        assert_eq!(offsets.block_rq_key_offset, Some(40));
        assert_eq!(offsets.block_rq_issue_nr_sector_offset, Some(48));
        assert_eq!(offsets.block_rq_complete_nr_sector_offset, Some(48));
        assert_eq!(offsets.block_rq_issue_rwbs_offset, Some(56));
        assert_eq!(offsets.block_rq_complete_rwbs_offset, Some(56));
    }

    #[test]
    fn block_io_request_pointer_absent_falls_back_to_dev_sector() {
        let temp = tempfile::tempdir().unwrap();
        let events_root = temp.path();

        let format = block_rq_format(None, Some((48, 4)), Some((56, 8)));
        write_block_format(events_root, "block_rq_issue", &format);
        write_block_format(events_root, "block_rq_complete", &format);

        let offsets = validate_block_io_tracepoint_offsets(events_root);

        assert!(offsets.block_rq);
        assert!(offsets.block_rq_has_rwbs);
        assert_eq!(offsets.block_rq_key_offset, None);
        assert_eq!(offsets.block_rq_issue_nr_sector_offset, Some(48));
        assert_eq!(offsets.block_rq_complete_nr_sector_offset, Some(48));
    }

    #[test]
    fn block_io_rwbs_absent_keeps_rwbs_globals_unset() {
        let temp = tempfile::tempdir().unwrap();
        let events_root = temp.path();

        let format = block_rq_format(Some((40, 8)), Some((48, 4)), None);
        write_block_format(events_root, "block_rq_issue", &format);
        write_block_format(events_root, "block_rq_complete", &format);

        let offsets = validate_block_io_tracepoint_offsets(events_root);

        assert!(offsets.block_rq);
        assert!(!offsets.block_rq_has_rwbs);
        assert_eq!(offsets.block_rq_key_offset, Some(40));
        assert_eq!(offsets.block_rq_issue_rwbs_offset, None);
        assert_eq!(offsets.block_rq_complete_rwbs_offset, None);
    }

    #[test]
    fn block_io_malformed_small_field_sizes_are_not_used() {
        let temp = tempfile::tempdir().unwrap();
        let events_root = temp.path();

        let format = block_rq_format(Some((40, 4)), Some((48, 2)), Some((56, 4)));
        write_block_format(events_root, "block_rq_issue", &format);
        write_block_format(events_root, "block_rq_complete", &format);

        let offsets = validate_block_io_tracepoint_offsets(events_root);

        assert!(offsets.block_rq);
        assert!(!offsets.block_rq_has_rwbs);
        assert_eq!(offsets.block_rq_key_offset, None);
        assert_eq!(offsets.block_rq_issue_nr_sector_offset, None);
        assert_eq!(offsets.block_rq_complete_nr_sector_offset, None);
        assert_eq!(offsets.block_rq_issue_rwbs_offset, None);
        assert_eq!(offsets.block_rq_complete_rwbs_offset, None);
    }

    #[test]
    fn block_io_invalid_dev_sector_metadata_disables_block_rq() {
        let temp = tempfile::tempdir().unwrap();
        let events_root = temp.path();

        let bad_format = "name: block_rq\nID: 1\nformat:\n\
            \tfield:dev_t dev;\toffset:12;\tsize:4;\tsigned:0;\n\
            \tfield:sector_t sector;\toffset:16;\tsize:8;\tsigned:0;\n\
            \tfield:void *rq;\toffset:40;\tsize:8;\tsigned:0;\n\
            \tfield:unsigned int nr_sector;\toffset:48;\tsize:4;\tsigned:0;\n\
            \tfield:char rwbs[8];\toffset:56;\tsize:8;\tsigned:0;\n";

        write_block_format(events_root, "block_rq_issue", bad_format);
        write_block_format(events_root, "block_rq_complete", bad_format);

        let offsets = validate_block_io_tracepoint_offsets(events_root);

        assert!(!offsets.block_rq);
        assert_eq!(offsets.block_rq_key_offset, None);
        assert_eq!(offsets.block_rq_issue_nr_sector_offset, None);
        assert_eq!(offsets.block_rq_complete_nr_sector_offset, None);
        assert_eq!(offsets.block_rq_issue_rwbs_offset, None);
        assert_eq!(offsets.block_rq_complete_rwbs_offset, None);
    }

    #[test]
    fn block_io_request_pointer_accepts_legacy_req_field_name() {
        let temp = tempfile::tempdir().unwrap();
        let events_root = temp.path();

        let format = "name: block_rq\nID: 1\nformat:\n\
            \tfield:dev_t dev;\toffset:8;\tsize:4;\tsigned:0;\n\
            \tfield:sector_t sector;\toffset:16;\tsize:8;\tsigned:0;\n\
            \tfield:void *req;\toffset:40;\tsize:8;\tsigned:0;\n\
            \tfield:unsigned int nr_sector;\toffset:48;\tsize:4;\tsigned:0;\n\
            \tfield:char rwbs[8];\toffset:56;\tsize:8;\tsigned:0;\n";

        write_block_format(events_root, "block_rq_issue", format);
        write_block_format(events_root, "block_rq_complete", format);

        let offsets = validate_block_io_tracepoint_offsets(events_root);

        assert!(offsets.block_rq);
        assert_eq!(offsets.block_rq_key_offset, Some(40));
    }

    #[test]
    fn block_io_issue_complete_request_pointer_mismatch_uses_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let events_root = temp.path();

        let issue_format = block_rq_format(Some((40, 8)), Some((48, 4)), Some((56, 8)));
        let complete_format = block_rq_format(Some((44, 8)), Some((48, 4)), Some((56, 8)));
        write_block_format(events_root, "block_rq_issue", &issue_format);
        write_block_format(events_root, "block_rq_complete", &complete_format);

        let offsets = validate_block_io_tracepoint_offsets(events_root);

        assert!(offsets.block_rq);
        assert!(offsets.block_rq_has_rwbs);
        assert_eq!(offsets.block_rq_key_offset, None);
        assert_eq!(offsets.block_rq_issue_nr_sector_offset, Some(48));
        assert_eq!(offsets.block_rq_complete_nr_sector_offset, Some(48));
    }

    #[test]
    fn parse_tracepoint_format_extracts_field_size_offset_and_signed_flag() {
        let format = parse_tracepoint_format(
            PathBuf::from("/tmp/format"),
            "\tfield:int pid;\toffset:24;\tsize:4;\tsigned:1;\n\
             \tfield:char rwbs[8];\toffset:32;\tsize:8;\tsigned:0;\n",
        );

        assert_eq!(
            format.fields.get("pid"),
            Some(&TracepointField {
                name: "pid".to_owned(),
                offset: 24,
                size: 4,
                signed: true,
                declaration: "field:int pid;\toffset:24;\tsize:4;\tsigned:1;".to_owned(),
            })
        );
        assert_eq!(
            format.fields.get("rwbs"),
            Some(&TracepointField {
                name: "rwbs".to_owned(),
                offset: 32,
                size: 8,
                signed: false,
                declaration: "field:char rwbs[8];\toffset:32;\tsize:8;\tsigned:0;".to_owned(),
            })
        );
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TracepointPreflightReport {
    pub sched_wakeup: String,
    pub sched_switch: String,
    pub sched_wakeup_new: String,
    pub sched_wakeup_new_coverage: String,
    pub sched_migrate_task: String,
    pub cpu_frequency: String,
    pub sched_stat_wait: String,
    pub irq_handler: String,
    pub block_rq: String,
    pub block_io_correlation_basis: String,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

pub fn tracepoint_preflight(
    events_root: &Path,
    wants_cpu_freq: bool,
    wants_stat_wait: bool,
    wants_irq_latency: bool,
    wants_block_io: bool,
    wants_follow_exec: bool,
) -> TracepointPreflightReport {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    let sched_wakeup = required_tracepoint_status(
        &events_root.join("sched/sched_wakeup/format"),
        &[("pid", 24), ("prio", 28), ("target_cpu", 32)],
        "sched_wakeup",
        &mut errors,
    );
    let sched_switch = required_tracepoint_status(
        &events_root.join("sched/sched_switch/format"),
        &[
            ("prev_pid", 24),
            ("prev_state", 32),
            ("next_comm", 40),
            ("next_pid", 56),
            ("next_prio", 60),
        ],
        "sched_switch",
        &mut errors,
    );
    let sched_wakeup_new = optional_tracepoint_status(
        &events_root.join("sched/sched_wakeup_new/format"),
        &[("pid", 24), ("prio", 28), ("target_cpu", 32)],
        "sched_wakeup_new",
        true,
        &mut warnings,
    );
    let sched_migrate_task = optional_tracepoint_status(
        &events_root.join("sched/sched_migrate_task/format"),
        &[("pid", 12), ("orig_cpu", 20), ("dest_cpu", 24)],
        "sched_migrate_task",
        true,
        &mut warnings,
    );
    let cpu_frequency = optional_tracepoint_status(
        &events_root.join("power/cpu_frequency/format"),
        &[("state", 8), ("cpu_id", 12)],
        "cpu_frequency",
        wants_cpu_freq,
        &mut warnings,
    );
    let sched_stat_wait = optional_tracepoint_status(
        &events_root.join("sched/sched_stat_wait/format"),
        &[("pid", 8), ("delay", 16)],
        "sched_stat_wait",
        wants_stat_wait,
        &mut warnings,
    );

    let irq_entry = events_root.join("irq/irq_handler_entry/format");
    let irq_exit = events_root.join("irq/irq_handler_exit/format");
    let irq_handler = if !wants_irq_latency {
        "not_requested".to_owned()
    } else if irq_entry.exists() && irq_exit.exists() {
        let entry_ok =
            validate_tracepoint_format_at_named(&irq_entry, "irq_handler_entry", &[("irq", 8)])
                .is_ok();
        let exit_ok =
            validate_tracepoint_format_at_named(&irq_exit, "irq_handler_exit", &[("irq", 8)])
                .is_ok()
                && require_tracepoint_field(&irq_exit, "ret").is_ok();
        if entry_ok && exit_ok {
            "ok".to_owned()
        } else {
            warnings.push("IRQ tracepoint formats are present but layouts differ".to_owned());
            "mismatch".to_owned()
        }
    } else {
        warnings.push("IRQ tracepoint formats are missing".to_owned());
        "missing".to_owned()
    };

    let (block_rq, block_io_correlation_basis) =
        block_tracepoint_preflight(events_root, wants_block_io, &mut warnings);

    let sched_wakeup_new_coverage =
        sched_wakeup_new_coverage_status(&sched_wakeup_new, &mut warnings);

    if wants_follow_exec {
        let exec_path = events_root.join("sched/sched_process_exec/format");
        if !exec_path.exists() {
            warnings.push(
                "sched_process_exec tracepoint missing; follow-exec cleanup may be degraded"
                    .to_owned(),
            );
        }
    }

    TracepointPreflightReport {
        sched_wakeup,
        sched_switch,
        sched_wakeup_new,
        sched_wakeup_new_coverage,
        sched_migrate_task,
        cpu_frequency,
        sched_stat_wait,
        irq_handler,
        block_rq,
        block_io_correlation_basis,
        warnings,
        errors,
    }
}

fn required_tracepoint_status(
    path: &Path,
    expected_offsets: &[(&str, usize)],
    name: &str,
    errors: &mut Vec<String>,
) -> String {
    match validate_tracepoint_format_at_named(path, name, expected_offsets) {
        Ok(()) => "ok".to_owned(),
        Err(err) => {
            errors.push(format!(
                "{name} tracepoint unavailable or incompatible: {err:#}"
            ));
            if path.exists() {
                "mismatch".to_owned()
            } else {
                "missing".to_owned()
            }
        }
    }
}

fn optional_tracepoint_status(
    path: &Path,
    expected_offsets: &[(&str, usize)],
    name: &str,
    wanted: bool,
    warnings: &mut Vec<String>,
) -> String {
    if !path.exists() {
        if wanted {
            warnings.push(format!("{name} tracepoint format is missing"));
        }
        return "missing".to_owned();
    }

    match validate_tracepoint_format_at_named(path, name, expected_offsets) {
        Ok(()) => "ok".to_owned(),
        Err(err) => {
            if wanted {
                warnings.push(format!("{name} tracepoint layout differs: {err:#}"));
            }
            "mismatch".to_owned()
        }
    }
}

fn sched_wakeup_new_coverage_status(
    sched_wakeup_new_status: &str,
    warnings: &mut Vec<String>,
) -> String {
    match sched_wakeup_new_status {
        "ok" => "full".to_owned(),
        "not_requested" => "not_requested".to_owned(),
        _ => {
            warnings.push(
                "optional sched_wakeup_new tracepoint unavailable; sched_wakeup remains required and usable, but wakeups for newly created tasks may have reduced coverage"
                    .to_owned(),
            );
            "reduced-new-task-wakeup-coverage".to_owned()
        }
    }
}

#[cfg(test)]
mod sched_wakeup_new_coverage_tests {
    use super::*;

    #[test]
    fn coverage_is_full_when_sched_wakeup_new_is_available() {
        let mut warnings = Vec::new();

        let coverage = sched_wakeup_new_coverage_status("ok", &mut warnings);

        assert_eq!(coverage, "full");
        assert!(warnings.is_empty());
    }

    #[test]
    fn coverage_warns_without_claiming_scheduler_wakeup_is_broken_when_missing() {
        let mut warnings = Vec::new();

        let coverage = sched_wakeup_new_coverage_status("missing", &mut warnings);

        assert_eq!(coverage, "reduced-new-task-wakeup-coverage");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("sched_wakeup_new"));
        assert!(warnings[0].contains("sched_wakeup remains required and usable"));
        assert!(warnings[0].contains("newly created tasks"));
    }

    #[test]
    fn coverage_is_not_requested_when_optional_tracepoint_is_not_requested() {
        let mut warnings = Vec::new();

        let coverage = sched_wakeup_new_coverage_status("not_requested", &mut warnings);

        assert_eq!(coverage, "not_requested");
        assert!(warnings.is_empty());
    }
}

fn block_tracepoint_preflight(
    events_root: &Path,
    wants_block_io: bool,
    warnings: &mut Vec<String>,
) -> (String, String) {
    let (block_rq, block_io_correlation_basis) = if !wants_block_io {
        ("not_requested".to_owned(), "not_requested".to_owned())
    } else {
        let offsets = validate_block_io_tracepoint_offsets(events_root);
        if offsets.block_rq {
            let basis = if offsets.block_rq_key_offset.is_some() {
                "request-pointer"
            } else {
                warnings.push(
                    "block I/O request-pointer key unavailable; dev+sector correlation is approximate"
                        .to_owned(),
                );
                "dev+sector"
            };
            ("ok".to_owned(), basis.to_owned())
        } else {
            // Error/warning already logged by validate_block_io_tracepoint_offsets
            ("missing".to_owned(), "unavailable".to_owned())
        }
    };

    (block_rq, block_io_correlation_basis)
}

#[cfg(test)]
fn parse_tracepoint_offsets(format_content: &str) -> BTreeMap<String, TracepointField> {
    parse_tracepoint_format(PathBuf::from("tracepoint"), format_content).fields
}

#[cfg(test)]
fn find_request_key_offset(offsets: &BTreeMap<String, TracepointField>) -> Option<u32> {
    for name in ["rq", "req", "request"] {
        if let Some(field) = offsets.get(name)
            && field.offset >= 8
            && field.offset % 8 == 0
            && field.size == 8
        {
            return Some(field.offset);
        }
    }

    None
}

#[cfg(test)]
fn matching_request_key_offset(
    issue_offsets: &BTreeMap<String, TracepointField>,
    complete_offsets: &BTreeMap<String, TracepointField>,
) -> Option<u32> {
    let issue_key_offset = find_request_key_offset(issue_offsets);
    let complete_key_offset = find_request_key_offset(complete_offsets);

    if issue_key_offset == complete_key_offset {
        issue_key_offset
    } else {
        None
    }
}

fn validate_tracepoint_formats(
    events_root: &Path,
    config: &crate::config::model::MonitorConfig,
) -> anyhow::Result<TracepointAvailability> {
    validate_tracepoint_format_at(
        &events_root.join("sched/sched_wakeup/format"),
        &[("pid", 24), ("prio", 28), ("target_cpu", 32)],
    )?;
    let sched_wakeup_new = validate_optional_tracepoint_format_at(
        &events_root.join("sched/sched_wakeup_new/format"),
        "sched_wakeup_new",
        &[("pid", 24), ("prio", 28), ("target_cpu", 32)],
        true,
    )?;
    validate_tracepoint_format_at_named(
        &events_root.join("sched/sched_switch/format"),
        "sched_switch",
        &[
            ("prev_pid", 24),
            ("prev_state", 32),
            ("next_comm", 40),
            ("next_pid", 56),
            ("next_prio", 60),
        ],
    )?;

    let sched_migrate_task = validate_optional_tracepoint_format_at(
        &events_root.join("sched/sched_migrate_task/format"),
        "sched_migrate_task",
        &[("pid", 12), ("orig_cpu", 20), ("dest_cpu", 24)],
        true,
    )?;
    let cpu_frequency = if config.probes.cpu_freq {
        validate_optional_tracepoint_format_at(
            &events_root.join("power/cpu_frequency/format"),
            "cpu_frequency",
            &[("state", 8), ("cpu_id", 12)],
            true,
        )?
    } else {
        false
    };
    let sched_stat_wait = if config.probes.stat_wait {
        validate_optional_tracepoint_format_at(
            &events_root.join("sched/sched_stat_wait/format"),
            "sched_stat_wait",
            &[("pid", 8), ("delay", 16)],
            true,
        )?
    } else {
        false
    };

    let irq_entry = events_root.join("irq/irq_handler_entry/format");
    let irq_exit = events_root.join("irq/irq_handler_exit/format");
    let irq_handler = if config.probes.irq_latency && irq_entry.exists() && irq_exit.exists() {
        validate_tracepoint_format_at_named(&irq_entry, "irq_handler_entry", &[("irq", 8)])?;
        validate_tracepoint_format_at_named(&irq_exit, "irq_handler_exit", &[("irq", 8)])?;

        // Validation-only for now. The eBPF program does not currently read
        // irq_handler_exit.ret, but the field must exist for kernels where IRQ exit
        // semantics are expected by the IRQ tracing path.
        let _ret_offset = require_tracepoint_field(&irq_exit, "ret")?;

        true
    } else {
        false
    };
    if !irq_handler && config.probes.irq_latency {
        log::warn!("IRQ tracepoint formats missing; continuing without IRQ latency probe");
    }

    let block_io = if config.probes.block_io {
        validate_block_io_tracepoint_offsets(events_root)
    } else {
        BlockIoTracepointOffsets::default()
    };

    let block_rq = block_io.block_rq;
    let block_rq_has_rwbs = block_io.block_rq_has_rwbs;
    let block_rq_key_offset = block_io.block_rq_key_offset;
    let block_rq_issue_nr_sector_offset = block_io.block_rq_issue_nr_sector_offset;
    let block_rq_issue_rwbs_offset = block_io.block_rq_issue_rwbs_offset;
    let block_rq_complete_nr_sector_offset = block_io.block_rq_complete_nr_sector_offset;
    let block_rq_complete_rwbs_offset = block_io.block_rq_complete_rwbs_offset;

    let sched_process_exit = validate_optional_tracepoint_format_at(
        &events_root.join("sched/sched_process_exit/format"),
        "sched_process_exit",
        &[],
        true,
    )?;

    let sched_process_exec = events_root.join("sched/sched_process_exec/format");
    let sched_process_exec = if config.safety.follow_exec && sched_process_exec.exists() {
        validate_tracepoint_format_at_named(&sched_process_exec, "sched_process_exec", &[])?;
        true
    } else {
        false
    };

    Ok(TracepointAvailability {
        sched_wakeup_new,
        sched_migrate_task,
        cpu_frequency,
        sched_stat_wait,
        irq_handler,
        block_rq,
        block_rq_has_rwbs,
        block_rq_key_offset,
        block_rq_issue_nr_sector_offset,
        block_rq_issue_rwbs_offset,
        block_rq_complete_nr_sector_offset,
        block_rq_complete_rwbs_offset,
        sched_process_exit,
        sched_process_exec,
    })
}

fn validate_optional_tracepoint_format_at(
    path: &Path,
    name: &str,
    expected_offsets: &[(&str, usize)],
    warn_on_missing: bool,
) -> anyhow::Result<bool> {
    if !path.exists() {
        if warn_on_missing {
            log::warn!(
                "optional tracepoint format missing: {}; continuing without {name}",
                path.display()
            );
        }
        return Ok(false);
    }

    validate_tracepoint_format_at_named(path, name, expected_offsets)?;
    Ok(true)
}

fn validate_tracepoint_format_at(
    path: &Path,
    expected_offsets: &[(&str, usize)],
) -> anyhow::Result<()> {
    let tracepoint_name = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("tracepoint");

    validate_tracepoint_format_at_named(path, tracepoint_name, expected_offsets)
}

fn validate_tracepoint_format_at_named(
    path: &Path,
    tracepoint_name: &str,
    expected_offsets: &[(&str, usize)],
) -> anyhow::Result<()> {
    let format = fs::read_to_string(path)
        .with_context(|| format!("failed to read tracepoint format {}", path.display()))?;

    validate_tracepoint_format_named(tracepoint_name, &format, expected_offsets).with_context(
        || {
            format!(
                "{tracepoint_name} tracepoint format {} did not match the eBPF program assumptions",
                path.display(),
            )
        },
    )
}

fn require_tracepoint_field(format_path: &Path, field_name: &str) -> anyhow::Result<u32> {
    let contents = fs::read_to_string(format_path)
        .with_context(|| format!("failed to read tracepoint format {}", format_path.display()))?;

    parse_tracepoint_field_offset(&contents, field_name).with_context(|| {
        format!(
            "tracepoint format {} is missing required field {:?}",
            format_path.display(),
            field_name
        )
    })
}

fn parse_tracepoint_field_offset(format_content: &str, field_name: &str) -> anyhow::Result<u32> {
    let format = parse_tracepoint_format(PathBuf::from("tracepoint"), format_content);
    format
        .fields
        .get(field_name)
        .map(|f| f.offset)
        .ok_or_else(|| anyhow::anyhow!("missing tracepoint field {:?}", field_name))
}

#[cfg(test)]
fn validate_tracepoint_format(
    format: &str,
    expected_offsets: &[(&str, usize)],
) -> anyhow::Result<()> {
    validate_tracepoint_format_named("tracepoint", format, expected_offsets)
}

fn validate_tracepoint_format_named(
    tracepoint_name: &str,
    format_content: &str,
    expected_offsets: &[(&str, usize)],
) -> anyhow::Result<()> {
    let format = parse_tracepoint_format(PathBuf::from(tracepoint_name), format_content);
    let fields = &format.fields;

    for &(field_name, expected_offset) in expected_offsets {
        let Some(field) = fields.get(field_name) else {
            return Err(tracepoint_missing_field_error(
                tracepoint_name,
                field_name,
                fields,
            ));
        };

        if field.offset as usize != expected_offset {
            return Err(tracepoint_offset_mismatch_error(
                tracepoint_name,
                field_name,
                expected_offset,
                field,
            ));
        }
    }

    Ok(())
}

fn tracepoint_offset_mismatch_error(
    tracepoint_name: &str,
    field_name: &str,
    expected: usize,
    field: &TracepointField,
) -> anyhow::Error {
    anyhow::anyhow!(
        "{} tracepoint layout mismatch for field `{}`: expected offset {}, got {}. Parsed declaration: `{}`{}",
        tracepoint_name,
        field_name,
        expected,
        field.offset,
        field.declaration,
        tracepoint_layout_hint(tracepoint_name, field_name),
    )
}

fn tracepoint_missing_field_error(
    tracepoint_name: &str,
    field_name: &str,
    fields: &BTreeMap<String, TracepointField>,
) -> anyhow::Error {
    let available = fields.keys().cloned().collect::<Vec<_>>().join(", ");

    anyhow::anyhow!(
        "{} tracepoint missing expected field `{}`. Available parsed fields: [{}].{}",
        tracepoint_name,
        field_name,
        available,
        tracepoint_layout_hint(tracepoint_name, field_name),
    )
}

fn tracepoint_layout_hint(tracepoint_name: &str, field_name: &str) -> &'static str {
    if tracepoint_name == "sched_switch"
        && matches!(
            field_name,
            "prev_state" | "next_comm" | "next_pid" | "next_prio"
        )
    {
        " Hint: `sched_switch` layout differs from stutter's eBPF read offsets. A common cause is a different `prev_state` field type/size, which shifts later fields such as `next_comm`, `next_pid`, and `next_prio`. stutter rejects this layout to avoid reading the wrong tracepoint bytes."
    } else {
        " Hint: the running kernel tracepoint format does not match stutter's compiled eBPF read offsets. stutter rejects this layout to avoid mis-decoding tracepoint data."
    }
}

/// Read an eBPF object from an external file path.
///
/// Returns an error if the file cannot be read or is empty.
pub(crate) fn read_prebuilt_bpf_object(path: &Path) -> anyhow::Result<Vec<u8>> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read prebuilt BPF object {}", path.display()))?;

    if bytes.is_empty() {
        anyhow::bail!("prebuilt BPF object {} is empty", path.display());
    }

    Ok(bytes)
}

/// Resolve the eBPF object bytes to load.
///
/// If `STUTTER_BPF_OBJECT` is set, reads that file at runtime. This allows
/// developers to test alternate objects without rebuilding userspace, and
/// packagers to ship a separate object file.
///
/// If the env var is not set, uses the object embedded at build time via
/// `aya::include_bytes_aligned!`.
///
/// If `STUTTER_BPF_OBJECT` is set but the file is unreadable or empty, this
/// function returns an error — it does **not** silently fall back to the
/// embedded object.
fn ebpf_object_bytes() -> anyhow::Result<Cow<'static, [u8]>> {
    if let Ok(path_str) = std::env::var("STUTTER_BPF_OBJECT") {
        let path = PathBuf::from(path_str);
        log::info!("using_prebuilt_bpf_object path={}", path.display());

        let bytes = read_prebuilt_bpf_object(&path)
            .with_context(|| format!("STUTTER_BPF_OBJECT={}", path.display()))?;

        Ok(Cow::Owned(bytes))
    } else {
        Ok(Cow::Borrowed(aya::include_bytes_aligned!(concat!(
            env!("OUT_DIR"),
            "/stutter"
        ))))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    // tokio::time::sleep removed as unused
    use super::*;

    #[test]
    fn parses_tracepoint_field_offsets() {
        let format = r#"
field:unsigned short common_type; offset:0; size:2; signed:0;
field:char prev_comm[16]; offset:8; size:16; signed:1;
field:pid_t prev_pid; offset:24; size:4; signed:1;
field:int prev_prio; offset:28; size:4; signed:1;
field:long prev_state; offset:32; size:8; signed:1;
field:char next_comm[16]; offset:40; size:16; signed:1;
field:pid_t next_pid; offset:56; size:4; signed:1;
field:int next_prio; offset:60; size:4; signed:1;
"#;

        let offsets = parse_tracepoint_offsets(format);

        assert_eq!(offsets.get("next_comm").map(|f| f.offset), Some(40));
        assert_eq!(offsets.get("next_pid").map(|f| f.offset), Some(56));
        assert_eq!(offsets.get("next_prio").map(|f| f.offset), Some(60));
        assert_eq!(offsets.get("next_comm").map(|f| f.size), Some(16));
        assert_eq!(offsets.get("next_pid").map(|f| f.size), Some(4));
        assert_eq!(offsets.get("next_prio").map(|f| f.size), Some(4));
    }

    #[test]
    fn parse_tracepoint_fields_preserves_original_declaration() {
        let format = "    field:char next_comm[16]; offset:40; size:16; signed:1;\n";

        let fields = parse_tracepoint_offsets(format);
        let field = fields.get("next_comm").unwrap();

        assert_eq!(field.name, "next_comm");
        assert_eq!(field.offset, 40);
        assert_eq!(field.size, 16);
        assert!(field.signed);
        assert_eq!(
            field.declaration,
            "field:char next_comm[16]; offset:40; size:16; signed:1;",
        );
    }

    #[test]
    fn request_pointer_key_requires_matching_issue_and_complete_offsets() {
        let issue_offsets =
            parse_tracepoint_offsets("field:struct request *rq; offset:40; size:8; signed:0;");
        let complete_offsets =
            parse_tracepoint_offsets("field:struct request *rq; offset:40; size:8; signed:0;");

        assert_eq!(
            matching_request_key_offset(&issue_offsets, &complete_offsets),
            Some(40),
        );
    }

    #[test]
    fn request_pointer_key_rejects_mismatched_or_missing_complete_offset() {
        let issue_offsets =
            parse_tracepoint_offsets("field:struct request *rq; offset:40; size:8; signed:0;");
        let mismatched_complete_offsets =
            parse_tracepoint_offsets("field:struct request *rq; offset:48; size:8; signed:0;");
        let missing_complete_offsets =
            parse_tracepoint_offsets("field:dev_t dev; offset:8; size:4; signed:0;");

        assert_eq!(
            matching_request_key_offset(&issue_offsets, &mismatched_complete_offsets),
            None,
        );
        assert_eq!(
            matching_request_key_offset(&issue_offsets, &missing_complete_offsets),
            None,
        );
    }

    #[test]
    fn request_pointer_key_rejects_wrong_size() {
        let issue_offsets = parse_tracepoint_offsets("field:u32 rq; offset:40; size:4; signed:0;");
        let complete_offsets =
            parse_tracepoint_offsets("field:u32 rq; offset:40; size:4; signed:0;");

        assert_eq!(
            matching_request_key_offset(&issue_offsets, &complete_offsets),
            None,
        );
    }

    #[test]
    fn validates_expected_tracepoint_offsets() {
        let format = r#"
field:char next_comm[16]; offset:40; size:16; signed:1;
field:pid_t next_pid; offset:56; size:4; signed:1;
field:int next_prio; offset:60; size:4; signed:1;
"#;

        validate_tracepoint_format(
            format,
            &[("next_comm", 40), ("next_pid", 56), ("next_prio", 60)],
        )
        .unwrap();
    }

    #[test]
    fn tracepoint_mismatch_error_includes_declaration_and_sched_switch_hint() {
        let format = r#"
    field:char prev_comm[16]; offset:8; size:16; signed:1;
    field:pid_t prev_pid; offset:24; size:4; signed:1;
    field:int prev_prio; offset:28; size:4; signed:1;
    field:int prev_state; offset:32; size:4; signed:1;
    field:char next_comm[16]; offset:36; size:16; signed:1;
    field:pid_t next_pid; offset:52; size:4; signed:1;
    field:int next_prio; offset:56; size:4; signed:1;
"#;

        let err = validate_tracepoint_format_named("sched_switch", format, &[("next_pid", 56)])
            .unwrap_err();
        let text = err.to_string();

        assert!(text.contains("sched_switch"));
        assert!(text.contains("next_pid"));
        assert!(text.contains("expected offset 56"));
        assert!(text.contains("got 52"));
        assert!(text.contains("field:pid_t next_pid; offset:52; size:4; signed:1;"));
        assert!(text.contains("prev_state"));
        assert!(text.contains("rejects this layout"));
    }

    #[test]
    fn tracepoint_missing_field_error_lists_available_fields() {
        let format = r#"
field:pid_t prev_pid; offset:24; size:4; signed:1;
field:long prev_state; offset:32; size:8; signed:1;
"#;

        let err = validate_tracepoint_format_named("sched_switch", format, &[("next_pid", 56)])
            .unwrap_err();
        let text = err.to_string();

        assert!(text.contains("missing expected field"));
        assert!(text.contains("next_pid"));
        assert!(text.contains("prev_pid"));
        assert!(text.contains("prev_state"));
    }

    #[test]
    fn validates_irq_tracepoint_offsets() {
        let format = r#"
field:unsigned short common_type; offset:0; size:2; signed:0;
field:int irq; offset:8; size:4; signed:1;
"#;

        validate_tracepoint_format(format, &[("irq", 8)]).unwrap();
    }

    #[test]
    fn rejects_bad_irq_tracepoint_offsets() {
        let format = "field:int irq; offset:12; size:4; signed:1;";

        let err = validate_tracepoint_format(format, &[("irq", 8)]).unwrap_err();
        assert!(err.to_string().contains("expected offset 8, got 12"));
    }

    const IRQ_HANDLER_EXIT_FORMAT_WITH_RET: &str = r#"
name: irq_handler_exit
ID: 1234
format:
	field:unsigned short common_type;	offset:0;	size:2;	signed:0;
	field:unsigned char common_flags;	offset:2;	size:1;	signed:0;
	field:unsigned char common_preempt_count;	offset:3;	size:1;	signed:0;
	field:int common_pid;	offset:4;	size:4;	signed:1;

	field:int irq;	offset:8;	size:4;	signed:1;
	field:int ret;	offset:12;	size:4;	signed:1;

print fmt: "irq=%d ret=%d", REC->irq, REC->ret
"#;

    const IRQ_HANDLER_EXIT_FORMAT_MISSING_RET: &str = r#"
name: irq_handler_exit
ID: 1234
format:
	field:unsigned short common_type;	offset:0;	size:2;	signed:0;
	field:int common_pid;	offset:4;	size:4;	signed:1;
	field:int irq;	offset:8;	size:4;	signed:1;
"#;

    #[test]
    fn parse_tracepoint_field_offset_finds_irq_and_ret() {
        assert_eq!(
            parse_tracepoint_field_offset(IRQ_HANDLER_EXIT_FORMAT_WITH_RET, "irq").unwrap(),
            8
        );

        assert_eq!(
            parse_tracepoint_field_offset(IRQ_HANDLER_EXIT_FORMAT_WITH_RET, "ret").unwrap(),
            12
        );
    }

    #[test]
    fn parse_tracepoint_field_offset_errors_when_ret_missing() {
        let err =
            parse_tracepoint_field_offset(IRQ_HANDLER_EXIT_FORMAT_MISSING_RET, "ret").unwrap_err();

        assert!(err.to_string().contains("ret"));
    }

    #[test]
    fn parse_tracepoint_field_offset_matches_exact_field_name() {
        let format = r#"
	field:int return_code;	offset:8;	size:4;	signed:1;
	field:int ret;	offset:12;	size:4;	signed:1;
"#;

        assert_eq!(parse_tracepoint_field_offset(format, "ret").unwrap(), 12);
    }

    #[test]
    fn optional_tracepoint_format_missing_is_not_an_error() {
        let dir = temp_dir("optional-tracepoint");
        fs::create_dir_all(&dir).unwrap();

        let available = validate_optional_tracepoint_format_at(
            &dir.join("missing/format"),
            "missing",
            &[("pid", 24)],
            true,
        )
        .unwrap();

        assert!(!available);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn parses_mem_available_from_proc_meminfo() {
        let meminfo = "MemTotal:       32768000 kB\nMemAvailable:   12345 kB\n";

        assert_eq!(parse_mem_available_bytes(meminfo), Some(12_641_280));
    }

    #[test]
    fn dynamic_map_sizing_grows_when_memory_is_plentiful() {
        let sizing = map_sizing_from_memory(MemorySnapshot {
            locked_memory_limit_bytes: None,
            available_memory_bytes: Some(128 * 1024 * 1024 * 1024),
            page_size: 4096,
        });

        assert_eq!(sizing.events_ringbuf_bytes, MAX_EVENTS_RINGBUF_BYTES);
        assert_eq!(sizing.wakeup_data_entries, MAX_WAKEUP_DATA_ENTRIES);
    }

    #[test]
    fn dynamic_map_sizing_respects_finite_memlock_budget() {
        let sizing = map_sizing_from_memory(MemorySnapshot {
            locked_memory_limit_bytes: Some(1024 * 1024),
            available_memory_bytes: Some(128 * 1024 * 1024 * 1024),
            page_size: 4096,
        });

        assert_eq!(sizing.events_ringbuf_bytes, 256 * 1024);
        assert_eq!(sizing.wakeup_data_entries, 8_192);
    }

    #[test]
    fn ring_buffer_size_is_power_of_two_and_page_aligned() {
        let size = ring_buffer_size_from_budget(900 * 1024, 64 * 1024, 16 * 1024 * 1024, 4096);

        assert_eq!(size, 512 * 1024);
    }

    #[test]
    fn gates_optional_tracepoint_validation_by_config() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let dir = temp_dir("gate-validation");

        // Required tracepoints must exist for validate_tracepoint_formats to succeed
        let sched_wakeup = dir.join("sched/sched_wakeup");
        fs::create_dir_all(&sched_wakeup).unwrap();
        fs::write(
            sched_wakeup.join("format"),
            "field:pid_t pid; offset:24; size:4; signed:1;\nfield:int prio; offset:28; size:4; signed:1;\nfield:int target_cpu; offset:32; size:4; signed:1;",
        ).unwrap();

        let sched_switch = dir.join("sched/sched_switch");
        fs::create_dir_all(&sched_switch).unwrap();
        fs::write(
            sched_switch.join("format"),
            "field:char prev_comm[16]; offset:8; size:16; signed:1;\nfield:pid_t prev_pid; offset:24; size:4; signed:1;\nfield:int prev_prio; offset:28; size:4; signed:1;\nfield:long prev_state; offset:32; size:8; signed:1;\nfield:char next_comm[16]; offset:40; size:16; signed:1;\nfield:pid_t next_pid; offset:56; size:4; signed:1;\nfield:int next_prio; offset:60; size:4; signed:1;",
        ).unwrap();

        let sched_process_exit = dir.join("sched/sched_process_exit");
        fs::create_dir_all(&sched_process_exit).unwrap();
        fs::write(
            sched_process_exit.join("format"),
            "field:pid_t pid; offset:12; size:4; signed:1;",
        )
        .unwrap();

        // Create a fake format file with WRONG offset for cpu_frequency
        let cpu_freq_dir = dir.join("power/cpu_frequency");
        fs::create_dir_all(&cpu_freq_dir).unwrap();
        fs::write(
            cpu_freq_dir.join("format"),
            "field:int state; offset:99; size:4; signed:1;\nfield:int cpu_id; offset:103; size:4; signed:1;",
        ).unwrap();

        // Create a fake format file with WRONG offset for sched_stat_wait
        let stat_wait_dir = dir.join("sched/sched_stat_wait");
        fs::create_dir_all(&stat_wait_dir).unwrap();
        fs::write(
            stat_wait_dir.join("format"),
            "field:pid_t pid; offset:99; size:4; signed:1;\nfield:u64 delay; offset:103; size:8; signed:0;",
        ).unwrap();

        // Create a fake format file with WRONG offset for IRQ
        let irq_entry_dir = dir.join("irq/irq_handler_entry");
        fs::create_dir_all(&irq_entry_dir).unwrap();
        fs::write(
            irq_entry_dir.join("format"),
            "field:int irq; offset:99; size:4; signed:1;",
        )
        .unwrap();
        let irq_exit_dir = dir.join("irq/irq_handler_exit");
        fs::create_dir_all(&irq_exit_dir).unwrap();
        fs::write(
            irq_exit_dir.join("format"),
            "field:int irq; offset:99; size:4; signed:1;",
        )
        .unwrap();

        let mut config = match crate::cli::parse_app_command_from([
            "stutter", "monitor", "--pid", "42",
        ])
        .unwrap()
        {
            crate::commands::AppCommand::Monitor(c) => (*c.config).clone(),
            _ => unreachable!(),
        };

        // Validating with optional features DISABLED should SUCCEED even with wrong formats
        config.probes.cpu_freq = false;
        config.probes.stat_wait = false;
        config.probes.irq_latency = false;
        config.probes.block_io = false;

        let availability = validate_tracepoint_formats(&dir, &config).unwrap();
        assert!(!availability.cpu_frequency);
        assert!(!availability.sched_stat_wait);
        assert!(!availability.irq_handler);

        // Validating with cpu_freq = true should FAIL
        config.probes.cpu_freq = true;
        let err = validate_tracepoint_formats(&dir, &config).unwrap_err();
        assert!(err.to_string().contains("cpu_frequency"));
        config.probes.cpu_freq = false;

        // Validating with stat_wait = true should FAIL
        config.probes.stat_wait = true;
        let err = validate_tracepoint_formats(&dir, &config).unwrap_err();
        assert!(err.to_string().contains("sched_stat_wait"));
        config.probes.stat_wait = false;

        // Validating with irq_latency = true should FAIL
        config.probes.irq_latency = true;
        // irq_latency also requires --irq N in CLI, but validate_tracepoint_formats
        // only cares about the irq_latency flag and existence of files.
        let err = validate_tracepoint_formats(&dir, &config).unwrap_err();
        assert!(err.to_string().contains("irq_handler_entry"));
        config.probes.irq_latency = false;

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn validates_sched_process_exit_availability() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let dir = temp_dir("process-exit");

        // Required tracepoints
        let sched_wakeup = dir.join("sched/sched_wakeup");
        fs::create_dir_all(&sched_wakeup).unwrap();
        fs::write(
            sched_wakeup.join("format"),
            "field:pid_t pid; offset:24; size:4; signed:1;\nfield:int prio; offset:28; size:4; signed:1;\nfield:int target_cpu; offset:32; size:4; signed:1;",
        ).unwrap();

        let sched_switch = dir.join("sched/sched_switch");
        fs::create_dir_all(&sched_switch).unwrap();
        fs::write(
            sched_switch.join("format"),
            "field:char prev_comm[16]; offset:8; size:16; signed:1;\nfield:pid_t prev_pid; offset:24; size:4; signed:1;\nfield:int prev_prio; offset:28; size:4; signed:1;\nfield:long prev_state; offset:32; size:8; signed:1;\nfield:char next_comm[16]; offset:40; size:16; signed:1;\nfield:pid_t next_pid; offset:56; size:4; signed:1;\nfield:int next_prio; offset:60; size:4; signed:1;",
        ).unwrap();

        let config = match crate::cli::parse_app_command_from(["stutter", "monitor", "--pid", "42"])
            .unwrap()
        {
            crate::commands::AppCommand::Monitor(c) => (*c.config).clone(),
            _ => unreachable!(),
        };

        // Case 1: sched/sched_process_exit/format missing
        let availability = validate_tracepoint_formats(&dir, &config).unwrap();
        assert!(!availability.sched_process_exit);

        // Case 2: sched/sched_process_exit/format present
        let sched_process_exit = dir.join("sched/sched_process_exit");
        fs::create_dir_all(&sched_process_exit).unwrap();
        fs::write(
            sched_process_exit.join("format"),
            "field:pid_t pid; offset:12; size:4; signed:1;",
        )
        .unwrap();

        let availability = validate_tracepoint_formats(&dir, &config).unwrap();
        assert!(availability.sched_process_exit);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn ringbuf_override_applies_and_rounds() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let config = match crate::cli::parse_app_command_from([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--ringbuf-size-kb",
            "1000",
        ])
        .unwrap()
        {
            crate::commands::AppCommand::Monitor(c) => (*c.config).clone(),
            _ => unreachable!(),
        };

        let sizing = map_sizing_for_config(&config);
        // 1000 KB = 1024000 bytes.
        // next_power_of_two(1024000) = 1048576 (1 MiB)
        assert_eq!(sizing.events_ringbuf_bytes, 1024 * 1024);
    }

    #[test]
    fn wakeup_map_factor_applies_and_clamps() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let config = match crate::cli::parse_app_command_from([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--wakeup-map-factor",
            "4",
            "--max-tasks",
            "1000",
        ])
        .unwrap()
        {
            crate::commands::AppCommand::Monitor(c) => (*c.config).clone(),
            _ => unreachable!(),
        };

        let sizing = map_sizing_for_config(&config);
        // 1000 * 4 = 4000.
        // MIN_WAKEUP_DATA_ENTRIES = 4096.
        assert_eq!(sizing.wakeup_data_entries, 4096);

        let config2 = match crate::cli::parse_app_command_from([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--wakeup-map-factor",
            "10",
            "--max-tasks",
            "2000",
        ])
        .unwrap()
        {
            crate::commands::AppCommand::Monitor(c) => (*c.config).clone(),
            _ => unreachable!(),
        };
        let sizing2 = map_sizing_for_config(&config2);
        // 2000 * 10 = 20000.
        assert_eq!(sizing2.wakeup_data_entries, 20000);
    }

    #[test]
    fn rejects_invalid_map_tuning_values() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        // ringbuf too small
        let err = crate::cli::parse_app_command_from([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--ringbuf-size-kb",
            "63",
        ])
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("--ringbuf-size-kb must be between 64 and 16384")
        );

        // ringbuf too large
        let err = crate::cli::parse_app_command_from([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--ringbuf-size-kb",
            "16385",
        ])
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("--ringbuf-size-kb must be between 64 and 16384")
        );

        // wakeup factor zero
        let err = crate::cli::parse_app_command_from([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--wakeup-map-factor",
            "0",
        ])
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("--wakeup-map-factor must be between 1 and 64")
        );

        // wakeup factor too large
        let err = crate::cli::parse_app_command_from([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--wakeup-map-factor",
            "65",
        ])
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("--wakeup-map-factor must be between 1 and 64")
        );
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        dir
    }

    #[test]
    fn read_prebuilt_bpf_object_reads_non_empty_file() {
        let dir = temp_dir("prebuilt-bpf");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stutter.bpf.o");

        fs::write(&path, b"fake-bpf-object").unwrap();

        let bytes = read_prebuilt_bpf_object(&path).unwrap();
        assert_eq!(bytes, b"fake-bpf-object");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn read_prebuilt_bpf_object_rejects_empty_file() {
        let dir = temp_dir("prebuilt-bpf-empty");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stutter.bpf.o");

        fs::write(&path, b"").unwrap();

        let err = read_prebuilt_bpf_object(&path).unwrap_err();
        assert!(err.to_string().contains("empty"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn read_prebuilt_bpf_object_rejects_missing_file() {
        let dir = temp_dir("prebuilt-bpf-missing");
        let path = dir.join("nonexistent.bpf.o");

        let err = read_prebuilt_bpf_object(&path).unwrap_err();
        assert!(err.to_string().contains("failed to read"));
    }
}
