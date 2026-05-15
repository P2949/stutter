use std::time::Duration;

use anyhow::Context;

use crate::{
    actions::{
        RollbackToken, SafetyClass, TuningAction,
        cpu_affinity::CpuAffinityProfileAction,
        runner::{
            ActionHooks, ActionRunPolicy, AuditedActionResult, run_audited_action_with_hooks,
        },
    },
    autotune::candidate::{
        CandidateAction, CandidateDryRunRecord, dry_run_record_from_action_state,
    },
    daemon_policy::{ActionDescriptor, ActionSource, DaemonPolicy, PolicyIntent},
};

#[derive(Clone, Debug)]
pub struct ApplyCandidateOutcome {
    pub candidate_name: String,
    pub action_kind: String,
    pub affected_tasks: usize,
    pub safety_class: SafetyClass,
    pub rollback_performed: bool,
}

pub trait CandidateActionExecutor {
    fn candidate_name(&self) -> &str;
    fn action_kind(&self) -> &'static str;
    fn descriptor(&self) -> ActionDescriptor;
    fn safety_class(&self) -> SafetyClass {
        self.descriptor().safety_class
    }
    fn dry_run(&self) -> anyhow::Result<CandidateDryRunRecord>;
    fn apply_with_audit(&self, run_policy: ActionRunPolicy) -> anyhow::Result<AuditedActionResult>;
    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()>;
}

struct PlannedActionExecutor<A: TuningAction + 'static> {
    candidate_name: String,
    action_kind: &'static str,
    action: A,
}

impl<A: TuningAction + 'static> PlannedActionExecutor<A> {
    fn new(candidate_name: String, action_kind: &'static str, action: A) -> Self {
        Self {
            candidate_name,
            action_kind,
            action,
        }
    }
}

impl<A: TuningAction + 'static> CandidateActionExecutor for PlannedActionExecutor<A> {
    fn candidate_name(&self) -> &str {
        &self.candidate_name
    }

    fn action_kind(&self) -> &'static str {
        self.action_kind
    }

    fn descriptor(&self) -> ActionDescriptor {
        ActionDescriptor {
            action_id: self.action.id(),
            action_kind: self.action_kind.to_owned(),
            safety_class: self.action.safety_class(),
            effect_scope: crate::daemon_policy::ActionEffectScope::LocalProcessTree,
            rollback: crate::daemon_policy::RollbackRequirement::RequiredBeforeApply,
            persistent_effect: false,
            touches_system_wide_state: false,
            requires_explicit_target: true,
            confidence: None,
        }
    }

    fn dry_run(&self) -> anyhow::Result<CandidateDryRunRecord> {
        let state = self.action.dry_run()?;
        Ok(dry_run_record_from_action_state(
            self.candidate_name.clone(),
            self.action.safety_class(),
            state,
        ))
    }

    fn apply_with_audit(&self, run_policy: ActionRunPolicy) -> anyhow::Result<AuditedActionResult> {
        run_audited_action_with_hooks(
            "autotune candidate",
            &self.action,
            run_policy,
            ActionHooks::none(),
        )
        .map_err(anyhow::Error::new)
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        self.action.rollback(token)
    }
}

struct DescribedActionExecutor<A: TuningAction + 'static> {
    candidate_name: String,
    descriptor: ActionDescriptor,
    action: A,
}

impl<A: TuningAction + 'static> DescribedActionExecutor<A> {
    fn new(candidate: &CandidateAction, action: A) -> Self {
        Self {
            candidate_name: candidate.candidate_name().to_owned(),
            descriptor: candidate.descriptor(),
            action,
        }
    }
}

impl<A: TuningAction + 'static> CandidateActionExecutor for DescribedActionExecutor<A> {
    fn candidate_name(&self) -> &str {
        &self.candidate_name
    }

    fn action_kind(&self) -> &'static str {
        action_kind_static(&self.descriptor.action_kind)
    }

    fn descriptor(&self) -> ActionDescriptor {
        self.descriptor.clone()
    }

    fn dry_run(&self) -> anyhow::Result<CandidateDryRunRecord> {
        let state = self.action.dry_run()?;
        Ok(dry_run_record_from_action_state(
            self.candidate_name.clone(),
            self.action.safety_class(),
            state,
        ))
    }

    fn apply_with_audit(&self, run_policy: ActionRunPolicy) -> anyhow::Result<AuditedActionResult> {
        run_audited_action_with_hooks(
            "autotune candidate",
            &self.action,
            run_policy,
            ActionHooks::none(),
        )
        .map_err(anyhow::Error::new)
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        self.action.rollback(token)
    }
}

