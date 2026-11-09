//! resctrl L3 cache allocation as a prepared transaction (package W11c).
//!
//! Separate from the read-only monitor. A Stutter-owned control group is created with a bounded,
//! contiguous capacity bitmask; the root (default) group's `schemata` is the bystander guard and
//! must be unchanged after every step. Restore removes only the group this transaction created,
//! and only while the group's identity is unchanged; the kernel returns any tasks to the default
//! group. A memory-bandwidth (MBA) percentage may be added to the same group, and named processes
//! may be moved into it: each by live start time, every thread, and only a task that is still in
//! the default group (another group's tasks are never taken). Experimental, never autonomous, and
//! absent hardware yields `Unavailable`, not a pass.
use std::{
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

const GROUP_PREFIX: &str = "stutter-cat-";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatRequest {
    /// Group name suffix, unique per transaction.
    pub name: String,
    /// Contiguous capacity bitmask to give the group on every L3 domain.
    pub mask: u64,
    /// Memory-bandwidth percentage on every MB domain (percentage mode only); `None` leaves MBA.
    pub mba_percent: Option<u32>,
    /// Processes to move into the group, each bound to its live start time.
    pub tasks: Vec<CatTask>,
}

/// One process to assign, identified by pid and the start time it was named with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CatTask {
    pub task: u32,
    pub start_ticks: u64,
}

/// Most processes one group may take.
pub const MAX_ASSIGNED_TASKS: usize = 64;

