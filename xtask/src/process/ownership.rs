//! Owner marking and reconciliation of supervised processes.
//!
//! Every supervised child inherits `STUTTER_XTASK_OWNER` = `<pid>:<starttime>` of the supervising
//! process, even after it detaches with `setsid`. A process carrying that marker whose owner is
//! gone (a crashed or killed xtask) is an orphan of this tool, and one whose owner is the current
//! process but outlived its stage is an escapee. Both are found by marker and verified owner
//! identity, never by executable name, and only processes of the same user are touched.
use std::{fs, os::unix::fs::MetadataExt, path::Path, thread, time::Duration};

pub const OWNER_ENV: &str = "STUTTER_XTASK_OWNER";
/// Most `/proc` entries examined per scan.
const MAX_ENTRIES: usize = 32_768;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owner {
    pub pid: u32,
    pub start_ticks: u64,
    /// Which stage of the owner this is; a sweep after one stage never touches another stage.
    pub stage: u64,
}

impl Owner {
    pub fn render(self) -> String {
        format!("{}:{}:{}", self.pid, self.start_ticks, self.stage)
    }

    pub fn with_stage(self, stage: u64) -> Self {
        Self { stage, ..self }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split(':');
        Some(Self {
            pid: parts.next()?.parse().ok()?,
            start_ticks: parts.next()?.parse().ok()?,
            stage: parts.next().map_or(Some(0), |stage| stage.parse().ok())?,
        })
    }
}

fn start_ticks(proc_root: &Path, pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(proc_root.join(pid.to_string()).join("stat")).ok()?;
    let (_, after_comm) = stat.rsplit_once(") ")?;
    after_comm.split_whitespace().nth(19)?.parse().ok()
}

/// The identity of the calling process.
pub fn current_owner(proc_root: &Path) -> Option<Owner> {
    let pid = std::process::id();
    Some(Owner {
        pid,
        start_ticks: start_ticks(proc_root, pid)?,
        stage: 0,
    })
}

/// Whether `owner` is still the same live process.
pub fn owner_alive(proc_root: &Path, owner: Owner) -> bool {
    start_ticks(proc_root, owner.pid) == Some(owner.start_ticks)
}

fn marker_of(proc_root: &Path, pid: u32) -> Option<Owner> {
    let environ = fs::read(proc_root.join(pid.to_string()).join("environ")).ok()?;
    let needle = format!("{OWNER_ENV}=");
    environ
        .split(|byte| *byte == 0)
        .filter_map(|entry| std::str::from_utf8(entry).ok())
        .find_map(|entry| entry.strip_prefix(needle.as_str()))
        .and_then(Owner::parse)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Marked {
    pub pid: u32,
    pub start_ticks: u64,
    pub owner: Owner,
}

/// Same-user processes (other than this one) that carry an owner marker.
pub fn marked_processes(proc_root: &Path, uid: u32) -> Vec<Marked> {
    let Ok(entries) = fs::read_dir(proc_root) else {
        return Vec::new();
    };
    let me = std::process::id();
    let mut found = Vec::new();
    for entry in entries.flatten().take(MAX_ENTRIES) {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == me || entry.metadata().map_or(true, |meta| meta.uid() != uid) {
            continue;
        }
        let (Some(owner), Some(start)) = (marker_of(proc_root, pid), start_ticks(proc_root, pid))
        else {
            continue;
        };
        found.push(Marked {
            pid,
            start_ticks: start,
            owner,
        });
    }
    found
}

/// TERM, wait up to `grace`, then KILL each listed process, re-checking its identity first so a
/// recycled pid is never signalled. Returns how many were signalled.
pub fn terminate(proc_root: &Path, targets: &[Marked], grace: Duration) -> usize {
    let still_same =
        |target: &Marked| start_ticks(proc_root, target.pid) == Some(target.start_ticks);
    let signal = |target: &Marked, sig: libc::c_int| {
        if still_same(target) {
            // SAFETY: a single explicit, positive pid whose identity was just re-verified.
            unsafe { libc::kill(target.pid as libc::pid_t, sig) };
        }
    };
    for target in targets {
        signal(target, libc::SIGTERM);
    }
    let deadline = std::time::Instant::now() + grace;
    while targets.iter().any(still_same) && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    for target in targets {
        signal(target, libc::SIGKILL);
    }
    targets.len()
}

/// Kills escapees of `owner`: marked processes it owns that outlived their stage.
pub fn sweep_owned(proc_root: &Path, uid: u32, owner: Owner, grace: Duration) -> usize {
    let escapees: Vec<Marked> = marked_processes(proc_root, uid)
        .into_iter()
        .filter(|marked| marked.owner == owner)
        .collect();
    terminate(proc_root, &escapees, grace)
}

/// Kills orphans: marked processes whose owner no longer exists (crashed or killed xtask).
pub fn reconcile_orphans(proc_root: &Path, uid: u32, grace: Duration) -> usize {
    let orphans: Vec<Marked> = marked_processes(proc_root, uid)
        .into_iter()
        .filter(|marked| !owner_alive(proc_root, marked.owner))
        .collect();
    terminate(proc_root, &orphans, grace)
}
