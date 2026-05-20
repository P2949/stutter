//! Test modules for `autotune::apply_low_risk` split by low-risk apply behavior area.
//!
//! Owns test module wiring and shared apply-low-risk test fixtures.
//! Does not own production apply-low-risk behavior.

mod audit;
mod experiment_resolution;
mod policy;
mod rollback;
mod target_resolution;

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::*;
use crate::{
    actions::{
        ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass, TuningAction,
        cpu_affinity::CpuAffinityProfileAction, runner::ActionRunPolicy,
    },
    autotune::candidate::CandidateDryRunRecord,
    profiles::Profile,
};

#[derive(Default)]
struct FakeExecutor {
    candidate_name: String,
    action_kind: &'static str,
    safety_class: SafetyClass,
    dry_run_record: Option<CandidateDryRunRecord>,
    apply_token: Option<RollbackToken>,
    dry_run_calls: usize,
    apply_calls: usize,
    rollback_calls: usize,
}

impl FakeExecutor {
    fn low_risk() -> Self {
        Self {
            candidate_name: "game-main".to_owned(),
            action_kind: "cpu_affinity_profile",
            safety_class: SafetyClass::ReversibleLowRisk,
            dry_run_record: Some(CandidateDryRunRecord {
                candidate_name: "game-main".to_owned(),
                affected_tasks: 31,
                warnings: Vec::new(),
                safety_class: SafetyClass::ReversibleLowRisk,
                eligible: true,
                reason: None,
            }),
            apply_token: Some(RollbackToken::CpuAffinityRestoreFile {
                path: PathBuf::from("/tmp/stutter-test-restore.json"),
                affected_tasks: 31,
            }),
            dry_run_calls: 0,
            apply_calls: 0,
            rollback_calls: 0,
        }
    }
}

impl LowRiskActionExecutor for FakeExecutor {
    fn candidate_name(&self) -> &str {
        &self.candidate_name
    }

    fn action_kind(&self) -> &'static str {
        self.action_kind
    }

    fn safety_class(&self) -> SafetyClass {
        self.safety_class.clone()
    }

    fn dry_run(&mut self) -> anyhow::Result<CandidateDryRunRecord> {
        self.dry_run_calls += 1;
        let record = self.dry_run_record.clone();
        // invariant: fake dry-run record must be configured in tests
        let record = record.expect("fake dry-run record must be configured");
        Ok(record)
    }

    fn apply(&mut self) -> anyhow::Result<RollbackToken> {
        self.apply_calls += 1;
        let token = self.apply_token.clone();
        // invariant: fake rollback token must be configured in tests
        let token = token.expect("fake rollback token must be configured");
        Ok(token)
    }

    fn rollback(&mut self, _token: &RollbackToken) -> anyhow::Result<()> {
        self.rollback_calls += 1;
        Ok(())
    }
}

struct TestAction {
    id: &'static str,
    safety_class: SafetyClass,
    should_fail_apply: bool,
    should_fail_verify: bool,
    affected_tasks: usize,
}

impl TuningAction for TestAction {
    fn id(&self) -> ActionId {
        ActionId::new(self.id.to_owned())
    }

    fn describe(&self) -> String {
        "test action".to_owned()
    }

    fn safety_class(&self) -> SafetyClass {
        self.safety_class.clone()
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        Ok(Vec::new())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        Ok(ActionState {
            applied: false,
            affected_tasks: self.affected_tasks,
            checked_tasks: self.affected_tasks,
            pending_changes: self.affected_tasks,
            warnings: Vec::new(),
        })
    }

    fn apply(&self) -> crate::actions::ApplyResult {
        let res = (|| {
            if self.should_fail_apply {
                anyhow::bail!("intentional apply failure");
            }

            Ok(RollbackToken::CpuAffinityRestoreFile {
                path: PathBuf::from("/tmp/stutter-test-restore.json"),
                affected_tasks: self.affected_tasks,
            })
        })();
        res.map_err(Into::into)
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        if self.should_fail_verify {
            anyhow::bail!("intentional verify failure");
        }

        Ok(ActionState {
            applied: true,
            affected_tasks: self.affected_tasks,
            checked_tasks: self.affected_tasks,
            pending_changes: 0,
            warnings: Vec::new(),
        })
    }

    fn rollback(&self, _token: &RollbackToken) -> anyhow::Result<()> {
        Ok(())
    }
}

fn apply_policy() -> ActionRunPolicy {
    ActionRunPolicy::apply_low_risk(crate::daemon_policy::ActionSource::Test, false)
}

fn test_cpu_affinity_profile_action() -> CpuAffinityProfileAction {
    CpuAffinityProfileAction {
        tree_pid: 0,
        profile: Profile {
            name: "game-main".to_owned(),
            rules: Vec::new(),
        },
        force_restore_overwrite: false,
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-autotune-low-risk-test-{name}-{}-{}",
        std::process::id(),
        crate::audit::unix_nanos_now()
    ));
    // invariant: creating temp dir should succeed in tests
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn fake_proc_with_comm(proc_root: &Path, pid: u32, comm: &str) {
    let dir = proc_root.join(pid.to_string());
    // invariant: creating fake proc dir should succeed in tests
    fs::create_dir_all(&dir).unwrap();
    // invariant: writing fake comm file should succeed in tests
    fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
}
