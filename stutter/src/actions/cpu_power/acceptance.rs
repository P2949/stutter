//! Real-kernel acceptance of the CPU-power (EPP) transaction against the per-task `cpufreq`
//! fixture of the acceptance case (see `task_lifecycle`); `/sys` itself is never written here.
use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};
use serde_json::Value;

use super::CpuPowerAction;
use crate::{
    actions::{
        RollbackToken, TaskIdentity, TuningAction,
        task_lifecycle::{LifecycleSpec, Originals, verify_task_lifecycle},
    },
    autotune::{
        acceptance_mutation::{AcceptanceMutation, acceptance_policy},
        objective::ObjectiveKind,
        planning::{candidate::CandidateAction, executable_plan::CpuPowerActionPlan},
    },
};

fn epp_file(root: &Path, tid: u32) -> PathBuf {
    root.join(format!(
        "devices/system/cpu/cpu{tid}/cpufreq/energy_performance_preference"
    ))
}

fn check_token(root: &Path, token: &RollbackToken, original: &Originals) -> anyhow::Result<()> {
    let records = token
        .as_cpu_power_restore()
        .context("rollback token is not a CPU power restore")?;
    ensure!(
        records.len() == original.len(),
        "durable rollback record count differs"
    );
    for (tid, (_, before)) in original {
        let record = records
            .iter()
            .find(|record| record.path == epp_file(root, *tid))
            .with_context(|| format!("no record for the fixture of task {tid}"))?;
        ensure!(
            record.original_value == *before && record.identity.is_some(),
            "rollback record value or cpufreq identity differs for {tid}"
        );
    }
    Ok(())
}

/// Scope must be a caller-owned disposable tree and a prepared fixture; the audit file must be new.
pub fn verify_cpu_power_lifecycle(
    fixture_root: &Path,
    tree_pid: u32,
    expected_starttime: u64,
    audit_path: &Path,
) -> anyhow::Result<Value> {
    let root = fixture_root.to_path_buf();
    let ids: Vec<u32> = crate::process_tree::target_snapshot(
        crate::process_tree::TargetSnapshotInput::default().tree_pids(&[tree_pid]),
    )
    .tasks
    .values()
    .map(|task| task.task_id().as_u32())
    .collect();
    crate::autotune::acceptance_mutation::prepare_acceptance_sysfs(&root, &ids)?;
    let source = crate::daemon_policy::ActionSource::Cli;
    let mut apply_policy =
        crate::actions::runner::ActionRunPolicy::apply_medium_risk(source, false);
    apply_policy.policy = acceptance_policy(AcceptanceMutation::CpuPowerEpp);
    let build_root = root.clone();
    let token_root = root.clone();
    let read_root = root.clone();
    verify_task_lifecycle(LifecycleSpec {
        label: "cpu-power",
        tree_pid,
        expected_starttime,
        audit_path,
        apply_policy,
        build: &move |targets: Vec<TaskIdentity>| -> anyhow::Result<Box<dyn TuningAction>> {
            let action = CpuPowerAction {
                sysfs_root: build_root.clone(),
                cpus: targets.iter().map(|t| t.tid.as_u32()).collect(),
                scaling_governor: None,
                energy_performance_preference: Some("performance".to_owned()),
            };
            let candidate = CandidateAction::CpuPower {
                plan: CpuPowerActionPlan {
                    name: "acceptance-cpu-power".to_owned(),
                    action,
                    evidence: Vec::new(),
                    objective: ObjectiveKind::GameRunnableLatency,
                },
            };
            crate::actions::default_action_factory_registry()
                .build(&candidate)
                .map_err(anyhow::Error::new)
        },
        read: &move |tid| {
            Ok(std::fs::read_to_string(epp_file(&read_root, tid))?
                .trim()
                .to_owned())
        },
        applied: &|_| Ok("performance".to_owned()),
        check_token: &move |token, original| check_token(&token_root, token, original),
    })
}
