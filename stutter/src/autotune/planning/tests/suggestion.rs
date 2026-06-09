use std::{fs, path::Path};

use super::{
    super::{candidate::*, dry_run::*, executable_plan::*, plan_io::*, suggestion::*},
    support::*,
};
use crate::{
    actions::{
        SafetyClass, TaskIdentity,
        irq_affinity::{IrqAffinityAction, IrqAffinityEvidence, IrqAffinityRisk},
        nice::{NiceAction, NicePolicy},
    },
    affinity::CpuMask,
    autotune::objective::ObjectiveKind,
    daemon_policy::{ActionDescriptor, ActionEffectScope, DaemonMode, RollbackRequirement},
    process_tree::TaskClass,
    profiles::{Profile, ProfileRule},
};

#[test]
fn suggestion_from_dry_run_record_renders_requested_shape() {
    let record = CandidateDryRunRecord {
        candidate_name: "game-main-suggested".to_owned(),
        affected_tasks: 31,
        warnings: Vec::new(),
        safety_class: SafetyClass::ReversibleLowRisk,
        eligible: true,
        reason: None,
    };

    let suggestion = suggestion_from_dry_run_record(
        &record,
        1234,
        None,
        SafetyClass::ReversibleLowRisk,
        "scheduler pressure detected on Game/WineServer classes",
    )
    .unwrap();

    let rendered = render_candidate_suggestion(&suggestion);

    assert!(rendered.contains("candidate=game-main-suggested"));
    assert!(rendered.contains("action=cpu-affinity-profile"));
    assert!(rendered.contains("affected_tasks=31"));
    assert!(rendered.contains("safety=ReversibleLowRisk"));
    assert!(rendered.contains("reason=\"scheduler pressure detected on Game/WineServer classes\""));
    assert!(rendered.contains("note=\"suggest mode did not apply this change\""));
    assert!(rendered.contains("required_mode=apply-low-risk"));
    assert!(rendered.contains("dry_run_command=\"stutter apply-profile --tree-pid 1234 --profile <generated-or-existing-profile> --dry-run\""));
    assert!(rendered.contains("manual_apply_command=\"stutter apply-profile --tree-pid 1234 --profile <generated-or-existing-profile>\""));
}

#[test]
fn generic_candidate_suggestion_writes_plan_file_and_uses_apply_candidate_command() {
    let plan_dir = temp_candidate_plan_dir("generic-nice");
    let candidate = CandidateAction::Nice {
        plan: NiceActionPlan {
            name: "nice-browser-helper".to_owned(),
            action: NiceAction {
                targets: vec![TaskIdentity {
                    tid: (1234).into(),
                    process_pid: Some((1234).into()),
                    comm: Some("browser".to_owned()),
                    starttime_ticks: Some(77),
                }],
                nice: 5,
                policy: NicePolicy::default(),
            },
            target_root_pid: Some(1234),
            evidence: vec![CandidateEvidence::new("cpu_pressure", "high", 0.9)],
            objective: ObjectiveKind::DesktopInteractivity,
        },
    };
    let records = vec![CandidateDryRunRecord {
        candidate_name: candidate.candidate_name().to_owned(),
        affected_tasks: 1,
        warnings: Vec::new(),
        safety_class: candidate.safety_class(),
        eligible: true,
        reason: None,
    }];

    let suggestions = suggestions_from_candidates_and_dry_run_records(
        std::slice::from_ref(&candidate),
        &records,
        &plan_dir,
        None,
        SafetyClass::ReversibleMediumRisk,
        "compile CPU pressure",
    )
    .unwrap();

    assert_eq!(suggestions.len(), 1);
    let suggestion = &suggestions[0];
    let plan_path = candidate_plan_path(&candidate, &plan_dir);

    assert!(plan_path.exists());
    assert_eq!(suggestion.candidate_name, "nice-browser-helper");
    assert_eq!(suggestion.action_kind, "nice");
    assert_eq!(suggestion.objective, ObjectiveKind::DesktopInteractivity);
    assert_eq!(suggestion.evidence.len(), 1);
    assert_eq!(suggestion.required_mode, DaemonMode::ApplyMediumRisk);
    assert_eq!(
        suggestion.required_safety_class,
        SafetyClass::ReversibleMediumRisk
    );
    assert_eq!(
        suggestion.dry_run_command.as_deref(),
        Some(format!(
            "stutter autotune apply-candidate --candidate-json {} --dry-run",
            plan_path.display()
        ))
        .as_deref()
    );
    assert_eq!(
        suggestion.manual_apply_command.as_deref(),
        Some(format!(
            "stutter autotune apply-candidate --candidate-json {}",
            plan_path.display()
        ))
        .as_deref()
    );

    let decoded: CandidatePlanFile =
        serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
    assert_eq!(decoded.candidate.candidate_name, "nice-browser-helper");
    assert_eq!(decoded.candidate.action_kind, "nice");
    assert!(decoded.executable.is_some());
    assert_eq!(
        decoded.policy_intent,
        crate::daemon_policy::PolicyIntent::Apply
    );
    assert!(decoded.policy_explanation.verdict.is_allowed());
    assert_eq!(
        decoded.apply_command.as_deref(),
        Some(format!(
            "stutter autotune apply-candidate --candidate-json {}",
            plan_path.display()
        ))
        .as_deref()
    );
    assert!(decoded.dry_run_command.ends_with("--dry-run"));
    assert!(decoded.rollback_command.contains("emergency-restore"));

    let rendered = render_candidate_suggestion(suggestion);
    assert!(rendered.contains("action=nice"));
    assert!(rendered.contains("action_kind=nice"));
    assert!(rendered.contains("dry_run_command=\"stutter autotune apply-candidate"));
    assert!(rendered.contains("manual_apply_command=\"stutter autotune apply-candidate"));
}

