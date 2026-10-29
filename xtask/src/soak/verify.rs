//! The acceptance analysis of a soak: counts against the minimums, and the zero-violation list
//! of the plan, read back from what the controller itself wrote (history, audit, journal, ledger)
//! and from the round records.
use std::{collections::BTreeMap, fs, path::Path};

use serde::Serialize;

use super::{
    history::{HistoryEvent, audit_events, first_resolutions, read_events},
    records::{Profile, RoundRecord, SoakState, read_rounds, read_state, unix_ms},
};

#[derive(Debug, Default, Serialize)]
pub struct Counts {
    pub rounds: usize,
    pub rounds_ok: usize,
    pub wall_clock_hours: f64,
    pub experiments: usize,
    pub kept: usize,
    pub reverted: usize,
    pub by_operation: BTreeMap<String, usize>,
    pub focus_transitions: usize,
    pub controller_sessions: usize,
    pub controller_restarts: usize,
    pub worker_restarts: usize,
    pub suspend_resume_cycles: usize,
    /// Rounds whose every record ended `ok` with no violation.
    pub clean_rounds: usize,
    /// Scheduled 16-round cycles whose rounds are all present and clean (never inferred from the
    /// total round count: a failed or interrupted round breaks its cycle).
    pub clean_complete_cycles: usize,
    /// Clean rounds per schedule kind (`plain`, `controller_crash`, ...).
    pub clean_rounds_by_kind: BTreeMap<String, usize>,
    pub kept_by_operation: BTreeMap<String, usize>,
    pub reverted_by_operation: BTreeMap<String, usize>,
}

#[derive(Debug, Serialize)]
pub struct Check {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub profile: Profile,
    pub passed: bool,
    pub counts: Counts,
    pub checks: Vec<Check>,
}

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

/// The graduated operations: the only action kinds an autonomous experiment may apply.
const SUPPORTED_KINDS: [&str; 2] = ["cpu_affinity_profile", "cgroup_placement"];
const KEEP_COOLDOWN_MS: u128 = 60_000;
const REVERT_COOLDOWN_MS: u128 = 120_000;
const SAME_ACTION_SPACING_MS: u128 = 300_000;

fn check(checks: &mut Vec<Check>, name: &str, problems: Vec<String>, ok_detail: &str) {
    let passed = problems.is_empty();
    checks.push(Check {
        name: name.to_owned(),
        passed,
        detail: if passed {
            ok_detail.to_owned()
        } else {
            problems.join("; ")
        },
    });
}

pub fn counts(state: &SoakState, rounds: &[RoundRecord], events: &[HistoryEvent]) -> Counts {
    let mut counts = Counts {
        rounds: rounds.len(),
        rounds_ok: rounds.iter().filter(|round| round.outcome == "ok").count(),
        ..Counts::default()
    };
    let end = rounds
        .iter()
        .map(|round| round.finished_unix_ms)
        .max()
        .unwrap_or(0);
    counts.wall_clock_hours = state
        .started_unix_ms
        .map_or(0.0, |start| end.saturating_sub(start) as f64 / 3_600_000.0);
    for event in first_resolutions(events) {
        counts.experiments += 1;
        let operation = event.action_kind.clone().unwrap_or_default();
        if event.is_keep() {
            counts.kept += 1;
            *counts
                .kept_by_operation
                .entry(operation.clone())
                .or_default() += 1;
        } else {
            counts.reverted += 1;
            *counts
                .reverted_by_operation
                .entry(operation.clone())
                .or_default() += 1;
        }
        *counts.by_operation.entry(operation).or_default() += 1;
    }
    for round in rounds {
        let of = |kind: &str| {
            round
                .events
                .iter()
                .filter(|event| event.kind == kind)
                .count()
        };
        counts.focus_transitions += of("game_start") + of("game_exit") + of("game_killed");
        counts.controller_sessions += round.sessions.len();
        counts.controller_restarts += round
            .sessions
            .iter()
            .filter(|s| {
                matches!(
                    s.ended_by.as_str(),
                    "kill9" | "sigint" | "faulted_with_worker"
                )
            })
            .count();
        counts.worker_restarts += of("worker_kill").min(of("worker_start"));
        if of("suspend") > 0 && of("resume") > 0 && round.violations.is_empty() {
            counts.suspend_resume_cycles += of("suspend").min(of("resume"));
        }
    }
    coverage(rounds, &mut counts);
    counts
}

