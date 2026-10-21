//! Real-kernel acceptance of the existing CPU-affinity action, with explicit
//! evidence paths. This does not bypass its policy, audited runner or rollback.
use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, ensure};
use serde_json::{Value, json};

use super::{
    ActionPhase, TuningAction,
    cpu_affinity::CpuAffinityProfileAction,
    runner::{ActionRunPolicy, run_audited_action_with_audit_path},
};

fn kernel_mask(tid: u32) -> anyhow::Result<crate::affinity::CpuMask> {
    let status = std::fs::read_to_string(format!("/proc/{tid}/status"))?;
    let cpus = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .context("kernel affinity status missing")?;
    crate::affinity::CpuMask::parse(cpus.trim())
}

fn readback(
    original: &BTreeMap<u32, (u64, crate::affinity::CpuMask)>,
    expected: Option<&crate::affinity::CpuMask>,
) -> anyhow::Result<()> {
    for (tid, (ticks, before)) in original {
        ensure!(
            crate::process_tree::process_starttime_at(Path::new("/proc"), *tid) == Some(*ticks),
            "acceptance task identity changed: {tid}"
        );
        ensure!(
            &kernel_mask(*tid)? == expected.unwrap_or(before),
            "kernel affinity readback differs for task {tid}"
        );
    }
    Ok(())
}

