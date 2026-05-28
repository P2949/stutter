use std::{fs, path::Path};

use anyhow::Context;

use super::{
    apply::read_task_ioprio,
    model::{IoPrioAction, IoPrioClass, IoPrioPolicy, IoPrioTargetSnapshot, IoPrioValue},
};
use crate::actions::{ActionBoundaryError, ActionState, ActionWarning, TaskIdentity};

impl IoPrioAction {
    pub fn preflight_with_policy(
        &self,
        policy: &IoPrioPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_at(Path::new("/proc"), policy)
    }

    pub(crate) fn preflight_at(
        &self,
        proc_root: &Path,
        policy: &IoPrioPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.collect_target_snapshots_at(proc_root, policy)
            .map(|snapshots| {
                snapshots
                    .into_iter()
                    .flat_map(|(_, warnings)| warnings)
                    .collect()
            })
    }

    pub(crate) fn dry_run_at(
        &self,
        proc_root: &Path,
        policy: &IoPrioPolicy,
    ) -> anyhow::Result<ActionState> {
        let snapshots = self.collect_target_snapshots_at(proc_root, policy)?;
        let requested = self.ioprio.encode()?;
        let mut warnings = Vec::new();
        let mut pending_changes = 0usize;

        for (snapshot, target_warnings) in snapshots {
            warnings.extend(target_warnings);
            if snapshot.current_ioprio != requested {
                pending_changes += 1;
            }
        }

        Ok(ActionState {
            applied: false,
            affected_tasks: pending_changes,
            checked_tasks: self.targets.len(),
            pending_changes,
            warnings,
        })
    }

    pub(crate) fn verify_at(
        &self,
        proc_root: &Path,
        policy: &IoPrioPolicy,
    ) -> anyhow::Result<ActionState> {
        let snapshots = self.collect_target_snapshots_at(proc_root, policy)?;
        let requested = self.ioprio.encode()?;
        let mut warnings = Vec::new();
        let mut pending_changes = 0usize;

        for (snapshot, target_warnings) in snapshots {
            warnings.extend(target_warnings);
            if snapshot.current_ioprio != requested {
                pending_changes += 1;
            }
        }

        Ok(ActionState {
            applied: !self.targets.is_empty() && pending_changes == 0,
            affected_tasks: self.targets.len(),
            checked_tasks: self.targets.len(),
            pending_changes,
            warnings,
        })
    }

    pub(crate) fn collect_target_snapshots_at(
        &self,
        proc_root: &Path,
        policy: &IoPrioPolicy,
    ) -> anyhow::Result<Vec<(IoPrioTargetSnapshot, Vec<ActionWarning>)>> {
        validate_policy_and_request(policy, self.ioprio)?;

        if self.targets.is_empty() {
            return Err(ActionBoundaryError::MissingExplicitTargets {
                action_kind: "ioprio",
            }
            .into());
        }

        let mut snapshots = Vec::with_capacity(self.targets.len());

        for target in &self.targets {
            let snapshot = read_target_snapshot_at(proc_root, target)
                .with_context(|| format!("failed to preflight ioprio target tid={}", target.tid))?;
            let warnings = identity_warnings(target, &snapshot);
            snapshots.push((snapshot, warnings));
        }

        Ok(snapshots)
    }
}

pub(crate) fn validate_policy_and_request(
    policy: &IoPrioPolicy,
    requested: IoPrioValue,
) -> anyhow::Result<()> {
    if !policy.allow_ioprio_changes {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "ioprio",
            requirement: "allow_ioprio_changes",
        }
        .into());
    }

    if policy.require_strong_block_io_evidence && !policy.strong_block_io_evidence {
        return Err(ActionBoundaryError::EvidenceRequired {
            action_kind: "ioprio",
            evidence: "strong_block_io_evidence",
        }
        .into());
    }

    validate_ioprio_value(requested)?;

    match requested.class {
        IoPrioClass::Realtime if !policy.allow_realtime_class => {
            return Err(ActionBoundaryError::PolicyDenied {
                action_kind: "ioprio",
                requirement: "allow_realtime_class",
            }
            .into());
        }
        IoPrioClass::None if !policy.allow_none_class => {
            return Err(ActionBoundaryError::PolicyDenied {
                action_kind: "ioprio",
                requirement: "allow_none_class",
            }
            .into());
        }
        IoPrioClass::BestEffort => {
            let level = requested.level.unwrap_or(4);
            if level > policy.max_best_effort_level {
                return Err(ActionBoundaryError::InvalidValue {
                    action_kind: "ioprio",
                    field: "best_effort_level".to_owned(),
                    reason: format!(
                        "requested level {level} exceeds policy maximum {}",
                        policy.max_best_effort_level
                    ),
                }
                .into());
            }
        }
        IoPrioClass::Idle | IoPrioClass::Realtime | IoPrioClass::None => {}
    }

    Ok(())
}

pub(crate) fn validate_ioprio_value(value: IoPrioValue) -> anyhow::Result<()> {
    match value.class {
        IoPrioClass::None | IoPrioClass::Idle => {
            if value.level.is_some() {
                return Err(ActionBoundaryError::InvalidValue {
                    action_kind: "ioprio",
                    field: "level".to_owned(),
                    reason: format!(
                        "I/O priority class {} must not specify a level",
                        value.class.label()
                    ),
                }
                .into());
            }
        }
        IoPrioClass::BestEffort | IoPrioClass::Realtime => {
            let Some(level) = value.level else {
                return Err(ActionBoundaryError::InvalidValue {
                    action_kind: "ioprio",
                    field: "level".to_owned(),
                    reason: format!(
                        "I/O priority class {} requires level 0..=7",
                        value.class.label()
                    ),
                }
                .into());
            };

            if level > 7 {
                return Err(ActionBoundaryError::InvalidValue {
                    action_kind: "ioprio",
                    field: "level".to_owned(),
                    reason: format!(
                        "I/O priority class {} level {} is outside range 0..=7",
                        value.class.label(),
                        level
                    ),
                }
                .into());
            }
        }
    }

    Ok(())
}

pub(crate) fn read_target_snapshot_at(
    proc_root: &Path,
    target: &TaskIdentity,
) -> anyhow::Result<IoPrioTargetSnapshot> {
    if target.tid == 0 {
        return Err(ActionBoundaryError::InvalidTargetTid {
            action_kind: "ioprio",
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
            action_kind: "ioprio",
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

    let current_ioprio = read_task_ioprio(target.tid.as_u32())
        .with_context(|| format!("current I/O priority is unreadable for tid={}", target.tid))?;
    let current_value = IoPrioValue::decode(current_ioprio).with_context(|| {
        format!(
            "current I/O priority value is invalid for tid={}",
            target.tid
        )
    })?;

    Ok(IoPrioTargetSnapshot {
        tid: target.tid.as_u32(),
        process_pid: target.process_pid.map(|pid| pid.as_u32()),
        comm,
        starttime_ticks: Some(starttime_ticks),
        exe,
        current_ioprio,
        current_value,
    })
}

pub(crate) fn identity_warnings(
    target: &TaskIdentity,
    snapshot: &IoPrioTargetSnapshot,
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
