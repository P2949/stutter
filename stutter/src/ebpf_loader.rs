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
    DROP_RINGBUF_RESERVE_FAILED, DROP_WAKER_MAP_INSERT_FAILED, DROP_WAKEUP_TIMES_INSERT_FAILED,
};
use tokio::io::unix::AsyncFd;

const DEFAULT_AVAILABLE_MEMORY_BYTES: u64 = 1 << 30;
const AVAILABLE_MEMORY_BUDGET_DIVISOR: u64 = 64;
const MEMLOCK_BUDGET_NUMERATOR: u64 = 3;
const MEMLOCK_BUDGET_DENOMINATOR: u64 = 4;
const WAKEUP_TIMES_ENTRY_ESTIMATED_BYTES: u64 = 64;
const MIN_WAKEUP_TIMES_ENTRIES: u32 = 4_096;
const MAX_WAKEUP_TIMES_ENTRIES: u32 = 1_048_576;
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
    drop_counters: PerCpuArray<MapData, u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DropCountersSnapshot {
    pub wakeup_times_insert_failed: u64,
    pub ringbuf_reserve_failed: u64,
    #[serde(default)]
    pub irq_start_times_insert_failed: u64,
    #[serde(default)]
    pub waker_map_insert_failed: u64,
    #[serde(default)]
    pub block_start_insert_failed: u64,
}

impl DropCountersSnapshot {
    pub fn total(&self) -> u64 {
        self.wakeup_times_insert_failed
            .saturating_add(self.ringbuf_reserve_failed)
            .saturating_add(self.irq_start_times_insert_failed)
            .saturating_add(self.waker_map_insert_failed)
            .saturating_add(self.block_start_insert_failed)
    }
}

impl LoadedEbpf {
    pub fn snapshot_drop_counters(&self) -> DropCountersSnapshot {
        DropCountersSnapshot {
            wakeup_times_insert_failed: drop_counter_value(
                &self.drop_counters,
                DROP_WAKEUP_TIMES_INSERT_FAILED,
            ),
            ringbuf_reserve_failed: drop_counter_value(
                &self.drop_counters,
                DROP_RINGBUF_RESERVE_FAILED,
            ),
            irq_start_times_insert_failed: drop_counter_value(
                &self.drop_counters,
                DROP_IRQ_START_TIMES_INSERT_FAILED,
            ),
            waker_map_insert_failed: drop_counter_value(
                &self.drop_counters,
                DROP_WAKER_MAP_INSERT_FAILED,
            ),
            block_start_insert_failed: drop_counter_value(
                &self.drop_counters,
                DROP_BLOCK_START_INSERT_FAILED,
            ),
        }
    }
}