/// Scope must be a caller-owned disposable tree. Both evidence files must be
/// absent; original user restore/audit paths are never read or overwritten.
pub fn verify_cpu_affinity_lifecycle(
    tree_pid: u32,
    expected_starttime: u64,
    cpu: u32,
    restore_path: &Path,
    audit_path: &Path,
) -> anyhow::Result<Value> {
    ensure!(
        !restore_path.exists() && !audit_path.exists(),
        "acceptance state paths must be new"
    );
    ensure!(
        crate::process_tree::process_starttime_at(Path::new("/proc"), tree_pid)
            == Some(expected_starttime),
        "acceptance root identity mismatch"
    );
    let snapshot = crate::process_tree::target_snapshot(
        crate::process_tree::TargetSnapshotInput::default().tree_pids(&[tree_pid]),
    );
    ensure!(
        (3..=8).contains(&snapshot.tasks.len()),
        "acceptance expects a stable disposable tree of 3 to 8 tasks"
    );
    let desired = crate::affinity::CpuMask::parse(&cpu.to_string())?;
    let process_count = snapshot
        .tasks
        .values()
        .map(|task| task.process_id())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    ensure!(
        process_count == 3,
        "acceptance requires parent plus two disposable child processes"
    );
    ensure!(
        desired.is_subset_of(&crate::affinity::CpuMask::online_cpus()?),
        "selected CPU is offline"
    );
    let mut original = BTreeMap::new();
    let task_processes = snapshot
        .tasks
        .values()
        .map(|task| (task.task_id().as_u32(), task.process_id().as_u32()))
        .collect::<BTreeMap<_, _>>();
    for task in snapshot.tasks.values() {
        let tid = task.task_id().as_u32();
        let mask = kernel_mask(tid)?;
        ensure!(
            desired.is_subset_of(&mask) && mask != desired,
            "no observable permitted affinity change for task {tid}"
        );
        let ticks = crate::process_tree::process_starttime_at(Path::new("/proc"), tid)
            .context("task identity missing")?;
        original.insert(tid, (ticks, mask));
    }
    let action = CpuAffinityProfileAction {
        restore_path: Some(restore_path.to_owned()),
        tree_pid,
        force_restore_overwrite: false,
        profile: crate::profiles::Profile {
            name: "acceptance-disposable-affinity".into(),
            rules: vec![crate::profiles::ProfileRule {
                affinity: Some(desired.clone()),
                nice: None,
                ionice: None,
                match_class: vec![],
                match_comm: vec![crate::process_tree::CompiledPattern::new("/.*/".into())?],
            }],
        },
    };
    let mut dry_policy = ActionRunPolicy::dry_run(crate::daemon_policy::ActionSource::Cli);
    dry_policy.max_affected_tasks = Some(8);
    let dry = run_audited_action_with_audit_path(
        "affinity-acceptance-dry-run",
        &action,
        dry_policy,
        audit_path,
    )?;
    ensure!(
        dry.state.affected_tasks == original.len() && !dry.state.applied,
        "dry-run scope mismatch"
    );
    ensure!(!restore_path.exists(), "dry-run wrote restore state");
    readback(&original, None)?;
    let mut policy =
        ActionRunPolicy::apply_low_risk(crate::daemon_policy::ActionSource::Cli, false);
    policy.max_affected_tasks = Some(8);
    policy.max_total_duration = Some(std::time::Duration::from_secs(10));
    let applied = run_audited_action_with_audit_path(
        "affinity-acceptance-apply",
        &action,
        policy,
        audit_path,
    )?;
    let token = super::RollbackToken::CpuAffinityRestoreFile {
        path: restore_path.to_owned(),
        affected_tasks: original.len(),
    };
    // All post-apply assertions stay inside this closure so rollback runs even
    // when evidence validation fails.
    let verified = (|| -> anyhow::Result<()> {
        ensure!(
            applied
                .rollback
                .as_ref()
                .and_then(|token| token.as_cpu_affinity_restore_file())
                == Some((restore_path, original.len())),
            "applied action omitted or changed rollback token"
        );
        ensure!(
            applied.state.applied && applied.state.checked_tasks == original.len(),
            "action did not verify complete scope"
        );
        let state = crate::profile_restore::load_restore_state(restore_path)?;
        ensure!(
            state.affinity_records.len() == original.len(),
            "durable rollback record count differs"
        );
        for record in &state.affinity_records {
            let (ticks, mask) = original
                .get(&record.tid.as_u32())
                .context("unexpected task in rollback journal")?;
            ensure!(
                record.task_starttime_ticks == Some(*ticks)
                    && &record.original_mask == mask
                    && record.applied_mask == desired,
                "rollback state identity or original affinity differs"
            );
            ensure!(
                record.process_pid.map(|pid| pid.as_u32())
                    == task_processes.get(&record.tid.as_u32()).copied(),
                "rollback process identity differs"
            );
        }
        let mut archive = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(restore_path.with_extension("before-rollback.json"))?;
        serde_json::to_writer_pretty(&mut archive, &state)?;
        archive.sync_all()?;
        readback(&original, Some(&desired))
    })();
    let rollback = action.rollback(&token);
    let restored = readback(&original, None);
    let mut events = crate::audit::read_audit_tail(audit_path, 100)?;
    let mut event = events.last().cloned().context("action audit missing")?;
    event.unix_nanos = crate::audit::unix_nanos_now();
    event.command = "affinity-acceptance-rollback".into();
    event.action_phase = Some(ActionPhase::Rollback);
    event.success = rollback.is_ok() && restored.is_ok();
    event.message = format!("rollback={rollback:?}; independent_kernel_restoration={restored:?}");
    crate::audit::append_audit_event_to_path(audit_path, &event)?;
    rollback?;
    restored?;
    verified?;
    event.command = "affinity-acceptance-final".into();
    event.action_phase = None;
    event.success = true;
    event.message =
        "original masks and task identities independently verified after rollback".into();
    crate::audit::append_audit_event_to_path(audit_path, &event)?;
    events = crate::audit::read_audit_tail(audit_path, 100)?;
    for phase in [
        ActionPhase::Preflight,
        ActionPhase::DryRun,
        ActionPhase::Apply,
        ActionPhase::Verify,
        ActionPhase::Rollback,
    ] {
        ensure!(
            events
                .iter()
                .any(|event| event.action_phase == Some(phase) && event.success),
            "missing successful {phase:?} audit event"
        );
    }
    Ok(
        json!({"target_pid":tree_pid,"target_starttime_ticks":expected_starttime,"tasks":original.iter().map(|(tid,(ticks,mask))|json!({"tid":tid,"process_pid":task_processes.get(tid),"starttime_ticks":ticks,"original_mask":mask.to_range_string(),"applied_mask":desired.to_range_string(),"restored_mask":mask.to_range_string()})).collect::<Vec<_>>(),
        "process_count":process_count,"audit_events":events.len(),"dry_run_unchanged":true,"durable_rollback_verified":true,"independent_kernel_restore_verified":true}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_root_is_refused_before_state_or_kernel_mutation() {
        let root = crate::test_support::TestRoot::new("affinity-acceptance-stale");
        let restore = root.join("restore.json");
        let audit = root.join("audit.jsonl");
        let pid = std::process::id();
        let before = kernel_mask(pid).unwrap();
        assert!(verify_cpu_affinity_lifecycle(pid, 0, 0, &restore, &audit).is_err());
        assert_eq!(kernel_mask(pid).unwrap(), before);
        assert!(!restore.exists() && !audit.exists());
    }
    #[test]
    fn existing_restore_state_is_preserved_and_never_loaded_as_acceptance_state() {
        let root = crate::test_support::TestRoot::new("affinity-acceptance-existing");
        let restore = root.join("restore.json");
        let audit = root.join("audit.jsonl");
        std::fs::write(&restore, b"original sentinel state").unwrap();
        assert!(verify_cpu_affinity_lifecycle(std::process::id(), 0, 0, &restore, &audit).is_err());
        assert_eq!(std::fs::read(restore).unwrap(), b"original sentinel state");
        assert!(!audit.exists());
    }
}