#[test]
fn high_risk_system_candidate_suggestion_is_dry_run_only() {
    let plan_dir = temp_candidate_plan_dir("high-risk-irq");
    let candidate = CandidateAction::IrqAffinity {
        plan: IrqAffinityActionPlan {
            name: "irq-affinity-44-high-risk".to_owned(),
            action: IrqAffinityAction::new(
                44,
                "gpu".to_owned(),
                "2".to_owned(),
                IrqAffinityRisk::HighRisk,
                IrqAffinityEvidence {
                    strong_irq_evidence: true,
                    stable_irq_identity: false,
                    known_device_mapping: true,
                    observed_irq: Some(44),
                    observed_device_hint: Some("gpu".to_owned()),
                    reason: "test IRQ pressure".to_owned(),
                },
            ),
            evidence: vec![CandidateEvidence::new("irq", "gpu", 0.8)],
            objective: ObjectiveKind::IrqOverlapReduction,
        },
    };
    let records = vec![CandidateDryRunRecord {
        candidate_name: candidate.candidate_name().to_owned(),
        affected_tasks: 1,
        warnings: Vec::new(),
        safety_class: candidate.safety_class(),
        eligible: true,
        reason: None,
    }];

    let suggestions = suggestions_from_candidates_and_dry_run_records(
        std::slice::from_ref(&candidate),
        &records,
        &plan_dir,
        None,
        SafetyClass::HighRisk,
        "IRQ overlap detected",
    )
    .unwrap();

    assert_eq!(suggestions.len(), 1);
    let suggestion = &suggestions[0];
    let plan_path = candidate_plan_path(&candidate, &plan_dir);

    assert!(plan_path.exists());
    assert_eq!(suggestion.action_kind, "irq_affinity");
    assert_eq!(suggestion.required_mode, DaemonMode::ApplyHighRisk);
    assert_eq!(suggestion.required_safety_class, SafetyClass::HighRisk);
    assert!(suggestion.dry_run_command.is_some());
    assert_eq!(suggestion.manual_apply_command, None);
    assert!(
        suggestion
            .manual_only_reason
            .as_deref()
            .unwrap_or_default()
            .contains("manual-only high-risk/system-adjacent")
    );

    let raw_plan: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&plan_path).unwrap()).unwrap();
    assert!(raw_plan.get("apply_command").is_none());
    assert!(
        raw_plan["policy_explanation"]["final_reason"]
            .as_str()
            .unwrap_or_default()
            .contains("high-risk apply")
            || raw_plan["policy_explanation"]["final_reason"]
                .as_str()
                .unwrap_or_default()
                .contains("system-wide action")
    );

    let dry_run_plan = apply_candidate_plan_file(&plan_path, true).unwrap();
    assert_eq!(
        dry_run_plan.candidate.candidate_name,
        "irq-affinity-44-high-risk"
    );
    assert!(dry_run_plan.executable.is_none());
    assert_eq!(dry_run_plan.apply_command, None);

    let err = apply_candidate_plan_file(&plan_path, false)
        .unwrap_err()
        .to_string();
    assert!(err.contains("manual_only_high_risk"));
}

