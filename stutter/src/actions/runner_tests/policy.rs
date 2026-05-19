//! Policy tests extracted from `actions::runner`.
//!
//! Owns runner policy construction, policy rejection, capability rejection, scope-limit rejection, and API signature tests.
//! Does not own audit-log, timeout, rollback, hook-failure, or production runner behavior.

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::super::{
        super::*, action_phases, all_capabilities_available, apply_policy, temp_dir, terminal_event,
    };
    use crate::actions::{ActionFailure, fake_action::FakeAction};

    #[test]
    fn action_run_policy_constructors_resolve_policies_through_builder() {
        let config = crate::daemon::DaemonConfig {
            mode: DaemonMode::ApplyLowRisk,
            source: ActionSource::Test,
            ..crate::daemon::DaemonConfig::default()
        };
        let expected = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        });
        let run_policy = ActionRunPolicy::apply_low_risk(ActionSource::Test, false);

        assert_eq!(run_policy.policy, expected);
        assert!(!run_policy.dry_run);
        assert_eq!(run_policy.max_affected_tasks, None);
    }

    #[test]
    fn runner_policy_rejection_happens_before_apply() {
        let dir = temp_dir("policy-rejection");
        let audit_path = dir.join("audit.jsonl");
        let action = FakeAction::new().with_safety_class(SafetyClass::ReversibleMediumRisk);

        let result = run_audited_action_with_audit_path(
            "fake-controller",
            &action,
            ActionRunPolicy::apply_low_risk(ActionSource::Test, false),
            &audit_path,
        );

        assert!(result.is_err());
        assert_eq!(action.events(), vec!["preflight", "dry_run"]);
        assert!(!action.applied());
        assert!(!action.rolled_back());

        let err = result.unwrap_err();
        assert!(matches!(
            err.failure(),
            ActionFailure::PolicyRejected { .. }
        ));
        assert!(err.to_string().contains("policy rejected"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(
            action_phases(&events),
            vec![
                Some(crate::actions::ActionPhase::Preflight),
                Some(crate::actions::ActionPhase::DryRun),
                Some(crate::actions::ActionPhase::Preflight),
            ]
        );
        let terminal = terminal_event(&events);
        assert!(!terminal.success);
        assert_eq!(terminal.error_category.as_deref(), Some("policy_rejected"));
        assert!(terminal.message.contains("policy rejected"));
        assert!(
            terminal
                .message
                .contains("safety class ReversibleMediumRisk")
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn runner_context_capability_rejection_happens_before_apply() {
        let dir = temp_dir("capability-rejection");
        let audit_path = dir.join("audit.jsonl");
        let action = FakeAction::new().with_action_id("uclamp:min");
        let mut capabilities = all_capabilities_available();
        capabilities.uclamp_available = false;

        let result = run_audited_action_with_audit_path(
            "fake-controller",
            &action,
            apply_policy().with_capabilities(capabilities),
            &audit_path,
        );

        assert!(result.is_err());
        assert_eq!(action.events(), vec!["preflight"]);
        assert!(!action.applied());
        assert!(!action.rolled_back());

        let err = result.unwrap_err();
        assert!(matches!(
            err.failure(),
            ActionFailure::PolicyRejected { .. }
        ));
        assert!(err.to_string().contains("uclamp"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 2);
        let terminal = terminal_event(&events);
        assert!(!terminal.success);
        assert_eq!(terminal.error_category.as_deref(), Some("policy_rejected"));
        assert!(terminal.message.contains("uclamp"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn runner_scope_limit_rejection_happens_after_dry_run_before_apply() {
        let dir = temp_dir("scope-limit");
        let audit_path = dir.join("audit.jsonl");
        let action = FakeAction::new().with_affected_tasks(8);

        let result = run_audited_action_with_audit_path(
            "fake-controller",
            &action,
            apply_policy().with_max_affected_tasks(3),
            &audit_path,
        );

        assert!(result.is_err());
        assert_eq!(action.events(), vec!["preflight", "dry_run"]);
        assert!(!action.applied());
        assert!(!action.rolled_back());

        let err = result.unwrap_err();
        assert!(matches!(
            err.failure(),
            ActionFailure::ScopeLimitExceeded { .. }
        ));
        assert!(err.to_string().contains("exceeding scope limit 3"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 3);
        let terminal = terminal_event(&events);
        assert!(!terminal.success);
        assert_eq!(
            terminal.error_category.as_deref(),
            Some("scope_limit_exceeded")
        );
        assert_eq!(
            terminal.action_phase,
            Some(crate::actions::ActionPhase::DryRun)
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn action_runner_signature_prevents_apply_without_policy() {
        let _runner: fn(
            &str,
            &FakeAction,
            ActionRunPolicy,
        ) -> Result<AuditedActionResult, ActionError> = run_audited_action::<FakeAction>;
        let _runner_with_audit_path: fn(
            &str,
            &FakeAction,
            ActionRunPolicy,
            &Path,
        ) -> Result<AuditedActionResult, ActionError> =
            run_audited_action_with_audit_path::<FakeAction>;
    }
}
