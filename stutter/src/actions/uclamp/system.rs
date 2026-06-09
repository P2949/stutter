use std::{fs, path::Path};

use anyhow::Context;

use super::{
    models::{UclampCurrentValues, UclampTargetSnapshot},
    validate::validate_uclamp_value,
};
use crate::actions::{ActionBoundaryError, ActionWarning, TaskIdentity, syscalls::SchedUclamp};

pub(crate) fn read_target_snapshot_at(
    proc_root: &Path,
    target: &TaskIdentity,
) -> anyhow::Result<UclampTargetSnapshot> {
    if target.tid == 0 {
        return Err(ActionBoundaryError::InvalidTargetTid {
            action_kind: "uclamp",
            tid: target.tid.as_u32(),
        }
        .into());
    }

    let stat_path = proc_root.join(target.tid.to_string()).join("stat");
    let stat = fs::read_to_string(&stat_path).with_context(|| {
        format!(
            "target task does not exist or stat is unreadable: {}",
            stat_path.display()
        )
    })?;
    let starttime_ticks = parse_stat_starttime(&stat)
        .with_context(|| format!("failed to parse starttime from {}", stat_path.display()))?;

    if let Some(expected_starttime) = target.starttime_ticks
        && expected_starttime != starttime_ticks
    {
        return Err(ActionBoundaryError::TargetIdentityMismatch {
            action_kind: "uclamp",
            tid: target.tid.as_u32(),
            expected_starttime,
            actual_starttime: starttime_ticks,
        }
        .into());
    }

    let comm_path = proc_root.join(target.tid.to_string()).join("comm");
    let comm = fs::read_to_string(comm_path)
        .ok()
        .map(|comm| comm.trim().to_owned())
        .filter(|comm| !comm.is_empty());
    let exe = fs::read_link(proc_root.join(target.tid.to_string()).join("exe")).ok();

    let current = if proc_root == Path::new("/proc") {
        read_task_uclamp(target.tid.as_u32())
            .or_else(|_| read_task_uclamp_from_sched_at(proc_root, target.tid.as_u32()))
    } else {
        read_task_uclamp_from_sched_at(proc_root, target.tid.as_u32())
    }
    .with_context(|| format!("current uclamp is unreadable for tid={}", target.tid))?;

    Ok(UclampTargetSnapshot {
        tid: target.tid.as_u32(),
        process_pid: target.process_pid.map(|pid| pid.as_u32()),
        comm,
        starttime_ticks: Some(starttime_ticks),
        exe,
        current,
    })
}

pub(crate) fn identity_warnings(
    target: &TaskIdentity,
    snapshot: &UclampTargetSnapshot,
) -> Vec<ActionWarning> {
    let mut warnings = Vec::new();

    if let (Some(expected_comm), Some(actual_comm)) = (&target.comm, &snapshot.comm)
        && expected_comm != actual_comm
    {
        warnings.push(ActionWarning {
            message: format!(
                "target tid={} comm changed from {:?} to {:?}; continuing because starttime matched or was not provided",
                target.tid, expected_comm, actual_comm
            ),
        });
    }

    if target.process_pid.is_none() {
        warnings.push(ActionWarning {
            message: format!(
                "target tid={} has no process_pid identity; rollback will use tid only",
                target.tid
            ),
        });
    }

    warnings
}

pub(crate) fn parse_stat_starttime(stat: &str) -> anyhow::Result<u64> {
    let close_paren = stat
        .rfind(')')
        .context("stat line does not contain closing comm parenthesis")?;
    let fields = stat[close_paren + 1..]
        .split_whitespace()
        .collect::<Vec<_>>();

    let starttime_ticks = fields
        .get(19)
        .context("stat line missing starttime field")?
        .parse::<u64>()
        .context("invalid starttime field")?;

    Ok(starttime_ticks)
}

fn read_task_uclamp(tid: u32) -> anyhow::Result<UclampCurrentValues> {
    let attr = crate::actions::syscalls::sched_getattr(tid)
        .with_context(|| format!("sched_getattr({tid}) failed"))?;

    Ok(UclampCurrentValues {
        sched_util_min: attr.util_min,
        sched_util_max: attr.util_max,
    })
}

pub(crate) fn set_task_uclamp(tid: u32, values: UclampCurrentValues) -> anyhow::Result<()> {
    validate_uclamp_value("sched_util_min", values.sched_util_min)?;
    validate_uclamp_value("sched_util_max", values.sched_util_max)?;

    if values.sched_util_min > values.sched_util_max {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "uclamp",
            field: "sched_util_min".to_owned(),
            reason: format!(
                "sched_util_min {} is greater than sched_util_max {}",
                values.sched_util_min, values.sched_util_max
            ),
        }
        .into());
    }

    crate::actions::syscalls::sched_setattr(
        tid,
        SchedUclamp {
            util_min: values.sched_util_min,
            util_max: values.sched_util_max,
        },
    )
    .with_context(|| {
        format!(
            "sched_setattr({}, util_min={}, util_max={}) failed",
            tid, values.sched_util_min, values.sched_util_max
        )
    })
}

pub(crate) fn read_task_uclamp_from_sched_at(
    proc_root: &Path,
    tid: u32,
) -> anyhow::Result<UclampCurrentValues> {
    let sched_path = proc_root.join(tid.to_string()).join("sched");
    let sched = fs::read_to_string(&sched_path)
        .with_context(|| format!("failed to read {}", sched_path.display()))?;
    parse_sched_uclamp(&sched).with_context(|| {
        format!(
            "failed to parse uclamp values from {}",
            sched_path.display()
        )
    })
}

pub(crate) fn parse_sched_uclamp(sched: &str) -> anyhow::Result<UclampCurrentValues> {
    let mut util_min = None;
    let mut util_max = None;

    for line in sched.lines() {
        if let Some(value) = sched_line_value(line, "uclamp.min") {
            util_min = Some(value);
        } else if let Some(value) = sched_line_value(line, "uclamp.max") {
            util_max = Some(value);
        }
    }

    Ok(UclampCurrentValues {
        sched_util_min: util_min.context("missing uclamp.min")?,
        sched_util_max: util_max.context("missing uclamp.max")?,
    })
}

fn sched_line_value(line: &str, key: &str) -> Option<u32> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with(key) {
        return None;
    }

    let (_, value) = trimmed.split_once(':')?;
    value.trim().parse::<u32>().ok()
}
