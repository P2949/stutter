use std::io;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use stutter_core::ids::Tid;

use super::ProfileRule;
use crate::{actions::ioprio::IoPrioValue, affinity::CpuMask, process_tree::TaskInfo};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActionStatus {
    NoAction,
    AlreadySatisfied,
    Pending,
    SkippedDeadTask,
    SkippedIncompleteIdentity,
    ReadError,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProfileTaskActionDecision {
    pub affinity: Option<ProfileAffinityDecision>,
    pub nice: Option<ProfileNiceDecision>,
    pub ionice: Option<ProfileIoniceDecision>,
}

impl ProfileTaskActionDecision {
    pub(super) fn task_disappeared(&self) -> bool {
        self.affinity
            .as_ref()
            .is_some_and(|decision| decision.status == ActionStatus::SkippedDeadTask)
            || self
                .nice
                .as_ref()
                .is_some_and(|decision| decision.status == ActionStatus::SkippedDeadTask)
            || self
                .ionice
                .as_ref()
                .is_some_and(|decision| decision.status == ActionStatus::SkippedDeadTask)
    }

    pub(super) fn pending(&self) -> bool {
        self.affinity_pending() || self.nice_pending() || self.ionice_pending()
    }

    pub(super) fn affinity_pending(&self) -> bool {
        self.affinity
            .as_ref()
            .is_some_and(|decision| decision.status == ActionStatus::Pending)
    }

    pub(super) fn nice_pending(&self) -> bool {
        self.nice
            .as_ref()
            .is_some_and(|decision| decision.status == ActionStatus::Pending)
    }

    pub(super) fn ionice_pending(&self) -> bool {
        self.ionice
            .as_ref()
            .is_some_and(|decision| decision.status == ActionStatus::Pending)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProfileAffinityDecision {
    pub status: ActionStatus,
    pub current: Option<CpuMask>,
    pub desired: Option<CpuMask>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProfileNiceDecision {
    pub status: ActionStatus,
    pub current: Option<i32>,
    pub desired: Option<i32>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProfileIoniceDecision {
    pub status: ActionStatus,
    pub current_encoded: Option<i32>,
    pub current: Option<IoPrioValue>,
    pub desired_encoded: Option<i32>,
    pub desired: Option<IoPrioValue>,
    pub reason: Option<String>,
}

pub(super) fn decide_profile_task_actions<FA, FN, FI>(
    task: &TaskInfo,
    rule: &ProfileRule,
    read_allowed_mask: &mut FA,
    read_nice: &mut FN,
    read_ioprio: &mut FI,
) -> anyhow::Result<ProfileTaskActionDecision>
where
    FA: FnMut(Tid) -> io::Result<CpuMask>,
    FN: FnMut(Tid) -> anyhow::Result<i32>,
    FI: FnMut(Tid) -> anyhow::Result<i32>,
{
    let mut decision = ProfileTaskActionDecision {
        affinity: None,
        nice: None,
        ionice: None,
    };

    if let Some(desired_mask) = &rule.affinity {
        let current = match read_allowed_mask(task.task_id()) {
            Ok(mask) => mask,
            Err(err) if err.raw_os_error() == Some(libc::ESRCH) => {
                decision.affinity = Some(ProfileAffinityDecision {
                    status: ActionStatus::SkippedDeadTask,
                    current: None,
                    desired: Some(desired_mask.clone()),
                    reason: Some("task disappeared while reading current affinity".to_owned()),
                });
                return Ok(decision);
            }
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "failed to read CPU affinity for TID {}: {err}",
                    task.tid
                ));
            }
        };

        let status = if current == *desired_mask {
            ActionStatus::AlreadySatisfied
        } else if task_has_complete_identity(task) {
            ActionStatus::Pending
        } else {
            ActionStatus::SkippedIncompleteIdentity
        };
        let reason = (status == ActionStatus::SkippedIncompleteIdentity)
            .then(|| "task identity was incomplete".to_owned());
        decision.affinity = Some(ProfileAffinityDecision {
            status,
            current: Some(current),
            desired: Some(desired_mask.clone()),
            reason,
        });
    }

    if let Some(desired_nice) = rule.nice {
        let current = match read_nice(task.task_id()) {
            Ok(nice) => nice,
            Err(err) if anyhow_raw_os_error(&err) == Some(libc::ESRCH) => {
                decision.nice = Some(ProfileNiceDecision {
                    status: ActionStatus::SkippedDeadTask,
                    current: None,
                    desired: Some(desired_nice),
                    reason: Some("task disappeared while reading current nice".to_owned()),
                });
                return Ok(decision);
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("failed to read nice for profile target TID {}", task.tid)
                });
            }
        };

        let status = if current == desired_nice {
            ActionStatus::AlreadySatisfied
        } else if task_has_complete_identity(task) {
            ActionStatus::Pending
        } else {
            ActionStatus::SkippedIncompleteIdentity
        };
        let reason = (status == ActionStatus::SkippedIncompleteIdentity)
            .then(|| "task identity was incomplete".to_owned());
        decision.nice = Some(ProfileNiceDecision {
            status,
            current: Some(current),
            desired: Some(desired_nice),
            reason,
        });
    }

    if let Some(desired_ioprio) = rule.ionice {
        let current_encoded = match read_ioprio(task.task_id()) {
            Ok(ioprio) => ioprio,
            Err(err) if anyhow_raw_os_error(&err) == Some(libc::ESRCH) => {
                decision.ionice = Some(ProfileIoniceDecision {
                    status: ActionStatus::SkippedDeadTask,
                    current_encoded: None,
                    current: None,
                    desired_encoded: Some(desired_ioprio.encode()?),
                    desired: Some(desired_ioprio),
                    reason: Some("task disappeared while reading current I/O priority".to_owned()),
                });
                return Ok(decision);
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "failed to read I/O priority for profile target TID {}",
                        task.tid
                    )
                });
            }
        };
        let desired_encoded = desired_ioprio.encode()?;
        let current = IoPrioValue::decode(current_encoded).ok();

        let status = if current_encoded == desired_encoded {
            ActionStatus::AlreadySatisfied
        } else if task_has_complete_identity(task) {
            ActionStatus::Pending
        } else {
            ActionStatus::SkippedIncompleteIdentity
        };
        let reason = (status == ActionStatus::SkippedIncompleteIdentity)
            .then(|| "task identity was incomplete".to_owned());
        decision.ionice = Some(ProfileIoniceDecision {
            status,
            current_encoded: Some(current_encoded),
            current,
            desired_encoded: Some(desired_encoded),
            desired: Some(desired_ioprio),
            reason,
        });
    }

    Ok(decision)
}

pub(super) fn task_has_complete_identity(task: &TaskInfo) -> bool {
    task.process_starttime_ticks.is_some() && task.task_starttime_ticks.is_some()
}

pub(super) fn anyhow_raw_os_error(err: &anyhow::Error) -> Option<i32> {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<io::Error>())
        .and_then(io::Error::raw_os_error)
}
