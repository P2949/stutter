//! Where a live suite actually runs: directly (the caller already has privilege) or through the
//! privileged validation runner, which launches it as the campaign user with a fixed minimal
//! capability set. Both produce the same raw directory, so everything downstream is identical.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use stutter_validation_runner::{
    client::{Client, SOCKET_ENV},
    model::{ErrorCode, LiveSuite, RegisteredWorkload, Request, ResponseStatus},
};

/// Everything a backend needs; all of it derives from campaign state, none from a prompt.
pub struct SuiteInvocation<'a> {
    pub task: &'a str,
    pub suite: &'static str,
    pub attempt: u32,
    pub seconds: u64,
    pub rounds: Option<u32>,
    /// `Some` runs the registered fixed-compile workload instead of the built-in one.
    pub work_units: Option<u64>,
    pub raw: &'a Path,
    pub log: &'a Path,
    /// Extra suite arguments a direct run appends (a runner request carries its own typed fields).
    pub extra_args: &'a [String],
    /// The mutation a recovery, lifecycle or benefit suite drives (`None` is the suite's default).
    pub mutation: Option<&'a str>,
    pub stutter: &'a Path,
    pub xtask: &'a Path,
    pub cwd: &'a Path,
    pub campaign_id: &'a str,
    pub manifest_sha256: &'a str,
}

#[derive(Debug, Eq, PartialEq)]
pub enum BackendError {
    /// Not authorized or no privilege available: a block, not a retry.
    Privilege(String),
    Failed(String),
}

pub trait SuiteBackend {
    fn execute(&self, invocation: &SuiteInvocation<'_>) -> Result<i32, BackendError>;
}

pub struct DirectBackend;

/// Fixed script (no caller text is interpolated into it): optionally delegates a cgroup subtree to
/// the campaign user the way the runner does, then drops to that user with only the capabilities
/// the runner grants a suite and runs the unchanged suite binary.
const DELEGATE_AND_DROP: &str = r#"
set -eu
if [ "$STUTTER_DELEGATE_CGROUP" = "true" ]; then
  name="stutter-validation-$(printf '%s' "$STUTTER_CAMPAIGN" | tr -c 'A-Za-z0-9-' '-')"
  root="/sys/fs/cgroup/$name"
  echo "+cpu +cpuset" > /sys/fs/cgroup/cgroup.subtree_control 2>/dev/null || true
  mkdir -p "$root/suite"
  echo "+cpu +cpuset" > "$root/cgroup.subtree_control"
  chown "$STUTTER_OWNER" "$root" "$root/suite"
  for f in "$root"/cgroup.procs "$root"/cgroup.threads "$root"/cgroup.subtree_control "$root"/cpuset.* "$root"/cpu.* "$root"/suite/cgroup.procs "$root"/suite/cgroup.threads; do
    [ -f "$f" ] && chown "$STUTTER_OWNER" "$f" || true
  done
  echo $$ > "$root/suite/cgroup.procs"
fi
uid="${STUTTER_OWNER%%:*}"; gid="${STUTTER_OWNER##*:}"
set +e
setpriv --reuid "$uid" --regid "$gid" --init-groups --inh-caps=+sys_resource,+sys_nice --ambient-caps=+sys_resource,+sys_nice -- "$@"
status=$?
if [ "$STUTTER_DELEGATE_CGROUP" = "true" ]; then
  rmdir "$root/target" "$root/suite" "$root" 2>/dev/null || true
fi
exit $status
"#;

impl DirectBackend {
    /// A mutation that lowers a task's niceness again needs `CAP_SYS_RESOURCE` in the process that
    /// spawns the disposable workload, and one that moves tasks between cgroups needs a delegated
    /// subtree (the runner provides both to every suite it launches). A direct run has no runner,
    /// so it asks non-interactive sudo to prepare exactly that and start the unchanged suite as
    /// the same user, never as root and never with a password prompt.
    fn command(inv: &SuiteInvocation<'_>, args: &[String]) -> Command {
        if !matches!(inv.mutation, Some("nice" | "cgroup")) {
            let mut command = Command::new(inv.xtask);
            command.args(args);
            return command;
        }
        let owner = std::fs::metadata("/proc/self")
            .map(|meta| std::os::unix::fs::MetadataExt::uid(&meta))
            .unwrap_or(0);
        let group = std::fs::metadata("/proc/self")
            .map(|meta| std::os::unix::fs::MetadataExt::gid(&meta))
            .unwrap_or(0);
        let mut command = Command::new("sudo");
        command
            .args(["-n", "env"])
            .arg(format!(
                "PATH={}",
                std::env::var("PATH").unwrap_or_default()
            ))
            .arg(format!(
                "HOME={}",
                std::env::var("HOME").unwrap_or_default()
            ))
            .arg(format!(
                "STUTTER_DELEGATE_CGROUP={}",
                inv.mutation == Some("cgroup")
            ))
            .arg(format!("STUTTER_CAMPAIGN={}", inv.campaign_id))
            .arg(format!("STUTTER_OWNER={owner}:{group}"))
            .args(["sh", "-c", DELEGATE_AND_DROP, "stutter-direct-suite"])
            .arg(inv.xtask)
            .args(args);
        command
    }
}

impl SuiteBackend for DirectBackend {
    fn execute(&self, inv: &SuiteInvocation<'_>) -> Result<i32, BackendError> {
        let failed = |error: std::io::Error| BackendError::Failed(error.to_string());
        let mut args = vec![
            "live-acceptance".to_owned(),
            inv.suite.to_owned(),
            "--stutter-bin".to_owned(),
            inv.stutter.to_string_lossy().into_owned(),
            "--out".to_owned(),
            inv.raw.to_string_lossy().into_owned(),
        ];
        if LiveSuite::from_xtask_name(inv.suite).is_some_and(LiveSuite::takes_seconds) {
            args.extend(["--seconds".to_owned(), inv.seconds.to_string()]);
        }
        if let Some(rounds) = inv.rounds {
            args.extend(["--rounds".to_owned(), rounds.to_string()]);
        }
        if inv.suite == "suspend-resume" {
            // This suite really suspends the machine; a campaign stage asks for it explicitly.
            args.push("--allow-system-suspend".to_owned());
        }
        args.extend(inv.extra_args.iter().cloned());
        if let Some(mutation) = inv.mutation {
            args.extend(["--mutation".to_owned(), mutation.to_owned()]);
        }
        if let Some(units) = inv.work_units {
            let campaign = inv.raw.ancestors().nth(4).unwrap_or(inv.cwd);
            args.extend([
                "--workload-label".to_owned(),
                super::workloads::fixed_compile::LABEL.to_owned(),
                "--work-units".to_owned(),
                units.to_string(),
                "--".to_owned(),
                inv.xtask.to_string_lossy().into_owned(),
                "validation-campaign".to_owned(),
                "workload".to_owned(),
                "fixed-compile".to_owned(),
                "--cycles".to_owned(),
                units.to_string(),
                "--target-dir".to_owned(),
                super::workloads::fixed_compile::target_dir(campaign)
                    .to_string_lossy()
                    .into_owned(),
            ]);
        }
        if let Some(parent) = inv.log.parent() {
            fs::create_dir_all(parent).map_err(failed)?;
        }
        let log = fs::File::create(inv.log).map_err(failed)?;
        let status = Self::command(inv, &args)
            .current_dir(inv.cwd)
            .stdout(log.try_clone().map_err(failed)?)
            .stderr(log)
            .stdin(Stdio::null())
            .status()
            .map_err(failed)?;
        Ok(status.code().unwrap_or(-1))
    }
}

fn runner_mutation(name: &str) -> Result<stutter_validation_runner::model::Mutation, BackendError> {
    use stutter_validation_runner::model::Mutation;
    match name {
        "affinity" => Ok(Mutation::Affinity),
        "uclamp" => Ok(Mutation::Uclamp),
        "nice" => Ok(Mutation::Nice),
        "cgroup" => Ok(Mutation::Cgroup),
        other => Err(BackendError::Failed(format!(
            "the runner does not know the mutation {other}"
        ))),
    }
}

/// Sends a typed request to the privileged runner.
pub struct RunnerBackend {
    pub socket: PathBuf,
}

/// Rejections that mean "this campaign may not do that", which retrying cannot fix.
fn is_privilege_denial(code: ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::UnauthorizedPeer
            | ErrorCode::UnknownCampaign
            | ErrorCode::ManifestMismatch
            | ErrorCode::ActionNotPermitted
            | ErrorCode::TaskNotPermitted
            | ErrorCode::SchemaMismatch
            | ErrorCode::Cancelled
    )
}

