//! Validate the complete parsed batch that automatic recovery will restore.
use std::collections::BTreeSet;

use anyhow::ensure;
use stutter_core::ids::{Pid, Tid};
type Identity = (Tid, Option<Pid>, Option<u64>, Option<u64>);

pub(super) fn validate_profile(
    state: &crate::profile_restore::ProfileRestoreState,
    expected: usize,
) -> anyhow::Result<()> {
    let affinity = state.affinity_records.iter().map(|r| {
        (
            r.tid,
            r.process_pid,
            r.process_starttime_ticks,
            r.task_starttime_ticks,
        )
    });
    let nice = state.nice_records.iter().map(|r| {
        (
            r.tid,
            r.process_pid,
            r.process_starttime_ticks,
            r.task_starttime_ticks,
        )
    });
    let ionice = state.ionice_records.iter().map(|r| {
        (
            r.tid,
            r.process_pid,
            r.process_starttime_ticks,
            r.task_starttime_ticks,
        )
    });
    validate(affinity.chain(nice).chain(ionice), expected)
}
pub(super) fn validate_affinity(
    state: &crate::affinity::RestoreState,
    expected: usize,
) -> anyhow::Result<()> {
    validate(
        state.records.iter().map(|r| {
            (
                r.tid,
                r.process_pid,
                r.process_starttime_ticks,
                r.task_starttime_ticks,
            )
        }),
        expected,
    )
}
fn validate(records: impl Iterator<Item = Identity>, expected: usize) -> anyhow::Result<()> {
    let mut tasks = BTreeSet::new();
    for (tid, pid, process_ticks, task_ticks) in records {
        ensure!(
            tid.as_u32() > 0
                && pid.is_some_and(|pid| pid.as_u32() > 0)
                && process_ticks.is_some_and(|ticks| ticks > 0)
                && task_ticks.is_some_and(|ticks| ticks > 0),
            "automatic affinity recovery refuses unverified legacy or incomplete task identity for TID {tid}; original state retained for explicit manual inspection/restore"
        );
        tasks.insert(tid);
    }
    ensure!(
        tasks.len() >= expected,
        "automatic affinity recovery has {} original task identities for {expected} affected tasks; original state retained",
        tasks.len()
    );
    Ok(())
}
