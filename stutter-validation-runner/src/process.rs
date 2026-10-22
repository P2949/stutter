//! Launching validation work. Executable paths derive from the authorization (repository +
//! pinned digests), never from a request; children drop to the campaign UID with a fixed
//! minimal capability set before `exec`.
use std::{
    fs,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use stutter_core::hash::sha256_hex;

use crate::{
    authorization::Denial,
    credentials::{
        CapabilitySet, Identity, drop_all_privileges, profiling_capabilities, restrict_child,
    },
    model::{Authorization, Envelope, ErrorCode, Request},
    sys,
};

/// Longest a single suite may run before the runner kills it.
const SUITE_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);
/// SCHED_FIFO priority of the controlled contender (matches the manual `chrt -f 50`).
const BURST_PRIORITY: i32 = 50;

fn failure(message: impl Into<String>) -> Denial {
    (ErrorCode::ExecutionFailed, message.into())
}

/// A fully derived launch: nothing in it was chosen by the caller beyond typed parameters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub attempt_dir: PathBuf,
    pub raw: PathBuf,
    pub path_env: String,
}

fn campaign_dir(auth: &Authorization) -> PathBuf {
    Path::new(&auth.repository)
        .join(".llm-runs/validation")
        .join(&auth.campaign_id)
}

pub fn xtask_path(auth: &Authorization) -> PathBuf {
    Path::new(&auth.repository).join("target/debug/xtask")
}

pub fn stutter_path(auth: &Authorization) -> PathBuf {
    Path::new(&auth.repository).join("target/debug/stutter")
}

/// The pinned binary must still hash to the digest recorded at bootstrap.
pub fn verify_pinned(path: &Path, expected_sha256: &str) -> Result<(), Denial> {
    let bytes =
        fs::read(path).map_err(|error| failure(format!("read {}: {error}", path.display())))?;
    if sha256_hex(&bytes) != expected_sha256 {
        return Err(failure(format!(
            "{} no longer matches its pinned digest; refusing to execute it",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("binary")
        )));
    }
    Ok(())
}

pub fn plan_live_suite(
    auth: &Authorization,
    envelope: &Envelope,
) -> Result<PlannedCommand, Denial> {
    let Request::ExecuteLiveSuite {
        suite,
        attempt,
        seconds,
        rounds,
    } = &envelope.body
    else {
        return Err((
            ErrorCode::InvalidArgument,
            "not a live-suite request".to_owned(),
        ));
    };
    let attempt_dir = campaign_dir(auth)
        .join("attempts")
        .join(&envelope.task_id)
        .join(format!("{attempt:04}"));
    let raw = attempt_dir.join("raw");
    let mut args = vec![
        "live-acceptance".to_owned(),
        suite.xtask_name().to_owned(),
        "--stutter-bin".to_owned(),
        stutter_path(auth).to_string_lossy().into_owned(),
        "--out".to_owned(),
        raw.to_string_lossy().into_owned(),
    ];
    if let Some(seconds) = seconds {
        args.extend(["--seconds".to_owned(), seconds.to_string()]);
    }
    if let Some(rounds) = rounds {
        args.extend(["--rounds".to_owned(), rounds.to_string()]);
    }
    Ok(PlannedCommand {
        program: xtask_path(auth),
        args,
        cwd: PathBuf::from(&auth.repository),
        attempt_dir,
        raw,
        path_env: auth.path_env.clone(),
    })
}

pub fn plan_scheduler_burst(
    auth: &Authorization,
    envelope: &Envelope,
) -> Result<PlannedCommand, Denial> {
    let Request::ExecuteSchedulerBurst {
        burst_ms,
        period_ms,
        seconds,
        ..
    } = &envelope.body
    else {
        return Err((
            ErrorCode::InvalidArgument,
            "not a scheduler-burst request".to_owned(),
        ));
    };
    let attempt_dir = campaign_dir(auth).join("burst").join(&envelope.request_id);
    Ok(PlannedCommand {
        program: xtask_path(auth),
        args: vec![
            "live-burst".to_owned(),
            "--burst-ms".to_owned(),
            burst_ms.to_string(),
            "--period-ms".to_owned(),
            period_ms.to_string(),
            "--seconds".to_owned(),
            seconds.to_string(),
        ],
        cwd: PathBuf::from(&auth.repository),
        raw: attempt_dir.join("unused"),
        attempt_dir,
        path_env: auth.path_env.clone(),
    })
}

/// Executes typed actions. A trait so the server can be tested without root or real suites.
pub trait ActionExecutor: Send + Sync {
    fn live_suite(&self, auth: &Authorization, envelope: &Envelope) -> Result<Value, Denial>;
    fn scheduler_burst(&self, auth: &Authorization, envelope: &Envelope) -> Result<Value, Denial>;
}

/// The real executor. `drop_to_owner` is on for the root service; tests without root run the
/// same code path with it off (ids unchanged) and assert on the resulting process state.
pub struct SystemExecutor {
    pub drop_to_owner: bool,
}

impl SystemExecutor {
    fn identity(&self, auth: &Authorization) -> Option<Identity> {
        self.drop_to_owner.then_some(Identity {
            uid: auth.owner_uid,
            gid: auth.owner_gid,
        })
    }

