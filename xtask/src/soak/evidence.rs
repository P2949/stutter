//! The raw evidence directory of a verified soak, in the shape the canonical exporter takes
//! (`xtask evidence-export --raw <soak>/evidence-raw`). It carries compact summaries of the round
//! records, the controller's own history and the fault/audit trail, never the isolated `HOME`.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::ensure;
use serde_json::{Value, json};

use super::{
    history::{audit_events, first_resolutions, read_events},
    records::{Profile, RoundRecord, SoakState, read_rounds, read_state},
    sha256,
    verify::{Report, render},
};
use crate::evidence_metadata::{self as shared, SUITE_AUTOTUNE_SMOKE, SUITE_AUTOTUNE_SOAK, Source};

pub(super) fn suite_for(profile: Profile) -> &'static str {
    match profile {
        Profile::Full => SUITE_AUTOTUNE_SOAK,
        Profile::Smoke => SUITE_AUTOTUNE_SMOKE,
    }
}

fn scope(profile: Profile, hours: f64) -> String {
    let kind = match profile {
        Profile::Full => {
            "coverage-qualified real-system bounded-controller soak (minimum dwell, complete \
             deterministic cycles, explicit repeated fault coverage, zero violations)"
        }
        Profile::Smoke => "short production-controller acceptance",
    };
    format!(
        "{kind} ({hours:.2} h minimum dwell) of the production `stutter autotune --mode apply-medium-risk` \
         controller and the production privileged worker as separate processes, on disposable \
         game-like workloads, with no validation override: every apply passed through the \
         autonomous support registry. Both graduated operations (cpu_affinity_profile, \
         cgroup_placement) ran, with kept and reverted experiments and the injected faults of the \
         schedule."
    )
}

const NOT_CLAIMED: [&str; 6] = [
    "the daemon/service always_on_autotune integration (the packaged service runs apply-low-risk)",
    "any operation other than cpu_affinity_profile and cgroup_placement",
    "any system-wide operation (CPU EPP, GPU power, VM tuning, IRQ affinity)",
    "any workload other than the controlled game-like workloads of the schedule",
    "real games or real user sessions",
    "a threshold or objective decision (those are the mutation-graduation campaigns)",
];

fn rounds_summary(rounds: &[RoundRecord]) -> Value {
    json!(
        rounds
            .iter()
            .map(|round| {
                let mut events = BTreeMap::<&str, usize>::new();
                for event in &round.events {
                    *events.entry(event.kind.as_str()).or_default() += 1;
                }
                json!({
                    "round": round.round,
                    "kind": round.kind,
                    "scenario": round.scenario,
                    "operations": round.operations,
                    "outcome": round.outcome,
                    "seconds": round.finished_unix_ms.saturating_sub(round.started_unix_ms) / 1_000,
                    "controller_sessions": round.sessions.len(),
                    "events": events,
                    "violations": round.violations,
                })
            })
            .collect::<Vec<_>>()
    )
}

