//! Whether Stutter still owns a task's cgroup placement. Restoration writes only while it does;
//! a placement that cannot be read is unknown, and unknown is never permission to write.
use std::path::Path;

use super::{fs_io::resolve_cgroup_fs_path, prepared::cgroup_id, procfs};
use crate::actions::CgroupRestoreRecord;

pub(super) enum Ownership {
    /// The task is in the cgroup Stutter put it in, and that cgroup is the one Stutter chose.
    Owned,
    /// Someone else moved the task, or replaced the cgroup behind the same path.
    Superseded(String),
    /// The task exited; nothing is left to restore.
    Gone,
    /// The task is alive but its placement could not be established.
    Unknown(String),
}

pub(super) fn applied_ownership(
    proc_root: &Path,
    cgroup_root: &Path,
    record: &CgroupRestoreRecord,
    tid: u32,
) -> Ownership {
    // A record without an applied placement makes no ownership claim (legacy tokens).
    let Some(applied) = &record.applied_cgroup else {
        return Ownership::Owned;
    };
    let live = match procfs::read_proc_cgroup_path_at(proc_root, tid) {
        Ok(live) => live,
        Err(error) if !procfs::task_exists(proc_root, tid) => {
            let _ = error;
            return Ownership::Gone;
        }
        Err(error) => return Ownership::Unknown(format!("{error:#}")),
    };
    if live != *applied {
        return Ownership::Superseded(format!(
            "stutter put tid={tid} in {} and it is now in {}",
            applied.display(),
            live.display()
        ));
    }
    let Some(expected) = record.applied_cgroup_id else {
        return Ownership::Owned;
    };
    match resolve_cgroup_fs_path(cgroup_root, &live).and_then(|path| cgroup_id(&path)) {
        Ok(id) if id == expected => Ownership::Owned,
        Ok(id) => Ownership::Superseded(format!(
            "cgroup {} for tid={tid} is a different cgroup now (inode {id}, stutter chose {expected})",
            live.display()
        )),
        Err(error) => Ownership::Unknown(format!("{error:#}")),
    }
}
