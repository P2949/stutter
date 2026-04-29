use std::{collections::BTreeMap, fs, path::Path};

use anyhow::Context;
use aya::{
    Ebpf,
    maps::{HashMap as AyaHashMap, MapData, PerCpuArray, RingBuf},
    programs::TracePoint,
};
use serde::{Deserialize, Serialize};
use stutter_common::{DROP_RINGBUF_RESERVE_FAILED, DROP_WAKEUP_TIMES_INSERT_FAILED};
use tokio::io::unix::AsyncFd;

pub struct LoadedEbpf {
    #[allow(dead_code)]
    ebpf: Ebpf,
    pub events: AsyncFd<RingBuf<MapData>>,
    pub target_pid_map: AyaHashMap<MapData, u32, u8>,
    pub target_irq_map: Option<AyaHashMap<MapData, u32, u8>>,
    drop_counters: PerCpuArray<MapData, u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DropCountersSnapshot {
    pub wakeup_times_insert_failed: u64,
    pub ringbuf_reserve_failed: u64,
}

impl DropCountersSnapshot {
    pub fn total(&self) -> u64 {
        self.wakeup_times_insert_failed
            .saturating_add(self.ringbuf_reserve_failed)
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
    if tracepoints.irq_handler {
        attach_tracepoint(&mut ebpf, "irq_handler_entry", "irq", "irq_handler_entry")
            .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
        attach_tracepoint(&mut ebpf, "irq_handler_exit", "irq", "irq_handler_exit")
            .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;
    }

    let target_pid_map = AyaHashMap::try_from(
        ebpf.take_map("TARGET_PIDS")
            .ok_or_else(|| crate::error::StutterError::EbpfLoad("TARGET_PIDS map not found".to_owned()))?,
    )
    .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;

    let target_irq_map = ebpf
        .take_map("TARGET_IRQS")
        .map(AyaHashMap::try_from)
        .transpose()
        .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;

    let drop_counters = PerCpuArray::try_from(
        ebpf.take_map("DROP_COUNTERS")
            .ok_or_else(|| crate::error::StutterError::EbpfLoad("DROP_COUNTERS map not found".to_owned()))?,
    )
    .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;

    let events = RingBuf::try_from(
        ebpf.take_map("EVENTS")
            .ok_or_else(|| crate::error::StutterError::EbpfLoad("EVENTS map not found".to_owned()))?,
    )
    .map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;

    let events = AsyncFd::new(events).map_err(|e| crate::error::StutterError::EbpfLoad(e.to_string()))?;

    Ok(LoadedEbpf {
        ebpf,
        events,
        target_pid_map,
        target_irq_map,
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

fn drop_counter_value(counters: &PerCpuArray<MapData, u64>, key: u32) -> u64 {
    counters
        .get(&key, 0)
        .map(|values| values.iter().copied().sum())
        .unwrap_or(0)
}

struct TracepointAvailability {
    sched_wakeup_new: bool,
    irq_handler: bool,
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
    let irq_handler = events_root.join("irq/irq_handler_entry/format").exists()
        && events_root.join("irq/irq_handler_exit/format").exists();
    if !irq_handler {
        log::warn!("IRQ tracepoint formats missing; continuing without IRQ latency probe");
    }
    Ok(TracepointAvailability {
        sched_wakeup_new,
        irq_handler,
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

    let offsets = parse_tracepoint_offsets(&format);

    for (field, expected_offset) in expected_offsets {
        let Some(actual_offset) = offsets.get(*field) else {
            anyhow::bail!(
                "tracepoint format {} missing field {field}; expected offset {expected_offset};\n\nThis usually means your kernel's tracepoint layout differs from the eBPF program's assumptions. Try rebuilding the eBPF program against your current kernel headers or upgrading your kernel. If that doesn't help, please open an issue providing your kernel version and the contents of {}.",
                path.display(), path.display()
            );
        };

        if *actual_offset != *expected_offset {
            anyhow::bail!(
                "tracepoint format {} field {field} offset mismatch: expected {expected_offset}, got {actual_offset};\n\nThis indicates the kernel tracepoint layout changed. Try building the eBPF program for your running kernel (ensure you have rust-src and bpf-linker available) or upgrade your kernel. If the problem persists, please open an issue and include the format file contents: {}",
                path.display(), path.display()
            );
        }
    }

    Ok(())
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

#[allow(dead_code)]
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
