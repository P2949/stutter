use crate::actions::{
    error::ActionResult,
    model::{ActionId, ActionState, ActionWarning, SafetyClass},
    token::RollbackToken,
};

pub trait TuningAction {
    fn id(&self) -> ActionId;
    fn describe(&self) -> String;
    fn safety_class(&self) -> SafetyClass;

    fn descriptor(&self) -> crate::daemon_policy::ActionDescriptor {
        let action_id = self.id();
        crate::daemon_policy::ActionDescriptor {
            action_kind: action_id.0.clone(),
            action_id,
            safety_class: self.safety_class(),
            effect_scope: crate::daemon_policy::ActionEffectScope::LocalProcessTree,
            rollback: crate::daemon_policy::RollbackRequirement::RequiredBeforeApply,
            persistent_effect: false,
            touches_system_wide_state: false,
            requires_explicit_target: true,
            confidence: None,
        }
    }

    fn preflight(&self) -> ActionResult<Vec<ActionWarning>>;
    fn dry_run(&self) -> ActionResult<ActionState>;
    fn apply(&self) -> ActionResult<RollbackToken>;
    fn verify(&self) -> ActionResult<ActionState>;
    fn rollback(&self, token: &RollbackToken) -> ActionResult<()>;
}
