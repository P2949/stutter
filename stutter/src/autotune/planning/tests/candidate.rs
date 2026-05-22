use std::collections::BTreeSet;

use super::{
    super::{candidate::*, dry_run::*, executable_plan::*, profile_candidates::*, suggestion::*},
    support::*,
};
use crate::{
    actions::{ActionState, ActionWarning, SafetyClass},
    affinity::CpuMask,
    autotune::{conflicts::ActionConflictGroup, objective::ObjectiveKind},
    daemon_policy::ActionEffectScope,
    process_tree::TaskClass,
    profiles::{Profile, ProfileRule},
};

#[test]
fn generate_profile_candidates_excludes_current_profile() {
    let profiles = vec![profile("current"), profile("candidate")];

    let plan = generate_profile_candidate_plan_with_checker(
        &profiles,
        1234,
        Some("current"),
        &BTreeSet::new(),
        status_for_profile,
    );

    assert_eq!(plan.optimization_candidates.len(), 1);
    assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
    assert!(
        plan.rejected
            .iter()
            .any(|rejected| rejected.profile_name == "current"
                && rejected.reason == "current profile")
    );
}

#[test]
fn generate_profile_candidates_excludes_profiles_that_fail_dry_run() {
    let profiles = vec![profile("dry-run-fails"), profile("candidate")];

    let plan = generate_profile_candidate_plan_with_checker(
        &profiles,
        1234,
        None,
        &BTreeSet::new(),
        status_for_profile,
    );

    assert_eq!(plan.optimization_candidates.len(), 1);
    assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
    assert!(
        plan.rejected
            .iter()
            .any(|rejected| rejected.profile_name == "dry-run-fails"
                && rejected.reason.contains("dry-run failed"))
    );
}

#[test]
fn generate_profile_candidates_excludes_zero_matched_tasks() {
    let profiles = vec![profile("zero-match"), profile("candidate")];

    let plan = generate_profile_candidate_plan_with_checker(
        &profiles,
        1234,
        None,
        &BTreeSet::new(),
        status_for_profile,
    );

    assert_eq!(plan.optimization_candidates.len(), 1);
    assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
    assert!(
        plan.rejected
            .iter()
            .any(|rejected| rejected.profile_name == "zero-match"
                && rejected.reason == "zero matched tasks")
    );
}

#[test]
fn generate_profile_candidates_puts_recently_failed_names_last() {
    let profiles = vec![
        profile("recently-failed"),
        profile("fresh"),
        profile("another-fresh"),
    ];
    let recently_failed_profiles = BTreeSet::from(["recently-failed".to_owned()]);

    let plan = generate_profile_candidate_plan_with_checker(
        &profiles,
        1234,
        None,
        &recently_failed_profiles,
        status_for_profile,
    );

    let names = plan
        .optimization_candidates
        .iter()
        .map(CandidateAction::profile_name)
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["fresh", "another-fresh", "recently-failed"]);
}

#[test]
fn baseline_online_is_recovery_fallback_not_optimization_candidate() {
    let profiles = vec![profile("baseline-online"), profile("candidate")];

    let plan = generate_profile_candidate_plan_with_checker(
        &profiles,
        1234,
        None,
        &BTreeSet::new(),
        status_for_profile,
    );

    assert_eq!(plan.optimization_candidates.len(), 1);
    assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
    assert_eq!(
        plan.recovery_fallback
            .as_ref()
            .map(CandidateAction::profile_name),
        Some("baseline-online")
    );
}

#[test]
fn public_generate_profile_candidates_returns_optimization_candidates_only() {
    let profiles = vec![profile("baseline-online"), profile("candidate")];

    let plan = generate_profile_candidate_plan_with_checker(
        &profiles,
        1234,
        None,
        &BTreeSet::new(),
        status_for_profile,
    );

    assert_eq!(plan.optimization_candidates.len(), 1);
    assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
    assert!(plan.recovery_fallback.is_some());
}

#[test]
fn generate_profile_candidates_for_observation_without_target_pid_returns_no_candidates() {
    let profiles = vec![Profile {
        name: "fixture-game-helper".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::Game],
            match_comm: Vec::new(),
        }],
    }];

    let observation = crate::autotune::observation::AutotuneObservation {
        target_root_pid: None,
        active_tasks: vec![crate::autotune::observation::ActiveTaskSnapshot {
            tid: 1234,
            process_pid: 1234,
            comm: "game-main".to_owned(),
            class: TaskClass::Game,
            process_starttime_ticks: Some(10),
            task_starttime_ticks: Some(1234),
            cgroup_path: Some("/user.slice/fixture.scope".to_owned()),
        }],
        ..crate::autotune::observation::AutotuneObservation::default()
    };

    let plan = generate_profile_candidate_plan_for_observation(&profiles, &observation);

    assert!(plan.optimization_candidates.is_empty());
    assert!(plan.recovery_fallback.is_none());
    assert!(plan.rejected.is_empty());
}

