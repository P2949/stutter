//! Reads a live process's identity and ancestry from procfs for source attribution.
use std::{fs, os::unix::fs::MetadataExt, path::Path};

use super::frame_source::LiveProcess;
use crate::process::snapshot::parse_proc_stat_starttime;

/// Longest ancestor chain followed; a deeper chain is cut rather than trusted.
const MAX_ANCESTORS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClockNamespaceIdentity {
    pub device: u64,
    pub inode: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ClockNamespaceDenial {
    #[error("CLOCK_MONOTONIC namespace identity is unavailable")]
    Unavailable,
    #[error("producer and observer use different CLOCK_MONOTONIC namespaces")]
    DifferentNamespace,
}

impl ClockNamespaceDenial {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "clock_namespace_unavailable",
            Self::DifferentNamespace => "clock_namespace_mismatch",
        }
    }
}

/// CLOCK_MONOTONIC is time-namespace scoped. Compare the active `time` namespace, not
/// `time_for_children`: unshare(CLONE_NEWTIME) changes only the latter for the caller.
/// Source: https://man7.org/linux/man-pages/man7/time_namespaces.7.html
pub fn shared_monotonic_namespace(
    proc_root: &Path,
    producer: u32,
    observer: u32,
) -> Result<ClockNamespaceIdentity, ClockNamespaceDenial> {
    let identity = |pid: u32| -> Result<ClockNamespaceIdentity, ClockNamespaceDenial> {
        let metadata = fs::metadata(proc_root.join(pid.to_string()).join("ns/time"))
            .map_err(|_| ClockNamespaceDenial::Unavailable)?;
        Ok(ClockNamespaceIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    };
    let producer = identity(producer)?;
    let observer = identity(observer)?;
    if producer == observer {
        Ok(producer)
    } else {
        Err(ClockNamespaceDenial::DifferentNamespace)
    }
}

fn parent_pid(stat: &str) -> Option<u32> {
    let (_, after_comm) = stat.rsplit_once(") ")?;
    after_comm.split_whitespace().nth(1)?.parse().ok()
}

/// `pid` as it is now: its start ticks, owner and ancestors as `(pid, start_ticks)`, nearest
/// first. `None` when it is gone. An ancestor whose start time cannot be read ends the chain, so
/// an unverifiable link is never assumed.
pub fn live_process(proc_root: &Path, pid: u32) -> Option<LiveProcess> {
    let dir = proc_root.join(pid.to_string());
    let stat = fs::read_to_string(dir.join("stat")).ok()?;
    let start_ticks = parse_proc_stat_starttime(&stat)?;
    let uid = fs::metadata(&dir).ok()?.uid();
    let mut ancestors = Vec::new();
    let mut next = parent_pid(&stat)?;
    while next > 1 && ancestors.len() < MAX_ANCESTORS {
        let Ok(parent_stat) = fs::read_to_string(proc_root.join(next.to_string()).join("stat"))
        else {
            break;
        };
        let Some(ticks) = parse_proc_stat_starttime(&parent_stat) else {
            break;
        };
        ancestors.push((next, ticks));
        let Some(grandparent) = parent_pid(&parent_stat) else {
            break;
        };
        if grandparent == next {
            break;
        }
        next = grandparent;
    }
    Some(LiveProcess {
        start_ticks,
        uid,
        ancestors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(comm: &str, ppid: u32, start: u64) -> String {
        // pid (comm) state ppid pgrp session tty tpgid flags minflt cminflt majflt cmajflt utime
        // stime cutime cstime priority nice threads itrealvalue starttime ...
        format!("1 ({comm}) S {ppid} 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 {start} 0 0")
    }

    fn put(root: &Path, pid: u32, comm: &str, ppid: u32, start: u64) {
        let dir = root.join(pid.to_string());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("stat"), stat(comm, ppid, start)).unwrap();
    }

    #[test]
    fn ancestry_carries_start_ticks_and_stops_at_a_gap() {
        let root = std::env::temp_dir().join(format!("stutter-proc-view-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        put(&root, 300, "game (x)", 200, 30);
        put(&root, 200, "wine", 100, 20);
        put(&root, 100, "steam", 50, 10);
        // 50 does not exist: the chain ends there rather than guessing.
        let live = live_process(&root, 300).unwrap();
        assert_eq!(live.start_ticks, 30);
        assert_eq!(live.ancestors, vec![(200, 20), (100, 10)]);
        assert!(live_process(&root, 999).is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn monotonic_clock_identity_is_the_active_namespace_and_unknown_is_not_equal() {
        let root =
            std::env::temp_dir().join(format!("stutter-clock-namespace-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for pid in [100, 200] {
            fs::create_dir_all(root.join(format!("{pid}/ns"))).unwrap();
        }
        assert_eq!(
            shared_monotonic_namespace(&root, 100, 200),
            Err(ClockNamespaceDenial::Unavailable)
        );
        fs::write(root.join("100/ns/time"), "namespace handle fixture").unwrap();
        fs::hard_link(root.join("100/ns/time"), root.join("200/ns/time")).unwrap();
        // A future children's namespace is different; it must not replace the active clock.
        fs::write(root.join("100/ns/time_for_children"), "other namespace").unwrap();
        assert!(shared_monotonic_namespace(&root, 100, 200).is_ok());
        fs::remove_file(root.join("200/ns/time")).unwrap();
        fs::write(root.join("200/ns/time"), "a distinct active namespace").unwrap();
        assert_eq!(
            shared_monotonic_namespace(&root, 100, 200),
            Err(ClockNamespaceDenial::DifferentNamespace)
        );
        fs::remove_dir_all(root).unwrap();
    }
}
