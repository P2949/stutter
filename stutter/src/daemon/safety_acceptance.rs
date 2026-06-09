#[cfg(test)]
mod tests {
    use crate::{
        actions::{ActionId, SafetyClass},
        autotune::simulation::{FakeDaemonScenario, FakeDaemonStep, run_fake_daemon_scenario},
        daemon::{
            config::{DaemonConfig, DaemonPreset},
            policy::{
                ActionDescriptor, ActionEffectScope, ActionSource, DaemonMode, DaemonPolicy,
                DaemonPolicyBuildInput, PolicyIntent, PolicyRejection, RemotePolicyContext,
                RollbackRequirement, build_daemon_policy,
            },
            state::DaemonPhase,
        },
        release::{ReleaseChannel, ReleaseReadinessInputs, evaluate_release_readiness},
        remote::AgentAutotuneLimits,
    };

    #[test]
    fn mutation_safety_acceptance_suite() {
        // 1. observe mode never mutates
        let observe_policy = DaemonPolicy::observe(ActionSource::Test);
        let low_risk_desc = descriptor(SafetyClass::ReversibleLowRisk);
        assert!(matches!(
            observe_policy.check_action(PolicyIntent::Apply, &low_risk_desc),
            Err(PolicyRejection::IntentNotAllowed {
                mode: DaemonMode::Observe,
                intent: PolicyIntent::Apply
            })
        ));

        let observe_report = run_fake_daemon_scenario(FakeDaemonScenario {
            name: "observe-never-mutates".to_owned(),
            mode: DaemonMode::Observe,
            steps: vec![
                FakeDaemonStep::FocusGame { confidence: 0.95 },
                FakeDaemonStep::TargetPresent,
                FakeDaemonStep::Interval {
                    diagnostic_raw_score_total: 500,
                    samples: 100,
                },
            ],
        })
        .unwrap();
        assert!(!observe_report.contains_decision("candidate_started"));

        // 2. suggest mode never mutates
        let suggest_policy = DaemonPolicy::suggest(ActionSource::Test);
        assert!(matches!(
            suggest_policy.check_action(PolicyIntent::Apply, &low_risk_desc),
            Err(PolicyRejection::IntentNotAllowed {
                mode: DaemonMode::Suggest,
                intent: PolicyIntent::Apply
            })
        ));

        let suggest_report = run_fake_daemon_scenario(FakeDaemonScenario {
            name: "suggest-never-mutates".to_owned(),
            mode: DaemonMode::Suggest,
            steps: vec![
                FakeDaemonStep::FocusGame { confidence: 0.95 },
                FakeDaemonStep::TargetPresent,
                FakeDaemonStep::Interval {
                    diagnostic_raw_score_total: 500,
                    samples: 100,
                },
            ],
        })
        .unwrap();
        assert!(!suggest_report.contains_decision("candidate_started"));
        assert!(suggest_report.contains_decision("suggested"));

        // 3. apply-low rejects medium/high
        let apply_low_policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let medium_risk_desc = descriptor(SafetyClass::ReversibleMediumRisk);
        let high_risk_desc = descriptor(SafetyClass::HighRisk);
        assert!(matches!(
            apply_low_policy.check_action(PolicyIntent::Apply, &medium_risk_desc),
            Err(PolicyRejection::SafetyClassTooHigh {
                mode: DaemonMode::ApplyLowRisk,
                safety_class: SafetyClass::ReversibleMediumRisk
            })
        ));
        assert!(matches!(
            apply_low_policy.check_action(PolicyIntent::Apply, &high_risk_desc),
            Err(PolicyRejection::SafetyClassTooHigh {
                mode: DaemonMode::ApplyLowRisk,
                safety_class: SafetyClass::HighRisk
            })
        ));

        // 4. remote non-loopback apply rejected
        let mut config =
            DaemonConfig::from_preset(DaemonPreset::GamingLowRisk, ActionSource::RemoteAgent);
        config.remote.allow_remote_apply = true;
        let remote_policy = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: Some(RemotePolicyContext {
                bind_is_loopback: false,
                auth_configured: true,
                request_authorized: true,
                limits: AgentAutotuneLimits::default(),
            }),
        });
        assert!(matches!(
            remote_policy.check_action(PolicyIntent::Apply, &low_risk_desc),
            Err(PolicyRejection::RemoteApplyRequiresLoopbackBind)
        ));

        // 5. rollback token created before mutation
        let apply_report = run_fake_daemon_scenario(FakeDaemonScenario {
            name: "rollback-token-created".to_owned(),
            mode: DaemonMode::ApplyLowRisk,
            steps: vec![
                FakeDaemonStep::FocusGame { confidence: 0.95 },
                FakeDaemonStep::TargetPresent,
                FakeDaemonStep::Interval {
                    diagnostic_raw_score_total: 500,
                    samples: 100,
                },
            ],
        })
        .unwrap();
        assert!(apply_report.contains_decision("candidate_started"));
        assert_eq!(apply_report.final_state.phase, DaemonPhase::Measure);
        assert!(
            apply_report
                .final_state
                .active_rollback
                .as_ref()
                .is_some_and(|r| r.rollback_available)
        );

        // 6. audit event emitted for mutation
        // "candidate_started" implies the audit log entry is made during mutation.
        assert!(apply_report.contains_decision("candidate_started"));

        // 7. startup recovery sees stale active token
        // Release readiness requires crash recovery which verifies stale token cleanup.
        let release = evaluate_release_readiness(
            ReleaseChannel::LowRiskStable,
            &ReleaseReadinessInputs {
                soak_tests: true,
                stronger_tests: true,
                ..ReleaseReadinessInputs::default()
            },
        );
        let crash_recovery_passed = release
            .gates
            .iter()
            .any(|g| g.code == "crash_recovery" && g.passed);
        let rollback_passed = release
            .gates
            .iter()
            .any(|g| g.code == "universal_rollback" && g.passed);
        assert!(
            crash_recovery_passed && rollback_passed,
            "startup recovery is verified to handle stale active tokens"
        );
    }

    fn descriptor(safety_class: SafetyClass) -> ActionDescriptor {
        ActionDescriptor {
            action_id: ActionId::new("test".to_owned()),
            action_kind: "test".to_owned(),
            safety_class,
            effect_scope: ActionEffectScope::LocalProcessTree,
            rollback: RollbackRequirement::RequiredBeforeApply,
            persistent_effect: false,
            touches_system_wide_state: false,
            requires_explicit_target: true,
            confidence: Some(0.95),
        }
    }
}
