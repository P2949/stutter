//! Integrity downgrades over the production-composed synthetic campaign: evidence that is removed or
//! corrupted must fail verification, never fall back to weaker rules or read as an optional probe not run.

use std::os::unix::fs::PermissionsExt;

use super::baseline_campaign::{run_campaign, verify};

fn studies_dir(run: &super::baseline_campaign::Finished) -> std::path::PathBuf {
    std::fs::read_link(format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(&run.campaign)
    ))
    .unwrap()
    .join("studies")
}

#[test]
fn a_study_that_lost_its_raw_archive_fails_instead_of_replaying_from_the_game_directory() {
    let run = run_campaign();
    assert!(verify(&run).all_verified());
    // The game's own results directory is still there, which the old fallback would have trusted.
    let first = std::fs::read_dir(studies_dir(&run))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.join("raw").is_dir())
        .expect("a study with an archive");
    std::fs::remove_dir_all(first.join("raw")).unwrap();
    let verified = verify(&run);
    assert!(!verified.all_verified());
    assert!(
        verified.children.iter().any(|child| child
            .problem
            .as_deref()
            .is_some_and(|p| p.contains("RawArchiveMissing"))),
        "{:?}",
        verified
            .children
            .iter()
            .map(|c| &c.problem)
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_corrupt_telemetry_file_counts_against_the_study_and_is_not_read_as_an_unrun_probe() {
    let run = run_campaign();
    // No direct-frame probe ran in the synthetic campaign: that is the optional probe not collected.
    assert_eq!(verify(&run).telemetry_summary(), None);
    let first = std::fs::read_dir(studies_dir(&run))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.join("evidence").is_dir())
        .unwrap();
    let file = first.join("telemetry-receipts.json");
    std::fs::write(&file, b"{ not json").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let (eligible, collected) = verify(&run)
        .telemetry_summary()
        .expect("corrupt evidence is collected");
    assert_eq!(eligible, 0);
    assert!(collected > 0);
}

#[test]
fn admission_counts_the_archive_and_receipts_the_way_the_final_replay_does() {
    let run = run_campaign();
    let studies = std::fs::File::open(studies_dir(&run)).unwrap();
    let uid = crate::syscall::geteuid();
    let sealed: u64 = std::fs::read_dir(studies_dir(&run))
        .unwrap()
        .flatten()
        .map(|entry| {
            serde_json::from_slice::<crate::autotune::research::campaign_budget::Spent>(
                &std::fs::read(entry.path().join("budget.json")).unwrap(),
            )
            .unwrap()
            .evidence_bytes
        })
        .sum();
    let admitted =
        crate::autotune::research::campaign_budget::spent(&studies, uid, 1_000_000).unwrap();
    assert!(
        admitted.evidence_bytes > sealed,
        "{} vs {sealed}",
        admitted.evidence_bytes
    );
}

#[test]
fn a_spend_record_that_claims_less_active_time_than_its_attempts_witnessed_is_found() {
    let run = run_campaign();
    assert_eq!(verify(&run).budget_problem, None);
    let (path, witnessed) = std::fs::read_dir(studies_dir(&run))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter_map(|path| {
            let evidence = std::fs::File::open(path.join("evidence")).ok()?;
            let items = crate::autotune::research::report_store::read_evidence(
                &evidence,
                crate::syscall::geteuid(),
            )
            .ok()?;
            let seconds: u64 = crate::autotune::research::child_verify_rules::processes_of(&items)
                .iter()
                .map(|p| p.completed_unix.saturating_sub(p.started_unix))
                .sum();
            (seconds > 0).then_some((path, seconds))
        })
        .next()
        .expect("a study whose attempts witnessed time");
    let record = path.join("budget.json");
    let mut spent: crate::autotune::research::campaign_budget::Spent =
        serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
    spent.active_seconds = witnessed - 1;
    std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&record, serde_json::to_vec(&spent).unwrap()).unwrap();
    let problem = verify(&run).budget_problem.unwrap();
    assert!(
        problem.starts_with("BudgetActiveSecondsUnderstated"),
        "{problem}"
    );
}

#[test]
fn a_comparison_whose_analysis_was_refused_is_not_a_qualified_comparison() {
    use crate::autotune::research::{
        child_study::{Stage, StudyResult},
        report::ArmResult,
        study_executor::comparison_valid,
    };
    let arm = |verdict: Option<&str>| ArmResult {
        source: crate::autotune::research::ProtonSource::Ge,
        status: "x".into(),
        numeric_verdict: verdict.map(str::to_owned),
        pairs: 4,
        mean_effect_percent: None,
        ci_low_percent: None,
        ci_high_percent: None,
        rule_sha256: None,
    };
    assert_eq!(
        comparison_valid(Stage::Confirm, &[arm(Some("improved"))]),
        Some(true)
    );
    assert_eq!(
        comparison_valid(Stage::Holdout, &[arm(Some("equivalent")), arm(None)]),
        Some(false)
    );
    assert_eq!(comparison_valid(Stage::Confirm, &[]), Some(false));
    assert_eq!(comparison_valid(Stage::Screen, &[arm(None)]), None);
    // The production-composed campaign's confirmations all produced numeric verdicts, so they still qualify.
    let run = run_campaign();
    assert!(
        run.driver
            .dispositions()
            .iter()
            .all(|d| d.result == Some(StudyResult::Qualified))
    );
}

#[test]
fn the_resident_observation_is_read_from_each_measuring_childs_own_sealed_record() {
    let run = run_campaign();
    assert_eq!(
        verify(&run).resident_summary(),
        Err("ResidentObservationNotCollected")
    );
    // Give every measuring study the observation its attempts would have produced.
    let uid = crate::syscall::geteuid();
    for entry in std::fs::read_dir(studies_dir(&run)).unwrap().flatten() {
        let path = entry.path();
        let evidence = std::fs::File::open(path.join("evidence")).unwrap();
        let items = crate::autotune::research::report_store::read_evidence(&evidence, uid).unwrap();
        let attempts: Vec<_> = crate::autotune::research::child_verify_rules::processes_of(&items)
            .iter()
            .map(|p| {
                serde_json::json!({
                    "slot": p.slot, "pid": p.pid.as_u32(), "start_ticks": p.start_ticks, "uid": p.uid,
                    "window_start_unix": p.started_unix, "window_end_unix": p.completed_unix,
                    "renderer_bound": true, "cgroup": "0::/app.slice", "cpu_samples": 60,
                    "gpu_samples": 60, "psi_samples": 60, "lost_samples": 0,
                    "owners_identified": true, "runnable_events": 5, "runnable_lost": 0,
                    "runnable_claim": false,
                })
            })
            .collect();
        let file = path.join("resident-observation.json");
        std::fs::write(
            &file,
            serde_json::to_vec(&serde_json::json!({ "schema": 2, "attempts": attempts })).unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert_eq!(verify(&run).resident_summary(), Ok(()));
    // One corrupt record is named, never read as absent.
    let first = std::fs::read_dir(studies_dir(&run))
        .unwrap()
        .flatten()
        .next()
        .unwrap()
        .path();
    std::fs::write(first.join("resident-observation.json"), b"{ no").unwrap();
    assert!(verify(&run).resident_summary().is_err());
}
