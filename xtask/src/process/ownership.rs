//! Owner marking and reconciliation of supervised processes.
//!
//! Every supervised child inherits `STUTTER_XTASK_OWNER` = `<pid>:<starttime>` of the supervising
//! process, even after it detaches with `setsid`. A process carrying that marker whose owner is
//! gone (a crashed or killed xtask) is an orphan of this tool, and one whose owner is the current
//! process but outlived its stage is an escapee. Both are found by marker and verified owner
//! identity, never by executable name, and only processes of the same user are touched.
use std::{
    fs,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::fs::MetadataExt,
    },
    path::Path,
    thread,
    time::Duration,
};

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
        let owner = Self {
            pid: parts.next()?.parse().ok()?,
            start_ticks: parts.next()?.parse().ok()?,
            stage: parts.next().map_or(Some(0), |stage| stage.parse().ok())?,
        };
        (parts.next().is_none() && owner.pid > 1 && owner.start_ticks > 0).then_some(owner)
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
    // Open the kernel identity before checking starttime. A recycled numeric pid can never be
    // signalled between a /proc check and kill: pidfd_send_signal stays bound to this process.
    // A kernel that cannot provide pidfds fails closed rather than using the racy fallback.
    let handles: Vec<(&Marked, OwnedFd)> = targets
        .iter()
        .filter_map(|target| {
            if target.pid <= 1 {
                return None;
            }
            // SAFETY: pidfd_open takes a positive pid and zero flags and returns a new owned fd.
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, target.pid, 0) };
            if fd < 0 {
                return None;
            }
            // SAFETY: pidfd_open succeeded and returned a fresh fd owned by this scope.
            let handle = unsafe { OwnedFd::from_raw_fd(fd as i32) };
            (start_ticks(proc_root, target.pid) == Some(target.start_ticks))
                .then_some((target, handle))
        })
        .collect();
    let signal = |fd: &OwnedFd, sig: libc::c_int| {
        // SAFETY: fd is an owned process handle, with no siginfo and no flags.
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                fd.as_raw_fd(),
                sig,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            ) == 0
        }
    };
    let mut signalled = 0;
    for (_, fd) in &handles {
        signalled += usize::from(signal(fd, libc::SIGTERM));
    }
    let deadline = std::time::Instant::now() + grace;
    while handles.iter().any(|(_, fd)| signal(fd, 0)) && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    for (_, fd) in &handles {
        signal(fd, libc::SIGKILL);
    }
    signalled
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_markers_require_a_complete_non_system_identity() {
        for text in ["1:10:1", "0:10", "12:0:1", "12:10:1:garbage", "12:10:1:2"] {
            assert!(Owner::parse(text).is_none(), "{text}");
        }
        assert_eq!(Owner::parse("12:10").unwrap().stage, 0);
        assert_eq!(Owner::parse("12:10:4").unwrap().stage, 4);
    }

    #[test]
    fn a_changed_starttime_never_signals_the_replacement() {
        let mut sentinel = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let target = Marked {
            pid: sentinel.id(),
            start_ticks: 0,
            owner: current_owner(Path::new("/proc")).unwrap(),
        };
        let count = terminate(Path::new("/proc"), &[target], Duration::from_millis(20));
        let alive = sentinel.try_wait().unwrap().is_none();
        let _ = sentinel.kill();
        let _ = sentinel.wait();
        assert_eq!(count, 0);
        assert!(alive);
    }
}
