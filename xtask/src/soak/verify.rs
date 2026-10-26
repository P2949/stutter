//! The acceptance analysis of a soak: counts against the minimums, and the zero-violation list
//! of the plan, read back from what the controller itself wrote (history, audit, journal, ledger)
//! and from the round records.
use std::{collections::BTreeMap, fs, path::Path};

use serde::Serialize;

use super::{
    history::{HistoryEvent, audit_events, first_resolutions, read_events},
    records::{RoundRecord, SoakState, read_rounds, read_state, unix_ms},
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
}

#[derive(Debug, Serialize)]
pub struct Check {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub passed: bool,
    pub counts: Counts,
    pub checks: Vec<Check>,
}

pub const MIN_HOURS: f64 = 24.0;
pub const MIN_EXPERIMENTS: usize = 20;
pub const MIN_FOCUS_TRANSITIONS: usize = 10;
pub const MIN_CONTROLLER_RESTARTS: usize = 2;
pub const MIN_WORKER_RESTARTS: usize = 2;
pub const MIN_SUSPENDS: usize = 2;

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
        if event.is_keep() {
            counts.kept += 1;
        } else {
            counts.reverted += 1;
        }
        *counts
            .by_operation
            .entry(event.action_kind.clone().unwrap_or_default())
            .or_default() += 1;
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
    counts
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

    let minimums = [
        (
            counts.wall_clock_hours >= MIN_HOURS,
            format!(
                "{:.2} h of wall-clock time (>= {MIN_HOURS})",
                counts.wall_clock_hours
            ),
        ),
        (
            counts.experiments >= MIN_EXPERIMENTS,
            format!(
                "{} completed keep/revert experiments (>= {MIN_EXPERIMENTS})",
                counts.experiments
            ),
        ),
        (
            counts.focus_transitions >= MIN_FOCUS_TRANSITIONS,
            format!(
                "{} workload/focus transitions (>= {MIN_FOCUS_TRANSITIONS})",
                counts.focus_transitions
            ),
        ),
        (
            counts.controller_restarts >= MIN_CONTROLLER_RESTARTS,
            format!(
                "{} controller restarts (>= {MIN_CONTROLLER_RESTARTS})",
                counts.controller_restarts
            ),
        ),
        (
            counts.worker_restarts >= MIN_WORKER_RESTARTS,
            format!(
                "{} privileged-worker restarts (>= {MIN_WORKER_RESTARTS})",
                counts.worker_restarts
            ),
        ),
        (
            counts.suspend_resume_cycles >= MIN_SUSPENDS,
            format!(
                "{} suspend/resume cycles (>= {MIN_SUSPENDS})",
                counts.suspend_resume_cycles
            ),
        ),
        (
            counts.by_operation.len() >= 2,
            format!(
                "experiments on {} operations ({:?})",
                counts.by_operation.len(),
                counts.by_operation
            ),
        ),
        (
            counts.kept > 0 && counts.reverted > 0,
            format!("{} kept and {} reverted", counts.kept, counts.reverted),
        ),
    ];
    for (passed, detail) in minimums {
        checks.push(Check {
            name: "minimum".to_owned(),
            passed,
            detail,
        });
    }

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
        "soak {}: {:.2} h, {} rounds ({} clean), {} experiments ({} kept, {} reverted)\n",
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
}
