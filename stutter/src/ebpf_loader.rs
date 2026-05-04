use std::{collections::BTreeMap, fs, path::Path};

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
    DROP_RINGBUF_RESERVE_FAILED, DROP_WAKEUP_DATA_INSERT_FAILED,
};
use tokio::io::unix::AsyncFd;

use crate::cli::TARGET_PIDS_MAX;

const DEFAULT_AVAILABLE_MEMORY_BYTES: u64 = 1 << 30;
const AVAILABLE_MEMORY_BUDGET_DIVISOR: u64 = 64;
const MEMLOCK_BUDGET_NUMERATOR: u64 = 3;
const MEMLOCK_BUDGET_DENOMINATOR: u64 = 4;
const WAKEUP_DATA_ENTRY_ESTIMATED_BYTES: u64 = 64;
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
    pub prev_faults_map: Option<AyaHashMap<MapData, u32, [u64; 2]>>, // (tid) -> (maj, min)
    pub block_io_correlation_basis: BlockIoCorrelationBasis,
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
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DropCountersSnapshot {
    pub wakeup_data_insert_failed: u64,
    pub ringbuf_reserve_failed: u64,
    #[serde(default)]
    pub irq_start_times_insert_failed: u64,
    #[serde(default)]
    pub block_start_insert_failed: u64,
}

impl DropCountersSnapshot {
    pub fn total(&self) -> u64 {
        self.wakeup_data_insert_failed
            .saturating_add(self.ringbuf_reserve_failed)
            .saturating_add(self.irq_start_times_insert_failed)
            .saturating_add(self.block_start_insert_failed)
    }

