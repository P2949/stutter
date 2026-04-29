use std::{collections::BTreeMap, fs, path::Path};

use aya::{
    Ebpf,
    maps::{HashMap as AyaHashMap, MapData, RingBuf},
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
    drop_counters: AyaHashMap<MapData, u32, u64>,
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

pub fn load_and_attach() -> anyhow::Result<LoadedEbpf> {
    raise_memlock_limit();
    validate_tracepoint_formats(Path::new("/sys/kernel/tracing/events"))?;

    let mut ebpf = Ebpf::load(aya::include_bytes_aligned!(concat!(
        env!("OUT_DIR"),
        "/stutter"
    )))?;

    attach_tracepoint(&mut ebpf, "sched_wakeup", "sched", "sched_wakeup")?;
    attach_tracepoint(&mut ebpf, "sched_wakeup_new", "sched", "sched_wakeup_new")?;
    attach_tracepoint(&mut ebpf, "sched_switch", "sched", "sched_switch")?;
    attach_tracepoint(
        &mut ebpf,
        "sched_process_exit",
        "sched",
        "sched_process_exit",
    )?;

    let target_pid_map = AyaHashMap::try_from(
        ebpf.take_map("TARGET_PIDS")
            .ok_or_else(|| anyhow::anyhow!("TARGET_PIDS map not found"))?,
    )?;

    let drop_counters = AyaHashMap::try_from(
        ebpf.take_map("DROP_COUNTERS")
            .ok_or_else(|| anyhow::anyhow!("DROP_COUNTERS map not found"))?,
    )?;

    let events = RingBuf::try_from(
        ebpf.take_map("EVENTS")
            .ok_or_else(|| anyhow::anyhow!("EVENTS map not found"))?,
    )?;

    let events = AsyncFd::new(events)?;

    Ok(LoadedEbpf {
        ebpf,
        events,
        target_pid_map,
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

fn drop_counter_value(counters: &AyaHashMap<MapData, u32, u64>, key: u32) -> u64 {
    counters.get(&key, 0).unwrap_or(0)
}

fn validate_tracepoint_formats(events_root: &Path) -> anyhow::Result<()> {
    validate_tracepoint_format_at(
        &events_root.join("sched/sched_wakeup/format"),
        &[("pid", 24), ("prio", 28)],
    )?;
    validate_tracepoint_format_at(
        &events_root.join("sched/sched_wakeup_new/format"),
        &[("pid", 24), ("prio", 28)],
    )?;
    validate_tracepoint_format_at(
        &events_root.join("sched/sched_switch/format"),
        &[("next_comm", 40), ("next_pid", 56), ("next_prio", 60)],
    )?;
    Ok(())
}

fn validate_tracepoint_format_at(
    path: &Path,
    expected_offsets: &[(&str, usize)],
) -> anyhow::Result<()> {
    let format = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read tracepoint format {}: {e}", path.display()))?;
    validate_tracepoint_format(&format, expected_offsets).map_err(|e| {
        anyhow::anyhow!(
            "tracepoint format {} does not match eBPF offsets: {e}",
            path.display()
        )
    })
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
}
