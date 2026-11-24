//! Host facts three crosswalk dossiers observe, read from the kernel's own files while an attempt runs (never assumed):
//!
//! * 5.9 CPU topology: the online CPU list and a CPU's thread siblings are readable;
//! * 5.19 NUMA: the online node list is exactly one node, so no inter-node migration is possible;
//! * 5.20 memory guardrail: memory PSI, swap and the OOM-kill counter are all readable.
//!
//! The root is a parameter so the files can be a fixture's; production reads `/`. A file that is absent, unreadable or not
//! in the expected shape counts for nothing.

use std::{io::Read, path::Path};

const MAX_BYTES: u64 = 64 * 1024;

/// What one look at the host showed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostReadings {
    pub topology: bool,
    pub single_numa_node: bool,
    pub memory_guard: bool,
}

fn read(root: &Path, relative: &str) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(root.join(relative))
        .ok()?
        .take(MAX_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

/// Look at the host's files under `root`.
pub fn readings(root: &Path) -> HostReadings {
    let cpu = "sys/devices/system/cpu";
    let topology = read(root, &format!("{cpu}/online")).is_some_and(|text| !text.trim().is_empty())
        && read(root, &format!("{cpu}/cpu0/topology/thread_siblings_list"))
            .is_some_and(|text| !text.trim().is_empty());
    let single_numa_node =
        read(root, "sys/devices/system/node/online").is_some_and(|text| text.trim() == "0");
    let memory_guard = read(root, "proc/pressure/memory")
        .is_some_and(|text| text.starts_with("some "))
        && read(root, "proc/swaps").is_some()
        && read(root, "proc/vmstat")
            .is_some_and(|text| text.lines().any(|line| line.starts_with("oom_kill ")));
    HostReadings {
        topology,
        single_numa_node,
        memory_guard,
    }
}

#[cfg(test)]
#[path = "host_probe_tests.rs"]
mod tests;