#[test]
fn suggest_mode_emits_candidates_but_never_calls_apply() {
    let candidates = vec![CandidateAction::cpu_affinity_profile(
        profile("game-main-suggested"),
        1234,
    )];
    let mut runner = FakeDryRunner::default();

    let records = dry_run_candidates_with_runner(&candidates, &mut runner);
    let suggestions = suggestions_from_dry_run_records(
        &records,
        1234,
        None,
        SafetyClass::ReversibleLowRisk,
        "scheduler pressure detected on Game/WineServer classes",
    );

    assert_eq!(runner.dry_run_calls, 1);
    assert_eq!(runner.apply_calls, 0);
    assert_eq!(suggestions.len(), 1);

    let rendered = render_candidate_suggestion(&suggestions[0]);
    assert!(rendered.contains("candidate=game-main-suggested"));
    assert!(rendered.contains("note=\"suggest mode did not apply this change\""));
    assert!(rendered.contains("dry_run_command=\"stutter apply-profile --tree-pid 1234 --profile <generated-or-existing-profile> --dry-run\""));
}

#[test]
fn profile_with_zero_affected_tasks_is_rejected() {
    let record = CandidateDryRunRecord {
        candidate_name: "zero-task-profile".to_owned(),
        affected_tasks: 0,
        warnings: Vec::new(),
        safety_class: SafetyClass::ReversibleLowRisk,
        eligible: false,
        reason: Some("dry-run matched zero affected tasks".to_owned()),
    };

    let suggestion = suggestion_from_dry_run_record(
        &record,
        1234,
        None,
        SafetyClass::ReversibleLowRisk,
        "scheduler pressure detected on Game/WineServer classes",
    );

    assert!(suggestion.is_none());
    assert!(!record.eligible);
    assert_eq!(
        record.reason.as_deref(),
        Some("dry-run matched zero affected tasks")
    );
}

#[test]
fn profile_dry_run_warning_is_preserved() {
    let state = ActionState {
            applied: false,
            affected_tasks: 31,
            checked_tasks: 31,
            pending_changes: 31,
            warnings: vec![ActionWarning {
                message: "restore file already exists at /tmp/stutter-restore.json; new affinity records will be merged".to_owned(),
            }],
        };

    let record = dry_run_record_from_action_state(
        "warned-profile".to_owned(),
        SafetyClass::ReversibleLowRisk,
        state,
    );

    assert!(record.eligible);
    assert_eq!(record.affected_tasks, 31);
    assert_eq!(record.warnings.len(), 1);
    assert!(
        record.warnings[0]
            .message
            .contains("restore file already exists")
    );
}

#[test]
fn high_risk_candidates_are_blocked() {
    let record = CandidateDryRunRecord {
        candidate_name: "high-risk-profile".to_owned(),
        affected_tasks: 31,
        warnings: Vec::new(),
        safety_class: SafetyClass::HighRisk,
        eligible: true,
        reason: None,
    };

    let suggestion = suggestion_from_dry_run_record(
        &record,
        1234,
        None,
        SafetyClass::ReversibleLowRisk,
        "scheduler pressure detected on Game/WineServer classes",
    );

    assert!(suggestion.is_none());
}

#[test]
fn high_risk_candidates_are_allowed_when_policy_allows_high_risk() {
    let record = CandidateDryRunRecord {
        candidate_name: "high-risk-profile".to_owned(),
        affected_tasks: 31,
        warnings: Vec::new(),
        safety_class: SafetyClass::HighRisk,
        eligible: true,
        reason: None,
    };

    let suggestion = suggestion_from_dry_run_record(
        &record,
        1234,
        None,
        SafetyClass::HighRisk,
        "scheduler pressure detected on Game/WineServer classes",
    );

    assert!(suggestion.is_some());
    assert_eq!(suggestion.unwrap().safety, SafetyClass::HighRisk);
}