    pub fn total_excluding_block_io(&self) -> u64 {
        self.wakeup_data_insert_failed
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

pub fn load_and_attach(config: &crate::cli::Config) -> anyhow::Result<LoadedEbpf> {
    raise_memlock_limit();
    let map_sizing = dynamic_map_sizing();
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

    let mut ebpf = loader
        .load(aya::include_bytes_aligned!(concat!(
            env!("OUT_DIR"),
            "/stutter"
        )))
        .context("eBPF load failed")?;

    attach_tracepoint(&mut ebpf, "sched_wakeup", "sched", "sched_wakeup")
        .context("eBPF load failed: attach sched_wakeup")?;
    if tracepoints.sched_wakeup_new {
        attach_tracepoint(&mut ebpf, "sched_wakeup_new", "sched", "sched_wakeup_new")
            .context("eBPF load failed: attach sched_wakeup_new")?;
    }
    attach_tracepoint(&mut ebpf, "sched_switch", "sched", "sched_switch")
        .context("eBPF load failed: attach sched_switch")?;
    attach_tracepoint(
        &mut ebpf,
        "sched_process_exit",
        "sched",
        "sched_process_exit",
    )
    .context("eBPF load failed: attach sched_process_exit")?;

    if tracepoints.sched_migrate_task {
        attach_tracepoint(
            &mut ebpf,
            "sched_migrate_task",
            "sched",
            "sched_migrate_task",
        )
        .context("eBPF load failed: attach sched_migrate_task")?;
    }
    if tracepoints.cpu_frequency && config.cpu_freq {
        attach_tracepoint(&mut ebpf, "cpu_frequency", "power", "cpu_frequency")
            .context("eBPF load failed: attach cpu_frequency")?;
    }
    if tracepoints.sched_stat_wait && config.stat_wait {
        attach_tracepoint(&mut ebpf, "sched_stat_wait", "sched", "sched_stat_wait")
            .context("eBPF load failed: attach sched_stat_wait")?;
    }

    if tracepoints.irq_handler && config.irq_latency {
        attach_tracepoint(&mut ebpf, "irq_handler_entry", "irq", "irq_handler_entry")
            .context("eBPF load failed: attach irq_handler_entry")?;
        attach_tracepoint(&mut ebpf, "irq_handler_exit", "irq", "irq_handler_exit")
            .context("eBPF load failed: attach irq_handler_exit")?;
    }

    if tracepoints.block_rq && config.block_io {
        attach_tracepoint(&mut ebpf, "block_rq_issue", "block", "block_rq_issue")
            .context("eBPF load failed: attach block_rq_issue")?;
        attach_tracepoint(&mut ebpf, "block_rq_complete", "block", "block_rq_complete")
            .context("eBPF load failed: attach block_rq_complete")?;

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

    if tracepoints.sched_process_exec {
        attach_tracepoint(
            &mut ebpf,
            "sched_process_exec",
            "sched",
            "sched_process_exec",
        )
        .context("eBPF load failed: attach sched_process_exec")?;
    }

    if config.faults {
        // Fault perf events are optional correlation probes. If perf_event_open
        // is blocked by policy or capabilities, log a warning and continue rather
        // than aborting the whole profiler startup.
        if let Err(e) = attach_software_perf_event(&mut ebpf, "major_fault", 4) {
            log::warn!(
                "failed to attach major_fault perf event; continuing without fault probes: {}",
                e
            );
        }
        if let Err(e) = attach_software_perf_event(&mut ebpf, "minor_fault", 3) {
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

    if let Some(cgroup_path) = &config.cgroupv2 {
        // Pre-populate TARGET_PIDS from the cgroup hierarchy to avoid races
        // where a task appears in sched events before the eBPF-side target
        // maps are populated. Use a filtered snapshot to ensure that we
        // respect user-provided filters and do not exceed crate::cli::TARGET_PIDS_MAX
        // due to unrelated tasks in the same cgroup.
        let mut cache = crate::process_tree::ProcessCache::default();
        let snapshot =
            crate::process_tree::target_snapshot(crate::process_tree::TargetSnapshotInput {
                proc_root: Path::new("/proc"),
                manual_pids: &[],
                tree_pids: &[],
                cgroup_path: Some(cgroup_path),
                exclude_tree_pids: &config.exclude_tree_pids,
                filters: Some(&config.task_filters),
                keep_missing_pid: config.keep_missing_pid,
                cache: Some(&mut cache),
                previous_tasks: None,
            });
        let pids: Vec<_> = snapshot.tasks.keys().copied().collect();

        if pids.len() > TARGET_PIDS_MAX {
            anyhow::bail!(
                "cgroup target prepopulation failed: {} tasks in cgroup match filters, but target_pids_max is {}",
                pids.len(),
                crate::cli::TARGET_PIDS_MAX
            );
        }

        // Also respect the user-defined --max-tasks limit during prepopulation.
        if pids.len() > config.max_tasks {
            anyhow::bail!(
                "cgroup target prepopulation failed: {} tasks in cgroup match filters, but --max-tasks is {}",
                pids.len(),
                config.max_tasks
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
                crate::cli::TARGET_PIDS_MAX
            );
        }
    }

    Ok(LoadedEbpf {
        _ebpf: ebpf,
        events,
        target_pid_map,
        target_irq_map,
        prev_faults_map,
        block_io_correlation_basis,
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

fn attach_software_perf_event(
    ebpf: &mut Ebpf,
    program_name: &str,
    config: u64,
) -> anyhow::Result<()> {
    let program: &mut PerfEvent = ebpf
        .program_mut(program_name)
        .ok_or_else(|| anyhow::anyhow!("{program_name} program not found"))?
        .try_into()?;

    program.load()?;

    for cpu in online_cpus().map_err(|e| anyhow::anyhow!("{}: {}", e.0, e.1))? {
        let sw_event = match config {
            3 => aya::programs::perf_event::SoftwareEvent::PageFaultsMin,
            4 => aya::programs::perf_event::SoftwareEvent::PageFaultsMaj,
            _ => unreachable!(),
        };
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
struct EbpfMapSizing {
    events_ringbuf_bytes: u32,
    wakeup_data_entries: u32,
    locked_memory_limit_bytes: Option<u64>,
    available_memory_bytes: Option<u64>,
}

fn dynamic_map_sizing() -> EbpfMapSizing {
    let snapshot = MemorySnapshot {
        locked_memory_limit_bytes: locked_memory_limit_bytes(),
        available_memory_bytes: available_memory_bytes(),
        page_size: system_page_size(),
    };
    map_sizing_from_memory(snapshot)
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
        .checked_div(WAKEUP_DATA_ENTRY_ESTIMATED_BYTES)
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
    let mut rlim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    let ret = unsafe { libc::getrlimit(libc::RLIMIT_MEMLOCK, &mut rlim) };
    if ret != 0 || rlim.rlim_cur == libc::RLIM_INFINITY {
        None
    } else {
        Some(rlim.rlim_cur)
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

struct TracepointAvailability {
    sched_wakeup_new: bool,
    sched_migrate_task: bool,
    cpu_frequency: bool,
    sched_stat_wait: bool,
    irq_handler: bool,
    block_rq: bool,
    block_rq_has_rwbs: bool,
    block_rq_key_offset: Option<u32>,
    block_rq_issue_nr_sector_offset: Option<u32>,
    block_rq_issue_rwbs_offset: Option<u32>,
    block_rq_complete_nr_sector_offset: Option<u32>,
    block_rq_complete_rwbs_offset: Option<u32>,
    sched_process_exec: bool,
}

fn validate_tracepoint_formats(
    events_root: &Path,
    config: &crate::cli::Config,
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
    validate_tracepoint_format_at(
        &events_root.join("sched/sched_switch/format"),
        &[("next_comm", 40), ("next_pid", 56), ("next_prio", 60)],
    )?;

    let sched_migrate_task = validate_optional_tracepoint_format_at(
        &events_root.join("sched/sched_migrate_task/format"),
        "sched_migrate_task",
        &[("pid", 12), ("orig_cpu", 20), ("dest_cpu", 24)],
        true,
    )?;
    let cpu_frequency = validate_optional_tracepoint_format_at(
        &events_root.join("power/cpu_frequency/format"),
        "cpu_frequency",
        &[("state", 8), ("cpu_id", 12)],
        config.cpu_freq,
    )?;
    let sched_stat_wait = validate_optional_tracepoint_format_at(
        &events_root.join("sched/sched_stat_wait/format"),
        "sched_stat_wait",
        &[("pid", 8), ("delay", 16)],
        config.stat_wait,
    )?;

    let irq_entry = events_root.join("irq/irq_handler_entry/format");
    let irq_exit = events_root.join("irq/irq_handler_exit/format");
    let irq_handler = if irq_entry.exists() && irq_exit.exists() {
        validate_tracepoint_format_at(&irq_entry, &[("irq", 8)])?;
        validate_tracepoint_format_at(&irq_exit, &[("irq", 8)])?;
        true
    } else {
        false
    };
    if !irq_handler && config.irq_latency {
        log::warn!("IRQ tracepoint formats missing; continuing without IRQ latency probe");
    }

    let block_rq_issue = events_root.join("block/block_rq_issue/format");
    let block_rq_complete = events_root.join("block/block_rq_complete/format");
    let mut block_rq = false;
    let mut block_rq_has_rwbs = false;
    let mut block_rq_key_offset = None;
    let mut block_rq_issue_nr_sector_offset = None;
    let mut block_rq_issue_rwbs_offset = None;
    let mut block_rq_complete_nr_sector_offset = None;
    let mut block_rq_complete_rwbs_offset = None;

    if block_rq_issue.exists() && block_rq_complete.exists() {
        let issue_ok =
            validate_tracepoint_format_at(&block_rq_issue, &[("dev", 8), ("sector", 16)]).is_ok();
        let complete_ok = validate_tracepoint_format_at(
            &block_rq_complete,
            &[("dev", 8), ("sector", 16), ("nr_sector", 24)],
        )
        .is_ok();

        if issue_ok && complete_ok {
            let complete_fmt = fs::read_to_string(&block_rq_complete)
                .with_context(|| format!("failed to read {}", block_rq_complete.display()))?;
            let complete_offsets = parse_tracepoint_offsets(&complete_fmt);

            let issue_fmt = fs::read_to_string(&block_rq_issue)
                .with_context(|| format!("failed to read {}", block_rq_issue.display()))?;
            let issue_offsets = parse_tracepoint_offsets(&issue_fmt);

            block_rq = true;
            block_rq_key_offset = matching_request_key_offset(&issue_offsets, &complete_offsets);

            let use_nr_sector = issue_offsets.contains_key("nr_sector")
                && complete_offsets.contains_key("nr_sector");
            let use_rwbs =
                issue_offsets.contains_key("rwbs") && complete_offsets.contains_key("rwbs");

            if use_nr_sector {
                block_rq_issue_nr_sector_offset =
                    issue_offsets.get("nr_sector").map(|f| f.offset as u32);
                block_rq_complete_nr_sector_offset =
                    complete_offsets.get("nr_sector").map(|f| f.offset as u32);
            }
            if use_rwbs {
                block_rq_issue_rwbs_offset = issue_offsets.get("rwbs").map(|f| f.offset as u32);
                block_rq_complete_rwbs_offset =
                    complete_offsets.get("rwbs").map(|f| f.offset as u32);
            }

            block_rq_has_rwbs = block_rq_complete_rwbs_offset.is_some();

            if block_rq_key_offset.is_none() && config.block_io {
                let issue_key_offset = find_request_key_offset(&issue_offsets);
                let complete_key_offset = find_request_key_offset(&complete_offsets);
                log::warn!(
                    "request pointer key unavailable or mismatched between block_rq_issue ({issue_key_offset:?}) and block_rq_complete ({complete_key_offset:?}); falling back to metadata hashing"
                );

                if !use_nr_sector || !use_rwbs {
                    log::warn!(
                        "Block I/O correlation fallback is approximate: nr_sector available on both? {use_nr_sector}, rwbs available on both? {use_rwbs}. Missing fields will be excluded from the correlation hash."
                    );
                }
            }
        } else if config.block_io {
            log::warn!(
                "block I/O tracepoint missing required fields or layout mismatch; continuing without block I/O correlation"
            );
        }
    }

    let sched_process_exec = events_root.join("sched/sched_process_exec/format");
    let sched_process_exec = if sched_process_exec.exists() {
        validate_tracepoint_format_at(&sched_process_exec, &[])?;
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

    validate_tracepoint_format_at(path, expected_offsets)?;
    Ok(true)
}

fn validate_tracepoint_format_at(
    path: &Path,
    expected_offsets: &[(&str, usize)],
) -> anyhow::Result<()> {
    let format = fs::read_to_string(path)
        .with_context(|| format!("failed to read tracepoint format {}", path.display()))?;

    validate_tracepoint_format(&format, expected_offsets).with_context(|| {
        format!(
            "tracepoint format {} did not match the eBPF program assumptions",
            path.display()
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TracepointField {
    offset: usize,
    size: usize,
}

fn parse_tracepoint_offsets(format: &str) -> BTreeMap<String, TracepointField> {
    let mut fields = BTreeMap::new();

    for line in format.lines() {
        let line = line.trim();
        if !line.starts_with("field:") {
            continue;
        }

        let parts: Vec<_> = line.split(';').map(str::trim).collect();
        let Some(field_part) = parts.first() else {
            continue;
        };

        if !field_part.starts_with("field:") {
            continue;
        }

        let Some(field_name) = field_name_from_part(field_part) else {
            continue;
        };

        let mut offset = None;
        let mut size = None;

        for part in parts.iter().skip(1) {
            if let Some(val) = part.strip_prefix("offset:") {
                offset = val.trim().parse::<usize>().ok();
            } else if let Some(val) = part.strip_prefix("size:") {
                size = val.trim().parse::<usize>().ok();
            }
        }

        if let (Some(offset), Some(size)) = (offset, size) {
            fields.insert(field_name, TracepointField { offset, size });
        }
    }

    fields
}

fn find_request_key_offset(offsets: &BTreeMap<String, TracepointField>) -> Option<u32> {
    for name in ["rq", "req", "request"] {
        if let Some(field) = offsets.get(name)
            && field.offset >= 8
            && field.offset % 8 == 0
            && field.size == 8
        {
            return Some(field.offset as u32);
        }
    }

    None
}

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

fn validate_tracepoint_format(
    format: &str,
    expected_offsets: &[(&str, usize)],
) -> anyhow::Result<()> {
    let offsets = parse_tracepoint_offsets(format);

    for (field, expected_offset) in expected_offsets {
        let Some(actual_field) = offsets.get(*field) else {
            anyhow::bail!("missing field {field}");
        };

        if actual_field.offset != *expected_offset {
            anyhow::bail!(
                "field {field} offset mismatch: expected {expected_offset}, got {}",
                actual_field.offset
            );
        }
    }

    Ok(())
}

fn field_name_from_part(field_part: &str) -> Option<String> {
    let declaration = field_part.strip_prefix("field:")?.trim();
    let token = declaration.split_whitespace().last()?;
    let token = token.trim_start_matches('*');

    let field_name = match token.split_once('[') {
        Some((name, _)) => name,
        None => token,
    };

    if field_name.is_empty() {
        return None;
    }

    Some(field_name.to_owned())
}

fn raise_memlock_limit() {
    let rlim = libc::rlimit {
        rlim_cur: libc::RLIM_INFINITY,
        rlim_max: libc::RLIM_INFINITY,
    };

    let ret = unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &rlim) };
    if ret != 0 {
        eprintln!("warning: failed to raise RLIMIT_MEMLOCK");
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
    fn rejects_mismatched_tracepoint_offsets() {
        let format = r#"
field:char next_comm[16]; offset:40; size:16; signed:1;
field:pid_t next_pid; offset:52; size:4; signed:1;
field:int next_prio; offset:60; size:4; signed:1;
"#;

        let err = validate_tracepoint_format(format, &[("next_pid", 56)]).unwrap_err();
        assert!(err.to_string().contains("next_pid"));
        assert!(err.to_string().contains("expected 56, got 52"));
    }

    #[test]
    fn rejects_missing_tracepoint_fields() {
        let err = validate_tracepoint_format("field:int pid; offset:24;", &[("next_pid", 56)])
            .unwrap_err();
        assert!(err.to_string().contains("missing field next_pid"));
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
        assert!(err.to_string().contains("expected 8, got 12"));
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
}
