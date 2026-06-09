//! Compatibility facade for procfs process scanning, task classification, and target expansion.
//!
//! Owns:
//! - stable `crate::process_tree::*` re-exports while implementation lives in focused
//!   `crate::process::*` modules.
//!
//! Does not own:
//! - procfs reads, cgroup traversal, classification heuristics, snapshot construction, tree
//!   rendering, action execution, daemon runtime policy, remote API handling, report rendering,
//!   live recorder buffers, or autotune controller state.
//!
//! Main entry points:
//! - `ScanBudget`, `ProcessCache`, `ProcInfo`, `TaskInfo`, `TaskClass`, `TargetSnapshot`,
//!   `scan_processes_at`, `target_snapshot`, `diff_tasks_ref`, `find_auto_target_pids`,
//!   `expand_tasks_at`, `classify_task_with_context`, `render_tree_at`, and
//!   `collect_cgroup_pids_at`.
//!
//! Safety, mutation, and persistence invariants live with the owning child modules.

#[cfg(test)]
pub use crate::process::model::{PriorityBand, priority_band_for_class};
pub(crate) use crate::process::procfs::read_proc_info_at;
pub use crate::process::{
    cgroup::collect_cgroup_pids_at,
    classify::{classify_task, classify_task_with_context},
    model::{
        CachedProcInfo, CompiledPattern, DEFAULT_MAX_PROC_SCAN_MS, DEFAULT_MAX_THREADS_PER_PROCESS,
        ProcInfo, ProcessCache, ScanBudget, ScanBudgetReport, TargetDiffAction, TargetDiffRef,
        TargetSnapshot, TargetSnapshotInput, TaskClass, TaskFilters, TaskInfo, TaskMap,
        same_logical_task, sched_policy_name,
    },
    procfs::{
        descendants_of, scan_processes_at, task_comm_at, thread_ids_of_at, thread_ids_of_at_limited,
    },
    snapshot::{
        diff_tasks_ref, expand_tasks_at, find_auto_target_pids, parse_proc_stat_policy,
        parse_proc_stat_starttime, process_starttime_at, target_snapshot,
    },
    tree::{render_tree, render_tree_at},
};

#[cfg(test)]
#[path = "process/tests/community_rules.rs"]
mod community_rules_process_tree_tests;

#[cfg(test)]
#[path = "process/tests/scanner.rs"]
mod tests;
