use super::*;
use crate::autotune::research::{
    ResearchAction, ResearchFactor, ResearchTool,
    child_intent::child_intent,
    child_study::{ChildStudy, Stage, plan},
    pilot_rule::freeze,
    tests::intent,
};

const SIX: [ProtonSource; 6] = [
    ProtonSource::ValveStable,
    ProtonSource::ValveExperimental,
    ProtonSource::CachyOs,
    ProtonSource::CachyOsV3,
    ProtonSource::Ge,
    ProtonSource::Em,
];

fn parent() -> ResearchIntentV1 {
    let mut intent = intent();
    intent.tools = SIX
        .iter()
        .enumerate()
        .map(|(index, source)| ResearchTool {
            source: *source,
            identity_sha256: format!("{index:x}").repeat(64)[..64].to_owned(),
        })
        .collect();
    intent.actions = vec![ResearchAction::CpuAffinityProfile];
    intent.factors = vec![ResearchFactor::NtSync];
    intent.max_attempts = 120;
    intent
}

fn study_of(parent: &ResearchIntentV1, stage: fn(&Stage) -> bool) -> ChildStudy {
    plan(parent)
        .unwrap()
        .into_iter()
        .find(|study| stage(&study.stage))
        .unwrap()
}

fn built(parent: &ResearchIntentV1, study: &ChildStudy) -> ResearchIntentV1 {
    use crate::autotune::research::child_study::StudyArms;
    let arms: Vec<_> = match study.arms() {
        StudyArms::All => parent.tools.iter().map(|tool| tool.source).collect(),
        StudyArms::Pair => vec![ProtonSource::ValveStable, ProtonSource::Ge],
        StudyArms::Single => vec![ProtonSource::ValveStable],
    };
    let rule = matches!(study.stage, Stage::Confirm | Stage::Holdout)
        .then(|| freeze(&[70.0, 73.0, 67.0]).unwrap());
    let mut child = child_intent(parent, study, &arms, rule.as_ref()).unwrap();
    child.prefix_cohort = Some("warm".into());
    child
}

fn name(study: &ChildStudy) -> String {
    study.study_sha256[..16].to_owned()
}

#[test]
fn every_stage_of_a_real_shaped_plan_is_derived_from_its_parent() {
    let parent = parent();
    for study in plan(&parent).unwrap() {
        let child = built(&parent, &study);
        assert!(
            derived_from(&child, &parent, &name(&study)),
            "{:?} {:?}",
            study.profile,
            study.stage
        );
    }
}

#[test]
fn a_confirmation_without_its_rule_or_another_stage_with_one_is_refused() {
    let parent = parent();
    let confirm = study_of(&parent, |stage| matches!(stage, Stage::Confirm));
    let mut child = built(&parent, &confirm);
    child.confirmation_rule = None;
    assert!(!derived_from(&child, &parent, &name(&confirm)));
    let pilot = study_of(&parent, |stage| matches!(stage, Stage::Pilot));
    let mut child = built(&parent, &pilot);
    child.confirmation_rule = Some(freeze(&[70.0, 73.0, 67.0]).unwrap());
    assert!(!derived_from(&child, &parent, &name(&pilot)));
}

#[test]
fn an_inconsistent_rule_is_refused() {
    let parent = parent();
    let confirm = study_of(&parent, |stage| matches!(stage, Stage::Confirm));
    let mut child = built(&parent, &confirm);
    if let Some(rule) = child.confirmation_rule.as_mut() {
        rule.min_effect_bp += 1;
    }
    assert!(!derived_from(&child, &parent, &name(&confirm)));
}

#[test]
fn only_the_exact_planned_digest_binds_a_child() {
    let parent = parent();
    let pilot = study_of(&parent, |stage| matches!(stage, Stage::Pilot));
    let mut child = built(&parent, &pilot);
    // Same first sixteen characters, different remainder: a forged binding.
    let forged = format!("{}{}", &pilot.study_sha256[..16], "0".repeat(48));
    child.child_study_sha256 = Some(forged);
    assert!(!derived_from(&child, &parent, &name(&pilot)));
    // A directory name that names no planned study is refused.
    assert!(!derived_from(
        &built(&parent, &pilot),
        &parent,
        "0123456789abcdef"
    ));
}

