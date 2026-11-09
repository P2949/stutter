//! Device-wide block I/O scheduler selection as a manual, policy-gated action (package W11e).
//!
//! HighRisk and system-wide, never an executable autotune plan, never autonomous. The default
//! policy denies; a device must be explicitly authorized as disposable and must not be protected.
use std::path::PathBuf;

use super::{
    ActionBoundaryError, ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass,
    TuningAction,
    block_scheduler::{BlockSchedulerPrepared, BlockSchedulerRequest, RealBlockFiles, prepare},
    block_scheduler_rollback::{SYS_BLOCK, restore_all},
};

#[derive(Clone, Debug, Default)]
pub struct BlockSchedulerPolicy {
    /// Nothing is changed unless this is set explicitly.
    pub allow: bool,
}

#[derive(Clone, Debug)]
pub struct BlockSchedulerAction {
    pub sys_block: PathBuf,
    pub request: BlockSchedulerRequest,
}

impl BlockSchedulerAction {
    pub fn new(request: BlockSchedulerRequest) -> Self {
        Self {
            sys_block: PathBuf::from(SYS_BLOCK),
            request,
        }
    }

    fn prepared(&self, policy: &BlockSchedulerPolicy) -> anyhow::Result<BlockSchedulerPrepared> {
        if !policy.allow {
            return Err(ActionBoundaryError::PolicyDenied {
                action_kind: "block_io_scheduler",
                requirement: "allow_block_scheduler",
            }
            .into());
        }
        prepare(&self.sys_block, &self.request)
            .map_err(|error| anyhow::anyhow!("block_io_scheduler refused: {error:?}"))
    }

    pub fn preflight_with_policy(
        &self,
        policy: &BlockSchedulerPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.prepared(policy)?;
        Ok(vec![ActionWarning {
            message: "a block scheduler change is device-wide and affects every task using the device; only an authorized, unprotected device is changed".to_owned(),
        }])
    }

    pub fn apply_with_policy(
        &self,
        policy: &BlockSchedulerPolicy,
    ) -> anyhow::Result<RollbackToken> {
        let committed = self
            .prepared(policy)?
            .commit(&RealBlockFiles)
            .map_err(|error| anyhow::anyhow!("block scheduler not in effect: {error:?}"))?;
        Ok(RollbackToken::BlockSchedulerRestore {
            records: vec![committed.record()],
        })
    }
}

impl TuningAction for BlockSchedulerAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!(
            "block-scheduler:{}:{}",
            self.request.device, self.request.scheduler
        ))
    }

    fn describe(&self) -> String {
        format!(
            "select the {} I/O scheduler on block device {}",
            self.request.scheduler, self.request.device
        )
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::HighRisk
    }

    fn descriptor(&self) -> crate::daemon_policy::ActionDescriptor {
        crate::daemon_policy::ActionDescriptor {
            action_kind: "block_io_scheduler".to_owned(),
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
        self.preflight_with_policy(&BlockSchedulerPolicy::default())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.prepared(&BlockSchedulerPolicy::default())
            .map(|_| ActionState {
                applied: false,
                affected_tasks: 0,
                checked_tasks: 0,
                pending_changes: 1,
                warnings: Vec::new(),
            })
    }

    fn apply(&self) -> crate::actions::ApplyResult {
        self.apply_with_policy(&BlockSchedulerPolicy::default())
            .map_err(Into::into)
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        let queue = self
            .sys_block
            .join(&self.request.device)
            .join("queue/scheduler");
        let applied = std::fs::read_to_string(queue)
            .is_ok_and(|text| text.contains(&format!("[{}]", self.request.scheduler)));
        Ok(ActionState {
            applied,
            affected_tasks: 0,
            checked_tasks: 0,
            pending_changes: usize::from(!applied),
            warnings: Vec::new(),
        })
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let RollbackToken::BlockSchedulerRestore { records } = token else {
            return Err(crate::actions::ActionError::invalid_rollback_token_kind(
                token.kind_error("block-scheduler-restore"),
            )
            .into());
        };
        let check = restore_all(records);
        if check.unproven != 0 || check.replaced != 0 || check.foreign != 0 {
            return Err(ActionBoundaryError::restore_failed(
                "block_io_scheduler",
                check.messages.join("; "),
            )
            .into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "block_scheduler_action_tests.rs"]
mod tests;