#[derive(Debug, PartialEq, Eq)]
pub enum CatDenial {
    /// resctrl is not mounted or has no L3 allocation.
    Unavailable,
    BadName,
    /// Zero, non-contiguous, below the hardware minimum width or outside the supported mask.
    BadMask,
    /// No free class of service remains.
    NoFreeClosid,
    GroupExists,
    /// The root schemata could not be read or has no L3 line.
    RootUnreadable,
    /// Bandwidth allocation was requested but is absent, not in percentage mode, or has no domains.
    MbaUnavailable,
    /// Below the hardware minimum, above 100, or not a multiple of the bandwidth granularity.
    BadBandwidth,
    /// A task is not the live process that was named, or has no threads to move.
    TaskIdentity,
    /// A task is not in the default group: it belongs to someone else's group and is not taken.
    TaskNotInDefaultGroup,
    TooManyTasks,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CatCommitError {
    CreateFailed,
    /// The group was created but its kernel identity could not be read, so it was never recorded;
    /// it was removed again (`removed` says whether that worked).
    IdentityUnreadable {
        removed: bool,
    },
    /// The group was created but its schemata did not read back as requested; it was removed.
    ReadbackMismatch {
        removed: bool,
    },
    /// The default group's allocation changed during the commit: not ours; group removed.
    BystanderChanged {
        removed: bool,
    },
    /// A thread did not end up in the group; the group was removed, which returns every task to
    /// the default group.
    TaskAssignmentFailed {
        removed: bool,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum CatRestore {
    Removed,
    /// Already gone.
    Absent,
    /// The path now names a different group.
    UnsafeToRestore,
    /// Removal failed.
    Unproven,
}

#[derive(Debug)]
pub struct CatPrepared {
    group: PathBuf,
    /// One schemata line per resource the group sets (`L3:`, optionally `MB:`).
    lines: Vec<String>,
    root_schemata: String,
    /// Every thread id to move, resolved at prepare time.
    threads: Vec<u32>,
}

/// What recovery needs to remove exactly the group this transaction created.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResctrlGroupRecord {
    pub group: PathBuf,
    pub dev: u64,
    pub ino: u64,
}

#[derive(Debug)]
pub struct CatCommitted {
    group: PathBuf,
    identity: (u64, u64),
}

fn read(path: &Path) -> io::Result<String> {
    fs::read_to_string(path)
}

fn contiguous(mask: u64) -> bool {
    mask != 0 && {
        let shifted = mask >> mask.trailing_zeros();
        shifted & (shifted + 1) == 0
    }
}

fn value(path: &Path) -> Option<u64> {
    read(path).ok()?.trim().parse().ok()
}

fn hex(path: &Path) -> Option<u64> {
    u64::from_str_radix(read(path).ok()?.trim(), 16).ok()
}

/// The domain ids in the root's `L3:0=fff;1=fff` line.
fn l3_domains(schemata: &str) -> Option<Vec<String>> {
    let line = schemata
        .lines()
        .find_map(|l| l.trim().strip_prefix("L3:"))?;
    let ids: Vec<String> = line
        .split(';')
        .filter_map(|part| part.split_once('=').map(|(id, _)| id.trim().to_owned()))
        .collect();
    (!ids.is_empty()).then_some(ids)
}

/// The domain ids on the root schemata line that starts with `resource` (`MB:0=100;1=100`).
fn domains_of(schemata: &str, resource: &str) -> Option<Vec<String>> {
    let line = schemata
        .lines()
        .find_map(|l| l.trim().strip_prefix(resource))?;
    let ids: Vec<String> = line
        .split(';')
        .filter_map(|part| part.split_once('=').map(|(id, _)| id.trim().to_owned()))
        .collect();
    (!ids.is_empty()).then_some(ids)
}

/// A task lookup: the live start time and every thread id, or `None` when it is gone.
pub type TaskInspector<'a> = &'a dyn Fn(u32) -> Option<(u64, Vec<u32>)>;

/// [`prepare_with`] for a request that moves no processes.
pub fn prepare(root: &Path, request: &CatRequest) -> Result<CatPrepared, CatDenial> {
    prepare_with(root, request, &|_| None)
}

/// Validates against the mounted hierarchy and captures the bystander snapshot; writes nothing.
pub fn prepare_with(
    root: &Path,
    request: &CatRequest,
    inspect: TaskInspector<'_>,
) -> Result<CatPrepared, CatDenial> {
    let info = root.join("info/L3");
    let cbm_mask = hex(&info.join("cbm_mask")).ok_or(CatDenial::Unavailable)?;
    let min_bits = value(&info.join("min_cbm_bits")).unwrap_or(1);
    let closids = value(&info.join("num_closids")).ok_or(CatDenial::Unavailable)?;
    if request.name.is_empty()
        || request.name.len() > 32
        || !request
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(CatDenial::BadName);
    }
    if !contiguous(request.mask)
        || request.mask & !cbm_mask != 0
        || u64::from(request.mask.count_ones()) < min_bits
    {
        return Err(CatDenial::BadMask);
    }
    let root_schemata = read(&root.join("schemata")).map_err(|_| CatDenial::RootUnreadable)?;
    let domains = l3_domains(&root_schemata).ok_or(CatDenial::RootUnreadable)?;
    // The default group uses one class of service, every group directory one more, and the new
    // group needs one of its own.
    let groups = fs::read_dir(root)
        .map_err(|_| CatDenial::Unavailable)?
        .flatten()
        .filter(|entry| entry.path().join("schemata").is_file())
        .count() as u64;
    if groups + 2 > closids {
        return Err(CatDenial::NoFreeClosid);
    }
    let group = root.join(format!("{GROUP_PREFIX}{}", request.name));
    if group.exists() {
        return Err(CatDenial::GroupExists);
    }
    let masks = domains
        .iter()
        .map(|id| format!("{id}={:x}", request.mask))
        .collect::<Vec<_>>()
        .join(";");
    let mut lines = vec![format!("L3:{masks}")];
    if let Some(percent) = request.mba_percent {
        let info = root.join("info/MB");
        let min = value(&info.join("min_bandwidth")).ok_or(CatDenial::MbaUnavailable)?;
        let gran = value(&info.join("bandwidth_gran")).ok_or(CatDenial::MbaUnavailable)?;
        let mb_domains = domains_of(&root_schemata, "MB:").ok_or(CatDenial::MbaUnavailable)?;
        // Only percentage mode: the root line carries percentages, not MBps targets.
        let root_in_percent = root_schemata
            .lines()
            .find_map(|l| l.trim().strip_prefix("MB:"))
            .is_some_and(|line| {
                line.split(';')
                    .filter_map(|part| part.split_once('=')?.1.trim().parse::<u64>().ok())
                    .all(|v| v <= 100)
            });
        if !root_in_percent || gran == 0 {
            return Err(CatDenial::MbaUnavailable);
        }
        let percent = u64::from(percent);
        if percent < min || percent > 100 || percent % gran != 0 {
            return Err(CatDenial::BadBandwidth);
        }
        let parts = mb_domains
            .iter()
            .map(|id| format!("{id}={percent}"))
            .collect::<Vec<_>>()
            .join(";");
        lines.push(format!("MB:{parts}"));
    }
    if request.tasks.len() > MAX_ASSIGNED_TASKS {
        return Err(CatDenial::TooManyTasks);
    }
    let in_default: std::collections::BTreeSet<u32> = read(&root.join("tasks"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect();
    let mut threads = Vec::new();
    for task in &request.tasks {
        let (live_start, tids) = inspect(task.task).ok_or(CatDenial::TaskIdentity)?;
        if live_start != task.start_ticks || tids.is_empty() {
            return Err(CatDenial::TaskIdentity);
        }
        if !tids.iter().all(|tid| in_default.contains(tid)) {
            return Err(CatDenial::TaskNotInDefaultGroup);
        }
        threads.extend(tids);
    }
    if threads.len() > MAX_ASSIGNED_TASKS * 64 {
        return Err(CatDenial::TooManyTasks);
    }
    Ok(CatPrepared {
        group,
        lines,
        root_schemata,
        threads,
    })
}

impl CatPrepared {
    /// The schemata lines that will be written, one per resource.
    pub fn schemata_lines(&self) -> &[String] {
        &self.lines
    }

    /// The first (L3) line.
    pub fn schemata(&self) -> &str {
        &self.lines[0]
    }

    pub fn commit(self) -> Result<CatCommitted, CatCommitError> {
        self.commit_with(|group| fs::metadata(group).map(|meta| (meta.dev(), meta.ino())))
    }

    fn commit_with(
        self,
        identity_of: impl Fn(&Path) -> io::Result<(u64, u64)>,
    ) -> Result<CatCommitted, CatCommitError> {
        fs::create_dir(&self.group).map_err(|_| CatCommitError::CreateFailed)?;
        let remove = || remove_group(&self.group).is_ok();
        // `create_dir` is exclusive, so the path names the group this call just made; if its
        // identity cannot be read it cannot be recorded, so it is removed here, not leaked.
        let Ok(identity) = identity_of(&self.group) else {
            return Err(CatCommitError::IdentityUnreadable { removed: remove() });
        };
        let wrote = fs::write(
            self.group.join("schemata"),
            format!("{}\n", self.lines.join("\n")),
        )
        .is_ok();
        let applied = wrote
            && read(&self.group.join("schemata")).ok().is_some_and(|text| {
                self.lines
                    .iter()
                    .all(|want| text.lines().any(|line| line.trim() == want))
            });
        if !applied {
            return Err(CatCommitError::ReadbackMismatch { removed: remove() });
        }
        let root = self.group.parent().unwrap_or(&self.group);
        if read(&root.join("schemata")).ok().as_deref() != Some(self.root_schemata.as_str()) {
            return Err(CatCommitError::BystanderChanged { removed: remove() });
        }
        // One thread per write, as the kernel takes them; every one must read back as a member.
        // One thread per write, as the kernel takes them; each must read back as a member
        // before the next is moved.
        let tasks = self.group.join("tasks");
        let assigned = self.threads.iter().all(|tid| {
            fs::write(&tasks, tid.to_string()).is_ok()
                && read(&tasks).ok().is_some_and(|text| {
                    text.lines()
                        .any(|line| line.trim().parse::<u32>().ok() == Some(*tid))
                })
        });
        if !assigned {
            return Err(CatCommitError::TaskAssignmentFailed { removed: remove() });
        }
        Ok(CatCommitted {
            group: self.group,
            identity,
        })
    }
}

impl CatCommitted {
    pub fn record(&self) -> ResctrlGroupRecord {
        ResctrlGroupRecord {
            group: self.group.clone(),
            dev: self.identity.0,
            ino: self.identity.1,
        }
    }

    /// Removes the group this transaction created, if the path still names it.
    pub fn restore(self) -> CatRestore {
        remove_recorded(&self.record())
    }
}

/// Removes the recorded group only while the path still names the same kernel object.
pub fn remove_recorded(record: &ResctrlGroupRecord) -> CatRestore {
    match fs::metadata(&record.group) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => CatRestore::Absent,
        Err(_) => CatRestore::Unproven,
        Ok(meta) if (meta.dev(), meta.ino()) != (record.dev, record.ino) => {
            CatRestore::UnsafeToRestore
        }
        Ok(_) => {
            if remove_group(&record.group).is_ok() {
                CatRestore::Removed
            } else {
                CatRestore::Unproven
            }
        }
    }
}

/// resctrl is a kernfs tree: `rmdir` removes a group with its control files. A fake root is a plain
/// directory, so tests clear its files first.
fn remove_group(path: &Path) -> io::Result<()> {
    #[cfg(test)]
    for entry in fs::read_dir(path)?.flatten() {
        if entry.path().is_dir() {
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
    }
    fs::remove_dir(path)
}

#[cfg(test)]
#[path = "resctrl_alloc_tests.rs"]
mod tests;
