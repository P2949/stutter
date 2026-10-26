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
        let status = Command::new(inv.xtask)
            .args(&args)
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
