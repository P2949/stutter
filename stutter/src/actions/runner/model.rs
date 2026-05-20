//! Data model and hook/policy configuration for audited action runs.
//!
//! Owns result DTOs, hook registration, and run policy construction. It must not perform audit
//! writes, rollback handling, or action execution.

use std::time::Duration;

use crate::{
    actions::{ActionState, RollbackToken, SafetyClass, TuningAction},
    daemon::DaemonConfig,
    daemon_policy::{
        ActionSource, DaemonMode, DaemonPolicy, DaemonPolicyBuildInput, DaemonPolicyContext,
        build_daemon_policy,
    },
};

#[derive(Debug)]
pub struct AuditedActionResult {
    pub state: ActionState,
    pub rollback: Option<RollbackToken>,
}

type ActionHookResult = anyhow::Result<()>;
type AfterApplyHook<'a> = dyn FnMut(&RollbackToken) -> ActionHookResult + 'a;
type RollbackHook<'a> = dyn FnMut(&RollbackToken) -> ActionHookResult + 'a;

pub(crate) struct ActionHooks<'a> {
    after_apply: Option<Box<AfterApplyHook<'a>>>,
    after_rollback: Option<Box<RollbackHook<'a>>>,
}

impl<'a> ActionHooks<'a> {
    pub(crate) fn none() -> Self {
        Self {
            after_apply: None,
            after_rollback: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn after_apply<F>(after_apply: F) -> Self
    where
        F: FnMut(&RollbackToken) -> ActionHookResult + 'a,
    {
        Self {
            after_apply: Some(Box::new(after_apply)),
            after_rollback: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_after_rollback<F>(mut self, after_rollback: F) -> Self
    where
        F: FnMut(&RollbackToken) -> ActionHookResult + 'a,
    {
        self.after_rollback = Some(Box::new(after_rollback));
        self
    }

    pub(super) fn run_after_apply(&mut self, rollback: &RollbackToken) -> ActionHookResult {
        if let Some(after_apply) = self.after_apply.as_mut() {
            after_apply(rollback)?;
        }

        Ok(())
    }

    pub(super) fn run_after_rollback(&mut self, rollback: &RollbackToken) -> ActionHookResult {
        if let Some(after_rollback) = self.after_rollback.as_mut() {
            after_rollback(rollback)?;
        }

        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct ActionRunPolicy {
    pub policy: DaemonPolicy,
    pub context: DaemonPolicyContext,
    pub max_affected_tasks: Option<usize>,
    pub max_total_duration: Option<Duration>,
    pub dry_run: bool,
}

impl ActionRunPolicy {
    pub fn dry_run(source: ActionSource) -> Self {
        Self::for_mode(DaemonMode::Suggest, source, true)
    }

    pub fn apply_low_risk(source: ActionSource, dry_run: bool) -> Self {
        Self::for_mode(DaemonMode::ApplyLowRisk, source, dry_run)
    }

    pub fn apply_medium_risk(source: ActionSource, dry_run: bool) -> Self {
        Self::for_mode(DaemonMode::ApplyMediumRisk, source, dry_run)
    }

    pub fn for_action<A: TuningAction>(action: &A, dry_run: bool, source: ActionSource) -> Self {
        if dry_run {
            return Self::dry_run(source);
        }

        match action.safety_class() {
            SafetyClass::ObserveOnly | SafetyClass::ReversibleLowRisk => {
                Self::apply_low_risk(source, false)
            }
            SafetyClass::ReversibleMediumRisk | SafetyClass::HighRisk => {
                Self::apply_medium_risk(source, false)
            }
        }
    }

    fn for_mode(mode: DaemonMode, source: ActionSource, dry_run: bool) -> Self {
        let config = DaemonConfig {
            mode,
            source,
            ..DaemonConfig::default()
        };

        Self {
            policy: build_daemon_policy(DaemonPolicyBuildInput {
                config: &config,
                remote_context: None,
            }),
            context: DaemonPolicyContext::default(),
            max_affected_tasks: None,
            max_total_duration: None,
            dry_run,
        }
    }

    #[cfg(test)]
    pub fn with_capabilities(
        mut self,
        capabilities: crate::daemon::capabilities::DaemonCapabilities,
    ) -> Self {
        self.context.capabilities = Some(capabilities);
        self
    }

    #[cfg(test)]
    pub fn with_max_affected_tasks(mut self, max_affected_tasks: usize) -> Self {
        self.max_affected_tasks = Some(max_affected_tasks);
        self
    }

    #[cfg(test)]
    pub fn with_max_total_duration(mut self, max_total_duration: Duration) -> Self {
        self.max_total_duration = Some(max_total_duration);
        self
    }
}
