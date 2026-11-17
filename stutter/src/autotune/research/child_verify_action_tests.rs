//! A qualified action study through the verifier's own `study_checks`: its receipts must be about
//! completed attempts the schedule treated On, and every such attempt must carry one.

use std::os::unix::fs::PermissionsExt;

use serde_json::json;

use super::*;
use crate::autotune::research::{
    child_ledger::Disposition,
    child_study::StudyResult,
    cyberpunk::{
        CyberpunkBenchmarkCompletion, CyberpunkProcessIdentity, CyberpunkScenario,
        NativeBenchmarkMetrics,
    },
    journal::AttemptSlot,
    report::AttemptEvidenceV1,
    report_store::write_evidence,
};

fn completion(slot: u32) -> CyberpunkBenchmarkCompletion {
    CyberpunkBenchmarkCompletion {
        schema_version: 1,
        app_id: 1_091_500,
        build_id: 42,
        scene: CyberpunkScenario::BuiltinBenchmark,
        settings_sha256: "c".repeat(64),
        proton_source: ProtonSource::ValveStable,
        proton_identity_sha256: "d".repeat(64),
        process: CyberpunkProcessIdentity {
            pid: stutter_core::ids::Pid::new(100 + slot),
            start_ticks: 5000 + u64::from(slot),
            uid: crate::syscall::geteuid(),
            app_id: 1_091_500,
            build_id: 42,
            executable_sha256: "b".repeat(64),
        },
        started_unix_nanos: 1,
        completed_unix_nanos: 2,
        result_directory: "result".into(),
        metrics: NativeBenchmarkMetrics {
            average_fps: 60.0,
            minimum_fps: None,
            maximum_fps: None,
        },
        context: None,
        loaded_runtime: None,
        runtime_binding: None,
        files: vec![],
    }
}

fn receipt(slot: u32) -> serde_json::Value {
    let state = json!({ "cpus_allowed": "0-15", "cgroup": "/a", "tasks": { "1": "0-15" },
                        "identity": [5, 1000] });
    json!({ "action": "CpuAffinityProfile", "transaction": format!("tx-{slot}"),
            "pid": 100 + slot, "start_ticks": 5000 + slot, "uid": crate::syscall::geteuid(),
            "original": state, "restored": state })
}

/// A decided-qualified action study over eight completed attempts, with `receipts` recorded.
fn check(receipts: &[serde_json::Value], drop_completion_of: Option<u32>) -> anyhow::Result<()> {
    let parent = parent();
    let uid = crate::syscall::geteuid();
    let study = plan(&parent)
        .unwrap()
        .into_iter()
        .find(|study| matches!(study.stage, Stage::Action(_)))
        .unwrap();
    let child = built(&parent, &study);
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join(name(&study));
    std::fs::create_dir_all(dir.join("evidence")).unwrap();
    for path in [&dir, &dir.join("evidence")] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let private = |file: &str, bytes: Vec<u8>| {
        let path = dir.join(file);
        std::fs::write(&path, bytes).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    };
    private("intent.json", serde_json::to_vec(&child).unwrap());
    private(
        "action-receipts.json",
        serde_json::to_vec(receipts).unwrap(),
    );
    let evidence = File::open(dir.join("evidence")).unwrap();
    for index in 0..study.min_attempts {
        let item = AttemptEvidenceV1 {
            schema_version: 1,
            slot_index: index,
            average_fps: 60.0,
            native_completion: (Some(index) != drop_completion_of)
                .then(|| Box::new(completion(index))),
        };
        let slot = AttemptSlot {
            study_sha256: String::new(),
            context_sha256: String::new(),
            index,
            arm: ProtonSource::ValveStable,
        };
        write_evidence(&evidence, &slot, &item).unwrap();
    }
    let dispositions = vec![Disposition {
        study_sha256: study.study_sha256.clone(),
        profile: study.profile,
        stage: study.stage,
        result: Some(StudyResult::Qualified),
        reason: String::new(),
    }];
    let studies = File::open(root.path()).unwrap();
    study_checks(
        &std::collections::BTreeMap::new(),
        &dispositions,
        &name(&study),
        &studies,
        uid,
        &parent,
    )
}

