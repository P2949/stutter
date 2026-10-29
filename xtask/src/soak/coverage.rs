//! The coverage-driven qualification of a soak: what each profile must have exercised
//! (`Minimums`), the clean-round and clean-cycle accounting, and the checks that compare them.
//! The full profile is a coverage floor, not a duration; the run waits for it (`coverage_reached`)
//! after its minimum dwell. Split from `verify` to keep both under the file-size limit.
use std::{collections::BTreeMap, path::Path};

use serde::Serialize;

use super::{
    history::read_events,
    records::{Profile, RoundRecord, read_rounds, read_state},
    verify::{Check, Counts, SUPPORTED_KINDS, counts},
};

/// What a profile must have exercised. Both profiles require both graduated operations, a keep
/// and a revert, and the same zero-violation checks. The short profile claims no coverage
/// floor; the full profile is a coverage-driven qualification: a minimum dwell, a count of
/// complete clean schedule cycles, repeated clean executions of every dangerous event kind, and
/// repeated keep and revert outcomes of each graduated operation. Time alone never passes it.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Minimums {
    pub hours: f64,
    pub experiments: usize,
    pub focus_transitions: usize,
    pub controller_restarts: usize,
    pub worker_restarts: usize,
    pub suspends: usize,
    pub clean_rounds: usize,
    pub clean_cycles: usize,
    /// Clean executions of each of `plan::DANGEROUS_KINDS`.
    pub per_kind: usize,
    pub per_operation_resolved: usize,
    pub per_operation_kept: usize,
    pub per_operation_reverted: usize,
}

impl Minimums {
    pub const FULL: Self = Self {
        hours: 8.0,
        experiments: 20,
        focus_transitions: 10,
        controller_restarts: 2,
        worker_restarts: 2,
        suspends: 2,
        clean_rounds: 96,
        clean_cycles: 6,
        per_kind: 6,
        per_operation_resolved: 12,
        per_operation_kept: 3,
        per_operation_reverted: 3,
    };
    pub const SMOKE: Self = Self {
        hours: 0.0,
        experiments: 0,
        focus_transitions: 1,
        controller_restarts: 1,
        worker_restarts: 1,
        suspends: 1,
        clean_rounds: 0,
        clean_cycles: 0,
        per_kind: 0,
        per_operation_resolved: 0,
        per_operation_kept: 0,
        per_operation_reverted: 0,
    };

    pub fn for_profile(profile: Profile) -> Self {
        match profile {
            Profile::Full => Self::FULL,
            Profile::Smoke => Self::SMOKE,
        }
    }
}

/// Clean rounds, per kind and per scheduled cycle. A round index is clean only if every record
/// written for it ended `ok` without a violation, so an interrupted or failed attempt never
/// counts toward the cycle it belongs to.
pub fn tally(rounds: &[RoundRecord], counts: &mut Counts) {
    let mut clean: BTreeMap<u32, (bool, &str)> = BTreeMap::new();
    for round in rounds {
        let ok = round.outcome == "ok" && round.violations.is_empty();
        let entry = clean
            .entry(round.round)
            .or_insert((true, round.kind.as_str()));
        entry.0 &= ok;
        entry.1 = round.kind.as_str();
    }
    for (clean, kind) in clean.values() {
        if *clean {
            counts.clean_rounds += 1;
            *counts
                .clean_rounds_by_kind
                .entry((*kind).to_owned())
                .or_default() += 1;
        }
    }
    let length = u32::try_from(super::plan::CYCLE_LEN).unwrap_or(u32::MAX);
    let last = clean.keys().next_back().copied().unwrap_or(0);
    counts.clean_complete_cycles = (0..=last / length)
        .filter(|cycle| {
            (cycle * length..(cycle + 1) * length)
                .all(|index| clean.get(&index).is_some_and(|(ok, _)| *ok))
        })
        .count();
}

