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
            action_kind: action_id.as_str().to_owned(),
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

impl<T> TuningAction for Box<T>
where
    T: TuningAction + ?Sized,
{
    fn id(&self) -> ActionId {
        self.as_ref().id()
    }

    fn describe(&self) -> String {
        self.as_ref().describe()
    }

    fn safety_class(&self) -> SafetyClass {
        self.as_ref().safety_class()
    }

    fn descriptor(&self) -> crate::daemon_policy::ActionDescriptor {
        self.as_ref().descriptor()
    }

    fn preflight(&self) -> ActionResult<Vec<ActionWarning>> {
        self.as_ref().preflight()
    }

    fn dry_run(&self) -> ActionResult<ActionState> {
        self.as_ref().dry_run()
    }

    fn apply(&self) -> ActionResult<RollbackToken> {
        self.as_ref().apply()
    }

    fn verify(&self) -> ActionResult<ActionState> {
        self.as_ref().verify()
    }

    fn rollback(&self, token: &RollbackToken) -> ActionResult<()> {
        self.as_ref().rollback(token)
    }
}