pub fn executor_for_candidate(
    candidate: CandidateAction,
) -> anyhow::Result<Box<dyn CandidateActionExecutor>> {
    match candidate {
        CandidateAction::CpuAffinityProfile {
            profile_name,
            profile,
            tree_pid,
        } => Ok(Box::new(PlannedActionExecutor::new(
            profile_name,
            "cpu_affinity_profile",
            CpuAffinityProfileAction {
                tree_pid,
                profile,
                force_restore_overwrite: false,
            },
        ))),
        CandidateAction::Nice { plan } => {
            let candidate = CandidateAction::Nice { plan: plan.clone() };
            Ok(Box::new(DescribedActionExecutor::new(
                &candidate,
                plan.action,
            )))
        }
        CandidateAction::IoPrio { plan } => {
            let candidate = CandidateAction::IoPrio { plan: plan.clone() };
            Ok(Box::new(DescribedActionExecutor::new(
                &candidate,
                plan.action,
            )))
        }
        CandidateAction::Uclamp { plan } => {
            let candidate = CandidateAction::Uclamp { plan: plan.clone() };
            Ok(Box::new(DescribedActionExecutor::new(
                &candidate,
                plan.action,
            )))
        }
        CandidateAction::CgroupPlacement { plan } => {
            let candidate = CandidateAction::CgroupPlacement { plan: plan.clone() };
            Ok(Box::new(DescribedActionExecutor::new(
                &candidate,
                plan.action,
            )))
        }
        other => anyhow::bail!(
            "generic apply executor does not support candidate '{}' action_kind={} safety={:?}",
            other.candidate_name(),
            other.action_kind(),
            other.safety_class()
        ),
    }
}

pub async fn run_apply_medium_risk_candidate(
    candidate: CandidateAction,
    duration: Duration,
) -> anyhow::Result<ApplyCandidateOutcome> {
    let executor = executor_for_candidate(candidate)?;
    run_apply_medium_risk_with_executor(executor.as_ref(), duration).await
}

pub async fn run_apply_medium_risk_with_executor(
    executor: &dyn CandidateActionExecutor,
    duration: Duration,
) -> anyhow::Result<ApplyCandidateOutcome> {
    ensure_medium_risk_action_allowed(&executor.descriptor())?;

    let dry_run = executor.dry_run()?;
    if !dry_run.eligible {
        anyhow::bail!(
            "candidate '{}' is not eligible for apply-medium-risk: {}",
            executor.candidate_name(),
            dry_run
                .reason
                .as_deref()
                .unwrap_or("dry-run did not produce an eligible candidate")
        );
    }

    let run_policy = ActionRunPolicy::apply_medium_risk(ActionSource::AutotuneRuntime, false);
    let result = executor.apply_with_audit(run_policy).with_context(|| {
        format!(
            "failed to apply medium-risk candidate '{}'",
            executor.candidate_name()
        )
    })?;
    let rollback = result
        .rollback
        .context("apply-medium-risk action completed without rollback token")?;
    let affected_tasks = rollback.affected_tasks();
    let mut guard = GenericRollbackGuard::new(executor, rollback);

    if !duration.is_zero() {
        tokio::time::sleep(duration).await;
    }

    guard.rollback_now().with_context(|| {
        format!(
            "failed to rollback medium-risk candidate '{}'",
            executor.candidate_name()
        )
    })?;

    Ok(ApplyCandidateOutcome {
        candidate_name: executor.candidate_name().to_owned(),
        action_kind: executor.action_kind().to_owned(),
        affected_tasks,
        safety_class: executor.safety_class(),
        rollback_performed: guard.rollback_performed(),
    })
}

pub fn ensure_medium_risk_action_allowed(descriptor: &ActionDescriptor) -> anyhow::Result<()> {
    let policy = DaemonPolicy::apply_medium_risk(ActionSource::AutotuneRuntime);
    policy
        .check_action(PolicyIntent::Apply, descriptor)
        .with_context(|| {
            format!(
                "apply-medium-risk blocked action_kind={} safety={:?}",
                descriptor.action_kind, descriptor.safety_class
            )
        })
}

struct GenericRollbackGuard<'a> {
    executor: &'a dyn CandidateActionExecutor,
    token: Option<RollbackToken>,
    rollback_performed: bool,
}

impl<'a> GenericRollbackGuard<'a> {
    fn new(executor: &'a dyn CandidateActionExecutor, token: RollbackToken) -> Self {
        Self {
            executor,
            token: Some(token),
            rollback_performed: false,
        }
    }

    fn rollback_now(&mut self) -> anyhow::Result<()> {
        if let Some(token) = self.token.take() {
            self.executor.rollback(&token)?;
            self.rollback_performed = true;
        }
        Ok(())
    }

    fn rollback_performed(&self) -> bool {
        self.rollback_performed
    }
}

