//! Controller failures that are *not* process death: a real syscall error mid-apply,
//! failed verification after real mutation, and failed rollback. The controller
//! exits on its own; every conclusion is re-read from the kernel and the files.
use super::{
    case::{
        archive, saved_state, start_actor, start_workload, token_tasks, verify_recovery_evidence,
    },
    *,
};

fn last_json_line(path: &Path) -> anyhow::Result<Value> {
    let line = fs::read_to_string(path)?
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .with_context(|| format!("{} is empty", path.display()))?;
    Ok(serde_json::from_str(&line)?)
}

pub(super) fn run(
    executable: &Path,
    parent: &Path,
    target: AcceptanceTarget,
    checkpoint: RecoveryCheckpoint,
) -> anyhow::Result<Value> {
    let mutation = target.mutation;
    let out = parent.join(checkpoint.as_str());
    fs::create_dir(&out)?;
    fs::write(out.join("acceptance.toml"), "# Isolated failure case\n")?;
    let mut workload = start_workload(executable, &out, 90)?;
    let root = workload.0.id();
    let root_ticks = start_ticks(root)?;
    case::prepare_fixture(&out, root, mutation)?;
    let before = procfs::snapshot(mutation, root, root_ticks)?;
    let desired = target.value;
    ensure!(
        before
            .values()
            .all(|task| procfs::applied_value(mutation, &task.mask, desired)
                .is_ok_and(|v| v != task.mask)),
        "no effective property change for owned helper"
    );
    let result = (|| -> anyhow::Result<Value> {
        write_json(&out.join("before.json"), &procfs::value(&before))?;
        let mut actor = start_actor(executable, &out, root, root_ticks, target, checkpoint)?;
        let mut token_before = Value::Null;
        let mut changed_at_verify = Value::Null;
        if checkpoint != RecoveryCheckpoint::PartialApplyFailure {
            wait_file(&out.join("verify-failure-ready.json"), &mut actor.child, 20)?;
            let current = procfs::snapshot(mutation, root, root_ticks)?;
            let changed = procfs::changed(mutation, &before, &current, desired)?;
            ensure!(changed == 5, "real apply did not change all five tasks");
            write_json(
                &out.join("at-verify-failure.json"),
                &procfs::value(&current),
            )?;
            let journal = read_json(&out.join("controller.json"))?;
            let token = &journal["rollback_token"];
            ensure!(
                journal["state"] == "applied" && token_tasks(token) == Some(5),
                "durable token missing before verification failure"
            );
            let saved = saved_state(token, mutation)?;
            if let Some((restore_path, _)) = &saved {
                ensure!(
                    restore_path.starts_with(out.join("experiments")),
                    "restore state outside the owned experiment directory"
                );
            }
            procfs::verify_saved(
                mutation,
                &before,
                token,
                saved.as_ref().map(|(_, saved)| saved),
                desired,
            )?;
            archive(&out, &out.join("controller.json"), "controller.json")?;
            if let Some((restore_path, _)) = &saved {
                archive(&out, restore_path, "restore.json")?;
            }
            token_before = journal["rollback_token"].clone();
            changed_at_verify = json!(changed);
            fs::write(out.join("verify-failure-ack"), b"")?;
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = actor.child.0.try_wait()? {
                break status;
            }
            ensure!(
                Instant::now() < deadline,
                "controller did not exit on its own"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        ensure!(status.success(), "controller exited with {status}");
        let report = read_json(&out.join("failure-report.json"))?;
        let rollback_expected = checkpoint != RecoveryCheckpoint::RollbackFailure;
        ensure!(
            report["rollback_present"] == true
                && report["rollback_completed"] == rollback_expected
                && report["future_mutation_rejected"] == true
                && report["executor_apply_calls"] == 1
                && report["runtime_phase"] == "Faulted",
            "controller failure report differs: {report}"
        );
        if checkpoint == RecoveryCheckpoint::PartialApplyFailure {
            let trace = fs::read_to_string(out.join("syscalls.log"))?;
            ensure!(
                trace.matches("EPERM").count() == 1,
                "exactly one injected EPERM expected"
            );
        }
        let audit = last_json_line(&out.join("actions.jsonl"))?;
        ensure!(
            audit["success"] == false,
            "terminal audit must be a failure"
        );
        let history = last_json_line(&out.join("history.jsonl"))?;
        ensure!(
            history["phase"] == "Faulted" && history["rollback_performed"] == rollback_expected,
            "history disagrees with rollback outcome: {history}"
        );
        let state = read_json(&out.join("daemon_state.json"))?;
        ensure!(state["phase"] == "faulted", "daemon state is not faulted");
        let journal = read_json(&out.join("controller.json"))?;
        let after_failure = procfs::snapshot(mutation, root, root_ticks)?;
        write_json(
            &out.join("after-failure.json"),
            &procfs::value(&after_failure),
        )?;
        let mut recovery = Value::Null;
        if rollback_expected {
            ensure!(
                journal["state"] == "clean" && state["active_rollback"].is_null(),
                "completed rollback left journal/daemon ownership"
            );
            ensure!(
                after_failure == before,
                "independent readback differs after rollback"
            );
        } else {
            ensure!(
                journal["state"] != "clean"
                    && journal["rollback_token"] == token_before
                    && state["active_rollback"]["token"] == token_before
                    && state["active_rollback"]["manual_restore_command"].is_string(),
                "failed rollback lost token ownership"
            );
            ensure!(
                procfs::changed(mutation, &before, &after_failure, desired)? == 5,
                "failed rollback must leave the retained physical mutation visible"
            );
            command(
                executable,
                &args(&[
                    "recovery-controller",
                    "recover",
                    "--out",
                    &out.to_string_lossy(),
                    "--expected-tasks",
                    "5",
                ]),
                &out,
                "emergency-recovery",
                Duration::from_secs(15),
            )?;
            recovery = read_json(&out.join("emergency-recovery.stdout"))?;
            ensure!(
                recovery["restored_tasks"] == 5 && recovery["journal_clean"] == true,
                "emergency recovery did not confirm restoration"
            );
            let restored = procfs::snapshot(mutation, root, root_ticks)?;
            ensure!(
                restored == before,
                "independent readback differs after emergency recovery"
            );
            write_json(&out.join("restored.json"), &procfs::value(&restored))?;
            verify_recovery_evidence(&out)?;
        }
        Ok(json!({
            "checkpoint":checkpoint.as_str(),"status":"PASS","processes":3,"tasks":5,
            "changed_at_verify_failure":changed_at_verify,"report":report,"terminal_audit":audit,
            "history":history,"journal_state_after_failure":journal["state"],
            "emergency_recovery":recovery,"independent_restoration":true,
        }))
    })();
    let _ = workload.0.kill();
    let root_reaped = workload.0.wait().is_ok();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !procfs::stopped(&before) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let cleanup = procfs::stopped(&before);
    write_json(
        &out.join("cleanup.json"),
        &json!({"root_reaped":root_reaped,"all_owned_task_identities_ceased_running":cleanup}),
    )?;
    let mut result = result?;
    ensure!(
        cleanup && root_reaped,
        "owned helper cleanup/reaping failed"
    );
    result["cleanup_verified"] = json!(true);
    write_json(&out.join("result.json"), &result)?;
    Ok(result)
}
