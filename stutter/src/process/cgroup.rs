//! Cgroup traversal helpers for process discovery.
//!
//! Owns recursive cgroup PID collection. Does not own procfs task scanning, classification,
//! target snapshots, or tree rendering.

use std::{collections::BTreeSet, fs, path::Path};

pub fn collect_cgroup_pids_at(cgroup_path: &Path, pids: &mut BTreeSet<u32>) {
    let procs = cgroup_path.join("cgroup.procs");
    let threads = cgroup_path.join("cgroup.threads");

    let mut read_list = |file: &Path| {
        if let Ok(content) = fs::read_to_string(file) {
            for line in content.lines() {
                if let Ok(pid) = line.trim().parse::<u32>() {
                    pids.insert(pid);
                }
            }
        }
    };

    read_list(&procs);
    read_list(&threads);

    if let Ok(entries) = fs::read_dir(cgroup_path) {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type()
                && file_type.is_dir()
            {
                collect_cgroup_pids_at(&entry.path(), pids);
            }
        }
    }
}