/// The coverage minima of `want` against `counts`, one check each. Thresholds of zero add no
/// check, so the short profile reports exactly the checks it always did.
pub fn minimum_checks(counts: &Counts, want: &Minimums) -> Vec<Check> {
    let mut checks = Vec::new();
    let mut push = |name: String, passed: bool, detail: String| {
        checks.push(Check {
            name,
            passed,
            detail,
        });
    };
    push(
        "minimum_wall_clock".to_owned(),
        counts.wall_clock_hours >= want.hours,
        format!(
            "{:.2} h of wall-clock time (>= {})",
            counts.wall_clock_hours, want.hours
        ),
    );
    push(
        "minimum_experiments".to_owned(),
        counts.experiments >= want.experiments,
        format!(
            "{} completed keep/revert experiments (>= {})",
            counts.experiments, want.experiments
        ),
    );
    push(
        "minimum_focus_transitions".to_owned(),
        counts.focus_transitions >= want.focus_transitions,
        format!(
            "{} workload/focus transitions (>= {})",
            counts.focus_transitions, want.focus_transitions
        ),
    );
    push(
        "minimum_controller_restarts".to_owned(),
        counts.controller_restarts >= want.controller_restarts,
        format!(
            "{} controller restarts (>= {})",
            counts.controller_restarts, want.controller_restarts
        ),
    );
    push(
        "minimum_worker_restarts".to_owned(),
        counts.worker_restarts >= want.worker_restarts,
        format!(
            "{} privileged-worker restarts (>= {})",
            counts.worker_restarts, want.worker_restarts
        ),
    );
    push(
        "minimum_suspend_resume".to_owned(),
        counts.suspend_resume_cycles >= want.suspends,
        format!(
            "{} suspend/resume cycles (>= {})",
            counts.suspend_resume_cycles, want.suspends
        ),
    );
    push(
        "minimum_operations".to_owned(),
        SUPPORTED_KINDS
            .iter()
            .all(|kind| counts.by_operation.contains_key(*kind)),
        format!(
            "experiments on {} operations ({:?}); both graduated operations are required",
            counts.by_operation.len(),
            counts.by_operation
        ),
    );
    push(
        "minimum_keep_and_revert".to_owned(),
        counts.kept > 0 && counts.reverted > 0,
        format!("{} kept and {} reverted", counts.kept, counts.reverted),
    );
    if want.clean_rounds > 0 {
        push(
            "minimum_clean_rounds".to_owned(),
            counts.clean_rounds >= want.clean_rounds,
            format!(
                "{} clean rounds (>= {})",
                counts.clean_rounds, want.clean_rounds
            ),
        );
    }
    if want.clean_cycles > 0 {
        push(
            "minimum_clean_cycles".to_owned(),
            counts.clean_complete_cycles >= want.clean_cycles,
            format!(
                "{} complete clean {}-round cycles (>= {})",
                counts.clean_complete_cycles,
                super::plan::CYCLE_LEN,
                want.clean_cycles
            ),
        );
    }
    if want.per_kind > 0 {
        for kind in super::plan::DANGEROUS_KINDS {
            let done = counts.clean_rounds_by_kind.get(kind).copied().unwrap_or(0);
            push(
                format!("minimum_round_kind_{kind}"),
                done >= want.per_kind,
                format!("{done} clean `{kind}` rounds (>= {})", want.per_kind),
            );
        }
    }
    if want.per_operation_resolved > 0 {
        for operation in SUPPORTED_KINDS {
            let get = |map: &BTreeMap<String, usize>| map.get(operation).copied().unwrap_or(0);
            let (resolved, kept, reverted) = (
                get(&counts.by_operation),
                get(&counts.kept_by_operation),
                get(&counts.reverted_by_operation),
            );
            push(
                format!("minimum_operation_{operation}"),
                resolved >= want.per_operation_resolved
                    && kept >= want.per_operation_kept
                    && reverted >= want.per_operation_reverted,
                format!(
                    "{resolved} resolved (>= {}), {kept} kept (>= {}), {reverted} reverted (>= {})",
                    want.per_operation_resolved,
                    want.per_operation_kept,
                    want.per_operation_reverted
                ),
            );
        }
    }
    checks
}