impl SuiteBackend for RunnerBackend {
    fn execute(&self, inv: &SuiteInvocation<'_>) -> Result<i32, BackendError> {
        let Some(suite) = LiveSuite::from_xtask_name(inv.suite) else {
            return Err(BackendError::Failed(format!(
                "{} is not a runner suite",
                inv.suite
            )));
        };
        let client = Client {
            socket: &self.socket,
            campaign_id: inv.campaign_id,
            manifest_sha256: inv.manifest_sha256,
        };
        let request = match inv.work_units {
            Some(work_units) => Request::ExecuteWorkloadOverhead {
                workload: RegisteredWorkload::FixedCompile,
                attempt: inv.attempt,
                seconds: inv.seconds,
                rounds: inv.rounds.unwrap_or_default(),
                work_units,
            },
            None => Request::ExecuteLiveSuite {
                suite,
                attempt: inv.attempt,
                seconds: suite.takes_seconds().then_some(inv.seconds),
                rounds: inv.rounds,
                benefit: None,
                mutation: match inv.mutation {
                    None => None,
                    Some(name) => Some(runner_mutation(name)?),
                },
            },
        };
        let response = client.send(inv.task, request).map_err(|error| {
            BackendError::Privilege(format!(
                "the validation runner at {} is unreachable: {error}",
                self.socket.display()
            ))
        })?;
        match response.status {
            ResponseStatus::Ok => Ok(response
                .data
                .as_ref()
                .and_then(|data| data["exit_code"].as_i64())
                .map_or(-1, |code| code as i32)),
            _ => {
                let error = response.error.expect("a refused response carries an error");
                let message = format!("runner refused: {:?}: {}", error.code, error.message);
                if is_privilege_denial(error.code) {
                    Err(BackendError::Privilege(message))
                } else {
                    Err(BackendError::Failed(message))
                }
            }
        }
    }
}

/// The runner when `STUTTER_VALIDATION_RUNNER` names a socket, otherwise direct execution.
pub fn backend_from_env() -> Box<dyn SuiteBackend> {
    match std::env::var_os(SOCKET_ENV).filter(|value| !value.is_empty()) {
        Some(socket) => Box::new(RunnerBackend {
            socket: PathBuf::from(socket),
        }),
        // A bootstrapped host serves the default socket; its absence means direct execution.
        None if Path::new(super::bootstrap::DEFAULT_SOCKET).exists() => Box::new(RunnerBackend {
            socket: PathBuf::from(super::bootstrap::DEFAULT_SOCKET),
        }),
        None => Box::new(DirectBackend),
    }
}
