//! Test modules for `actions::runner` split by audited-runner behavior area.
//!
//! Owns test module wiring and shared action-runner test fixtures.
//! Does not own production runner behavior.

mod audit;
mod hook_failures;
mod policy;
mod rollback;
mod timeout;

use std::{
    cell::{Cell, RefCell},
    fs,
    path::PathBuf,
};

use super::*;
use crate::actions::{
    ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass, TuningAction,
};

#[derive(Default)]
struct TestActionLog {
    events: RefCell<Vec<&'static str>>,
    mutated: Cell<bool>,
    rolled_back: Cell<bool>,
}

struct TestAction<'a> {
    should_fail_preflight: bool,
    should_fail_apply: bool,
    should_fail_verify: bool,
    should_fail_rollback: bool,
    affected_tasks: usize,
    log: &'a TestActionLog,
}

impl<'a> TestAction<'a> {
    fn new(log: &'a TestActionLog) -> Self {
        Self {
            should_fail_preflight: false,
            should_fail_apply: false,
            should_fail_verify: false,
            should_fail_rollback: false,
            affected_tasks: 5,
            log,
        }
    }

    fn with_preflight_failure(mut self) -> Self {
        self.should_fail_preflight = true;
        self
    }

    fn with_apply_failure(mut self) -> Self {
        self.should_fail_apply = true;
        self
    }

    fn with_verify_failure(mut self) -> Self {
        self.should_fail_verify = true;
        self
    }

    fn with_rollback_failure(mut self) -> Self {
        self.should_fail_rollback = true;
        self
    }

    fn with_affected_tasks(mut self, affected_tasks: usize) -> Self {
        self.affected_tasks = affected_tasks;
        self
    }
}

impl TuningAction for TestAction<'_> {
    fn id(&self) -> ActionId {
        ActionId::new("test-action".to_owned())
    }

    fn describe(&self) -> String {
        "test action".to_owned()
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::ReversibleLowRisk
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.log.events.borrow_mut().push("preflight");
        if self.should_fail_preflight {
            anyhow::bail!("preflight intentional failure");
        }
        Ok(vec![ActionWarning {
            message: "test preflight warning".to_owned(),
        }])
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.log.events.borrow_mut().push("dry_run");
        Ok(ActionState {
            applied: false,
            affected_tasks: self.affected_tasks,
            checked_tasks: self.affected_tasks,
            pending_changes: self.affected_tasks,
            warnings: vec![],
        })
    }

    fn apply(&self) -> crate::actions::ApplyResult {
        let res = (|| {
            self.log.events.borrow_mut().push("apply");
            if self.should_fail_apply {
                anyhow::bail!("apply intentional failure");
            }
            self.log.mutated.set(true);
            Ok(RollbackToken::CpuAffinityRestoreFile {
                path: PathBuf::from("/tmp/restore"),
                affected_tasks: self.affected_tasks,
            })
        })();
        res.map_err(Into::into)
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.log.events.borrow_mut().push("verify");
        if self.should_fail_verify {
            anyhow::bail!("verify intentional failure");
        }
        Ok(ActionState {
            applied: true,
            affected_tasks: self.affected_tasks,
            checked_tasks: self.affected_tasks,
            pending_changes: 0,
            warnings: vec![],
        })
    }

    fn rollback(&self, _token: &RollbackToken) -> anyhow::Result<()> {
        self.log.events.borrow_mut().push("rollback");
        if self.should_fail_rollback {
            anyhow::bail!("rollback intentional failure");
        }
        self.log.rolled_back.set(true);
        self.log.mutated.set(false);
        Ok(())
    }
}

fn apply_policy() -> ActionRunPolicy {
    ActionRunPolicy::apply_low_risk(ActionSource::Test, false)
}

fn dry_run_policy() -> ActionRunPolicy {
    ActionRunPolicy::dry_run(ActionSource::Test)
}

fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-actions-runner-test-{name}-{}-{}",
        std::process::id(),
        crate::audit::unix_nanos_now()
    ));
    // invariant: temp dir creation should always succeed in tests
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn all_capabilities_available() -> crate::daemon::capabilities::DaemonCapabilities {
    crate::daemon::capabilities::DaemonCapabilities {
        kernel_release: Some("6.9.1-test".to_owned()),
        btf_available: true,
        sched_tracepoints_available: true,
        perf_permissions_likely: true,
        perf_event_paranoid: Some(1),
        cgroup_v2_available: true,
        sched_ext_available: true,
        uclamp_available: true,
        ionice_available: true,
        irq_affinity_available: true,
        gpu_sysfs_available: true,
        privileged_worker_socket_reachable: Some(true),
    }
}

fn terminal_event(events: &[crate::audit::AuditEvent]) -> &crate::audit::AuditEvent {
    // invariant: tests expect at least one audit event
    events.last().expect("expected at least one audit event")
}

fn action_phases(events: &[crate::audit::AuditEvent]) -> Vec<Option<crate::actions::ActionPhase>> {
    events.iter().map(|event| event.action_phase).collect()
}