#[test]
fn the_budget_and_arm_count_must_be_the_stage_s_own() {
    let parent = parent();
    let pilot = study_of(&parent, |stage| matches!(stage, Stage::Pilot));
    let mut more = built(&parent, &pilot);
    more.max_attempts += 2;
    assert!(!derived_from(&more, &parent, &name(&pilot)));
    let mut three = built(&parent, &pilot);
    three.tools = parent.tools[..3].to_vec();
    assert!(!derived_from(&three, &parent, &name(&pilot)));
    let screen = study_of(&parent, |stage| matches!(stage, Stage::Screen));
    let mut partial = built(&parent, &screen);
    partial.tools.pop();
    assert!(!derived_from(&partial, &parent, &name(&screen)));
}

#[test]
fn a_study_the_ledger_decided_after_running_it_must_have_its_directory() {
    use std::os::unix::fs::PermissionsExt;

    use crate::autotune::research::{child_ledger::open_for_campaign, child_study::StudyResult};
    let parent = parent();
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let campaign = File::open(root.path()).unwrap();
    let uid = crate::syscall::geteuid();
    let mut ledger = open_for_campaign(&campaign, uid, &parent).unwrap();
    let plan = plan(&parent).unwrap();
    ledger
        .record_run(&plan[0], StudyResult::Qualified, "", 6)
        .unwrap();
    // A study decided without running it needs no directory.
    ledger
        .record(&plan[1], StudyResult::Blocked, "PredecessorNotQualified")
        .unwrap();
    let result = verify_children(&campaign, uid, &parent, &|_| "/nonexistent".into()).unwrap();
    assert_eq!(result.missing, vec![plan[0].study_sha256[..16].to_owned()]);
    assert!(!result.all_verified());
}

mod pilot_rule_traceability {
    use std::{
        collections::BTreeMap,
        os::{fd::AsFd, unix::fs::PermissionsExt},
    };

    use super::*;
    use crate::autotune::research::pilot_rule::{freeze, rule_from_runs};

    fn studies_with_empty_pilot(name: &str) -> (tempfile::TempDir, File) {
        let root = tempfile::tempdir().unwrap();
        let evidence = root.path().join(name).join("evidence");
        std::fs::create_dir_all(&evidence).unwrap();
        for dir in [root.path().join(name), evidence] {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let handle = File::open(root.path()).unwrap();
        (root, handle)
    }

    #[test]
    fn a_recorded_rule_without_runs_that_freeze_it_is_refused() {
        let (_root, studies) = studies_with_empty_pilot("0123456789abcdef");
        let rule = freeze(&[70.0, 73.0, 67.0]).unwrap();
        let rules = BTreeMap::from([(format!("0123456789abcdef{}", "0".repeat(48)), rule)]);
        let uid = crate::syscall::geteuid();
        let error = rule_matches_pilot(&rules, "0123456789abcdef", studies.as_fd(), uid)
            .unwrap_err()
            .to_string();
        assert!(error.contains("do not support a rule"), "{error}");
        // A pilot the ledger froze no rule for has nothing to match.
        assert!(
            rule_matches_pilot(&BTreeMap::new(), "0123456789abcdef", studies.as_fd(), uid).is_ok()
        );
    }

    #[test]
    fn the_rule_follows_the_noisier_arm_and_needs_exactly_two_arms() {
        let steady = [
            (ProtonSource::Ge, 70.0),
            (ProtonSource::Ge, 70.5),
            (ProtonSource::Ge, 69.5),
        ];
        let noisy = [
            (ProtonSource::CachyOs, 60.0),
            (ProtonSource::CachyOs, 66.0),
            (ProtonSource::CachyOs, 63.0),
        ];
        let both: Vec<_> = steady.iter().chain(noisy.iter()).copied().collect();
        let rule = rule_from_runs(&both).unwrap();
        assert_eq!(
            rule,
            rule_from_runs(
                &noisy
                    .iter()
                    .chain(steady.iter())
                    .copied()
                    .collect::<Vec<_>>()
            )
            .unwrap()
        );
        assert_eq!(rule_from_runs(&steady), Err("PilotArms"));
        assert_eq!(rule, freeze(&[60.0, 66.0, 63.0]).unwrap());
    }
}

mod receipts_and_rules {
    use std::os::unix::fs::PermissionsExt;

    use serde_json::json;

    use super::*;
    use crate::autotune::research::{
        child_ledger::ChildLedger, pilot_rule::FrozenConfirmationRule,
    };

