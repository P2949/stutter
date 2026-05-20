//! Process tree rendering helpers.
//!
//! Owns render-tree output construction and traversal ordering. Does not own procfs scanning,
//! target snapshot selection, task classification policy, or cgroup traversal.

use std::{collections::BTreeMap, path::Path};

use super::{
    classify::classify_task,
    model::{ProcInfo, ProcessCache, ScanBudget, ScanBudgetReport, TaskClass},
};
use crate::process_tree::{scan_processes_at, task_comm_at, thread_ids_of_at};

pub fn render_tree(root_pid: u32) -> anyhow::Result<String> {
    render_tree_at(Path::new("/proc"), root_pid)
}

pub fn render_tree_at(proc_root: &Path, root_pid: u32) -> anyhow::Result<String> {
    let budget = ScanBudget::default_proc_scan();
    let mut budget_report = ScanBudgetReport::default();
    let processes = scan_processes_at(
        proc_root,
        &mut ProcessCache::default(),
        &budget,
        &mut budget_report,
    );
    let Some(root) = processes.get(&root_pid) else {
        anyhow::bail!("process {root_pid} not found under {}", proc_root.display());
    };

    let mut children_by_parent: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for proc_info in processes.values() {
        children_by_parent
            .entry(proc_info.ppid)
            .or_default()
            .push(proc_info.pid);
    }

    let root_class = classify_task(&root.comm, &root.comm, &root.cmdline);
    let mut output = format!(
        "root {} {}{}\n",
        root.pid,
        root.comm,
        class_suffix(root_class)
    );

    render_children(
        proc_root,
        root_pid,
        &processes,
        &children_by_parent,
        "",
        false,
        &mut output,
    );

    Ok(output)
}

fn render_children(
    proc_root: &Path,
    pid: u32,
    processes: &BTreeMap<u32, ProcInfo>,
    children_by_parent: &BTreeMap<u32, Vec<u32>>,
    prefix: &str,
    mark_unknown_as_helper: bool,
    output: &mut String,
) {
    let mut entries = Vec::new();

    if let Some(children) = children_by_parent.get(&pid) {
        for child in children {
            if let Some(proc_info) = processes.get(child) {
                let mut class = classify_task(&proc_info.comm, &proc_info.comm, &proc_info.cmdline);
                if class == TaskClass::Unknown && mark_unknown_as_helper {
                    class = TaskClass::Helper;
                }
                entries.push(TreeEntry::Process {
                    pid: *child,
                    comm: proc_info.comm.clone(),
                    class,
                });
            }
        }
    }

    for tid in thread_ids_of_at(proc_root, pid) {
        if tid != pid && !processes.contains_key(&tid) {
            let comm = task_comm_at(proc_root, pid, tid).unwrap_or_else(|| {
                processes
                    .get(&pid)
                    .map(|proc_info| proc_info.comm.clone())
                    .unwrap_or_else(|| "?".to_owned())
            });

            let mut class = processes
                .get(&pid)
                .map(|proc_info| classify_task(&comm, &proc_info.comm, &proc_info.cmdline))
                .unwrap_or(TaskClass::Unknown);

            if class == TaskClass::Unknown && mark_unknown_as_helper {
                class = TaskClass::Helper;
            }

            entries.push(TreeEntry::Task { tid, comm, class });
        }
    }

    entries.sort_by_key(TreeEntry::id);

    for (idx, entry) in entries.iter().enumerate() {
        let is_last = idx + 1 == entries.len();
        let branch = if is_last { "└─ " } else { "├─ " };
        let child_prefix = if is_last {
            format!("{prefix}   ")
        } else {
            format!("{prefix}│  ")
        };

        match entry {
            TreeEntry::Process { pid, comm, class } => {
                output.push_str(&format!(
                    "{prefix}{branch}{pid} {comm}{}\n",
                    class_suffix(*class)
                ));
                render_children(
                    proc_root,
                    *pid,
                    processes,
                    children_by_parent,
                    &child_prefix,
                    true,
                    output,
                );
            }
            TreeEntry::Task { tid, comm, class } => {
                output.push_str(&format!(
                    "{prefix}{branch}{tid} {comm}{}\n",
                    class_suffix(*class)
                ));
            }
        }
    }
}

fn class_suffix(class: TaskClass) -> String {
    if class == TaskClass::Unknown {
        String::new()
    } else {
        format!(" [{class}]")
    }
}

enum TreeEntry {
    Process {
        pid: u32,
        comm: String,
        class: TaskClass,
    },
    Task {
        tid: u32,
        comm: String,
        class: TaskClass,
    },
}

impl TreeEntry {
    fn id(&self) -> u32 {
        match self {
            TreeEntry::Process { pid, .. } => *pid,
            TreeEntry::Task { tid, .. } => *tid,
        }
    }
}
