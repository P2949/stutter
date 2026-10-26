use std::{io::Write, os::unix::process::ExitStatusExt};

use super::*;

pub(super) struct Actor {
    pub(super) child: OwnedChild,
    pub(super) pid: u32,
    pub(super) ticks: u64,
    traced: bool,
    finished: bool,
}
impl Drop for Actor {
    fn drop(&mut self) {
        if !self.finished
            && self.child.0.try_wait().ok().flatten().is_none()
            && start_ticks(self.pid).ok() == Some(self.ticks)
        {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &self.pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}
impl Actor {
    pub(super) fn kill_and_wait(&mut self, out: &Path) -> anyhow::Result<Value> {
        ensure!(
            self.child.0.try_wait()?.is_none() && start_ticks(self.pid)? == self.ticks,
            "controller exited or identity changed before SIGKILL"
        );
        if self.traced {
            ensure!(
                Command::new("kill")
                    .args(["-KILL", "--", &self.pid.to_string()])
                    .status()?
                    .success(),
                "controller SIGKILL command failed"
            );
        } else {
            self.child.0.kill()?;
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        let status = loop {
            if let Some(status) = self.child.0.try_wait()? {
                break status;
            }
            ensure!(
                Instant::now() < deadline,
                "controller/tracer did not terminate after SIGKILL"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        if self.traced {
            let trace = fs::read_to_string(out.join("syscalls.log"))?;
            ensure!(
                trace.lines().any(|line| line.contains("killed by SIGKILL")
                    && (line.starts_with(&format!("{} ", self.pid))
                        || line.trim_start().starts_with("+++"))),
                "tracer did not observe controller SIGKILL"
            );
            ensure!(
                start_ticks(self.pid).ok() != Some(self.ticks),
                "tracer did not reap controller"
            );
        } else {
            ensure!(
                status.signal() == Some(9),
                "controller was not terminated by SIGKILL"
            );
        }
        self.finished = true;
        Ok(
            json!({"signal":9,"controller_pid":self.pid,"controller_starttime_ticks":self.ticks,"traced":self.traced,"wait_signal":status.signal(),"wait_code":status.code(),"termination_confirmed":true}),
        )
    }
}
pub(super) fn start_actor(
    executable: &Path,
    out: &Path,
    root: u32,
    root_ticks: u64,
    target: AcceptanceTarget,
    checkpoint: RecoveryCheckpoint,
) -> anyhow::Result<Actor> {
    let injection = checkpoint.strace_injection(target.mutation);
    let traced = injection.is_some();
    let mut command = if let Some(injection) = injection {
        let mut command = Command::new("strace");
        command
            .args(["-f", "--seccomp-bpf", "-yy", "-o"])
            .arg(out.join("syscalls.log"));
        if target.mutation == AcceptanceMutation::Cgroup {
            // Only the writes that move a task are traced: the destination's `cgroup.procs`.
            command.arg("-P").arg(cgroup_procs_of_target(root)?);
        }
        command
            .arg("-e")
            .arg(format!("trace={}", target.mutation.syscall()))
            .arg("-e")
            .arg(injection)
            .arg(executable);
        command
    } else {
        Command::new(executable)
    };
    command
        .args(["recovery-controller", "hold", "--out"])
        .arg(out)
        .args([
            "--tree-pid",
            &root.to_string(),
            "--tree-ticks",
            &root_ticks.to_string(),
            "--mutation",
            target.mutation.as_str(),
            "--value",
            &target.value.to_string(),
            "--owner-pid",
            &std::process::id().to_string(),
            "--owner-ticks",
            &start_ticks(std::process::id())?.to_string(),
            "--checkpoint",
            checkpoint.as_str(),
        ])
        .env("STUTTER_CONFIG", out.join("acceptance.toml"))
        // The in-process privileged service audits under $HOME; keep that inside the case.
        .env("HOME", out)
        .stdout(fs::File::create(out.join("controller.stdout"))?)
        .stderr(fs::File::create(out.join("controller.stderr"))?);
    let mut child = OwnedChild(command.spawn()?);
    let started = wait_file(&out.join("controller-started.json"), &mut child, 10)?;
    let pid = started["controller_pid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .context("controller PID missing")?;
    let ticks = started["controller_starttime_ticks"]
        .as_u64()
        .context("controller identity missing")?;
    ensure!(
        start_ticks(pid)? == ticks,
        "controller identity changed at startup"
    );
    ensure!(
        traced || pid == child.0.id(),
        "controller PID is not the owned child"
    );
    Ok(Actor {
        child,
        pid,
        ticks,
        traced,
        finished: false,
    })
}
pub(super) fn archive(out: &Path, source: &Path, name: &str) -> anyhow::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out.join(format!("{name}.before-crash")))?;
    file.write_all(&fs::read(source)?)?;
    file.sync_all()?;
    Ok(())
}
/// How many tasks the first unit of work changes. Most mutations act on one task per write; a
/// cgroup move is process-granular, so the first write moves every thread of the first process.
fn first_unit_effect(mutation: AcceptanceMutation, before: &procfs::Snapshot) -> usize {
    match (mutation, before.values().next()) {
        (AcceptanceMutation::Cgroup, Some(first)) => {
            before.values().filter(|task| task.pid == first.pid).count()
        }
        _ => 1,
    }
}

/// `cgroup.procs` of the `target` sibling of the cgroup the tree runs in.
fn cgroup_procs_of_target(root: u32) -> anyhow::Result<PathBuf> {
    let text = fs::read_to_string(format!("/proc/{root}/cgroup"))?;
    let original = text
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .context("cgroup v2 path missing")?;
    let parent = Path::new(original.trim())
        .parent()
        .context("the tree does not run in a delegated cgroup subtree")?;
    Ok(Path::new("/sys/fs/cgroup")
        .join(parent.strip_prefix("/")?)
        .join("target/cgroup.procs"))
}

/// How many tasks the durable token covers, in the token's own vocabulary.
pub(super) fn token_tasks(token: &Value) -> Option<u64> {
    token["affected_tasks"].as_u64().or_else(|| {
        token["records"]
            .as_array()
            .map(|records| records.len() as u64)
    })
}

/// The independent snapshot of the durable original state a token names: affinity keeps it in a
/// restore file the token points at (returned as its path), uclamp inside the token itself.
pub(super) fn saved_state(
    token: &Value,
    mutation: AcceptanceMutation,
) -> anyhow::Result<Option<(PathBuf, Value)>> {
    if mutation != AcceptanceMutation::Affinity {
        return Ok(None);
    }
    let path = PathBuf::from(
        token["restore_path"]
            .as_str()
            .context("rollback token restore path missing")?,
    );
    let saved = read_json(&path)?;
    Ok(Some((path, saved)))
}

pub(super) fn verify_recovery_evidence(out: &Path) -> anyhow::Result<()> {
    let journal = read_json(&out.join("controller.json"))?;
    ensure!(
        journal["state"] == "clean" && journal.get("rollback_token").is_none_or(Value::is_null),
        "recovery journal not clean"
    );
    let audit = fs::read_to_string(out.join("actions.jsonl"))?
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    let last = audit.last().context("recovery audit missing")?;
    ensure!(
        last["command"] == "autotune-startup-recovery"
            && last["success"] == true
            && last["affected_tasks"] == 5,
        "recovery audit does not confirm five restores"
    );
    let history = fs::read_to_string(out.join("history.jsonl"))?
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    ensure!(
        history.last().is_some_and(
            |event| event["rollback_performed"] == true && event["phase"] == "Cooldown"
        ),
        "recovery history missing successful rollback"
    );
    let state = read_json(&out.join("daemon-state.json"))?;
    ensure!(
        state["phase"] == "cooldown"
            && state["active_rollback"].is_null()
            && state["active_experiment"].is_null()
            && state["faulted"].is_null(),
        "daemon state disagrees with completed recovery"
    );
    Ok(())
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
    fs::write(out.join("acceptance.toml"), "# Isolated crash checkpoint\n")?;
    let mut workload = start_workload(executable, &out, 90)?;
    let root = workload.0.id();
    let root_ticks = start_ticks(root)?;
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
        let at_checkpoint = if checkpoint.strace_injection(mutation).is_some() {
            let deadline = Instant::now() + Duration::from_secs(10);
            let interrupted = checkpoint == RecoveryCheckpoint::InterruptedRollback;
            loop {
                ensure!(
                    actor.child.0.try_wait()?.is_none(),
                    "traced controller exited before the injected stall"
                );
                // Apply also passes through four changed tasks; only the production
                // Reverting(token) journal state distinguishes the rollback stall.
                if interrupted
                    && read_json(&out.join("controller.json"))
                        .map_or(true, |journal| journal["state"] != "reverting")
                {
                    ensure!(
                        Instant::now() < deadline,
                        "Reverting(token) never journaled"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                let current = procfs::snapshot(mutation, root, root_ticks)?;
                let count = procfs::changed(mutation, &before, &current, desired)?;
                let target = if interrupted {
                    before.len() - first_unit_effect(mutation, &before)
                } else {
                    first_unit_effect(mutation, &before)
                };
                if count == target {
                    break current;
                }
                ensure!(
                    if interrupted {
                        count > target
                    } else {
                        count == 0
                    },
                    "tracer missed the injected stall"
                );
                ensure!(Instant::now() < deadline, "no partial mutation observed");
                std::thread::sleep(Duration::from_millis(10));
            }
        } else {
            let ready = wait_file(&out.join("pause-ready.json"), &mut actor.child, 10)?;
            ensure!(
                ready["controller_pid"] == actor.pid && ready["checkpoint"] == checkpoint.as_str(),
                "checkpoint handshake differs"
            );
            ensure!(
                ready["real_verification_read_completed"]
                    == matches!(
                        checkpoint,
                        RecoveryCheckpoint::DuringVerify
                            | RecoveryCheckpoint::AfterVerify
                            | RecoveryCheckpoint::ManagerMeasuring
                    ),
                "verification handshake differs"
            );
            ensure!(
                ready["verification_returned_to_runner"]
                    == (checkpoint == RecoveryCheckpoint::AfterVerify),
                "verification-return handshake differs"
            );
            procfs::snapshot(mutation, root, root_ticks)?
        };
        let changed = procfs::changed(mutation, &before, &at_checkpoint, desired)?;
        let expected_changed = match checkpoint {
            RecoveryCheckpoint::BeforeApply | RecoveryCheckpoint::AfterPreparation => 0,
            RecoveryCheckpoint::MidApply => first_unit_effect(mutation, &before),
            RecoveryCheckpoint::InterruptedRollback => {
                before.len() - first_unit_effect(mutation, &before)
            }
            _ => 5,
        };
        ensure!(
            changed == expected_changed,
            "checkpoint changed-task count differs"
        );
        write_json(
            &out.join("at-checkpoint.json"),
            &procfs::value(&at_checkpoint),
        )?;
        let expected_tasks = if checkpoint == RecoveryCheckpoint::BeforeApply {
            0
        } else {
            5
        };
        if expected_tasks > 0 {
            let journal = read_json(&out.join("controller.json"))?;
            let expected_state = if matches!(
                checkpoint,
                RecoveryCheckpoint::AfterPreparation | RecoveryCheckpoint::MidApply
            ) {
                "applying"
            } else if checkpoint == RecoveryCheckpoint::ManagerMeasuring {
                "measuring"
            } else if checkpoint == RecoveryCheckpoint::InterruptedRollback {
                "reverting"
            } else {
                "applied"
            };
            let token = &journal["rollback_token"];
            ensure!(
                journal["state"] == expected_state && token_tasks(token) == Some(5),
                "durable checkpoint token missing"
            );
            let saved = saved_state(token, mutation)?;
            if let Some((restore_path, _)) = &saved {
                // Direct transactions use the harness path; the manager reserves an isolated
                // per-experiment path beneath the journal directory.
                let expected_restore = if matches!(
                    checkpoint,
                    RecoveryCheckpoint::ManagerMeasuring | RecoveryCheckpoint::InterruptedRollback
                ) {
                    restore_path.starts_with(out.join("experiments"))
                } else {
                    *restore_path == out.join("restore.json")
                };
                ensure!(
                    token["kind"] == "cpu-affinity-restore-file" && expected_restore,
                    "rollback token points outside the owned checkpoint"
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
        } else {
            ensure!(
                !out.join("controller.json").exists() && !out.join("restore.json").exists(),
                "before-apply checkpoint wrote mutation state"
            );
        }
        let killed = actor.kill_and_wait(&out)?;
        let after_kill = procfs::snapshot(mutation, root, root_ticks)?;
        ensure!(
            after_kill == at_checkpoint,
            "controller death changed the property; destructor cleanup cannot prove crash recovery"
        );
        write_json(&out.join("after-kill.json"), &procfs::value(&after_kill))?;
        command(
            executable,
            &args(&[
                "recovery-controller",
                "recover",
                "--out",
                &out.to_string_lossy(),
                "--expected-tasks",
                &expected_tasks.to_string(),
            ]),
            &out,
            "restart",
            Duration::from_secs(15),
        )?;
        let restart = read_json(&out.join("restart.stdout"))?;
        ensure!(
            controller_identity(&restart)? != (actor.pid, actor.ticks),
            "restart is the killed controller process identity"
        );
        ensure!(
            restart["restored_tasks"] == expected_tasks && restart["journal_clean"] == true,
            "restart did not confirm expected recovery"
        );
        let restored = procfs::snapshot(mutation, root, root_ticks)?;
        ensure!(
            restored == before,
            "independent original identity/property readback differs"
        );
        write_json(&out.join("restored.json"), &procfs::value(&restored))?;
        if expected_tasks > 0 {
            verify_recovery_evidence(&out)?;
        }
        Ok(
            json!({"checkpoint":checkpoint.as_str(),"status":"PASS","processes":3,"tasks":5,"changed_at_kill":changed,"prepared_tasks":expected_tasks,"kill":killed,"restart":restart,"independent_restoration":true,"audit_history_state_verified":expected_tasks > 0,"before_apply_clean":expected_tasks == 0}),
        )
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

/// Process identity is `(pid, starttime_ticks)`: Linux may legitimately reuse a numeric PID.
pub(super) fn controller_identity(value: &Value) -> anyhow::Result<(u32, u64)> {
    Ok((
        value["controller_pid"]
            .as_u64()
            .and_then(|pid| u32::try_from(pid).ok())
            .context("restart controller PID missing")?,
        value["controller_starttime_ticks"]
            .as_u64()
            .context("restart controller starttime missing")?,
    ))
}

/// Spawns the owned 3-process / 5-task workload and waits until it is ready.
pub(super) fn start_workload(
    executable: &Path,
    out: &Path,
    seconds: u64,
) -> anyhow::Result<OwnedChild> {
    let ready = out.join("workload-ready.json");
    let mut workload = OwnedChild(
        Command::new(executable)
            .args(["live-workload", "--descendants", "--seconds"])
            .arg(seconds.to_string())
            .arg("--ready")
            .arg(&ready)
            .stdout(Stdio::null())
            .stderr(fs::File::create(out.join("workload.stderr"))?)
            .unprivileged()
            .spawn()?,
    );
    wait_file(&ready, &mut workload, 5)?;
    Ok(workload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_identity_is_pid_and_starttime_not_numeric_pid() {
        let killed = (42_u32, 1_000_u64);
        let reused_pid = json!({"controller_pid":42,"controller_starttime_ticks":2_000});
        assert_ne!(controller_identity(&reused_pid).unwrap(), killed);
        let same = json!({"controller_pid":42,"controller_starttime_ticks":1_000});
        assert_eq!(controller_identity(&same).unwrap(), killed);
        assert!(controller_identity(&json!({"controller_pid":42})).is_err());
    }
}