pub fn load_and_attach(
    config: &crate::cli::Config,
) -> Result<LoadedEbpf, crate::error::StutterError> {
    raise_memlock_limit();
    let map_sizing = dynamic_map_sizing();
    log::info!(
        "ebpf_map_sizing locked_memory_limit={} available_memory={} events_ringbuf_bytes={} wakeup_times_entries={}",
        format_optional_bytes(map_sizing.locked_memory_limit_bytes),
        format_optional_bytes(map_sizing.available_memory_bytes),
        map_sizing.events_ringbuf_bytes,
        map_sizing.wakeup_times_entries,
    );
    let tracepoints = validate_tracepoint_formats(Path::new("/sys/kernel/tracing/events"))
        .map_err(|e| crate::error::StutterError::TracepointOffsetMismatch(e.to_string()))?;

    let mut loader = EbpfLoader::new();
    loader
        .map_max_entries("EVENTS", map_sizing.events_ringbuf_bytes)
        .map_max_entries("WAKEUP_TIMES", map_sizing.wakeup_times_entries);

    let mut ebpf = loader
        .load(aya::include_bytes_aligned!(concat!(
            env!("OUT_DIR"),
            "/stutter"
        )))
        .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;

    attach_tracepoint(&mut ebpf, "sched_wakeup", "sched", "sched_wakeup")
        .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
    if tracepoints.sched_wakeup_new {
        attach_tracepoint(&mut ebpf, "sched_wakeup_new", "sched", "sched_wakeup_new")
            .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
    }
    attach_tracepoint(&mut ebpf, "sched_switch", "sched", "sched_switch")
        .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
    attach_tracepoint(
        &mut ebpf,
        "sched_process_exit",
        "sched",
        "sched_process_exit",
    )
    .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;

    if tracepoints.sched_migrate_task {
        attach_tracepoint(
            &mut ebpf,
            "sched_migrate_task",
            "sched",
            "sched_migrate_task",
        )
        .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
    }
    if tracepoints.cpu_frequency {
        attach_tracepoint(&mut ebpf, "cpu_frequency", "power", "cpu_frequency")
            .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
    }
    if tracepoints.sched_stat_wait {
        attach_tracepoint(&mut ebpf, "sched_stat_wait", "sched", "sched_stat_wait")
            .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
    }

    if tracepoints.irq_handler {
        attach_tracepoint(&mut ebpf, "irq_handler_entry", "irq", "irq_handler_entry")
            .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
        attach_tracepoint(&mut ebpf, "irq_handler_exit", "irq", "irq_handler_exit")
            .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
    }

    if tracepoints.page_fault_user {
        attach_tracepoint(
            &mut ebpf,
            "page_fault_user",
            "exceptions",
            "page_fault_user",
        )
        .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
    }
    if tracepoints.block_rq {
        attach_tracepoint(&mut ebpf, "block_rq_issue", "block", "block_rq_issue")
            .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
        attach_tracepoint(&mut ebpf, "block_rq_complete", "block", "block_rq_complete")
            .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
        if !tracepoints.block_rq_has_rwbs {
            log::warn!(
                "block_rq tracepoints missing `rwbs`; block I/O correlation will continue but read/write flags are unavailable"
            );
        }
        log::warn!("block I/O correlation uses dev+sector hashing; concurrent same-sector requests may collide");
    }

    if tracepoints.sched_process_exec {
        attach_tracepoint(
            &mut ebpf,
            "sched_process_exec",
            "sched",
            "sched_process_exec",
        )
        .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
    }

    // Fault perf events are optional correlation probes. If perf_event_open
    // is blocked by policy or capabilities, log a warning and continue rather
    // than aborting the whole profiler startup.
    if let Err(e) = attach_software_perf_event(&mut ebpf, "major_fault", 4) {
        log::warn!("failed to attach major_fault perf event; continuing without fault probes: {}", e);
    }
    if let Err(e) = attach_software_perf_event(&mut ebpf, "minor_fault", 3) {
        log::warn!("failed to attach minor_fault perf event; continuing without fault probes: {}", e);
    }

    let mut target_pid_map = AyaHashMap::try_from(ebpf.take_map("TARGET_PIDS").ok_or_else(|| {
        crate::error::StutterError::EbpfLoad("TARGET_PIDS map not found".to_owned())
    })?)
    .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;

    let target_irq_map = ebpf
        .take_map("TARGET_IRQS")
        .map(AyaHashMap::try_from)
        .transpose()
        .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;

    let drop_counters = PerCpuArray::try_from(ebpf.take_map("DROP_COUNTERS").ok_or_else(|| {
        crate::error::StutterError::EbpfLoad("DROP_COUNTERS map not found".to_owned())
    })?)
    .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;

    let events =
        RingBuf::try_from(ebpf.take_map("EVENTS").ok_or_else(|| {
            crate::error::StutterError::EbpfLoad("EVENTS map not found".to_owned())
        })?)
        .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;

    let events =
        AsyncFd::new(events).map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;

    let prev_faults_map = ebpf
        .take_map("PREV_FAULTS")
        .map(AyaHashMap::try_from)
        .transpose()
        .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;

    if let Some(cgroup_path) = &config.cgroupv2 {
        // Pre-populate TARGET_PIDS from the cgroup hierarchy to avoid races
        // where a task appears in sched events before the eBPF-side cgroup
        // membership maps are populated.
        let mut pids = Vec::new();
        collect_cgroup_hierarchy_pids(cgroup_path, &mut pids)
            .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;

        pids.sort_unstable();
        pids.dedup();

        let mut failed_inserts = 0usize;
        for pid in pids.iter() {
            if target_pid_map.insert(*pid, 1, 0).is_err() {
                failed_inserts += 1;
            }
        }

        if failed_inserts > 0 {
            return Err(crate::error::StutterError::EbpfLoad(format!(
                "cgroup target prepopulation failed: {} tasks failed to insert (target_pids_max={}); use a smaller cgroup or explicitly set --allow-partial-cgroup (if supported)",
                failed_inserts,
                crate::TARGET_PIDS_MAX
            )));
        }

        if pids.len() > crate::TARGET_PIDS_MAX {
            return Err(crate::error::StutterError::EbpfLoad(format!(
                "cgroup target prepopulation failed: cgroup has {} tasks but target_pids_max is {}",
                pids.len(),
                crate::TARGET_PIDS_MAX
            )));
        }
    }

    Ok(LoadedEbpf {
        _ebpf: ebpf,
        events,
        target_pid_map,
        target_irq_map,
        prev_faults_map,
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
    wakeup_times_entries: u32,
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
    let wakeup_times_entries = wakeup_budget
        .checked_div(WAKEUP_TIMES_ENTRY_ESTIMATED_BYTES)
        .unwrap_or(0)
        .clamp(
            u64::from(MIN_WAKEUP_TIMES_ENTRIES),
            u64::from(MAX_WAKEUP_TIMES_ENTRIES),
        ) as u32;

    EbpfMapSizing {
        events_ringbuf_bytes,
        wakeup_times_entries,
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
    page_fault_user: bool,
    block_rq: bool,
    block_rq_has_rwbs: bool,
    sched_process_exec: bool,
}

fn validate_tracepoint_formats(events_root: &Path) -> anyhow::Result<TracepointAvailability> {
    validate_tracepoint_format_at(
        &events_root.join("sched/sched_wakeup/format"),
        &[("pid", 24), ("prio", 28), ("target_cpu", 32)],
    )?;
    let sched_wakeup_new = validate_optional_tracepoint_format_at(
        &events_root.join("sched/sched_wakeup_new/format"),
        &[("pid", 24), ("prio", 28), ("target_cpu", 32)],
    )?;
    validate_tracepoint_format_at(
        &events_root.join("sched/sched_switch/format"),
        &[("next_comm", 40), ("next_pid", 56), ("next_prio", 60)],
    )?;

    let sched_migrate_task = validate_optional_tracepoint_format_at(
        &events_root.join("sched/sched_migrate_task/format"),
        &[("pid", 12), ("orig_cpu", 20), ("dest_cpu", 24)],
    )?;
    let cpu_frequency = validate_optional_tracepoint_format_at(
        &events_root.join("power/cpu_frequency/format"),
        &[("state", 8), ("cpu_id", 12)],
    )?;
    let sched_stat_wait = validate_optional_tracepoint_format_at(
        &events_root.join("sched/sched_stat_wait/format"),
        &[("pid", 8), ("delay", 16)],
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
    if !irq_handler {
        log::warn!("IRQ tracepoint formats missing; continuing without IRQ latency probe");
    }

    let page_fault_user = events_root.join("exceptions/page_fault_user/format");
    let page_fault_user = if page_fault_user.exists() {
        validate_tracepoint_format_at(&page_fault_user, &[])?; // No specific fields needed for now
        true
    } else {
        false
    };

    let block_rq_issue = events_root.join("block/block_rq_issue/format");
    let block_rq_complete = events_root.join("block/block_rq_complete/format");
    let mut block_rq = false;
    let mut block_rq_has_rwbs = false;

    if block_rq_issue.exists() && block_rq_complete.exists() {
        // Parse offsets and ensure required fields exist. `rwbs` and a
        // request pointer are optional — presence improves correlation but
        // must not make the whole probe fatal if absent.
        let issue_fmt = fs::read_to_string(&block_rq_issue)
            .with_context(|| format!("failed to read {}", block_rq_issue.display()))?;
        let complete_fmt = fs::read_to_string(&block_rq_complete)
            .with_context(|| format!("failed to read {}", block_rq_complete.display()))?;

        let issue_offsets = parse_tracepoint_offsets(&issue_fmt);
        let complete_offsets = parse_tracepoint_offsets(&complete_fmt);

        let issue_ok = issue_offsets.contains_key("dev")
            && issue_offsets.contains_key("sector")
            && issue_offsets.contains_key("nr_sector");
        let complete_ok = complete_offsets.contains_key("dev")
            && complete_offsets.contains_key("sector")
            && complete_offsets.contains_key("nr_sector");

        if issue_ok && complete_ok {
            // Require `rwbs` in the complete tracepoint so the eBPF program's
            // assumptions about offsets hold. If `rwbs` is missing the kernel
            // layout differs and we must disable block I/O attachment to avoid
            // tracepoint read errors in the eBPF program.
            if complete_offsets.contains_key("rwbs") {
                block_rq = true;
                block_rq_has_rwbs = true;
            } else {
                log::warn!(
                    "block I/O tracepoint present but missing `rwbs` in block_rq_complete; disabling block I/O attachment to avoid eBPF/tracepoint mismatch"
                );
                block_rq = false;
            }
        } else {
            log::warn!(
                "block I/O tracepoint missing required fields; continuing without block I/O correlation"
            );
            block_rq = false;
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
        page_fault_user,
        block_rq,
        block_rq_has_rwbs,
        sched_process_exec,
    })
}

fn validate_optional_tracepoint_format_at(
    path: &Path,
    expected_offsets: &[(&str, usize)],
) -> anyhow::Result<bool> {
    if !path.exists() {
        log::warn!(
            "optional tracepoint format missing: {}; continuing without sched_wakeup_new",
            path.display()
        );
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

fn parse_tracepoint_offsets(format: &str) -> BTreeMap<String, usize> {
    let mut offsets = BTreeMap::new();

    for line in format.lines() {
        let line = line.trim();
        if !line.starts_with("field:") {
            continue;
        }

        let mut parts = line.split(';').map(str::trim);
        let Some(field_part) = parts.next() else {
            continue;
        };
        let Some(offset_part) = parts.find(|part| part.starts_with("offset:")) else {
            continue;
        };

        let Some(field_name) = field_name_from_part(field_part) else {
            continue;
        };
        let Some(offset) = offset_part
            .strip_prefix("offset:")
            .and_then(|offset| offset.trim().parse::<usize>().ok())
        else {
            continue;
        };

        offsets.insert(field_name, offset);
    }

    offsets
}

fn validate_tracepoint_format(
    format: &str,
    expected_offsets: &[(&str, usize)],
) -> anyhow::Result<()> {
    let offsets = parse_tracepoint_offsets(format);

    for (field, expected_offset) in expected_offsets {
        let Some(actual_offset) = offsets.get(*field) else {
            anyhow::bail!("missing field {field}");
        };

        if *actual_offset != *expected_offset {
            anyhow::bail!(
                "field {field} offset mismatch: expected {expected_offset}, got {actual_offset}"
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

fn collect_cgroup_hierarchy_pids(path: &Path, pids: &mut Vec<u32>) -> anyhow::Result<()> {
    // collect PIDs from cgroup.procs and cgroup.threads recursively
    let procs = path.join("cgroup.procs");
    let threads = path.join("cgroup.threads");

    let mut read_list = |file: &Path| -> anyhow::Result<()> {
        if !file.exists() {
            return Ok(());
        }
        let data = fs::read_to_string(file)
            .with_context(|| format!("failed to read {}", file.display()))?;
        for line in data.lines() {
            if let Ok(pid) = line.trim().parse::<u32>() {
                pids.push(pid);
            }
        }
        Ok(())
    };

    read_list(&procs)?;
    read_list(&threads)?;

    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type() && file_type.is_dir() {
                collect_cgroup_hierarchy_pids(&entry.path(), pids)?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
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

        assert_eq!(offsets.get("next_comm"), Some(&40));
        assert_eq!(offsets.get("next_pid"), Some(&56));
        assert_eq!(offsets.get("next_prio"), Some(&60));
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

        let available =
            validate_optional_tracepoint_format_at(&dir.join("missing/format"), &[("pid", 24)])
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
        assert_eq!(sizing.wakeup_times_entries, MAX_WAKEUP_TIMES_ENTRIES);
    }

    #[test]
    fn dynamic_map_sizing_respects_finite_memlock_budget() {
        let sizing = map_sizing_from_memory(MemorySnapshot {
            locked_memory_limit_bytes: Some(1024 * 1024),
            available_memory_bytes: Some(128 * 1024 * 1024 * 1024),
            page_size: 4096,
        });

        assert_eq!(sizing.events_ringbuf_bytes, 256 * 1024);
        assert_eq!(sizing.wakeup_times_entries, 8_192);
    }

    #[test]
    fn ring_buffer_size_is_power_of_two_and_page_aligned() {
        let size = ring_buffer_size_from_budget(900 * 1024, 64 * 1024, 16 * 1024 * 1024, 4096);

        assert_eq!(size, 512 * 1024);
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        dir
    }
}
