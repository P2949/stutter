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
    /// Recording of a campaign-owned target; unsupported unless an executor provides it.
    fn recording(&self, _auth: &Authorization, _envelope: &Envelope) -> Result<Value, Denial> {
        Err((
            ErrorCode::Unsupported,
            "recording is not available".to_owned(),
        ))
    }
    /// Overhead suite against a registered workload.
    fn workload_overhead(
        &self,
        _auth: &Authorization,
        _envelope: &Envelope,
    ) -> Result<Value, Denial> {
        Err((
            ErrorCode::Unsupported,
            "workload overhead is not available".to_owned(),
        ))
    }
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

impl SystemExecutor {
    /// Runs a planned command to completion as the campaign user with `capabilities`, logging
    /// into the attempt directory. Shared by every privileged action that waits for a child.
    fn run_planned(
        &self,
        auth: &Authorization,
        plan: &PlannedCommand,
        label: &str,
        capabilities: crate::credentials::CapabilitySet,
        timeout: Duration,
    ) -> Result<Value, Denial> {
        self.prepare_dir(auth, &plan.attempt_dir)?;
        let logs = plan.attempt_dir.join("logs");
        self.prepare_dir(auth, &logs)?;
        let log_path = logs.join(format!("{label}.log"));
        let log = fs::File::create(&log_path).map_err(|error| failure(format!("log: {error}")))?;
        if self.drop_to_owner {
            std::os::unix::fs::chown(&log_path, Some(auth.owner_uid), Some(auth.owner_gid))
                .map_err(|error| failure(format!("chown log: {error}")))?;
        }
        let mut command = Self::command(plan);
        command
            .stdout(
                log.try_clone()
                    .map_err(|error| failure(error.to_string()))?,
            )
            .stderr(log);
        restrict_child(&mut command, self.identity(auth), capabilities);
        let mut child = command
            .spawn()
            .map_err(|error| failure(format!("could not start {label}: {error}")))?;
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| failure(error.to_string()))?
            {
                break status;
            }
            if started.elapsed() > timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(failure(format!(
                    "{label} exceeded its time limit and was killed"
                )));
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        Ok(json!({
            "action": label,
            "exit_code": status.code(),
            "raw": plan.raw.strip_prefix(&auth.repository).unwrap_or(&plan.raw).to_string_lossy(),
        }))
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
        self.run_planned(
            auth,
            &plan,
            suite.xtask_name(),
            profiling_capabilities(suite.needs_scheduler_priority()),
            SUITE_TIMEOUT,
        )
    }

    fn recording(&self, auth: &Authorization, envelope: &Envelope) -> Result<Value, Denial> {
        let Request::ExecuteRecording {
            target_pid,
            target_starttime_ticks,
            seconds,
            capture,
            ..
        } = &envelope.body
        else {
            return Err((
                ErrorCode::InvalidArgument,
                "not a recording request".to_owned(),
            ));
        };
        verify_target(
            Path::new("/proc"),
            *target_pid,
            *target_starttime_ticks,
            auth.owner_uid,
        )?;
        verify_pinned(&stutter_path(auth), &auth.stutter_sha256)?;
        let capture_dir = campaign_dir(auth)
            .join("attempts")
            .join(&envelope.task_id)
            .join(format!("{:04}", recording_attempt(&envelope.body)))
            .join("captures")
            .join(format!("{capture:04}"));
        let mangohud = crate::process::newest_csv(&capture_dir.join("mangohud"));
        let plan = plan_recording(auth, envelope, mangohud.as_deref())?;
        self.prepare_dir(auth, &plan.raw)?;
        self.run_planned(
            auth,
            &plan,
            "record",
            profiling_capabilities(false),
            Duration::from_secs(*seconds + 120),
        )
    }

    fn workload_overhead(
        &self,
        auth: &Authorization,
        envelope: &Envelope,
    ) -> Result<Value, Denial> {
        let plan = plan_workload_overhead(auth, envelope)?;
        verify_pinned(&xtask_path(auth), &auth.xtask_sha256)?;
        verify_pinned(&stutter_path(auth), &auth.stutter_sha256)?;
        self.run_planned(
            auth,
            &plan,
            "overhead-workload",
            profiling_capabilities(false),
            SUITE_TIMEOUT,
        )
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

fn recording_attempt(request: &Request) -> u32 {
    match request {
        Request::ExecuteRecording { attempt, .. } => *attempt,
        _ => 0,
    }
}

/// The newest `*.csv` in `dir` (the MangoHud log of the run that just started).
pub fn newest_csv(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "csv"))
        .filter_map(|path| Some((path.metadata().ok()?.modified().ok()?, path)))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

