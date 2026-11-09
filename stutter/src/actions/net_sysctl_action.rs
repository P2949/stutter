//! A typed network sysctl write as a manual, policy-gated action (package W11f). HighRisk and
//! system-wide, never an executable autotune plan, never autonomous; the default policy denies and
//! a setting owned by bpftune is `UnsupportedDelegation`.
use std::path::PathBuf;

use super::{
    ActionBoundaryError, ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass,
    TuningAction,
    net_sysctl::{NetKnob, NetPrepared, prepare},
    net_sysctl_rollback::restore_all,
};

#[derive(Clone, Debug, Default)]
pub struct NetSysctlPolicy {
    /// Nothing is written unless this is set explicitly.
    pub allow: bool,
    /// A controller (bpftune) that owns network sysctls here, from the owner probe.
    pub external_owner: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NetSysctlAction {
    pub sys_root: PathBuf,
    pub proc_root: PathBuf,
    pub knob: NetKnob,
}

impl NetSysctlAction {
    pub fn new(knob: NetKnob) -> Self {
        Self {
            sys_root: PathBuf::from("/proc/sys"),
            proc_root: PathBuf::from("/proc"),
            knob,
        }
    }

    fn prepared(&self, policy: &NetSysctlPolicy) -> anyhow::Result<NetPrepared> {
        if !policy.allow {
            return Err(ActionBoundaryError::PolicyDenied {
                action_kind: "net_sysctl",
                requirement: "allow_net_sysctl",
            }
            .into());
        }
        prepare(
            &self.sys_root,
            &self.proc_root,
            &self.knob,
            policy.external_owner.as_deref(),
        )
        .map_err(|error| anyhow::anyhow!("net_sysctl refused: {error:?}"))
    }

    pub fn preflight_with_policy(
        &self,
        policy: &NetSysctlPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.prepared(policy)?;
        Ok(vec![ActionWarning {
            message: "a network sysctl is namespace-wide state; only the named value is written and restored in its own namespace".to_owned(),
        }])
    }

    pub fn apply_with_policy(&self, policy: &NetSysctlPolicy) -> anyhow::Result<RollbackToken> {
        let committed = self
            .prepared(policy)?
            .commit()
            .map_err(|error| anyhow::anyhow!("net sysctl not in effect: {error:?}"))?;
        Ok(RollbackToken::NetSysctlRestore {
            records: vec![committed.record()],
        })
    }
}

impl TuningAction for NetSysctlAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!("net-sysctl:{:?}", self.knob))
    }

    fn describe(&self) -> String {
        format!("set a network sysctl ({:?})", self.knob)
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::HighRisk
    }

    fn descriptor(&self) -> crate::daemon_policy::ActionDescriptor {
        crate::daemon_policy::ActionDescriptor {
            action_kind: "net_sysctl".to_owned(),
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
        self.preflight_with_policy(&NetSysctlPolicy::default())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.prepared(&NetSysctlPolicy::default())
            .map(|_| ActionState {
                applied: false,
                affected_tasks: 0,
                checked_tasks: 0,
                pending_changes: 1,
                warnings: Vec::new(),
            })
    }

    fn apply(&self) -> crate::actions::ApplyResult {
        self.apply_with_policy(&NetSysctlPolicy::default())
            .map_err(Into::into)
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        let applied = std::fs::read_to_string(self.sys_root.join(self.knob.file()))
            .is_ok_and(|text| super::net_sysctl::same_value(&text, &self.knob.render()));
        Ok(ActionState {
            applied,
            affected_tasks: 0,
            checked_tasks: 0,
            pending_changes: usize::from(!applied),
            warnings: Vec::new(),
        })
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let RollbackToken::NetSysctlRestore { records } = token else {
            return Err(crate::actions::ActionError::invalid_rollback_token_kind(
                token.kind_error("net-sysctl-restore"),
            )
            .into());
        };
        let check = restore_all(records);
        if check.unproven != 0 || check.replaced != 0 || check.foreign != 0 {
            return Err(ActionBoundaryError::restore_failed(
                "net_sysctl",
                check.messages.join("; "),
            )
            .into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "net_sysctl_action_tests.rs"]
mod tests;