    fn receipt() -> serde_json::Value {
        json!([{
            "transaction": "exp-1", "action": "CpuAffinityProfile",
            "pid": 4242, "start_ticks": 99, "uid": 1000,
            "original": {"cpus_allowed": "0-15", "cgroup": "0::/a",
                         "tasks": {"4242": "0-15", "4243": "0-15"}, "identity": [99, 1000]},
            "restored": {"cpus_allowed": "0-15", "cgroup": "0::/a",
                         "tasks": {"4242": "0-15"}, "identity": [99, 1000]}
        }])
    }

    #[test]
    fn only_a_complete_attributable_restoration_receipt_is_valid() {
        let action = ResearchAction::CpuAffinityProfile;
        assert!(receipts_valid(&receipt(), action, 1000).is_ok());
        let bad = |edit: &dyn Fn(&mut serde_json::Value)| {
            let mut value = receipt();
            edit(&mut value[0]);
            receipts_valid(&value, action, 1000).is_err()
        };
        assert!(
            receipts_valid(&json!([]), action, 1000).is_err(),
            "no receipts"
        );
        assert!(receipts_valid(&receipt(), ResearchAction::CgroupPlacement, 1000).is_err());
        assert!(
            receipts_valid(&receipt(), action, 1001).is_err(),
            "another owner"
        );
        assert!(bad(&|r| r["transaction"] = json!(null)), "no experiment");
        assert!(bad(&|r| r["pid"] = json!(0)));
        assert!(bad(&|r| r["restored"]["cgroup"] = json!("0::/b")));
        assert!(
            bad(&|r| r["restored"]["identity"] = json!([100, 1000])),
            "pid reuse"
        );
        assert!(bad(&|r| r["original"]["identity"] = json!(null)));
        assert!(
            bad(&|r| r["restored"]["tasks"]["4242"] = json!("2-3")),
            "a pinned thread"
        );
    }

    fn studies_with_child(name: &str, child: &ResearchIntentV1) -> (tempfile::TempDir, File) {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let file = dir.join("intent.json");
        std::fs::write(&file, serde_json::to_vec(child).unwrap()).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let handle = File::open(root.path()).unwrap();
        (root, handle)
    }

    #[test]
    fn a_confirmation_must_run_under_exactly_its_pilots_frozen_rule() {
        let parent = parent();
        let plan = plan(&parent).unwrap();
        let confirm = plan
            .iter()
            .find(|s| matches!(s.stage, Stage::Confirm))
            .unwrap();
        let pilot_digest = confirm.depends_on[0].clone();
        let frozen = freeze(&[70.0, 73.0, 67.0]).unwrap();
        let rules: std::collections::BTreeMap<String, FrozenConfirmationRule> =
            [(pilot_digest, frozen.clone())].into();
        let dispositions = ChildLedger::undecided(plan.clone());
        let name = confirm.study_sha256[..16].to_owned();
        let uid = crate::syscall::geteuid();

        let ok = built(&parent, confirm); // freezes the same 70/73/67 rule
        let (_keep, studies) = studies_with_child(&name, &ok);
        assert!(study_checks(&rules, &dispositions, &name, &studies, uid, &parent).is_ok());

        // Internally consistent, but not the pilot's: refused.
        let mut other = ok.clone();
        other.confirmation_rule = Some(freeze(&[69.0, 70.0, 71.0]).unwrap());
        let (_keep2, studies2) = studies_with_child(&name, &other);
        let error = study_checks(&rules, &dispositions, &name, &studies2, uid, &parent)
            .unwrap_err()
            .to_string();
        assert!(error.contains("frozen rule"), "{error}");

        // No pilot froze a rule at all: refused.
        let none = std::collections::BTreeMap::new();
        assert!(study_checks(&none, &dispositions, &name, &studies, uid, &parent).is_err());
    }
}

mod telemetry {
    use serde_json::json;

    use super::*;

    fn child(completed: &[u32], receipts: Option<serde_json::Value>) -> ChildVerification {
        ChildVerification {
            study: "0123456789abcdef".into(),
            reconstruction: true,
            raw_files_verified: true,
            runtime_matched: true,
            problem: None,
            report: Some(json!({ "attempts_completed": completed.len() })),
            action_receipts: None,
            prefix_cohort: None,
            replication: None,
            telemetry_receipts: receipts,
            completed_attempts: completed.to_vec(),
            completed_processes: completed
                .iter()
                .map(|slot| AttemptProcess {
                    slot: *slot,
                    pid: stutter_core::ids::Pid::new(100 + slot),
                    start_ticks: 5000 + u64::from(*slot),
                    uid: 1000,
                    started_unix: 10_000,
                    completed_unix: 10_100,
                })
                .collect(),
        }
    }