/// A recording may only observe a process owned by the campaign user whose start time matches
/// exactly: the runner never attaches to an arbitrary or replaced process.
pub fn verify_target(
    proc_root: &Path,
    pid: u32,
    starttime_ticks: u64,
    owner_uid: u32,
) -> Result<(), Denial> {
    let refuse = |why: String| Err((ErrorCode::InvalidArgument, why));
    let dir = proc_root.join(pid.to_string());
    let stat = fs::read_to_string(dir.join("stat")).map_err(|_| {
        (
            ErrorCode::InvalidArgument,
            format!("process {pid} does not exist"),
        )
    })?;
    // Field 22 (after the parenthesized comm, which may itself contain spaces and parentheses).
    let actual = stat
        .rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().nth(19))
        .and_then(|value| value.parse::<u64>().ok());
    if actual != Some(starttime_ticks) {
        return refuse(format!(
            "process {pid} has a different start time: it was replaced"
        ));
    }
    let status = fs::read_to_string(dir.join("status")).unwrap_or_default();
    let uids = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .map(|rest| {
            rest.split_whitespace()
                .map(|v| v.parse::<u32>().ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    // Real and effective uid must both be the campaign user's.
    if uids.len() < 2 || uids[0] != Some(owner_uid) || uids[1] != Some(owner_uid) {
        return refuse(format!("process {pid} is not owned by the campaign user"));
    }
    Ok(())
}

pub fn plan_recording(
    auth: &Authorization,
    envelope: &Envelope,
    mangohud_log: Option<&Path>,
) -> Result<PlannedCommand, Denial> {
    let Request::ExecuteRecording {
        attempt,
        capture,
        target_pid,
        seconds,
        probes,
        ..
    } = &envelope.body
    else {
        return Err((
            ErrorCode::InvalidArgument,
            "not a recording request".to_owned(),
        ));
    };
    let attempt_dir = campaign_dir(auth)
        .join("attempts")
        .join(&envelope.task_id)
        .join(format!("{attempt:04}"));
    let raw = attempt_dir
        .join("captures")
        .join(format!("{capture:04}"))
        .join("raw");
    let mut args = vec![
        "record".to_owned(),
        "--tree-pid".to_owned(),
        target_pid.to_string(),
        "--duration".to_owned(),
        seconds.to_string(),
        "--summary-ms".to_owned(),
        "250".to_owned(),
        "--out-dir".to_owned(),
        raw.to_string_lossy().into_owned(),
    ];
    let probe_flags: &[&str] = match probes {
        crate::model::RecordProbes::Core => &[
            "--no-hwmon",
            "--no-cpu-freq",
            "--no-faults",
            "--no-block-io",
            "--no-stat-wait",
        ],
        crate::model::RecordProbes::Corpus { .. } => &[
            "--hwmon",
            "--block-io",
            "--no-cpu-freq",
            "--no-faults",
            "--no-stat-wait",
        ],
    };
    args.extend(probe_flags.iter().map(|flag| (*flag).to_owned()));
    if let crate::model::RecordProbes::Corpus { irq: Some(irq) } = probes {
        args.extend([
            "--irq-latency".to_owned(),
            "--irq".to_owned(),
            irq.to_string(),
        ]);
    }
    if let Some(log) = mangohud_log {
        args.extend([
            "--mangohud-log".to_owned(),
            log.to_string_lossy().into_owned(),
        ]);
    }
    Ok(PlannedCommand {
        program: stutter_path(auth),
        args,
        cwd: PathBuf::from(&auth.repository),
        attempt_dir,
        raw,
        path_env: auth.path_env.clone(),
    })
}

pub fn plan_workload_overhead(
    auth: &Authorization,
    envelope: &Envelope,
) -> Result<PlannedCommand, Denial> {
    let Request::ExecuteWorkloadOverhead {
        workload,
        attempt,
        seconds,
        rounds,
        work_units,
    } = &envelope.body
    else {
        return Err((
            ErrorCode::InvalidArgument,
            "not a workload-overhead request".to_owned(),
        ));
    };
    let attempt_dir = campaign_dir(auth)
        .join("attempts")
        .join(&envelope.task_id)
        .join(format!("{attempt:04}"));
    let raw = attempt_dir.join("raw");
    let target_dir = campaign_dir(auth).join("workloads/fixed-compile-target");
    let xtask = xtask_path(auth);
    let args = vec![
        "live-acceptance".to_owned(),
        "overhead".to_owned(),
        "--stutter-bin".to_owned(),
        stutter_path(auth).to_string_lossy().into_owned(),
        "--out".to_owned(),
        raw.to_string_lossy().into_owned(),
        "--seconds".to_owned(),
        seconds.to_string(),
        "--rounds".to_owned(),
        rounds.to_string(),
        "--workload-label".to_owned(),
        workload.label().to_owned(),
        "--work-units".to_owned(),
        work_units.to_string(),
        "--".to_owned(),
        xtask.to_string_lossy().into_owned(),
        "validation-campaign".to_owned(),
        "workload".to_owned(),
        "fixed-compile".to_owned(),
        "--cycles".to_owned(),
        work_units.to_string(),
        "--target-dir".to_owned(),
        target_dir.to_string_lossy().into_owned(),
    ];
    Ok(PlannedCommand {
        program: xtask,
        args,
        cwd: PathBuf::from(&auth.repository),
        attempt_dir,
        raw,
        path_env: auth.path_env.clone(),
    })
}
