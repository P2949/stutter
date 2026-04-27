use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
};

#[derive(Debug, Clone)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub comm: String,
    pub cmdline: String,
}

pub fn scan_processes() -> BTreeMap<u32, ProcInfo> {
    let mut processes = BTreeMap::new();

    let Ok(entries) = fs::read_dir("/proc") else {
        return processes;
    };

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };

        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };

        if let Some(info) = read_proc_info(pid) {
            processes.insert(pid, info);
        }
    }

    processes
}

fn read_proc_info(pid: u32) -> Option<ProcInfo> {
    let status_path = format!("/proc/{pid}/status");
    let status = fs::read_to_string(status_path).ok()?;

    let mut ppid = None;
    let mut comm = None;

    for line in status.lines() {
        if let Some(value) = line.strip_prefix("Name:") {
            comm = Some(value.trim().to_owned());
        } else if let Some(value) = line.strip_prefix("PPid:") {
            ppid = value.trim().parse::<u32>().ok();
        }
    }

    let cmdline_path = format!("/proc/{pid}/cmdline");
    let cmdline = fs::read(cmdline_path)
        .ok()
        .map(|bytes| {
            bytes
                .split(|b| *b == 0)
                .filter(|part| !part.is_empty())
                .map(|part| String::from_utf8_lossy(part))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();

    Some(ProcInfo {
        pid,
        ppid: ppid?,
        comm: comm?,
        cmdline,
    })
}

pub fn descendants_of(root_pid: u32, processes: &BTreeMap<u32, ProcInfo>) -> BTreeSet<u32> {
    let mut children_by_parent: BTreeMap<u32, Vec<u32>> = BTreeMap::new();

    for proc_info in processes.values() {
        children_by_parent
            .entry(proc_info.ppid)
            .or_default()
            .push(proc_info.pid);
    }

    let mut result = BTreeSet::new();
    let mut queue = VecDeque::new();

    result.insert(root_pid);
    queue.push_back(root_pid);

    while let Some(parent) = queue.pop_front() {
        if let Some(children) = children_by_parent.get(&parent) {
            for child in children {
                if result.insert(*child) {
                    queue.push_back(*child);
                }
            }
        }
    }

    result
}

pub fn thread_ids_of(pid: u32) -> BTreeSet<u32> {
    let mut tids = BTreeSet::new();
    let task_path = format!("/proc/{pid}/task");

    let Ok(entries) = fs::read_dir(task_path) else {
        return tids;
    };

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };

        let Ok(tid) = name.parse::<u32>() else {
            continue;
        };

        tids.insert(tid);
    }

    tids
}