    fn eligible(attempt: u32) -> serde_json::Value {
        json!({ "attempt": attempt, "pid": 100 + attempt, "start_ticks": 5000 + attempt, "uid": 1000,
                "frame_verdict": "Eligible", "source_bound_to_game": true,
                "frame_seconds": [[10_050, 3_000]], "frame_seconds_truncated": false })
    }

    fn summary(children: Vec<ChildVerification>) -> Option<(u32, u32)> {
        ChildrenVerification {
            children,
            missing: Vec::new(),
        }
        .telemetry_summary()
    }

    #[test]
    fn each_completed_attempt_needs_exactly_one_eligible_game_bound_receipt_that_names_it() {
        // Nothing completed: no claim either way.
        assert_eq!(summary(vec![child(&[], None)]), None);
        // Completed attempts with no receipts are not eligible, never vacuously fine.
        assert_eq!(summary(vec![child(&[0, 1], None)]), Some((0, 2)));
        // One receipt per attempt, each naming its own attempt.
        let both = json!([eligible(0), eligible(1)]);
        assert_eq!(summary(vec![child(&[0, 1], Some(both))]), Some((2, 2)));
    }

    #[test]
    fn a_receipt_cannot_stand_in_for_another_attempt_or_be_counted_twice() {
        // A duplicate of attempt 0 covers nothing for attempt 1.
        let duplicated = json!([eligible(0), eligible(0)]);
        assert_eq!(
            summary(vec![child(&[0, 1], Some(duplicated))]),
            Some((0, 2))
        );
        // Two receipts for one attempt conflict: neither counts.
        let conflicting = json!([
            eligible(1),
            { "attempt": 1, "frame_verdict": "Lossy", "source_bound_to_game": true }
        ]);
        assert_eq!(summary(vec![child(&[1], Some(conflicting))]), Some((0, 1)));
        // A receipt for an attempt that did not complete, or one naming no attempt, counts for nothing.
        let stray =
            json!([eligible(7), { "frame_verdict": "Eligible", "source_bound_to_game": true }]);
        assert_eq!(summary(vec![child(&[0], Some(stray))]), Some((0, 1)));
        // An ineligible or unbound receipt does not count.
        let weak = json!([
            { "attempt": 0, "frame_verdict": "Lossy", "source_bound_to_game": true },
            { "attempt": 1, "frame_verdict": "Eligible", "source_bound_to_game": false }
        ]);
        assert_eq!(summary(vec![child(&[0, 1], Some(weak))]), Some((0, 2)));
        // Children add up.
        assert_eq!(
            summary(vec![
                child(&[0], Some(json!([eligible(0)]))),
                child(&[0, 1], None)
            ]),
            Some((1, 3))
        );
    }

    #[test]
    fn only_frames_inside_the_native_benchmark_window_count_toward_the_floor() {
        let at = |buckets: serde_json::Value, truncated: bool| {
            let mut receipt = eligible(0);
            receipt["frame_seconds"] = buckets;
            receipt["frame_seconds_truncated"] = json!(truncated);
            summary(vec![child(&[0], Some(json!([receipt])))])
        };
        // Enough frames inside the window (10 000 to 10 100).
        assert_eq!(at(json!([[10_050, 3_000]]), false), Some((1, 1)));
        // Frames split across the window edges still count while inside; those outside do not.
        assert_eq!(
            at(json!([[10_000, 1_200], [10_100, 1_200]]), false),
            Some((1, 1))
        );
        // Menu and loading-screen frames before the benchmark began, or after it ended, are not a
        // measurement however many there were.
        assert_eq!(
            at(json!([[9_000, 50_000], [10_050, 100]]), false),
            Some((0, 1))
        );
        assert_eq!(at(json!([[10_101, 50_000]]), false), Some((0, 1)));
        // A receipt that kept no buckets, or lost some, cannot show a window count.
        assert_eq!(at(json!([]), false), Some((0, 1)));
        assert_eq!(at(json!([[10_050, 3_000]]), true), Some((0, 1)));
        let mut legacy = eligible(0);
        legacy.as_object_mut().unwrap().remove("frame_seconds");
        legacy
            .as_object_mut()
            .unwrap()
            .remove("frame_seconds_truncated");
        assert_eq!(
            summary(vec![child(&[0], Some(json!([legacy])))]),
            Some((0, 1))
        );
    }

