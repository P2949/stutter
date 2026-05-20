//! Live target identity revalidation before privileged apply execution.

use std::{fmt, path::Path};

use crate::{actions::TaskIdentity, autotune::candidate::CandidateAction};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetRevalidationError {
    MissingTid {
        tid: u32,
    },
    StarttimeMismatch {
        tid: u32,
        expected: u64,
        actual: Option<u64>,
    },
    ProcessPidMismatch {
        tid: u32,
        expected: u32,
        actual: Option<u32>,
    },
    CommMismatch {
        tid: u32,
        expected: String,
        actual: Option<String>,
    },
}

impl TargetRevalidationError {
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::MissingTid { .. } => "target_revalidation_missing_tid",
            Self::StarttimeMismatch { .. } => "target_revalidation_starttime_mismatch",
            Self::ProcessPidMismatch { .. } => "target_revalidation_process_pid_mismatch",
            Self::CommMismatch { .. } => "target_revalidation_comm_mismatch",
        }
    }
}

impl fmt::Display for TargetRevalidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTid { tid } => {
                write!(f, "{}: tid={tid} is missing", self.reason_code())
            }
            Self::StarttimeMismatch {
                tid,
                expected,
                actual,
            } => write!(
                f,
                "{}: tid={tid} expected_starttime={expected} actual_starttime={actual:?}",
                self.reason_code()
            ),
            Self::ProcessPidMismatch {
                tid,
                expected,
                actual,
            } => write!(
                f,
                "{}: tid={tid} expected_process_pid={expected} actual_process_pid={actual:?}",
                self.reason_code()
            ),
            Self::CommMismatch {
                tid,
                expected,
                actual,
            } => write!(
                f,
                "{}: tid={tid} expected_comm={expected:?} actual_comm={actual:?}",
                self.reason_code()
            ),
        }
    }
}

impl std::error::Error for TargetRevalidationError {}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LiveTaskIdentity {
    tid: u32,
    process_pid: Option<u32>,
    comm: Option<String>,
    starttime_ticks: Option<u64>,
}

pub(crate) fn revalidate_candidate_targets(
    candidate: &CandidateAction,
    proc_root: &Path,
) -> anyhow::Result<()> {
    for target in candidate_task_identities(candidate) {
        revalidate_task_identity(&target, proc_root)
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    }

    Ok(())
}

fn revalidate_task_identity(
    target: &TaskIdentity,
    proc_root: &Path,
) -> Result<(), TargetRevalidationError> {
    let live = read_live_task_identity(proc_root, target)
        .ok_or(TargetRevalidationError::MissingTid { tid: target.tid })?;

    if let Some(expected_pid) = target.process_pid
        && live.process_pid != Some(expected_pid)
    {
        return Err(TargetRevalidationError::ProcessPidMismatch {
            tid: target.tid,
            expected: expected_pid,
            actual: live.process_pid,
        });
    }

    if let Some(expected_starttime) = target.starttime_ticks
        && live.starttime_ticks != Some(expected_starttime)
    {
        return Err(TargetRevalidationError::StarttimeMismatch {
            tid: target.tid,
            expected: expected_starttime,
            actual: live.starttime_ticks,
        });
    }

    if let Some(expected_comm) = target.comm.as_deref()
        && live.comm.as_deref() != Some(expected_comm)
    {
        return Err(TargetRevalidationError::CommMismatch {
            tid: target.tid,
            expected: expected_comm.to_owned(),
            actual: live.comm,
        });
    }

    Ok(())
}

fn candidate_task_identities(candidate: &CandidateAction) -> Vec<TaskIdentity> {
    match candidate {
        CandidateAction::Nice { plan } => plan.action.targets.clone(),
        CandidateAction::IoPrio { plan } => plan.action.targets.clone(),
        CandidateAction::Uclamp { plan } => plan.action.targets.clone(),
        CandidateAction::CgroupPlacement { plan } => plan
            .action
            .targets
            .iter()
            .map(|target| target.identity.clone())
            .collect(),
        CandidateAction::CpuAffinityProfile { plan } => vec![TaskIdentity {
            tid: plan.tree_pid,
            process_pid: Some(plan.tree_pid),
            comm: None,
            starttime_ticks: None,
        }],
        _ => Vec::new(),
    }
}

fn read_live_task_identity(proc_root: &Path, target: &TaskIdentity) -> Option<LiveTaskIdentity> {
    if let Some(process_pid) = target.process_pid {
        let task_stat = proc_root
            .join(process_pid.to_string())
            .join("task")
            .join(target.tid.to_string())
            .join("stat");
        if let Some(identity) = read_identity_from_stat(&task_stat, target.tid, Some(process_pid)) {
            return Some(identity);
        }

        let actual = read_top_level_task_identity(proc_root, target.tid);
        if actual.is_some() {
            return actual;
        }

        return None;
    }

    read_top_level_task_identity(proc_root, target.tid)
}

fn read_top_level_task_identity(proc_root: &Path, tid: u32) -> Option<LiveTaskIdentity> {
    let stat_path = proc_root.join(tid.to_string()).join("stat");
    let mut identity = read_identity_from_stat(&stat_path, tid, None)?;
    identity.process_pid = read_tgid_from_status(&proc_root.join(tid.to_string()).join("status"));
    Some(identity)
}

fn read_identity_from_stat(
    path: &Path,
    expected_tid: u32,
    process_pid: Option<u32>,
) -> Option<LiveTaskIdentity> {
    let stat = std::fs::read_to_string(path).ok()?;
    let (stat_tid, comm) = parse_proc_stat_tid_and_comm(&stat)?;
    if stat_tid != expected_tid {
        return None;
    }

    Some(LiveTaskIdentity {
        tid: expected_tid,
        process_pid,
        comm: Some(comm),
        starttime_ticks: crate::process_tree::parse_proc_stat_starttime(&stat),
    })
}

fn parse_proc_stat_tid_and_comm(stat: &str) -> Option<(u32, String)> {
    let open = stat.find('(')?;
    let close = stat.rfind(") ")?;
    let tid = stat[..open].trim().parse().ok()?;
    Some((tid, stat[open + 1..close].to_owned()))
}

fn read_tgid_from_status(path: &Path) -> Option<u32> {
    let status = std::fs::read_to_string(path).ok()?;
    status.lines().find_map(|line| {
        let value = line.strip_prefix("Tgid:")?;
        value.trim().parse().ok()
    })
}
