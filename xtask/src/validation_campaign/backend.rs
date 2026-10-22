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
    model::{ErrorCode, LiveSuite, Request, ResponseStatus},
};

/// Everything a backend needs; all of it derives from campaign state, none from a prompt.
pub struct SuiteInvocation<'a> {
    pub task: &'a str,
    pub suite: &'static str,
    pub attempt: u32,
    pub seconds: u64,
    pub rounds: Option<u32>,
    pub raw: &'a Path,
    pub log: &'a Path,
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
        let request = Request::ExecuteLiveSuite {
            suite,
            attempt: inv.attempt,
            seconds: suite.takes_seconds().then_some(inv.seconds),
            rounds: inv.rounds,
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