#[test]
fn cpu_affinity_suggestion_preserves_apply_profile_and_does_not_write_plan_file() {
    let plan_dir = temp_candidate_plan_dir("cpu-affinity-preserve-apply-profile");
    let profile = Profile {
        name: "game".to_owned(),
        rules: Vec::new(),
    };
    let candidate = CandidateAction::CpuAffinityProfile {
        plan: CpuAffinityProfilePlan {
            profile_name: "game".to_owned(),
            profile,
            tree_pid: 1234,
        },
    };
    let records = vec![CandidateDryRunRecord {
        candidate_name: candidate.candidate_name().to_owned(),
        affected_tasks: 1,
        warnings: Vec::new(),
        safety_class: candidate.safety_class(),
        eligible: true,
        reason: None,
    }];
    let profile_path = Path::new("/tmp/profile.toml");

    let suggestions = suggestions_from_candidates_and_dry_run_records(
        std::slice::from_ref(&candidate),
        &records,
        &plan_dir,
        Some(profile_path),
        SafetyClass::ReversibleMediumRisk,
        "scheduler pressure",
    )
    .unwrap();

    assert_eq!(suggestions.len(), 1);
    let suggestion = &suggestions[0];
    assert_eq!(suggestion.action_kind, "cpu_affinity_profile");
    assert_eq!(
        suggestion.dry_run_command.as_deref(),
        Some("stutter apply-profile --tree-pid 1234 --profile /tmp/profile.toml --dry-run")
    );
    assert_eq!(
        suggestion.manual_apply_command.as_deref(),
        Some("stutter apply-profile --tree-pid 1234 --profile /tmp/profile.toml")
    );
    assert!(!candidate_plan_path(&candidate, &plan_dir).exists());
}

#[test]
fn candidate_plan_file_can_embed_executable_process_local_payload() {
    let candidate = CandidateAction::Nice {
        plan: NiceActionPlan {
            name: "nice-browser-helper".to_owned(),
            action: NiceAction {
                targets: vec![TaskIdentity {
                    tid: (1234).into(),
                    process_pid: Some((1234).into()),
                    comm: Some("browser".to_owned()),
                    starttime_ticks: Some(77),
                }],
                nice: 5,
                policy: NicePolicy::default(),
            },
            target_root_pid: Some(1234),
            evidence: vec![CandidateEvidence::new("cpu_pressure", "high", 0.9)],
            objective: ObjectiveKind::DesktopInteractivity,
        },
    };

    let plan = CandidatePlanFile::from_candidate(&candidate, Some(1));
    let json = serde_json::to_string(&plan).unwrap();
    let decoded: CandidatePlanFile = serde_json::from_str(&json).unwrap();

    assert!(decoded.executable.is_some());
    assert!(decoded.policy_explanation.verdict.is_allowed());
    assert!(decoded.apply_command.is_some());
    let decoded_candidate = decoded.executable.unwrap().into_candidate();
    assert_eq!(decoded_candidate.action_kind(), "nice");
    assert_eq!(decoded_candidate.candidate_name(), "nice-browser-helper");
}