    #[test]
    fn a_receipt_must_be_about_the_process_the_sealed_native_completion_names() {
        // Right attempt number, wrong game process: another process's receipt cannot stand in.
        for (key, wrong) in [("pid", 999u64), ("start_ticks", 1), ("uid", 0)] {
            let mut receipt = eligible(0);
            receipt[key] = json!(wrong);
            assert_eq!(
                summary(vec![child(&[0], Some(json!([receipt])))]),
                Some((0, 1)),
                "{key}"
            );
        }
        // The matching process counts.
        assert_eq!(
            summary(vec![child(&[0], Some(json!([eligible(0)])))]),
            Some((1, 1))
        );
        // A completed attempt with no recorded native process has nothing to be matched to.
        let mut unrecorded = child(&[0], Some(json!([eligible(0)])));
        unrecorded.completed_processes.clear();
        assert_eq!(summary(vec![unrecorded]), Some((0, 1)));
        // A receipt that names no process at all does not count.
        let anonymous = json!([{ "attempt": 0, "frame_verdict": "Eligible",
                                  "source_bound_to_game": true }]);
        assert_eq!(summary(vec![child(&[0], Some(anonymous))]), Some((0, 1)));
    }
}

mod restoration {
    use serde_json::json;

    use super::*;
    use crate::autotune::research::{
        ResearchAction,
        child_ledger::Disposition,
        child_study::{Profile, StudyResult},
    };

    fn planned(result: Option<StudyResult>) -> Vec<Disposition> {
        vec![
            Disposition {
                study_sha256: "ab".repeat(32),
                profile: Profile::Long,
                stage: Stage::Action(ResearchAction::CpuAffinityProfile),
                result,
                reason: String::new(),
            },
            Disposition {
                study_sha256: "cd".repeat(32),
                profile: Profile::Cpu,
                stage: Stage::Screen,
                result: Some(StudyResult::Qualified),
                reason: String::new(),
            },
        ]
    }

    fn child(reconstruction: bool, receipts: bool) -> ChildVerification {
        ChildVerification {
            study: "abababababababab".into(),
            reconstruction,
            raw_files_verified: true,
            runtime_matched: true,
            problem: None,
            report: None,
            action_receipts: receipts.then(|| json!([{}])),
            prefix_cohort: None,
            replication: None,
            telemetry_receipts: None,
            completed_attempts: Vec::new(),
            completed_processes: Vec::new(),
        }
    }

    fn restored(children: Vec<ChildVerification>, planned: &[Disposition]) -> bool {
        ChildrenVerification {
            children,
            missing: Vec::new(),
        }
        .actions_restored(planned)
    }

    #[test]
    fn restoration_needs_every_planned_action_qualified_with_a_verified_child_that_has_receipts() {
        let qualified = planned(Some(StudyResult::Qualified));
        assert!(restored(vec![child(true, true)], &qualified));
        // Nothing is restored vacuously: no action study in the plan proves nothing.
        assert!(!restored(vec![child(true, true)], &[]));
        // An action that did not qualify, a child that failed verification, or one without
        // receipts is not a restoration.
        assert!(!restored(
            vec![child(true, true)],
            &planned(Some(StudyResult::Unqualified))
        ));
        assert!(!restored(vec![child(true, true)], &planned(None)));
        assert!(!restored(vec![child(false, true)], &qualified));
        assert!(!restored(vec![child(true, false)], &qualified));
        assert!(!restored(Vec::new(), &qualified));
    }

    #[test]
    fn an_unrestored_settings_edit_leaves_a_trace_that_blocks_restoration() {
        use crate::autotune::research::profile_applier::pending_edit;
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("UserSettings.json");
        assert!(!pending_edit(&settings));
        std::fs::write(dir.path().join("UserSettings.json.stutter-applied"), b"x").unwrap();
        assert!(
            pending_edit(&settings),
            "the applied marker alone is a trace"
        );
        std::fs::remove_file(dir.path().join("UserSettings.json.stutter-applied")).unwrap();
        std::fs::write(dir.path().join("UserSettings.json.stutter-original"), b"{}").unwrap();
        assert!(pending_edit(&settings), "the snapshot alone is a trace");
    }
}

#[path = "child_verify_action_tests.rs"]
mod action_attribution;
