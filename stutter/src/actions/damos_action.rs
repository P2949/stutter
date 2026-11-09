//! `DamosScheme` as a manual, policy-gated action (package W11c).
//!
//! Configures one Stutter-owned DAMOS scheme on an idle kdamond and tears exactly that
//! configuration down on rollback. Reclaim and LRU actions are irreversible in effect, so the
//! action is HighRisk, system-wide, never an executable autotune plan, and its recovery is manual
//! only. The default policy denies; irreversible actions additionally need acknowledgement.
use std::path::PathBuf;

use super::{
    ActionBoundaryError, ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass,
    TuningAction,
    damos_scheme::{DamosPrepared, DamosRequest, DamosRestore, prepare, teardown_recorded},
};

pub const DAMON_ADMIN_ROOT: &str = "/sys/kernel/mm/damon/admin";

#[derive(Clone, Debug, Default)]
pub struct DamosActionPolicy {
    /// Nothing is configured unless this is set explicitly.
    pub allow: bool,
}

#[derive(Clone, Debug)]
pub struct DamosSchemeAction {
    pub admin: PathBuf,
    pub request: DamosRequest,
}

impl DamosSchemeAction {
    pub fn new(request: DamosRequest) -> Self {
        Self {
            admin: PathBuf::from(DAMON_ADMIN_ROOT),
            request,
        }
    }

    fn prepared(&self, policy: &DamosActionPolicy) -> anyhow::Result<DamosPrepared> {
        if !policy.allow {
            return Err(ActionBoundaryError::PolicyDenied {
                action_kind: "damos_scheme",
                requirement: "allow_damos",
            }
            .into());
        }
        prepare(&self.admin, &self.request, |task| {
            crate::telemetry::proc_view::live_process(std::path::Path::new("/proc"), task)
                .map(|live| live.start_ticks)
        })
        .map_err(|error| anyhow::anyhow!("damos_scheme refused: {error:?}"))
    }

    pub fn preflight_with_policy(
        &self,
        policy: &DamosActionPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.prepared(policy)?;
        Ok(vec![ActionWarning {
            message: if self.request.action.irreversible() {
                "DAMOS reclaim or LRU actions change memory state that restoring the configuration cannot undo".to_owned()
            } else {
                "monitoring-only DAMOS scheme: no memory is reclaimed or moved".to_owned()
            },
        }])
    }

    pub fn apply_with_policy(&self, policy: &DamosActionPolicy) -> anyhow::Result<RollbackToken> {
        let committed = self
            .prepared(policy)?
            .commit()
            .map_err(|error| anyhow::anyhow!("damos scheme not in effect: {error:?}"))?;
        Ok(RollbackToken::DamosTeardown {
            records: vec![committed.record()],
        })
    }
}

impl TuningAction for DamosSchemeAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!("damos:scheme:task={}", self.request.task))
    }

    fn describe(&self) -> String {
        format!(
            "run a DAMOS scheme ({:?}) over one process's address space",
            self.request.action
        )
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::HighRisk
    }

    fn descriptor(&self) -> crate::daemon_policy::ActionDescriptor {
        crate::daemon_policy::ActionDescriptor {
            action_kind: "damos_scheme".to_owned(),
            action_id: self.id(),
            safety_class: self.safety_class(),
            effect_scope: crate::daemon_policy::ActionEffectScope::SystemWide,
            rollback: crate::daemon_policy::RollbackRequirement::RequiredBeforeApply,
            persistent_effect: self.request.action.irreversible(),
            touches_system_wide_state: true,
            requires_explicit_target: true,
            confidence: None,
        }
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_with_policy(&DamosActionPolicy::default())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.prepared(&DamosActionPolicy::default())
            .map(|_| ActionState {
                applied: false,
                affected_tasks: 0,
                checked_tasks: 0,
                pending_changes: 1,
                warnings: Vec::new(),
            })
    }

    fn apply(&self) -> crate::actions::ApplyResult {
        self.apply_with_policy(&DamosActionPolicy::default())
            .map_err(Into::into)
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        let running = std::fs::read_to_string(self.admin.join("kdamonds/0/state"))
            .is_ok_and(|text| text.trim() == "on");
        Ok(ActionState {
            applied: running,
            affected_tasks: 0,
            checked_tasks: 0,
            pending_changes: usize::from(!running),
            warnings: Vec::new(),
        })
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let RollbackToken::DamosTeardown { records } = token else {
            return Err(crate::actions::ActionError::invalid_rollback_token_kind(
                token.kind_error("damos-teardown"),
            )
            .into());
        };
        for record in records {
            match teardown_recorded(record) {
                DamosRestore::Cleared { .. } | DamosRestore::ClearedTargetReplaced { .. } => {}
                DamosRestore::ExternallyChanged => {
                    return Err(ActionBoundaryError::restore_failed(
                        "damos_scheme",
                        format!(
                            "{} no longer holds the recorded DAMOS configuration; left untouched",
                            record.kdamond.display()
                        ),
                    )
                    .into());
                }
                DamosRestore::Unproven => {
                    return Err(ActionBoundaryError::restore_failed(
                        "damos_scheme",
                        format!("could not prove {} was torn down", record.kdamond.display()),
                    )
                    .into());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "damos_action_tests.rs"]
mod tests;
