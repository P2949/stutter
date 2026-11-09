//! A typed cgroup resource-control write as a manual, policy-gated action (package W11a).
//!
//! Each knob (`cpu.weight`, `cpu.idle`, `cpu.max`, `cpuset.*`, `memory.*`, `io.weight`) is its own
//! operation; none inherits the maturity of cgroup placement. The action is HighRisk and
//! system-wide in scope, never an executable autotune plan, never autonomous, and the default
//! policy denies. The transaction preflights the hierarchy, aborts on a foreign change, reads the
//! value back, and records exactly what recovery needs.
use std::path::PathBuf;

use super::{
    control::CgroupControl,
    control_transaction::{
        ControlFiles, ControlPrepared, PrepareError, RealFiles, RestoreOutcome, prepare,
        restore_recorded,
    },
};
use crate::actions::{
    ActionBoundaryError, ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass,
    TuningAction,
};

#[derive(Clone, Debug, Default)]
pub struct CgroupControlPolicy {
    /// Nothing is written unless this is set explicitly.
    pub allow: bool,
    /// The separately verified target-device mechanism for `io.weight` (presence is not proof).
    pub io_weight_honored: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct CgroupControlAction {
    pub dir: PathBuf,
    pub control: CgroupControl,
}

impl CgroupControlAction {
    fn prepared(
        &self,
        policy: &CgroupControlPolicy,
        files: &dyn ControlFiles,
    ) -> anyhow::Result<ControlPrepared> {
        if !policy.allow {
            return Err(ActionBoundaryError::PolicyDenied {
                action_kind: "cgroup_resource_control",
                requirement: "allow_cgroup_control",
            }
            .into());
        }
        prepare(
            &self.dir,
            &self.control,
            crate::syscall::geteuid(),
            policy.io_weight_honored,
            files,
        )
        .map_err(|error: PrepareError| {
            anyhow::anyhow!("cgroup control {:?} refused: {error:?}", self.control)
        })
    }

    pub fn preflight_with_policy(
        &self,
        policy: &CgroupControlPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.prepared(policy, &RealFiles)?;
        Ok(vec![ActionWarning {
            message: format!(
                "{} is a {:?} control: only the named group is written and its baseline is restored while the value is still ours",
                self.control.file_and_controller().0,
                self.control.risk()
            ),
        }])
    }

    pub fn apply_with_policy(&self, policy: &CgroupControlPolicy) -> anyhow::Result<RollbackToken> {
        self.apply_with_files(policy, &RealFiles)
    }

    pub fn apply_with_files(
        &self,
        policy: &CgroupControlPolicy,
        files: &dyn ControlFiles,
    ) -> anyhow::Result<RollbackToken> {
        let committed = self
            .prepared(policy, files)?
            .commit(files)
            .map_err(|error| anyhow::anyhow!("cgroup control not in effect: {error:?}"))?;
        Ok(RollbackToken::CgroupControlRestore {
            records: vec![committed.record()],
        })
    }
}

impl TuningAction for CgroupControlAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!(
            "cgroup-control:{}:{}",
            self.control.file_and_controller().0,
            self.control.render()
        ))
    }

    fn describe(&self) -> String {
        format!(
            "write {} = {} in {}",
            self.control.file_and_controller().0,
            self.control.render(),
            self.dir.display()
        )
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::HighRisk
    }

    fn descriptor(&self) -> crate::daemon_policy::ActionDescriptor {
        crate::daemon_policy::ActionDescriptor {
            action_kind: "cgroup_resource_control".to_owned(),
            action_id: self.id(),
            safety_class: self.safety_class(),
            effect_scope: crate::daemon_policy::ActionEffectScope::SystemWide,
            rollback: crate::daemon_policy::RollbackRequirement::RequiredBeforeApply,
            persistent_effect: false,
            touches_system_wide_state: true,
            requires_explicit_target: true,
            confidence: None,
        }
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_with_policy(&CgroupControlPolicy::default())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.prepared(&CgroupControlPolicy::default(), &RealFiles)
            .map(|_| ActionState {
                applied: false,
                affected_tasks: 0,
                checked_tasks: 0,
                pending_changes: 1,
                warnings: Vec::new(),
            })
    }

    fn apply(&self) -> crate::actions::ApplyResult {
        self.apply_with_policy(&CgroupControlPolicy::default())
            .map_err(Into::into)
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        let (file, _) = self.control.file_and_controller();
        let applied = std::fs::read_to_string(self.dir.join(file))
            .is_ok_and(|text| self.control.readback_matches(&text));
        Ok(ActionState {
            applied,
            affected_tasks: 0,
            checked_tasks: 0,
            pending_changes: usize::from(!applied),
            warnings: Vec::new(),
        })
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let RollbackToken::CgroupControlRestore { records } = token else {
            return Err(crate::actions::ActionError::invalid_rollback_token_kind(
                token.kind_error("cgroup-control-restore"),
            )
            .into());
        };
        for record in records {
            match restore_recorded(record, &RealFiles) {
                RestoreOutcome::Restored | RestoreOutcome::AlreadyRestored => {}
                other => {
                    return Err(ActionBoundaryError::restore_failed(
                        "cgroup_resource_control",
                        format!("{} not restored: {other:?}", record.dir.display()),
                    )
                    .into());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "control_action_tests.rs"]
mod tests;