/// Clean rounds, per kind and per scheduled cycle. A round index is clean only if every record
/// written for it ended `ok` without a violation, so an interrupted or failed attempt never
/// counts toward the cycle it belongs to.
fn coverage(rounds: &[RoundRecord], counts: &mut Counts) {
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

pub fn verify(dir: &Path) -> anyhow::Result<Report> {
    let state = read_state(dir)?;
    let rounds = read_rounds(dir)?;
    let history = dir.join("home/.local/state/stutter/autotune/history.jsonl");
    let events = if history.exists() {
        read_events(&history)?
    } else {
        Vec::new()
    };
    let counts = counts(&state, &rounds, &events);
    let mut checks = Vec::new();

    checks.extend(minimum_checks(
        &counts,
        &Minimums::for_profile(state.profile),
    ));

    let mut rounds_problems = Vec::new();
    for round in &rounds {
        for violation in &round.violations {
            rounds_problems.push(format!(
                "round {} ({}): {violation}",
                round.round, round.kind
            ));
        }
        if round.outcome != "ok" {
            rounds_problems.push(format!(
                "round {} ({}) ended `{}`: {}",
                round.round,
                round.kind,
                round.outcome,
                round.error.clone().unwrap_or_default()
            ));
        }
    }
    check(
        &mut checks,
        "rounds_clean",
        rounds_problems,
        "every round ended clean: no unrecovered token, stale kept action, failed recovery or missing experiment",
    );

    check(
        &mut checks,
        "final_state",
        final_state_problems(dir),
        "the journal is clean and the ledger is gone at the end",
    );
    check(
        &mut checks,
        "support_matrix",
        matrix_problems(&events),
        "every applied experiment was a graduated operation under GameCpuSchedulerPressure in apply-medium-risk",
    );
    check(
        &mut checks,
        "conflicts_and_cooldowns",
        ordering_problems(&rounds, &events),
        "no conflicting kept group, cooldown violation or same-action respacing inside a controller session",
    );
    check(
        &mut checks,
        "faults_audited",
        audit_problems(dir, &rounds, &events),
        "every injected fault left its audit/history evidence",
    );

    let passed = checks.iter().all(|check| check.passed);
    Ok(Report {
        profile: state.profile,
        passed,
        counts,
        checks,
    })
}

fn final_state_problems(dir: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    let autotune = dir.join("home/.local/state/stutter/autotune");
    if let Ok(text) = fs::read_to_string(autotune.join("controller_journal.json")) {
        let state = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|value| value["state"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "unreadable".to_owned());
        if !matches!(state.as_str(), "clean" | "reverted") {
            problems.push(format!("the journal is `{state}`"));
        }
    }
    if autotune.join("kept-actions.json").exists() {
        problems.push("the kept-action ledger still holds entries".to_owned());
    }
    problems
}

fn matrix_problems(events: &[HistoryEvent]) -> Vec<String> {
    events
        .iter()
        .filter(|event| {
            matches!(
                event.decision.as_str(),
                "candidate_started" | "candidate_applied"
            )
        })
        .filter_map(|event| {
            let kind = event.action_kind.clone().unwrap_or_default();
            let outside = !SUPPORTED_KINDS.contains(&kind.as_str())
                || event.situation != "GameCpuSchedulerPressure"
                || event.mode != "ApplyMediumRisk";
            outside.then(|| {
                format!(
                    "{} {kind} in {} / {} is outside the support matrix",
                    event.decision, event.situation, event.mode
                )
            })
        })
        .collect()
}

fn conflicts(a: &str, b: &str) -> bool {
    // The graduated operations are CpuPlacement and CgroupPlacement: they conflict with each
    // other, and each with itself.
    SUPPORTED_KINDS.contains(&a) && SUPPORTED_KINDS.contains(&b)
}

/// Within one controller session (one workload): a kept action blocks every conflicting start,
/// the cooldown after a keep or revert holds, and the same action is not restarted early.
fn ordering_problems(rounds: &[RoundRecord], events: &[HistoryEvent]) -> Vec<String> {
    let mut problems = Vec::new();
    let sessions: Vec<(u128, u128)> = rounds
        .iter()
        .flat_map(|round| round.sessions.iter())
        .map(|s| {
            (
                u128::from(s.start_unix_ms) * 1_000_000,
                u128::from(s.end_unix_ms) * 1_000_000,
            )
        })
        .collect();
    for (start, end) in sessions {
        let mut kept: Vec<String> = Vec::new();
        let mut last_resolution: Option<(u128, bool)> = None;
        let mut last_start: BTreeMap<String, u128> = BTreeMap::new();
        let in_session = |e: &&HistoryEvent| e.unix_nanos >= start && e.unix_nanos <= end;
        let mut timeline: Vec<(&HistoryEvent, &str)> = events
            .iter()
            .filter(in_session)
            .filter(|e| e.decision == "candidate_started")
            .map(|e| (e, "candidate_started"))
            .chain(
                first_resolutions(events)
                    .into_iter()
                    .filter(in_session)
                    .map(|e| {
                        (
                            e,
                            if e.is_keep() {
                                "candidate_kept"
                            } else {
                                "candidate_reverted"
                            },
                        )
                    }),
            )
            .collect();
        timeline.sort_by_key(|(event, _)| event.unix_nanos);
        for (event, decision) in timeline {
            let kind = event.action_kind.clone().unwrap_or_default();
            match decision {
                "candidate_started" => {
                    if let Some(held) = kept.iter().find(|held| conflicts(held, &kind)) {
                        problems.push(format!("{kind} started while {held} was kept"));
                    }
                    if let Some((at, keep)) = last_resolution {
                        let cooldown = if keep {
                            KEEP_COOLDOWN_MS
                        } else {
                            REVERT_COOLDOWN_MS
                        };
                        if event.unix_nanos < at + cooldown * 1_000_000 {
                            problems.push(format!(
                                "{kind} started inside the cooldown of the previous experiment"
                            ));
                        }
                    }
                    let id = event.action_id.clone().unwrap_or(kind.clone());
                    if let Some(previous) = last_start.insert(id.clone(), event.unix_nanos)
                        && event.unix_nanos < previous + SAME_ACTION_SPACING_MS * 1_000_000
                    {
                        problems.push(format!("{id} restarted inside its minimum spacing"));
                    }
                }
                "candidate_kept" => {
                    kept.push(kind);
                    last_resolution = Some((event.unix_nanos, true));
                }
                "candidate_reverted" => last_resolution = Some((event.unix_nanos, false)),
                _ => {}
            }
        }
    }
    problems
}

/// Each injected fault must have left the evidence the plan requires.
fn audit_problems(dir: &Path, rounds: &[RoundRecord], events: &[HistoryEvent]) -> Vec<String> {
    let mut problems = Vec::new();
    let audit = dir.join("home/.local/state/stutter/audit/actions.jsonl");
    let recoveries = audit_events(&audit, "autotune-startup-recovery");
    if rounds.iter().any(|round| round.kind == "controller_crash") && recoveries.is_empty() {
        problems.push("controller crashes left no startup-recovery audit event".to_owned());
    }
    for round in rounds {
        let has = |kind: &str| round.events.iter().any(|event| event.kind == kind);
        match round.kind.as_str() {
            "controller_crash" if round.outcome == "ok" && !has("startup_recovery") => {
                problems.push(format!(
                    "round {}: the crash was not followed by a recorded recovery",
                    round.round
                ));
            }
            "worker_kill" if !has("worker_start") => {
                problems.push(format!(
                    "round {}: the worker was not restarted",
                    round.round
                ));
            }
            "suspend" if round.outcome == "ok" && !has("resume") => {
                problems.push(format!("round {}: no resume was recorded", round.round));
            }
            _ => {}
        }
    }
    let restored = events.iter().filter(|e| e.decision == "restored").count();
    if recoveries.iter().filter(|(ok, _)| *ok).count() > restored {
        problems.push("a successful startup recovery has no `restored` history event".to_owned());
    }
    problems
}

pub fn render(report: &Report) -> String {
    let mut text = format!(
        "{:?} soak {}: {:.2} h, {} rounds ({} clean), {} experiments ({} kept, {} reverted)\n",
        report.profile,
        if report.passed { "PASSED" } else { "FAILED" },
        report.counts.wall_clock_hours,
        report.counts.rounds,
        report.counts.rounds_ok,
        report.counts.experiments,
        report.counts.kept,
        report.counts.reverted
    );
    for check in &report.checks {
        text.push_str(&format!(
            "  {} {}: {}\n",
            if check.passed { "PASS" } else { "FAIL" },
            check.name,
            check.detail
        ));
    }
    text
}

pub fn status_line(dir: &Path) -> anyhow::Result<String> {
    let state = read_state(dir)?;
    let rounds = read_rounds(dir)?;
    let history = dir.join("home/.local/state/stutter/autotune/history.jsonl");
    let events = if history.exists() {
        read_events(&history)?
    } else {
        Vec::new()
    };
    let counts = counts(&state, &rounds, &events);
    let remaining = state.started_unix_ms.map_or(f64::NAN, |start| {
        (start as f64 + state.hours * 3_600_000.0 - unix_ms() as f64) / 3_600_000.0
    });
    Ok(format!(
        "soak {} ({:?}): next round {}, {:.2} h elapsed, {:.2} h left; {}",
        state.id,
        state.profile,
        state.next_round,
        counts.wall_clock_hours,
        remaining,
        serde_json::to_string(&counts)?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn session(start_s: u64, end_s: u64) -> RoundRecord {
        RoundRecord {
            sessions: vec![super::super::records::SessionRecord {
                start_unix_ms: start_s * 1_000,
                end_unix_ms: end_s * 1_000,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn an_apply_outside_the_matrix_is_reported() {
        let mut nice = event("candidate_applied", "nice", 10);
        nice.situation = "GameFocused".to_owned();
        assert_eq!(
            matrix_problems(&[event("candidate_applied", "cgroup_placement", 1)]).len(),
            0
        );
        assert_eq!(matrix_problems(&[nice]).len(), 1);
    }

    #[test]
    fn a_conflicting_start_after_a_keep_is_reported() {
        let rounds = [session(0, 1_000)];
        let events = [
            event("candidate_started", "cpu_affinity_profile", 10),
            event("candidate_kept", "cpu_affinity_profile", 60),
            event("candidate_started", "cgroup_placement", 400),
        ];
        assert!(
            ordering_problems(&rounds, &events)
                .iter()
                .any(|p| p.contains("was kept"))
        );
    }

    #[test]
    fn a_start_inside_the_cooldown_or_spacing_is_reported() {
        let rounds = [session(0, 1_000)];
        let events = [
            event("candidate_started", "cpu_affinity_profile", 10),
            event("candidate_reverted", "cpu_affinity_profile", 60),
            event("candidate_started", "cgroup_placement", 100),
            event("candidate_reverted", "cgroup_placement", 150),
            event("candidate_started", "cpu_affinity_profile", 280),
        ];
        let problems = ordering_problems(&rounds, &events);
        assert!(
            problems.iter().any(|p| p.contains("cooldown")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("spacing")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_clean_sequence_passes() {
        let rounds = [session(0, 2_000)];
        let events = [
            event("candidate_started", "cpu_affinity_profile", 10),
            event("candidate_reverted", "cpu_affinity_profile", 60),
            event("candidate_started", "cgroup_placement", 200),
            event("candidate_kept", "cgroup_placement", 260),
        ];
        assert!(ordering_problems(&rounds, &events).is_empty());
    }

    #[test]
    fn counts_follow_the_round_records() {
        let mut round = RoundRecord {
            outcome: "ok".to_owned(),
            ..Default::default()
        };
        round.events = [
            "game_start",
            "game_exit",
            "worker_kill",
            "worker_start",
            "suspend",
            "resume",
        ]
        .iter()
        .map(|kind| super::super::records::EventRecord {
            kind: (*kind).to_owned(),
            ..Default::default()
        })
        .collect();
        round.sessions = vec![super::super::records::SessionRecord {
            ended_by: "kill9".to_owned(),
            ..Default::default()
        }];
        let state = SoakState {
            schema_version: 1,
            id: "x".to_owned(),
            profile: super::super::records::Profile::Smoke,
            seed: 0,
            hours: 1.0,
            stutter_bin: "s".into(),
            stutter_sha256: String::new(),
            xtask_sha256: String::new(),
            source_commit: String::new(),
            created_unix_ms: 0,
            started_unix_ms: None,
            allow_dirty: false,
            cpus: None,
            next_round: 0,
            finished: false,
        };
        let events = [
            event("candidate_kept", "cgroup_placement", 1),
            event("candidate_reverted", "cpu_affinity_profile", 2),
        ];
        let counts = counts(&state, &[round], &events);
        assert_eq!(
            (
                counts.experiments,
                counts.kept,
                counts.reverted,
                counts.focus_transitions
            ),
            (2, 1, 1, 2)
        );
        assert_eq!(
            (
                counts.controller_restarts,
                counts.worker_restarts,
                counts.suspend_resume_cycles
            ),
            (1, 1, 1)
        );
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