#[test]
fn cpu_affinity_candidate_plan_file_is_manual_only_with_stable_rejection() {
    let plan_dir = temp_candidate_plan_dir("cpu-affinity-plan-manual-only");
    let candidate = CandidateAction::CpuAffinityProfile {
        plan: CpuAffinityProfilePlan {
            profile_name: "game".to_owned(),
            profile: Profile {
                name: "game".to_owned(),
                rules: vec![ProfileRule {
                    affinity: Some(CpuMask::parse("0").unwrap()),
                    nice: None,
                    ionice: None,
                    match_class: vec![TaskClass::Game],
                    match_comm: Vec::new(),
                }],
            },
            tree_pid: 1234,
        },
    };
    let plan_path = candidate_plan_path(&candidate, &plan_dir);

    let plan = write_candidate_plan_file(&plan_path, &candidate, Some(1)).unwrap();
    assert!(plan.executable.is_none());
    assert_eq!(plan.apply_command, None);
    assert_eq!(
        plan.manual_apply_command.as_deref(),
        Some("stutter apply-profile --tree-pid 1234 --profile <generated-or-existing-profile>")
    );
    assert_eq!(
        plan.manual_only_reason.as_deref(),
        Some("cpu-affinity profiles use apply-profile, not candidate-plan apply")
    );

    let decoded: CandidatePlanFile =
        serde_json::from_slice(&std::fs::read(&plan_path).unwrap()).unwrap();
    assert!(decoded.executable.is_none());
    assert_eq!(decoded.manual_only_reason, plan.manual_only_reason);

    let err = apply_candidate_plan_file(&plan_path, false)
        .unwrap_err()
        .to_string();
    assert!(err.contains("candidate_plan_manual_only"));
    assert!(err.contains("apply-profile"));
}

#[test]
fn suggestion_from_dry_run_record_uses_existing_profile_path_when_available() {
    let record = CandidateDryRunRecord {
        candidate_name: "game-main-suggested".to_owned(),
        affected_tasks: 31,
        warnings: Vec::new(),
        safety_class: SafetyClass::ReversibleLowRisk,
        eligible: true,
        reason: None,
    };

    let suggestion = suggestion_from_dry_run_record(
        &record,
        1234,
        Some(Path::new("/tmp/profiles.toml")),
        SafetyClass::ReversibleLowRisk,
        "scheduler pressure detected on Game/WineServer classes",
    )
    .unwrap();

    assert_eq!(
        suggestion.dry_run_command.as_deref(),
        Some("stutter apply-profile --tree-pid 1234 --profile /tmp/profiles.toml --dry-run")
    );
    assert_eq!(
        suggestion.manual_apply_command.as_deref(),
        Some("stutter apply-profile --tree-pid 1234 --profile /tmp/profiles.toml")
    );
}

