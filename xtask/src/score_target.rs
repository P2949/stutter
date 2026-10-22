//! Target identity for `score-evaluate`. A strict evaluation resolves the target from the
//! recorded target configuration (`tree_roots`, `manual_pids`), never from which process has
//! the most samples: a compositor or helper can outweigh the intended game. The sample
//! heuristic survives only for non-strict, exploratory use and is labelled as such.
use std::collections::BTreeSet;

use anyhow::{Context, bail};
use stutter::api::recorder::{IntervalRecord, SessionTask};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentitySource {
    TreeRoot,
    ManualPid,
    LegacySampleHeuristic,
}

impl IdentitySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TreeRoot => "tree_root",
            Self::ManualPid => "manual_pid",
            Self::LegacySampleHeuristic => "legacy_sample_heuristic",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TargetIdentity {
    pub pid: u32,
    pub starttime_ticks: u64,
    pub source: IdentitySource,
}

/// The one explicitly recorded target: exactly one tree root, else exactly one manual PID;
/// both present must name the same PID. Anything else is ambiguous.
fn explicit_target(
    tree_roots: &[u32],
    manual_pids: &[u32],
) -> anyhow::Result<(u32, IdentitySource)> {
    let roots = tree_roots.iter().copied().collect::<BTreeSet<_>>();
    let manual = manual_pids.iter().copied().collect::<BTreeSet<_>>();
    match (roots.len(), manual.len()) {
        (1, 0) => Ok((*roots.first().unwrap_or(&0), IdentitySource::TreeRoot)),
        (0, 1) => Ok((*manual.first().unwrap_or(&0), IdentitySource::ManualPid)),
        (1, 1) if roots == manual => Ok((*roots.first().unwrap_or(&0), IdentitySource::TreeRoot)),
        (0, 0) => bail!(
            "recording has no explicit target (no tree root or manual PID); record with --tree-pid"
        ),
        _ => bail!(
            "ambiguous recorded target: tree roots {roots:?}, manual PIDs {manual:?}; \
             record one explicit target with --tree-pid"
        ),
    }
}

fn heuristic_pid(records: &[IntervalRecord]) -> anyhow::Result<u32> {
    let mut samples = std::collections::BTreeMap::<u32, u64>::new();
    for record in records {
        if let Some(pid) = record.process_pid {
            *samples.entry(pid).or_default() += record.samples;
        }
    }
    samples
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(pid, _)| pid)
        .context("no sampled process")
}

/// Resolves the target and its start time. `strict` never falls back to the heuristic.
pub fn resolve(
    tree_roots: &[u32],
    manual_pids: &[u32],
    tasks: &[SessionTask],
    records: &[IntervalRecord],
    strict: bool,
) -> anyhow::Result<TargetIdentity> {
    let (pid, source) = match explicit_target(tree_roots, manual_pids) {
        Ok(explicit) => explicit,
        Err(error) if strict => return Err(error),
        Err(_) => (
            heuristic_pid(records)?,
            IdentitySource::LegacySampleHeuristic,
        ),
    };
    let starttime_ticks = tasks
        .iter()
        .find(|task| task.process_pid == Some(pid))
        .and_then(|task| task.process_starttime_ticks)
        .with_context(|| format!("start time of target pid {pid} missing from the session"))?;
    Ok(TargetIdentity {
        pid,
        starttime_ticks,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(pid: u32, ticks: u64) -> SessionTask {
        SessionTask {
            process_pid: Some(pid),
            process_starttime_ticks: Some(ticks),
            ..SessionTask::default()
        }
    }
    fn record(pid: u32, samples: u64) -> IntervalRecord {
        IntervalRecord {
            process_pid: Some(pid),
            samples,
            ..IntervalRecord::default()
        }
    }
    /// The game (100) is quieter than a compositor (200).
    fn noisy() -> (Vec<SessionTask>, Vec<IntervalRecord>) {
        (
            vec![task(100, 1111), task(200, 2222)],
            vec![record(100, 10), record(200, 5000)],
        )
    }

    #[test]
    fn an_explicit_tree_root_beats_a_noisier_unrelated_process() {
        let (tasks, records) = noisy();
        let target = resolve(&[100], &[], &tasks, &records, true).unwrap();
        assert_eq!(
            (target.pid, target.starttime_ticks, target.source),
            (100, 1111, IdentitySource::TreeRoot)
        );
    }

    #[test]
    fn one_manual_pid_is_the_target() {
        let (tasks, records) = noisy();
        let target = resolve(&[], &[100], &tasks, &records, true).unwrap();
        assert_eq!(
            (target.pid, target.source),
            (100, IdentitySource::ManualPid)
        );
    }

    #[test]
    fn a_tree_root_and_manual_pid_naming_the_same_process_are_accepted() {
        let (tasks, records) = noisy();
        let target = resolve(&[100], &[100, 100], &tasks, &records, true).unwrap();
        assert_eq!((target.pid, target.source), (100, IdentitySource::TreeRoot));
    }

    #[test]
    fn strict_resolution_rejects_ambiguous_and_missing_targets() {
        let (tasks, records) = noisy();
        for (roots, manual) in [
            (vec![100, 200], vec![]),
            (vec![], vec![100, 200]),
            (vec![100], vec![200]),
            (vec![100], vec![100, 200]),
            (vec![], vec![]),
        ] {
            assert!(
                resolve(&roots, &manual, &tasks, &records, true).is_err(),
                "{roots:?} {manual:?}"
            );
        }
    }

    #[test]
    fn a_target_without_a_recorded_start_time_is_an_error() {
        let (_, records) = noisy();
        assert!(resolve(&[100], &[], &[task(200, 2222)], &records, true).is_err());
    }

    #[test]
    fn non_strict_use_keeps_the_labelled_sample_heuristic() {
        let (tasks, records) = noisy();
        let target = resolve(&[], &[], &tasks, &records, false).unwrap();
        assert_eq!(
            (target.pid, target.source),
            (200, IdentitySource::LegacySampleHeuristic)
        );
        // An explicit target is still preferred when one exists.
        let explicit = resolve(&[100], &[], &tasks, &records, false).unwrap();
        assert_eq!(explicit.source, IdentitySource::TreeRoot);
        // Ambiguity falls back (non-strict) rather than failing.
        let ambiguous = resolve(&[100, 200], &[], &tasks, &records, false).unwrap();
        assert_eq!(ambiguous.source, IdentitySource::LegacySampleHeuristic);
    }
}