/// The slots the balanced schedule treats On, from the same function the verifier uses.
fn treated() -> Vec<u32> {
    let parent = parent();
    let study = plan(&parent)
        .unwrap()
        .into_iter()
        .find(|study| matches!(study.stage, Stage::Action(_)))
        .unwrap();
    let child = built(&parent, &study);
    crate::autotune::research::treatment::treatments_of(&child)
        .unwrap()
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(_, treatment)| **treatment == crate::autotune::research::Treatment::On)
        .map(|(slot, _)| slot as u32)
        .collect()
}

#[test]
fn a_qualified_action_study_is_verified_only_when_every_treated_attempt_was_acted_on() {
    let on = treated();
    assert_eq!(on.len(), 4, "four treated attempts in eight");
    let every: Vec<_> = on.iter().map(|slot| receipt(*slot)).collect();
    check(&every, None).unwrap();

    // One treated attempt never received the action: eight runs are not four applied pairs.
    let error = check(&every[1..], None).unwrap_err().to_string();
    assert!(
        error.contains("ActionNotAppliedOnEveryTreatedAttempt"),
        "{error}"
    );
    // A receipt on an attempt the schedule treated Off, with every On attempt also covered.
    let off = (0..8).find(|slot| !on.contains(slot)).unwrap();
    let mut extra = every.clone();
    extra.push(receipt(off));
    let error = check(&extra, None).unwrap_err().to_string();
    assert!(error.contains("ReceiptOnAnUntreatedAttempt"), "{error}");
    // A receipt for a process no completed attempt names.
    let mut stray = every.clone();
    stray.push(receipt(50));
    let error = check(&stray, None).unwrap_err().to_string();
    assert!(error.contains("ReceiptForNoCompletedAttempt"), "{error}");
    // A treated attempt whose sealed result names no process cannot be matched: not covered.
    assert!(check(&every, Some(on[0])).is_err());
}

#[test]
fn a_qualified_comparison_over_a_cold_or_unknown_cohort_is_not_comparable_but_a_screen_may_be_cold()
{
    use crate::autotune::research::child_study::Stage;
    let parent = parent();
    let uid = crate::syscall::geteuid();
    let plan = plan(&parent).unwrap();
    let verdict = |stage_is: fn(&Stage) -> bool, cohort: Option<&str>| {
        let study = plan.iter().find(|s| stage_is(&s.stage)).unwrap();
        let mut child = built(&parent, study);
        child.prefix_cohort = cohort.map(str::to_owned);
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(name(study));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let file = dir.join("intent.json");
        std::fs::write(&file, serde_json::to_vec(&child).unwrap()).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let dispositions = vec![Disposition {
            study_sha256: study.study_sha256.clone(),
            profile: study.profile,
            stage: study.stage,
            result: Some(StudyResult::Qualified),
            reason: String::new(),
        }];
        let rules = std::collections::BTreeMap::new();
        let studies = File::open(root.path()).unwrap();
        study_checks(&rules, &dispositions, &name(study), &studies, uid, &parent)
            .map_err(|error| error.to_string())
    };
    // The exploratory screen may run cold.
    assert!(verdict(|s| matches!(s, Stage::Screen), Some("cold")).is_ok());
    // A pilot is a comparison: warm only, and an unrecorded cohort is not assumed warm.
    assert!(verdict(|s| matches!(s, Stage::Pilot), Some("warm")).is_ok());
    for cohort in [Some("cold"), None] {
        let error = verdict(|s| matches!(s, Stage::Pilot), cohort).unwrap_err();
        assert!(
            error.contains("ColdOrUnknownCohortNotComparable"),
            "{error}"
        );
    }
}
