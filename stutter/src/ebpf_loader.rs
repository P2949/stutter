use std::{collections::BTreeMap, fs, path::Path};

use anyhow::Context;
use aya::{
    Ebpf,
    maps::{HashMap as AyaHashMap, MapData, PerCpuArray, RingBuf},
    programs::{PerfEvent, TracePoint},
    util::online_cpus,
};
use serde::{Deserialize, Serialize};
use stutter_common::{
    DROP_IRQ_START_TIMES_INSERT_FAILED, DROP_RINGBUF_RESERVE_FAILED,
    DROP_WAKER_MAP_INSERT_FAILED, DROP_WAKEUP_TIMES_INSERT_FAILED,
    DROP_BLOCK_START_INSERT_FAILED,
};
use tokio::io::unix::AsyncFd;

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

pub fn load_and_attach() -> Result<LoadedEbpf, crate::error::StutterError> {
    raise_memlock_limit();
    let tracepoints = validate_tracepoint_formats(Path::new("/sys/kernel/tracing/events"))
        .map_err(|e| crate::error::StutterError::TracepointOffsetMismatch(e.to_string()))?;

    let mut ebpf = Ebpf::load(aya::include_bytes_aligned!(concat!(
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
        attach_tracepoint(&mut ebpf, "sched_migrate_task", "sched", "sched_migrate_task")
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
        attach_tracepoint(&mut ebpf, "page_fault_user", "exceptions", "page_fault_user")
            .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
    }
    if tracepoints.block_rq {
        attach_tracepoint(&mut ebpf, "block_rq_issue", "block", "block_rq_issue")
            .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
        attach_tracepoint(&mut ebpf, "block_rq_complete", "block", "block_rq_complete")
            .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
    }

    attach_software_perf_event(&mut ebpf, "major_fault", 4) // PERF_COUNT_SW_PAGE_FAULTS_MAJ
        .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
    attach_software_perf_event(&mut ebpf, "minor_fault", 3) // PERF_COUNT_SW_PAGE_FAULTS_MIN
        .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;

    let target_pid_map = AyaHashMap::try_from(ebpf.take_map("TARGET_PIDS").ok_or_else(|| {
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

struct TracepointAvailability {
    sched_wakeup_new: bool,
    sched_migrate_task: bool,
    cpu_frequency: bool,
    sched_stat_wait: bool,
    irq_handler: bool,
    page_fault_user: bool,
    block_rq: bool,
}

fn validate_tracepoint_formats(events_root: &Path) -> anyhow::Result<TracepointAvailability> {
    validate_tracepoint_format_at(
        &events_root.join("sched/sched_wakeup/format"),
        &[("pid", 24), ("prio", 28)],
    )?;
    let sched_wakeup_new = validate_optional_tracepoint_format_at(
        &events_root.join("sched/sched_wakeup_new/format"),
        &[("pid", 24), ("prio", 28)],
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
    let block_rq = if block_rq_issue.exists() && block_rq_complete.exists() {
        validate_tracepoint_format_at(&block_rq_issue, &[("dev", 8), ("sector", 16), ("nr_sector", 24), ("rwbs", 32)])?;
        validate_tracepoint_format_at(&block_rq_complete, &[("dev", 8), ("sector", 16), ("nr_sector", 24)])?;
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
