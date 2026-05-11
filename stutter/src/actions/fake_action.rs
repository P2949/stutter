#![cfg(test)]
#![allow(dead_code)]

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use super::{ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass, TuningAction};
use crate::daemon_policy::{ActionDescriptor, ActionEffectScope, RollbackRequirement};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FakeActionSwitches {
    pub fail_preflight: bool,
    pub fail_apply: bool,
    pub fail_verify: bool,
    pub fail_rollback: bool,
    pub slow_apply: bool,
}

#[derive(Clone, Debug)]
pub struct FakeAction {
    action_id: ActionId,
    description: String,
    safety_class: SafetyClass,
    effect_scope: ActionEffectScope,
    rollback: RollbackRequirement,
    persistent_effect: bool,
    touches_system_wide_state: bool,
    requires_explicit_target: bool,
    confidence: Option<f32>,
    affected_tasks: usize,
    switches: FakeActionSwitches,
    slow_apply_duration: Duration,
    restore_path: PathBuf,
    state: Arc<FakeActionRuntimeState>,
}

#[derive(Debug, Default)]
struct FakeActionRuntimeState {
    events: Mutex<Vec<&'static str>>,
    applied: AtomicBool,
    rolled_back: AtomicBool,
}

impl Default for FakeAction {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeAction {
    pub fn new() -> Self {
        Self {
            action_id: ActionId("fake-action".to_owned()),
            description: "fake action".to_owned(),
            safety_class: SafetyClass::ReversibleLowRisk,
            effect_scope: ActionEffectScope::LocalProcessTree,
            rollback: RollbackRequirement::RequiredBeforeApply,
            persistent_effect: false,
            touches_system_wide_state: false,
            requires_explicit_target: true,
            confidence: None,
            affected_tasks: 5,
            switches: FakeActionSwitches::default(),
            slow_apply_duration: Duration::from_millis(25),
            restore_path: PathBuf::from("/tmp/stutter-fake-action-restore.json"),
            state: Arc::new(FakeActionRuntimeState::default()),
        }
    }

    pub fn with_switches(mut self, switches: FakeActionSwitches) -> Self {
        self.switches = switches;
        self
    }

    pub fn with_fail_preflight(mut self) -> Self {
        self.switches.fail_preflight = true;
        self
    }

    pub fn with_fail_apply(mut self) -> Self {
        self.switches.fail_apply = true;
        self
    }

    pub fn with_fail_verify(mut self) -> Self {
        self.switches.fail_verify = true;
        self
    }

    pub fn with_fail_rollback(mut self) -> Self {
        self.switches.fail_rollback = true;
        self
    }

    pub fn with_slow_apply(mut self) -> Self {
        self.switches.slow_apply = true;
        self
    }

    pub fn with_slow_apply_duration(mut self, duration: Duration) -> Self {
        self.slow_apply_duration = duration;
        self
    }

    pub fn with_affected_tasks(mut self, affected_tasks: usize) -> Self {
        self.affected_tasks = affected_tasks;
        self
    }

    pub fn with_safety_class(mut self, safety_class: SafetyClass) -> Self {
        self.safety_class = safety_class;
        self
    }

    pub fn with_effect_scope(mut self, effect_scope: ActionEffectScope) -> Self {
        self.effect_scope = effect_scope;
        self
    }

    pub fn with_rollback(mut self, rollback: RollbackRequirement) -> Self {
        self.rollback = rollback;
        self
    }

    pub fn with_persistent_effect(mut self, persistent_effect: bool) -> Self {
        self.persistent_effect = persistent_effect;
        self
    }

    pub fn with_system_wide_state(mut self) -> Self {
        self.touches_system_wide_state = true;
        self
    }

    pub fn with_requires_explicit_target(mut self, requires_explicit_target: bool) -> Self {
        self.requires_explicit_target = requires_explicit_target;
        self
    }

    pub fn with_confidence(mut self, confidence: Option<f32>) -> Self {
        self.confidence = confidence;
        self
    }

    pub fn with_action_id(mut self, action_id: impl Into<String>) -> Self {
        self.action_id = ActionId(action_id.into());
        self
    }

    pub fn with_restore_path(mut self, restore_path: PathBuf) -> Self {
        self.restore_path = restore_path;
        self
    }

    pub fn events(&self) -> Vec<&'static str> {
        self.state
            .events
            .lock()
            .expect("fake action events lock poisoned")
            .clone()
    }

    pub fn applied(&self) -> bool {
        self.state.applied.load(Ordering::SeqCst)
    }

    pub fn rolled_back(&self) -> bool {
        self.state.rolled_back.load(Ordering::SeqCst)
    }

    fn push_event(&self, event: &'static str) {
        self.state
            .events
            .lock()
            .expect("fake action events lock poisoned")
            .push(event);
    }

    fn action_state(&self, applied: bool, pending_changes: usize) -> ActionState {
        ActionState {
            applied,
            affected_tasks: self.affected_tasks,
            checked_tasks: self.affected_tasks,
            pending_changes,
            warnings: Vec::new(),
        }
    }
}

impl TuningAction for FakeAction {
    fn id(&self) -> ActionId {
        self.action_id.clone()
    }

    fn describe(&self) -> String {
        self.description.clone()
    }

    fn safety_class(&self) -> SafetyClass {
        self.safety_class.clone()
    }

    fn descriptor(&self) -> ActionDescriptor {
        ActionDescriptor {
            action_id: self.id(),
            action_kind: self.action_id.0.clone(),
            safety_class: self.safety_class.clone(),
            effect_scope: self.effect_scope,
            rollback: self.rollback,
            persistent_effect: self.persistent_effect,
            touches_system_wide_state: self.touches_system_wide_state,
            requires_explicit_target: self.requires_explicit_target,
            confidence: self.confidence,
        }
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.push_event("preflight");

        if self.switches.fail_preflight {
            anyhow::bail!("fake preflight failure");
        }

        Ok(vec![ActionWarning {
            message: "fake preflight warning".to_owned(),
        }])
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.push_event("dry_run");
        Ok(self.action_state(false, self.affected_tasks))
    }

    fn apply(&self) -> anyhow::Result<RollbackToken> {
        self.push_event("apply");

        if self.switches.fail_apply {
            anyhow::bail!("fake apply failure");
        }

        if self.switches.slow_apply {
            self.push_event("slow_apply");
            thread::sleep(self.slow_apply_duration);
        }

        self.state.applied.store(true, Ordering::SeqCst);

        Ok(RollbackToken::CpuAffinityRestoreFile {
            path: self.restore_path.clone(),
            affected_tasks: self.affected_tasks,
        })
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.push_event("verify");

        if self.switches.fail_verify {
            anyhow::bail!("fake verify failure");
        }

        Ok(self.action_state(true, 0))
    }

    fn rollback(&self, _token: &RollbackToken) -> anyhow::Result<()> {
        self.push_event("rollback");

        if self.switches.fail_rollback {
            anyhow::bail!("fake rollback failure");
        }

        self.state.rolled_back.store(true, Ordering::SeqCst);
        self.state.applied.store(false, Ordering::SeqCst);
        Ok(())
    }
}