fn history_summary(dir: &Path) -> anyhow::Result<Value> {
    let path = dir.join("home/.local/state/stutter/autotune/history.jsonl");
    let events = if path.exists() {
        read_events(&path)?
    } else {
        Vec::new()
    };
    let mut decisions = BTreeMap::<&str, usize>::new();
    for event in &events {
        *decisions.entry(event.decision.as_str()).or_default() += 1;
    }
    let experiments = first_resolutions(&events)
        .into_iter()
        .enumerate()
        .map(|(index, event)| {
            json!({
                "experiment": index + 1,
                "outcome": if event.is_keep() { "kept" } else { "reverted" },
                "action_kind": event.action_kind,
                "situation": event.situation,
                "mode": event.mode,
                "unix_nanos": event.unix_nanos.to_string(),
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({"decision_counts": decisions, "experiments": experiments}))
}

fn audit_summary(dir: &Path, rounds: &[RoundRecord]) -> Value {
    let audit = dir.join("home/.local/state/stutter/audit/actions.jsonl");
    let recoveries = audit_events(&audit, "autotune-startup-recovery");
    let faults = rounds
        .iter()
        .filter(|round| round.kind != "plain")
        .map(|round| json!({"round": round.round, "fault": round.kind, "outcome": round.outcome}))
        .collect::<Vec<_>>();
    json!({
        "startup_recovery": {
            "succeeded": recoveries.iter().filter(|(ok, _)| *ok).count(),
            "failed": recoveries.iter().filter(|(ok, _)| !*ok).count(),
        },
        "injected_faults": faults,
    })
}

fn write_json(path: &Path, value: &Value) -> anyhow::Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

/// Why a verified soak is not publishable evidence, or `None` when it is.
fn unpublishable(state: &SoakState) -> Option<&'static str> {
    if state.allow_dirty {
        return Some("it was started with --allow-dirty and is not bound to a clean source");
    }
    if !state.finished {
        return Some("it has not finished its wall-clock budget");
    }
    None
}

/// Writes `<dir>/evidence-raw` for a PASSED, finished, clean-source soak and returns it. Any
/// earlier evidence-raw is removed first so a failed re-verification never leaves a stale PASS.
pub(super) fn write_raw(
    root: &Path,
    dir: &Path,
    report: &Report,
) -> anyhow::Result<Option<PathBuf>> {
    let state = read_state(dir)?;
    let raw = dir.join("evidence-raw");
    if raw.exists() {
        fs::remove_dir_all(&raw)?;
    }
    if !report.passed {
        return Ok(None);
    }
    if let Some(reason) = unpublishable(&state) {
        println!("no portable evidence: {reason}");
        return Ok(None);
    }
    ensure!(
        sha256(&state.stutter_bin)? == state.stutter_sha256
            && sha256(&std::env::current_exe()?)? == state.xtask_sha256,
        "the binaries changed since `soak init`: a soak is bound to the binaries it started with, \
         so verify it with the same xtask"
    );
    let suite = suite_for(state.profile);
    fs::create_dir_all(raw.join("artifacts"))?;
    let environment = shared::capture(
        root,
        &state.stutter_bin,
        suite,
        "controlled_real",
        Source::Bound {
            commit: &state.source_commit,
            dirty: false,
        },
    )?;
    shared::write_environment(&raw, &environment)?;
    shared::require_clean(&environment)?;
    write_artifacts(&raw, dir, report, &state)?;
    Ok(Some(raw))
}

/// Everything in the raw directory except `environment.json`: the compact summaries, the verify
/// record and `result.json` (written last, so a missing result means the capture did not finish).
fn write_artifacts(
    raw: &Path,
    dir: &Path,
    report: &Report,
    state: &SoakState,
) -> anyhow::Result<()> {
    let suite = suite_for(state.profile);
    fs::create_dir_all(raw.join("artifacts"))?;
    let rounds = read_rounds(dir)?;
    write_json(&raw.join("artifacts/rounds.json"), &rounds_summary(&rounds))?;
    write_json(
        &raw.join("artifacts/controller-history.json"),
        &history_summary(dir)?,
    )?;
    write_json(
        &raw.join("artifacts/audit-faults.json"),
        &audit_summary(dir, &rounds),
    )?;
    write_json(&raw.join("verify.json"), &serde_json::to_value(report)?)?;
    fs::write(raw.join("report.txt"), render(report))?;

    let result = json!({
        "status": "PASS",
        "evidence_kind": "controlled_real",
        "profile": state.profile,
        "scope": scope(state.profile, state.hours),
        "not_claimed": NOT_CLAIMED,
        "operations": report.counts.by_operation.keys().collect::<Vec<_>>(),
        "counts": report.counts,
        "cases": report.checks.iter().map(|check| json!({
            "name": check.name,
            "status": if check.passed { "PASS" } else { "FAIL" },
        })).collect::<Vec<_>>(),
        "checks": report.checks,
        "error": null,
    });
    shared::write_result(raw, suite, result)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::{
        super::{
            plan,
            records::{EventRecord, Profile, RoundRecord, SessionRecord, SoakState, write_state},
            verify::{Report, verify},
        },
        shared, suite_for, unpublishable, write_artifacts,
    };
    use crate::evidence_bundle::{self, tests::Dir};

    fn state(profile: Profile) -> SoakState {
        SoakState {
            schema_version: 1,
            id: "soak-test".to_owned(),
            profile,
            seed: 0,
            hours: 1.0,
            stutter_bin: "stutter".into(),
            stutter_sha256: String::new(),
            xtask_sha256: String::new(),
            source_commit: evidence_bundle::tests::COMMIT.to_owned(),
            allow_dirty: false,
            created_unix_ms: 0,
            started_unix_ms: Some(0),
            cpus: Some((2, 1)),
            next_round: 0,
            finished: true,
        }
    }

    fn round(index: u32, kind: &str, events: &[&str], ended_by: &str) -> RoundRecord {
        RoundRecord {
            round: index,
            kind: kind.to_owned(),
            scenario: "beneficial".to_owned(),
            operations: "both".to_owned(),
            started_unix_ms: u64::from(index) * 100_000,
            finished_unix_ms: u64::from(index + 1) * 100_000,
            outcome: "ok".to_owned(),
            sessions: vec![SessionRecord {
                ended_by: ended_by.to_owned(),
                ..SessionRecord::default()
            }],
            events: events
                .iter()
                .map(|kind| EventRecord {
                    kind: (*kind).to_owned(),
                    ..EventRecord::default()
                })
                .collect(),
            ..RoundRecord::default()
        }
    }

    /// A soak directory with the activity of a short run: both operations, a keep and a revert,
    /// one controller restart, one worker restart, one suspend/resume and focus transitions.
    fn soak_dir(profile: Profile, violation: Option<&str>) -> Dir {
        let dir = Dir::new("soak");
        write_state(&dir.0, &state(profile)).unwrap();
        let mut rounds = [
            round(0, "plain", &["game_start", "game_exit"], "exited"),
            round(
                1,
                "worker_kill",
                &["game_start", "worker_kill", "worker_start", "game_exit"],
                "kill9",
            ),
            round(
                2,
                "suspend",
                &["game_start", "suspend", "resume", "game_exit"],
                "exited",
            ),
        ];
        rounds[1].violations.extend(violation.map(str::to_owned));
        let lines = rounds
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(dir.join("rounds.jsonl"), lines + "\n").unwrap();
        let history = dir.join("home/.local/state/stutter/autotune");
        fs::create_dir_all(&history).unwrap();
        let line = |decision: &str, kind: &str, id: &str, at: u64| {
            json!({"unix_nanos": at, "controller_id": "local-autotune", "mode": "ApplyMediumRisk",
                "situation": "GameCpuSchedulerPressure",
                "decision": {"decision": decision, "action_kind": kind},
                "experiment_id": id, "rollback_performed": false, "reason": "r"})
            .to_string()
        };
        fs::write(
            history.join("history.jsonl"),
            [
                line("candidate_kept", "cgroup_placement", "e1", 1),
                line("candidate_reverted", "cpu_affinity_profile", "e2", 2),
            ]
            .join("\n")
                + "\n",
        )
        .unwrap();
        dir
    }

    fn names(report: &Report, passed: bool) -> Vec<&str> {
        report
            .checks
            .iter()
            .filter(|check| check.passed == passed)
            .map(|check| check.name.as_str())
            .collect()
    }

    #[test]
    fn smoke_has_no_coverage_floor_and_full_requires_its_qualification() {
        let smoke = verify(&soak_dir(Profile::Smoke, None).0).unwrap();
        assert!(smoke.passed, "{:?}", names(&smoke, false));
        let full_dir = soak_dir(Profile::Full, None);
        let full = verify(&full_dir.0).unwrap();
        assert!(!full.passed);
        let failed = names(&full, false);
        for needed in [
            "minimum_wall_clock",
            "minimum_experiments",
            "minimum_clean_rounds",
            "minimum_clean_cycles",
            "minimum_round_kind_controller_crash",
            "minimum_operation_cgroup_placement",
        ] {
            assert!(failed.contains(&needed), "{needed}: {failed:?}");
        }
    }

    #[test]
    fn the_coverage_floor_is_only_waited_for_by_the_full_profile() {
        use crate::soak::coverage::coverage_reached;
        assert!(coverage_reached(&soak_dir(Profile::Smoke, None).0).unwrap());
        // Three rounds are nowhere near six clean cycles.
        assert!(!coverage_reached(&soak_dir(Profile::Full, None).0).unwrap());
    }

    #[test]
    fn every_zero_violation_check_is_common_to_both_profiles() {
        const COMMON: [&str; 5] = [
            "rounds_clean",
            "final_state",
            "support_matrix",
            "conflicts_and_cooldowns",
            "faults_audited",
        ];
        let violating =
            |profile| verify(&soak_dir(profile, Some("a stale kept action")).0).unwrap();
        for profile in [Profile::Smoke, Profile::Full] {
            let report = violating(profile);
            for name in COMMON {
                assert!(
                    report.checks.iter().any(|check| check.name == name),
                    "{profile:?} lacks {name}"
                );
            }
            assert!(!report.passed);
            assert!(names(&report, false).contains(&"rounds_clean"));
        }
        let smoke = verify(&soak_dir(Profile::Smoke, None).0).unwrap();
        assert!(COMMON.iter().all(|name| names(&smoke, true).contains(name)));
    }

    #[test]
    fn a_smoke_that_skipped_an_event_or_an_operation_fails() {
        let dir = soak_dir(Profile::Smoke, None);
        // Drop the only suspend/resume cycle: the short profile still requires one.
        let rounds = fs::read_to_string(dir.join("rounds.jsonl")).unwrap();
        let kept = rounds.lines().take(2).collect::<Vec<_>>().join("\n") + "\n";
        fs::write(dir.join("rounds.jsonl"), kept).unwrap();
        let report = verify(&dir.0).unwrap();
        assert!(names(&report, false).contains(&"minimum_suspend_resume"));
        // And one operation never ran.
        let history = dir.join("home/.local/state/stutter/autotune/history.jsonl");
        let only_cgroup = fs::read_to_string(&history)
            .unwrap()
            .lines()
            .filter(|line| !line.contains("cpu_affinity_profile"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(&history, only_cgroup).unwrap();
        let report = verify(&dir.0).unwrap();
        assert!(names(&report, false).contains(&"minimum_operations"));
        assert!(names(&report, false).contains(&"minimum_keep_and_revert"));
    }

    #[test]
    fn a_dirty_or_unfinished_soak_is_never_publishable() {
        assert_eq!(unpublishable(&state(Profile::Smoke)), None);
        let mut dirty = state(Profile::Smoke);
        dirty.allow_dirty = true;
        assert!(unpublishable(&dirty).is_some());
        let mut unfinished = state(Profile::Full);
        unfinished.finished = false;
        assert!(unpublishable(&unfinished).is_some());
    }

    #[test]
    fn source_binary_and_harness_mismatches_are_refused() {
        let ok = evidence_bundle::tests::environment("live_autotune_smoke");
        shared::require_clean(&ok).unwrap();
        for (field, value) in [
            ("source_dirty", json!(true)),
            ("source_matches_binary", json!(false)),
            ("source_matches_harness", json!(false)),
        ] {
            let mut environment = ok.clone();
            environment[field] = value;
            assert!(shared::require_clean(&environment).is_err(), "{field}");
        }
    }

    #[test]
    fn both_soak_suites_export_verify_and_index_exactly_the_two_graduated_operations() {
        for profile in [Profile::Smoke, Profile::Full] {
            let suite = suite_for(profile);
            let dir = soak_dir(profile, None);
            let raw = Dir::new("soak-raw");
            fs::write(
                raw.join("environment.json"),
                serde_json::to_vec(&evidence_bundle::tests::environment(suite)).unwrap(),
            )
            .unwrap();
            let report = verify(&dir.0).unwrap();
            // A short soak cannot pass the full gate; the artifact writer is exercised with the
            // smoke report for both suites, as the suite identity is all that differs.
            let report = if report.passed {
                report
            } else {
                verify(&soak_dir(Profile::Smoke, None).0).unwrap()
            };
            write_artifacts(&raw.0, &dir.0, &report, &state(profile)).unwrap();
            let out = Dir::new("soak-out");
            let bundle = out.join("bundle");
            evidence_bundle::export_bundle(
                std::path::Path::new("/nonexistent/checkout"),
                &raw.0,
                &bundle,
            )
            .unwrap();
            evidence_bundle::verify_bundle(&bundle).unwrap();
            let entry = evidence_bundle::verified_index_entry(&bundle).unwrap();
            assert_eq!(entry["suite"], suite);
            assert_eq!(
                entry["covers"],
                json!({"providers": ["cpu_affinity_profile", "cgroup_placement"]})
            );
        }
    }

    #[test]
    fn the_schedule_of_a_short_run_reaches_the_formal_event_floor() {
        // The formal smoke is only useful if its fixed schedule contains every required event
        // within the first rounds of a 60-minute budget.
        let kinds = (0..16)
            .map(|index| plan::round_for(index).kind.name())
            .collect::<Vec<_>>();
        for needed in ["controller_crash", "worker_kill", "suspend", "focus_swap"] {
            assert!(kinds.contains(&needed), "{kinds:?}");
        }
    }
}