#[test]
fn candidate_helpers_return_stable_metadata() {
    let candidate = CandidateAction::cpu_affinity_profile(profile("game-main"), 1234);

    assert_eq!(candidate.candidate_name(), "game-main");
    assert_eq!(candidate.target_root_pid(), Some(1234));
    assert_eq!(candidate.action_kind(), "cpu_affinity_profile");
    assert_eq!(candidate.safety_class(), SafetyClass::ReversibleLowRisk);
    assert_eq!(
        candidate.descriptor().effect_scope,
        ActionEffectScope::LocalProcessTree
    );
    assert_eq!(
        candidate.conflict_group(),
        ActionConflictGroup::CpuPlacement
    );
}

#[test]
fn generic_candidate_variant_reports_descriptor_scope_and_objective() {
    let candidate = CandidateAction::Nice {
        plan: NiceActionPlan {
            name: "nice-root-1234-to-5".to_owned(),
            action: crate::actions::nice::NiceAction {
                targets: vec![crate::actions::TaskIdentity {
                    tid: 1234,
                    process_pid: Some(1234),
                    comm: None,
                    starttime_ticks: None,
                }],
                nice: 5,
                policy: crate::actions::nice::NicePolicy::default(),
            },
            target_root_pid: Some(1234),
            evidence: vec![CandidateEvidence::new("situation", "CompileCpuBound", 0.8)],
            objective: ObjectiveKind::DesktopInteractivity,
        },
    };

    assert_eq!(candidate.candidate_name(), "nice-root-1234-to-5");
    assert_eq!(candidate.action_kind(), "nice");
    assert_eq!(candidate.safety_class(), SafetyClass::ReversibleMediumRisk);
    assert_eq!(
        candidate.effect_scope(),
        ActionEffectScope::LocalProcessTree
    );
    assert_eq!(candidate.target_root_pid(), Some(1234));
    assert_eq!(candidate.conflict_group(), ActionConflictGroup::CpuPriority);
    assert_eq!(candidate.objective(), ObjectiveKind::DesktopInteractivity);
}

#[test]
fn fake_candidate_uses_candidate_plan_metadata() {
    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake-plan".to_owned()),
        SafetyClass::ReversibleMediumRisk,
    );

    assert_eq!(candidate.candidate_name(), "fake-profile");
    assert_eq!(candidate.action_kind(), "fake");
    assert_eq!(candidate.target_root_pid(), None);
    assert_eq!(candidate.action_id().as_str(), "fake-plan");
    assert_eq!(candidate.safety_class(), SafetyClass::ReversibleMediumRisk);
    assert_eq!(candidate.effect_scope(), ActionEffectScope::ObserveOnly);
    assert!(candidate.evidence().is_empty());
    assert_eq!(candidate.objective(), ObjectiveKind::StutterScore);
    assert_eq!(candidate.conflict_group(), ActionConflictGroup::None);
    assert_eq!(candidate.describe(), "fake action fake-plan");
    assert!(!candidate.is_high_risk_system_adjacent());
    assert!(candidate.manual_only_reason().is_none());

    let high_risk_candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake-high-risk".to_owned()),
        SafetyClass::HighRisk,
    );

    assert!(high_risk_candidate.is_high_risk_system_adjacent());
    assert_eq!(
            high_risk_candidate.manual_only_reason(),
            Some(
                "manual-only high-risk/system-adjacent candidate; autonomous apply is disabled for action_kind=fake"
                    .to_owned()
            )
        );
}

#[test]
fn apply_candidate_requires_successful_eligibility_promotion() {
    let candidate = CandidateAction::cpu_affinity_profile(profile("game-main"), 1234);

    let apply_candidate =
        try_promote_to_apply_candidate(candidate.clone(), ApplyEligibility::approved()).unwrap();
    assert_eq!(
        apply_candidate.candidate().candidate_name(),
        candidate.candidate_name()
    );

    let denied =
        try_promote_to_apply_candidate(candidate, ApplyEligibility::denied("policy denied"))
            .unwrap_err();
    assert_eq!(denied.denial_message(), "policy denied");
}

#[test]
fn profile_with_nice_or_ionice_is_medium_risk_candidate() {
    let candidate = CandidateAction::cpu_affinity_profile(
        Profile {
            name: "background-demotion".to_owned(),
            rules: vec![ProfileRule {
                affinity: None,
                nice: Some(10),
                ionice: Some(crate::actions::ioprio::IoPrioValue::idle()),
                match_class: vec![TaskClass::Indexer],
                match_comm: Vec::new(),
            }],
        },
        1234,
    );

    assert_eq!(candidate.safety_class(), SafetyClass::ReversibleMediumRisk);
}
