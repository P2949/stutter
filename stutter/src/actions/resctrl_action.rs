//! `ResctrlCacheAllocation` as a manual, policy-gated action (package W11c).
//!
//! Creates a Stutter-owned resctrl group with a validated L3 capacity bitmask and removes exactly
//! that group on rollback. HighRisk and system-wide (shared cache), never an executable autotune
//! plan, never autonomous; the default policy denies. Tasks are not assigned here.
use std::path::PathBuf;

use super::{
    ActionBoundaryError, ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass,
    TuningAction,
    resctrl_alloc::{CatPrepared, CatRequest, prepare},
    resctrl_rollback::remove_all,
};

pub const RESCTRL_ROOT: &str = "/sys/fs/resctrl";

#[derive(Clone, Debug, Default)]
pub struct ResctrlActionPolicy {
    /// Nothing is created unless this is set explicitly.
    pub allow: bool,
}

#[derive(Clone, Debug)]
pub struct ResctrlCatAction {
    pub root: PathBuf,
    pub request: CatRequest,
}

impl ResctrlCatAction {
    pub fn new(name: impl Into<String>, mask: u64) -> Self {
        Self {
            root: PathBuf::from(RESCTRL_ROOT),
            request: CatRequest {
                name: name.into(),
                mask,
            },
        }
    }

    fn prepared(&self, policy: &ResctrlActionPolicy) -> anyhow::Result<CatPrepared> {
        if !policy.allow {
            return Err(ActionBoundaryError::PolicyDenied {
                action_kind: "resctrl_cache_allocation",
                requirement: "allow_resctrl",
            }
            .into());
        }
        prepare(&self.root, &self.request)
            .map_err(|error| anyhow::anyhow!("resctrl_cache_allocation refused: {error:?}"))
    }

    pub fn preflight_with_policy(
        &self,
        policy: &ResctrlActionPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.prepared(policy)?;
        Ok(vec![ActionWarning {
            message: "resctrl allocation is shared-cache state for the whole host; only the owned group is created and removed".to_owned(),
        }])
    }

    pub fn apply_with_policy(&self, policy: &ResctrlActionPolicy) -> anyhow::Result<RollbackToken> {
        let committed = self
            .prepared(policy)?
            .commit()
            .map_err(|error| anyhow::anyhow!("resctrl group not in effect: {error:?}"))?;
        Ok(RollbackToken::ResctrlGroupRemove {
            records: vec![committed.record()],
        })
    }
}

impl TuningAction for ResctrlCatAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!(
            "resctrl:cat:{}:mask={:x}",
            self.request.name, self.request.mask
        ))
    }

    fn describe(&self) -> String {
        format!(
            "create resctrl group {} with L3 capacity mask {:#x}",
            self.request.name, self.request.mask
        )
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::HighRisk
    }

    fn descriptor(&self) -> crate::daemon_policy::ActionDescriptor {
        crate::daemon_policy::ActionDescriptor {
            action_kind: "resctrl_cache_allocation".to_owned(),
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
        self.preflight_with_policy(&ResctrlActionPolicy::default())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.prepared(&ResctrlActionPolicy::default())
            .map(|_| ActionState {
                applied: false,
                affected_tasks: 0,
                checked_tasks: 0,
                pending_changes: 1,
                warnings: Vec::new(),
            })
    }

    fn apply(&self) -> crate::actions::ApplyResult {
        self.apply_with_policy(&ResctrlActionPolicy::default())
            .map_err(Into::into)
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        let group = self.root.join(format!("stutter-cat-{}", self.request.name));
        Ok(ActionState {
            applied: group.join("schemata").is_file(),
            affected_tasks: 0,
            checked_tasks: 0,
            pending_changes: usize::from(!group.join("schemata").is_file()),
            warnings: Vec::new(),
        })
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let RollbackToken::ResctrlGroupRemove { records } = token else {
            return Err(crate::actions::ActionError::invalid_rollback_token_kind(
                token.kind_error("resctrl-group-remove"),
            )
            .into());
        };
        let check = remove_all(records);
        anyhow::ensure!(
            check.unproven == 0 && check.replaced == 0,
            "resctrl removal not proven: {}",
            check.messages.join("; ")
        );
        Ok(())
    }
}

#[cfg(test)]
#[path = "resctrl_action_tests.rs"]
mod tests;
