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
    child_intent(parent, study, &arms, rule.as_ref()).unwrap()
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

    fn child(done: u64, receipts: Option<serde_json::Value>) -> ChildVerification {
        ChildVerification {
            study: "0123456789abcdef".into(),
            reconstruction: true,
            raw_files_verified: true,
            runtime_matched: true,
            problem: None,
            report: Some(json!({ "attempts_completed": done })),
            action_receipts: None,
            telemetry_receipts: receipts,
        }
    }

    fn eligible() -> serde_json::Value {
        json!({ "frame_verdict": "Eligible", "source_bound_to_game": true })
    }

    #[test]
    fn only_eligible_game_bound_receipts_count_against_the_completed_attempts() {
        let verification = |children| ChildrenVerification {
            children,
            missing: Vec::new(),
        };
        // Nothing completed: no claim either way.
        assert_eq!(verification(vec![child(0, None)]).telemetry_summary(), None);
        // Completed attempts with no receipts count as not eligible, never as vacuously fine.
        assert_eq!(
            verification(vec![child(4, None)]).telemetry_summary(),
            Some((0, 4))
        );
        let two = json!([eligible(), eligible()]);
        assert_eq!(
            verification(vec![child(2, Some(two))]).telemetry_summary(),
            Some((2, 2))
        );
        // A receipt that is not eligible, or whose source was not the game, does not count.
        let mixed = json!([
            eligible(),
            { "frame_verdict": "Lossy", "source_bound_to_game": true },
            { "frame_verdict": "Eligible", "source_bound_to_game": false }
        ]);
        assert_eq!(
            verification(vec![child(3, Some(mixed))]).telemetry_summary(),
            Some((1, 3))
        );
        // More receipts than attempts cannot inflate the count past the attempts.
        let many = json!([eligible(), eligible(), eligible()]);
        assert_eq!(
            verification(vec![child(2, Some(many))]).telemetry_summary(),
            Some((2, 2))
        );
        // Children add up.
        let one = json!([eligible()]);
        assert_eq!(
            verification(vec![child(1, Some(one)), child(3, None)]).telemetry_summary(),
            Some((1, 4))
        );
    }
}