/// The coverage floor of the profile is met (the dwell is a separate condition): every minimum
/// but the wall-clock one passes. The short profile has no coverage floor to wait for.
pub fn coverage_reached(dir: &Path) -> anyhow::Result<bool> {
    let state = read_state(dir)?;
    if state.profile != Profile::Full {
        return Ok(true);
    }
    let rounds = read_rounds(dir)?;
    let history = dir.join("home/.local/state/stutter/autotune/history.jsonl");
    let events = if history.exists() {
        read_events(&history)?
    } else {
        Vec::new()
    };
    let counts = counts(&state, &rounds, &events);
    Ok(
        minimum_checks(&counts, &Minimums::for_profile(state.profile))
            .iter()
            .filter(|check| check.name != "minimum_wall_clock")
            .all(|check| check.passed),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soak::{history::HistoryEvent, records::SoakState};

    fn event(decision: &str, kind: &str, at_s: u128) -> HistoryEvent {
        HistoryEvent {
            decision: decision.to_owned(),
            action_kind: Some(kind.to_owned()),
            experiment_id: Some(format!("{kind}-{at_s}")),
            action_id: Some(format!("{kind}:x")),
            situation: "GameCpuSchedulerPressure".to_owned(),
            controller_id: "local-autotune".to_owned(),
            mode: "ApplyMediumRisk".to_owned(),
            unix_nanos: at_s * 1_000_000_000,
            rollback_performed: false,
            reason: String::new(),
        }
    }

    // ---- the coverage-driven full qualification (8 h, 96 clean rounds, 6 clean cycles) ----

    use super::super::plan;

    const HOUR_MS: u64 = 3_600_000;

    /// `cycles` complete clean schedule cycles spread over `hours`, with the events the counts read
    /// and twelve resolved experiments per operation, half kept and half reverted.
    fn full_fixture(cycles: u32, hours: f64) -> (SoakState, Vec<RoundRecord>, Vec<HistoryEvent>) {
        let total = cycles * u32::try_from(plan::CYCLE_LEN).unwrap();
        // Rounded up so a fixture of `hours` never falls a millisecond short of them.
        let step = ((hours * HOUR_MS as f64) as u64).div_ceil(u64::from(total.max(1)));
        let rounds = (0..total)
            .map(|index| {
                let kind = plan::round_for(index).kind.name();
                let mut events = vec!["game_start", "game_exit"];
                match kind {
                    "worker_kill" => events.extend(["worker_kill", "worker_start"]),
                    "suspend" => events.extend(["suspend", "resume"]),
                    _ => {}
                }
                RoundRecord {
                    round: index,
                    kind: kind.to_owned(),
                    outcome: "ok".to_owned(),
                    started_unix_ms: u64::from(index) * step,
                    finished_unix_ms: u64::from(index + 1) * step,
                    sessions: vec![super::super::records::SessionRecord {
                        ended_by: "kill9".to_owned(),
                        ..Default::default()
                    }],
                    events: events
                        .into_iter()
                        .map(|kind| super::super::records::EventRecord {
                            kind: kind.to_owned(),
                            ..Default::default()
                        })
                        .collect(),
                    ..RoundRecord::default()
                }
            })
            .collect();
        let mut events = Vec::new();
        for (operation, base) in [("cpu_affinity_profile", 0), ("cgroup_placement", 1_000)] {
            for number in 0..12_u128 {
                let decision = if number % 2 == 0 {
                    "candidate_kept"
                } else {
                    "candidate_reverted"
                };
                events.push(event(decision, operation, base + number));
            }
        }
        let state = SoakState {
            schema_version: 1,
            id: "x".to_owned(),
            profile: Profile::Full,
            seed: 0,
            hours,
            stutter_bin: "s".into(),
            stutter_sha256: String::new(),
            xtask_sha256: String::new(),
            source_commit: String::new(),
            allow_dirty: false,
            created_unix_ms: 0,
            started_unix_ms: Some(0),
            cpus: None,
            next_round: 0,
            finished: true,
        };
        (state, rounds, events)
    }

    fn failing(counts: &Counts, want: &Minimums) -> Vec<String> {
        minimum_checks(counts, want)
            .into_iter()
            .filter(|check| !check.passed)
            .map(|check| check.name)
            .collect()
    }

    #[test]
    fn a_clean_eight_hour_six_cycle_fixture_passes_the_full_minimums() {
        let (state, rounds, events) = full_fixture(6, 8.0);
        let counts = counts(&state, &rounds, &events);
        assert_eq!(counts.clean_rounds, 96);
        assert_eq!(counts.clean_complete_cycles, 6);
        let failed = failing(&counts, &Minimums::FULL);
        assert!(failed.is_empty(), "{failed:?}");
    }

    #[test]
    fn eight_hours_alone_cannot_pass() {
        let (state, rounds, events) = full_fixture(1, 8.0);
        let failed = failing(&counts(&state, &rounds, &events), &Minimums::FULL);
        assert!(!failed.contains(&"minimum_wall_clock".to_owned()));
        assert!(
            failed.contains(&"minimum_clean_rounds".to_owned()),
            "{failed:?}"
        );
        assert!(
            failed.contains(&"minimum_clean_cycles".to_owned()),
            "{failed:?}"
        );
    }

    #[test]
    fn six_cycles_below_eight_hours_cannot_pass() {
        let (state, rounds, events) = full_fixture(6, 7.9);
        assert_eq!(
            failing(&counts(&state, &rounds, &events), &Minimums::FULL),
            ["minimum_wall_clock"]
        );
    }

    #[test]
    fn ninety_six_rounds_without_six_clean_cycles_cannot_pass() {
        // Seven cycles are 112 rounds, but two of them lose a round, leaving five clean cycles.
        let (state, mut rounds, events) = full_fixture(7, 8.0);
        rounds[3].outcome = "failed".to_owned();
        rounds[20].violations.push("a stale kept action".to_owned());
        let counts = counts(&state, &rounds, &events);
        assert!(counts.clean_rounds >= 96, "{}", counts.clean_rounds);
        assert_eq!(counts.clean_complete_cycles, 5);
        assert_eq!(failing(&counts, &Minimums::FULL), ["minimum_clean_cycles"]);
    }

    #[test]
    fn a_failed_or_interrupted_round_does_not_count_toward_a_clean_cycle() {
        let (state, mut rounds, events) = full_fixture(6, 8.0);
        rounds[0].outcome = "interrupted".to_owned();
        assert_eq!(counts(&state, &rounds, &events).clean_complete_cycles, 5);
        // A later clean retry of the same index does not repair the interrupted attempt.
        let mut retry = rounds[0].clone();
        retry.outcome = "ok".to_owned();
        rounds.push(retry);
        assert_eq!(counts(&state, &rounds, &events).clean_complete_cycles, 5);
    }

    #[test]
    fn a_missing_dangerous_event_class_cannot_pass() {
        for kind in plan::DANGEROUS_KINDS {
            let (state, mut rounds, events) = full_fixture(6, 8.0);
            for round in rounds.iter_mut().filter(|round| round.kind == kind) {
                round.kind = "plain".to_owned();
            }
            let failed = failing(&counts(&state, &rounds, &events), &Minimums::FULL);
            assert!(
                failed.contains(&format!("minimum_round_kind_{kind}")),
                "{kind}: {failed:?}"
            );
        }
    }

    #[test]
    fn a_provider_without_repeated_keep_and_revert_coverage_cannot_pass() {
        let (state, rounds, events) = full_fixture(6, 8.0);
        let without_reverts: Vec<_> = events
            .iter()
            .filter(|event| {
                !(event.action_kind.as_deref() == Some("cgroup_placement")
                    && event.decision == "candidate_reverted")
            })
            .cloned()
            .collect();
        // Dropping the reverts also thins the total, so that sanity floor may trip too; the
        // provider's own floor is what this test is about.
        let failed = failing(&counts(&state, &rounds, &without_reverts), &Minimums::FULL);
        assert!(
            failed.contains(&"minimum_operation_cgroup_placement".to_owned()),
            "{failed:?}"
        );
        assert!(!failed.contains(&"minimum_operation_cpu_affinity_profile".to_owned()));
        let only_one_operation: Vec<_> = events
            .iter()
            .filter(|event| event.action_kind.as_deref() == Some("cpu_affinity_profile"))
            .cloned()
            .collect();
        assert!(
            failing(
                &counts(&state, &rounds, &only_one_operation),
                &Minimums::FULL
            )
            .contains(&"minimum_operation_cgroup_placement".to_owned())
        );
    }

    #[test]
    fn the_smoke_profile_reports_the_checks_it_always_did() {
        let (state, rounds, events) = full_fixture(1, 1.0);
        let names: Vec<String> =
            minimum_checks(&counts(&state, &rounds, &events), &Minimums::SMOKE)
                .into_iter()
                .map(|check| check.name)
                .collect();
        assert_eq!(
            names,
            [
                "minimum_wall_clock",
                "minimum_experiments",
                "minimum_focus_transitions",
                "minimum_controller_restarts",
                "minimum_worker_restarts",
                "minimum_suspend_resume",
                "minimum_operations",
                "minimum_keep_and_revert",
            ]
        );
    }
}