    /// Creates `dir` (and any missing parents below the campaign directory) owned by the
    /// campaign user, so the unprivileged suite can write into it.
    fn prepare_dir(&self, auth: &Authorization, dir: &Path) -> Result<(), Denial> {
        let base = campaign_dir(auth);
        let mut current = base.clone();
        let relative = dir
            .strip_prefix(&base)
            .map_err(|_| failure("output directory escapes the campaign directory"))?;
        for component in relative.components() {
            current.push(component);
            if !current.exists() {
                fs::create_dir(&current).map_err(|error| failure(format!("mkdir: {error}")))?;
                fs::set_permissions(&current, fs::Permissions::from_mode(0o750))
                    .map_err(|error| failure(format!("chmod: {error}")))?;
                if self.drop_to_owner {
                    std::os::unix::fs::chown(&current, Some(auth.owner_uid), Some(auth.owner_gid))
                        .map_err(|error| failure(format!("chown: {error}")))?;
                }
            }
        }
        Ok(())
    }

    fn command(plan: &PlannedCommand) -> Command {
        let mut command = Command::new(&plan.program);
        command
            .args(&plan.args)
            .current_dir(&plan.cwd)
            .env_clear()
            .env("PATH", &plan.path_env)
            .env("HOME", &plan.attempt_dir)
            .stdin(Stdio::null());
        command
    }
}

impl ActionExecutor for SystemExecutor {
    fn live_suite(&self, auth: &Authorization, envelope: &Envelope) -> Result<Value, Denial> {
        let Request::ExecuteLiveSuite { suite, .. } = &envelope.body else {
            return Err((
                ErrorCode::InvalidArgument,
                "not a live-suite request".to_owned(),
            ));
        };
        let plan = plan_live_suite(auth, envelope)?;
        verify_pinned(&xtask_path(auth), &auth.xtask_sha256)?;
        verify_pinned(&stutter_path(auth), &auth.stutter_sha256)?;
        self.prepare_dir(auth, &plan.attempt_dir)?;
        let logs = plan.attempt_dir.join("logs");
        self.prepare_dir(auth, &logs)?;
        let log_path = logs.join("suite.log");
        let log = fs::File::create(&log_path).map_err(|error| failure(format!("log: {error}")))?;
        if self.drop_to_owner {
            std::os::unix::fs::chown(&log_path, Some(auth.owner_uid), Some(auth.owner_gid))
                .map_err(|error| failure(format!("chown log: {error}")))?;
        }
        let mut command = Self::command(&plan);
        command
            .stdout(
                log.try_clone()
                    .map_err(|error| failure(error.to_string()))?,
            )
            .stderr(log);
        restrict_child(
            &mut command,
            self.identity(auth),
            profiling_capabilities(suite.needs_scheduler_priority()),
        );
        let mut child = command
            .spawn()
            .map_err(|error| failure(format!("could not start {}: {error}", suite.xtask_name())))?;
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| failure(error.to_string()))?
            {
                break status;
            }
            if started.elapsed() > SUITE_TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                return Err(failure("the suite exceeded its time limit and was killed"));
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        Ok(json!({
            "suite": suite.xtask_name(),
            "exit_code": status.code(),
            "raw": plan.raw.strip_prefix(&auth.repository).unwrap_or(&plan.raw).to_string_lossy(),
        }))
    }

    fn scheduler_burst(&self, auth: &Authorization, envelope: &Envelope) -> Result<Value, Denial> {
        let Request::ExecuteSchedulerBurst { cpu, seconds, .. } = &envelope.body else {
            return Err((
                ErrorCode::InvalidArgument,
                "not a scheduler-burst request".to_owned(),
            ));
        };
        let (cpu, seconds) = (*cpu, *seconds);
        let plan = plan_scheduler_burst(auth, envelope)?;
        verify_pinned(&xtask_path(auth), &auth.xtask_sha256)?;
        let mut command = Self::command(&plan);
        command.stdout(Stdio::null()).stderr(Stdio::null());
        // While still privileged: pin to the target CPU and take SCHED_FIFO priority. Both
        // persist across the id/capability drop and the exec that follow.
        // SAFETY: the closure only performs raw syscalls (async-signal-safe), as `pre_exec`
        // requires after fork.
        unsafe {
            command.pre_exec(move || {
                sys::pin_to_cpu(cpu)?;
                sys::set_fifo_priority(BURST_PRIORITY)
            });
        }
        drop_all_privileges(&mut command, self.identity(auth));
        let mut child = command
            .spawn()
            .map_err(|error| failure(format!("could not start the scheduler burst: {error}")))?;
        let pid = child.id();
        // The helper ends itself after `seconds`; this thread reaps it and kills a straggler.
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(seconds + 5);
            while Instant::now() < deadline {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let _ = child.kill();
            let _ = child.wait();
        });
        Ok(json!({"pid": pid, "cpu": cpu, "seconds": seconds}))
    }
}

/// Capability sets a profiling suite receives; re-exported for tests and diagnostics.
pub fn suite_capabilities(needs_scheduler_priority: bool) -> CapabilitySet {
    profiling_capabilities(needs_scheduler_priority)
}