impl Drop for GenericRollbackGuard<'_> {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            if let Err(err) = self.executor.rollback(&token) {
                log::error!(
                    "autotune_apply_medium_risk_rollback_failed candidate={} error={err:#}",
                    self.executor.candidate_name()
                );
            } else {
                self.rollback_performed = true;
            }
        }
    }
}

fn action_kind_static(action_kind: &str) -> &'static str {
    match action_kind {
        "cpu_affinity_profile" => "cpu_affinity_profile",
        "nice" => "nice",
        "ionice" => "ionice",
        "uclamp" => "uclamp",
        "cgroup_placement" => "cgroup_placement",
        "irq_affinity" => "irq_affinity",
        "cpu_power" => "cpu_power",
        "gpu_power" => "gpu_power",
        "vm_knob" => "vm_knob",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        actions::{ActionId, ActionOutcome, ActionState, ActionWarning},
        daemon_policy::{ActionEffectScope, RollbackRequirement},
    };

    struct FakeMediumExecutor {
        descriptor: ActionDescriptor,
        dry_run: CandidateDryRunRecord,
        rollback: RollbackToken,
    }

    impl FakeMediumExecutor {
        fn new(action_kind: &'static str, scope: ActionEffectScope) -> Self {
            Self {
                descriptor: ActionDescriptor {
                    action_id: ActionId(format!("{action_kind}:fake")),
                    action_kind: action_kind.to_owned(),
                    safety_class: SafetyClass::ReversibleMediumRisk,
                    effect_scope: scope,
                    rollback: RollbackRequirement::RequiredBeforeApply,
                    persistent_effect: false,
                    touches_system_wide_state: false,
                    requires_explicit_target: true,
                    confidence: None,
                },
                dry_run: CandidateDryRunRecord {
                    candidate_name: action_kind.to_owned(),
                    affected_tasks: 1,
                    warnings: Vec::<ActionWarning>::new(),
                    safety_class: SafetyClass::ReversibleMediumRisk,
                    eligible: true,
                    reason: None,
                },
                rollback: RollbackToken::NiceRestore { records: vec![] },
            }
        }
    }

    impl CandidateActionExecutor for FakeMediumExecutor {
        fn candidate_name(&self) -> &str {
            &self.dry_run.candidate_name
        }

        fn action_kind(&self) -> &'static str {
            action_kind_static(&self.descriptor.action_kind)
        }

        fn descriptor(&self) -> ActionDescriptor {
            self.descriptor.clone()
        }

        fn dry_run(&self) -> anyhow::Result<CandidateDryRunRecord> {
            Ok(self.dry_run.clone())
        }

        fn apply_with_audit(
            &self,
            _run_policy: ActionRunPolicy,
        ) -> anyhow::Result<AuditedActionResult> {
            let state = ActionState {
                applied: true,
                affected_tasks: 1,
                checked_tasks: 1,
                pending_changes: 0,
                warnings: Vec::new(),
            };
            Ok(AuditedActionResult {
                state: state.clone(),
                rollback: Some(self.rollback.clone()),
                outcome: ActionOutcome {
                    action_id: self.descriptor.action_id.clone(),
                    safety_class: self.descriptor.safety_class.clone(),
                    dry_run: false,
                    preflight_warnings: Vec::new(),
                    state,
                    rollback: Some(self.rollback.clone()),
                    started_unix_nanos: 1,
                    finished_unix_nanos: 2,
                },
            })
        }

        fn rollback(&self, _token: &RollbackToken) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn medium_risk_policy_accepts_reversible_process_and_cgroup_descriptors() {
        let process = FakeMediumExecutor::new("nice", ActionEffectScope::LocalProcessTree);
        let cgroup = FakeMediumExecutor::new("cgroup_placement", ActionEffectScope::Cgroup);

        ensure_medium_risk_action_allowed(&process.descriptor()).unwrap();
        ensure_medium_risk_action_allowed(&cgroup.descriptor()).unwrap();
    }

    #[test]
    fn medium_risk_policy_rejects_high_risk_descriptors() {
        let mut high = FakeMediumExecutor::new("cpu_power", ActionEffectScope::CpuPower);
        high.descriptor.safety_class = SafetyClass::HighRisk;
        high.descriptor.touches_system_wide_state = true;

        assert!(ensure_medium_risk_action_allowed(&high.descriptor()).is_err());
    }

    #[tokio::test]
    async fn run_apply_medium_risk_with_executor_rolls_back() {
        let executor = FakeMediumExecutor::new("uclamp", ActionEffectScope::LocalProcessTree);

        let outcome = run_apply_medium_risk_with_executor(&executor, Duration::ZERO)
            .await
            .unwrap();

        assert_eq!(outcome.action_kind, "uclamp");
        assert_eq!(outcome.affected_tasks, 0);
        assert!(outcome.rollback_performed);
    }
}
