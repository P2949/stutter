use std::path::Path;

use super::snapshot::{FocusCache, FocusSnapshot, build_focus_snapshot_from_processes};
use crate::foreground::ForegroundWindowSnapshot;

pub fn focus_snapshot_at(
    proc_root: &Path,
    cache: &mut FocusCache,
    elapsed_ms: u64,
    foreground: Option<&ForegroundWindowSnapshot>,
) -> FocusSnapshot {
    let budget = crate::process_tree::ScanBudget::default_proc_scan();
    let mut budget_report = crate::process_tree::ScanBudgetReport::default();
    let processes = crate::process_tree::scan_processes_at(
        proc_root,
        &mut cache.proc_cache,
        &budget,
        &mut budget_report,
    );

    build_focus_snapshot_from_processes(
        proc_root,
        cache,
        elapsed_ms,
        processes,
        foreground.cloned(),
    )
}
