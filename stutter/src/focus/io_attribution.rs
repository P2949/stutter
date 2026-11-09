//! Which processes are producing the block I/O (package W11b).
//!
//! The per-process counters the focus snapshot already keeps (`read_bytes` / `write_bytes` deltas,
//! reset on a start-time change, so a reused pid is never charged for its predecessor) are ranked
//! into offenders with their exact identity. This is attribution for reports and for choosing a
//! lowering target: it names who moved the bytes in the interval, it never says the I/O hurt anyone,
//! and a protected process (the focused window, a game or other interactive class) is listed but
//! flagged so no caller can lower it by accident.
use super::{process_scan::is_non_service_interactive_class, snapshot::FocusProcess};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoOffender {
    pub pid: u32,
    /// Required: a process without a start time has no identity to act on and is never ranked.
    pub starttime_ticks: u64,
    pub comm: String,
    pub read_bytes: u64,
    pub write_bytes: u64,
    /// Share of all attributed bytes in the interval, in tenths of a percent (0..=1000).
    pub share_permille: u32,
    /// The focused window or an interactive class: never a lowering target.
    pub protected: bool,
}

/// The top `limit` processes by bytes moved in the interval, ignoring any that moved fewer than
/// `min_bytes`. Ties break by pid so the ranking is deterministic.
pub fn rank_io_offenders(
    processes: &[FocusProcess],
    min_bytes: u64,
    limit: usize,
) -> Vec<IoOffender> {
    let moved = |process: &FocusProcess| {
        process
            .read_bytes_delta
            .saturating_add(process.write_bytes_delta)
    };
    let total: u128 = processes.iter().map(|p| u128::from(moved(p))).sum();
    let mut ranked: Vec<&FocusProcess> = processes
        .iter()
        .filter(|process| process.starttime_ticks.is_some() && moved(process) >= min_bytes.max(1))
        .collect();
    ranked.sort_by(|a, b| moved(b).cmp(&moved(a)).then(a.pid.cmp(&b.pid)));
    ranked
        .into_iter()
        .take(limit)
        .filter_map(|process| {
            Some(IoOffender {
                pid: process.pid,
                starttime_ticks: process.starttime_ticks?,
                comm: process.comm.clone(),
                read_bytes: process.read_bytes_delta,
                write_bytes: process.write_bytes_delta,
                share_permille: u32::try_from(u128::from(moved(process)) * 1000 / total.max(1))
                    .unwrap_or(1000),
                protected: process.is_foreground_window_process
                    || is_non_service_interactive_class(process.classification.class),
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "io_attribution_tests.rs"]
mod tests;