#[test]
fn suggestion_from_dry_run_record_skips_ineligible_candidate() {
    let record = CandidateDryRunRecord {
        candidate_name: "bad-candidate".to_owned(),
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
}

#[test]
fn render_candidate_suggestion_escapes_reason_and_commands() {
    let suggestion = CandidateSuggestion {
            candidate_name: "candidate with space".to_owned(),
            action_kind: "cpu_affinity_profile".to_owned(),
            descriptor: ActionDescriptor {
                action_id: crate::actions::ActionId::new(
                    "cpu-affinity-profile:candidate with space".to_owned(),
                ),
                action_kind: "cpu_affinity_profile".to_owned(),
                safety_class: SafetyClass::ReversibleLowRisk,
                effect_scope: ActionEffectScope::LocalProcessTree,
                rollback: RollbackRequirement::RequiredBeforeApply,
                persistent_effect: false,
                touches_system_wide_state: false,
                requires_explicit_target: true,
                confidence: None,
            },
            objective: ObjectiveKind::StutterScore,
            evidence: Vec::new(),
            affected_tasks: 31,
            safety: SafetyClass::ReversibleLowRisk,
            reason: "scheduler \"pressure\"\nnext".to_owned(),
            dry_run_command: Some(
                "stutter apply-profile --tree-pid 1234 --profile /tmp/profile \"quoted\".toml --dry-run"
                    .to_owned(),
            ),
            manual_apply_command: Some(
                "stutter apply-profile --tree-pid 1234 --profile /tmp/profile \"quoted\".toml"
                    .to_owned(),
            ),
            required_mode: DaemonMode::ApplyLowRisk,
            required_safety_class: SafetyClass::ReversibleLowRisk,
            manual_only_reason: None,
        };

    let rendered = render_candidate_suggestion(&suggestion);

    assert!(rendered.contains("candidate=\"candidate with space\""));
    assert!(rendered.contains("reason=\"scheduler \\\"pressure\\\"\\nnext\""));
    assert!(rendered.contains("dry_run_command=\"stutter apply-profile --tree-pid 1234 --profile /tmp/profile \\\"quoted\\\".toml --dry-run\""));
    assert!(rendered.contains("manual_apply_command=\"stutter apply-profile --tree-pid 1234 --profile /tmp/profile \\\"quoted\\\".toml\""));
}

#[test]
fn low_risk_suggestion_renders_policy_aware_commands() {
    let suggestion = suggestion_from_dry_run_record(
        &dry_run_record(SafetyClass::ReversibleLowRisk),
        1234,
        Some(Path::new("profiles.toml")),
        SafetyClass::ReversibleLowRisk,
        "scheduler pressure detected",
    )
    .unwrap();

    assert_eq!(suggestion.required_mode, DaemonMode::ApplyLowRisk);
    assert_eq!(
        suggestion.required_safety_class,
        SafetyClass::ReversibleLowRisk
    );
    assert_eq!(
        suggestion.dry_run_command.as_deref(),
        Some("stutter apply-profile --tree-pid 1234 --profile profiles.toml --dry-run")
    );
    assert_eq!(
        suggestion.manual_apply_command.as_deref(),
        Some("stutter apply-profile --tree-pid 1234 --profile profiles.toml")
    );

    let rendered = render_candidate_suggestion(&suggestion);
    assert!(rendered.contains("suggest mode did not apply this change"));
    assert!(rendered.contains("required_mode=apply-low-risk"));
    assert!(rendered.contains("required_safety_class=ReversibleLowRisk"));
    assert!(rendered.contains("rollback=\"stutter restore\""));
    assert!(rendered.contains(
            "dry_run_command=\"stutter apply-profile --tree-pid 1234 --profile profiles.toml --dry-run\""
        ));
    assert!(rendered.contains(
        "manual_apply_command=\"stutter apply-profile --tree-pid 1234 --profile profiles.toml\""
    ));
}

#[test]
fn medium_risk_suggestion_requires_medium_mode_and_flag() {
    let suggestion = suggestion_from_dry_run_record(
        &dry_run_record(SafetyClass::ReversibleMediumRisk),
        1234,
        Some(Path::new("profiles.toml")),
        SafetyClass::ReversibleMediumRisk,
        "priority profile may help",
    )
    .unwrap();

    assert_eq!(suggestion.required_mode, DaemonMode::ApplyMediumRisk);
    assert_eq!(
        suggestion.required_safety_class,
        SafetyClass::ReversibleMediumRisk
    );
    assert_eq!(
        suggestion.manual_apply_command.as_deref(),
        Some("stutter apply-profile --tree-pid 1234 --profile profiles.toml --allow-medium-risk")
    );

    let rendered = render_candidate_suggestion(&suggestion);
    assert!(rendered.contains("required_mode=apply-medium-risk"));
    assert!(rendered.contains("required_safety_class=ReversibleMediumRisk"));
    assert!(rendered.contains("--allow-medium-risk"));
}

#[test]
fn high_risk_suggestion_suppresses_manual_apply_command() {
    let suggestion = suggestion_from_dry_run_record(
        &dry_run_record(SafetyClass::HighRisk),
        1234,
        Some(Path::new("profiles.toml")),
        SafetyClass::HighRisk,
        "high risk candidate",
    )
    .unwrap();

    assert_eq!(suggestion.required_mode, DaemonMode::ApplyHighRisk);
    assert_eq!(suggestion.required_safety_class, SafetyClass::HighRisk);
    assert_eq!(
        suggestion.dry_run_command.as_deref(),
        Some("stutter apply-profile --tree-pid 1234 --profile profiles.toml --dry-run")
    );
    assert_eq!(suggestion.manual_apply_command, None);

    let rendered = render_candidate_suggestion(&suggestion);
    assert!(rendered.contains("required_mode=apply-high-risk"));
    assert!(rendered.contains("manual_apply_command=none"));
    assert!(!rendered.contains("manual_apply_command=\"stutter apply-profile"));
}
