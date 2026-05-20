use std::{fs, path::Path};

use crate::{actions::model::TaskRestoreIdentity, process::snapshot::parse_proc_stat_starttime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreIdentityStatus {
    SameTask,
    Missing,
    Mismatch { reason: &'static str },
    UnknownLegacy,
}

pub fn verify_task_identity(
    proc_root: &Path,
    identity: &TaskRestoreIdentity,
) -> RestoreIdentityStatus {
    let tid = identity.tid;
    let stat_path = proc_root.join(tid.to_string()).join("stat");

    let Ok(stat_content) = fs::read_to_string(&stat_path) else {
        return RestoreIdentityStatus::Missing;
    };

    let current_starttime = parse_proc_stat_starttime(&stat_content);

    match (identity.task_starttime_ticks, current_starttime) {
        (Some(expected), Some(actual)) => {
            if expected != actual {
                return RestoreIdentityStatus::Mismatch {
                    reason: "starttime_ticks mismatch",
                };
            }
        }
        (None, _) => {
            return RestoreIdentityStatus::UnknownLegacy;
        }
        (_, None) => {
            return RestoreIdentityStatus::Mismatch {
                reason: "unable to parse current starttime",
            };
        }
    }

    RestoreIdentityStatus::SameTask
}
