use std::collections::BTreeSet;

use super::{model::WorkloadPolicyLintKind, *};
use crate::autotune::{objective::ObjectiveKind, situation::SituationKind};

#[test]
fn game_policy_enables_game_provider_families() {
    let rule = workload_policy_for_situation(SituationKind::GameCpuSchedulerPressure);

    assert!(rule.allowed_families.contains("cpu_affinity_profile"));
    assert!(rule.allowed_families.contains("uclamp"));
    assert!(rule.allowed_families.contains("gpu_power"));
    assert!(
        rule.allowed_objectives
            .contains(&ObjectiveKind::GameRunnableLatency)
    );
    assert!(rule.autonomous_families.contains("cpu_affinity_profile"));
}

#[test]
fn recording_policy_blocks_game_only_aggressive_families() {
    let rule = workload_policy_for_situation(SituationKind::Recording);

    assert!(!rule.allowed_families.contains("cpu_power"));
    assert!(!rule.allowed_families.contains("gpu_power"));
    assert!(!rule.allowed_families.contains("cpu_affinity_profile"));
    assert!(rule.allowed_families.contains("uclamp"));
}

#[test]
fn autonomous_policy_uses_autonomous_families_not_allowed_families() {
    let candidate = crate::autotune::planning::candidate::CandidateAction::fake(
        crate::actions::ActionId::new("fake-autonomous-test".to_owned()),
        crate::actions::SafetyClass::ReversibleLowRisk,
    );
    let allowed_only = WorkloadPolicyRule {
        situation: SituationKind::Unknown,
        allowed_families: BTreeSet::from(["fake".to_owned()]),
        allowed_objectives: BTreeSet::new(),
        autonomous_families: BTreeSet::new(),
    };

    assert!(allowed_only.allows_candidate(&candidate));
    assert!(!allowed_only.allows_autonomous_candidate(&candidate));

    let autonomous = WorkloadPolicyRule {
        autonomous_families: BTreeSet::from(["fake".to_owned()]),
        ..allowed_only
    };

    assert!(autonomous.allows_autonomous_candidate(&candidate));
}

#[test]
fn browser_policy_rejects_compile_throughput_objective() {
    let rule = workload_policy_for_situation(SituationKind::BrowserFocused);

    assert!(!rule.allows_objective(ObjectiveKind::CompileThroughputWithForegroundProtection));
}

#[test]
fn matrix_overrides_specific_situation_and_keeps_defaults() {
    let override_rule = WorkloadPolicyRule {
        situation: SituationKind::BrowserFocused,
        allowed_families: ["nice"].into_iter().map(str::to_owned).collect(),
        allowed_objectives: [ObjectiveKind::BrowserInteractivity].into_iter().collect(),
        autonomous_families: BTreeSet::new(),
    };

    let matrix = WorkloadPolicyMatrix::with_overrides(vec![override_rule]).unwrap();

    assert_eq!(
        matrix
            .rule_for(SituationKind::BrowserFocused)
            .allowed_families,
        ["nice"].into_iter().map(str::to_owned).collect()
    );
    assert!(
        matrix
            .rule_for(SituationKind::GameCpuSchedulerPressure)
            .allowed_families
            .contains("cpu_affinity_profile")
    );
}

#[test]
fn config_rule_validation_rejects_unknown_objective() {
    let unknown_objective = WorkloadPolicyRuleConfigFile {
        situation: "browser_focused".to_owned(),
        allowed_families: vec!["nice".to_owned()],
        allowed_objectives: vec!["not_real".to_owned()],
        autonomous_families: Vec::new(),
    };

    assert!(
        unknown_objective
            .into_rule()
            .unwrap_err()
            .to_string()
            .contains("invalid workload policy objective")
    );
}

#[test]
fn config_rule_validation_rejects_unknown_family_and_duplicate_situations() {
    let unknown_family = WorkloadPolicyRuleConfigFile {
        situation: "browser_focused".to_owned(),
        allowed_families: vec!["mystery_knob".to_owned()],
        allowed_objectives: vec!["browser_interactivity".to_owned()],
        autonomous_families: Vec::new(),
    };
    assert!(unknown_family.into_rule().is_err());

    let duplicate = WorkloadPolicyRuleConfigFile {
        situation: "browser_focused".to_owned(),
        allowed_families: vec!["nice".to_owned()],
        allowed_objectives: vec!["browser_interactivity".to_owned()],
        autonomous_families: Vec::new(),
    };

    assert!(
        parse_workload_policy_rule_configs(&[duplicate.clone(), duplicate])
            .unwrap_err()
            .to_string()
            .contains("duplicate workload policy rule")
    );
}

#[test]
fn default_workload_policy_has_no_error_lints_for_default_daemon_policies() {
    for preset in [
        crate::daemon::config::DaemonPreset::ObserveOnly,
        crate::daemon::config::DaemonPreset::GamingLowRisk,
        crate::daemon::config::DaemonPreset::GamingLaptopSafe,
        crate::daemon::config::DaemonPreset::WorkstationLowRisk,
        crate::daemon::config::DaemonPreset::DebugAggressive,
    ] {
        let config = crate::daemon::config::DaemonConfig::from_preset(
            preset,
            crate::daemon_policy::ActionSource::Test,
        );
        let policy = crate::daemon::policy::build_daemon_policy(
            crate::daemon::policy::DaemonPolicyBuildInput {
                config: &config,
                remote_context: None,
            },
        );
        let lints = lint_workload_policy(&WorkloadPolicyMatrix::default_rules(), &policy);

        assert!(
            lints
                .iter()
                .all(|lint| lint.severity == LintSeverity::Warning),
            "preset {preset:?} produced error lints: {lints:?}"
        );
    }
}

#[test]
fn linter_rejects_autonomous_system_wide_or_denied_families() {
    let mut config = crate::daemon::config::DaemonConfig::from_preset(
        crate::daemon::config::DaemonPreset::GamingLowRisk,
        crate::daemon_policy::ActionSource::Test,
    );
    config
        .safety
        .denied_action_families
        .insert("nice".to_owned());
    let policy =
        crate::daemon::policy::build_daemon_policy(crate::daemon::policy::DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        });
    let matrix = WorkloadPolicyMatrix {
        rules: vec![WorkloadPolicyRule {
            situation: SituationKind::GameFocused,
            allowed_families: ["gpu_power", "nice"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            allowed_objectives: [ObjectiveKind::ThermalRecovery].into_iter().collect(),
            autonomous_families: ["gpu_power", "nice"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }],
    };

    let lints = lint_workload_policy(&matrix, &policy);

    assert!(lints.iter().any(|lint| {
        lint.kind == WorkloadPolicyLintKind::MediumRiskSystemWideDenied
            && lint.severity == LintSeverity::Error
    }));
    assert!(lints.iter().any(|lint| {
        lint.kind == WorkloadPolicyLintKind::DeniedFamilyIsAutonomous
            && lint.severity == LintSeverity::Error
    }));
}
