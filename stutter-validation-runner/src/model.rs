//! Wire and authorization types. The runner exposes a closed set of typed actions; there is no
//! shell, no arbitrary exec and no caller-chosen executable path anywhere in this schema.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 2;

/// Which controller a campaign's authorization serves. The runner derives the campaign
/// directory and the reboot-resume command from this typed kind and nothing else: there is no
/// caller-supplied program or argument vector.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerKind {
    CoreValidation,
    MutationGraduation,
}

impl ControllerKind {
    /// Campaign state root, relative to the authorized repository.
    pub const fn campaign_root(self) -> &'static str {
        match self {
            Self::CoreValidation => ".llm-runs/validation",
            Self::MutationGraduation => ".llm-runs/mutation-validation",
        }
    }

    /// The xtask subcommand pair that resumes a campaign of this kind (followed by
    /// `--campaign <dir> --resume`).
    pub const fn resume_subcommand(self) -> [&'static str; 2] {
        match self {
            Self::CoreValidation => ["validation-campaign", "continue"],
            Self::MutationGraduation => ["mutation-campaign", "continue"],
        }
    }
}

/// Live-acceptance suites the runner may launch (as the campaign UID, with fixed capabilities).
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum LiveSuite {
    Observation,
    GameRunnable,
    Overhead,
    ScoreDegradation,
    KernelCompat,
    Affinity,
    Recovery,
}

impl LiveSuite {
    /// The `xtask live-acceptance <name>` subcommand.
    pub fn xtask_name(self) -> &'static str {
        match self {
            Self::Observation => "observation",
            Self::GameRunnable => "game-runnable",
            Self::Overhead => "overhead",
            Self::ScoreDegradation => "score-degradation",
            Self::KernelCompat => "kernel-compat",
            Self::Affinity => "affinity",
            Self::Recovery => "recovery",
        }
    }

    /// Whether the suite takes `--seconds`.
    /// Inverse of [`LiveSuite::xtask_name`].
    pub fn from_xtask_name(name: &str) -> Option<Self> {
        [
            Self::Observation,
            Self::GameRunnable,
            Self::Overhead,
            Self::ScoreDegradation,
            Self::KernelCompat,
            Self::Affinity,
            Self::Recovery,
        ]
        .into_iter()
        .find(|suite| suite.xtask_name() == name)
    }

    pub fn takes_seconds(self) -> bool {
        matches!(
            self,
            Self::Observation | Self::GameRunnable | Self::Overhead | Self::ScoreDegradation
        )
    }

    /// Suites that apply SCHED_FIFO contention themselves need `CAP_SYS_NICE` as well.
    pub fn needs_scheduler_priority(self) -> bool {
        matches!(self, Self::GameRunnable | Self::ScoreDegradation)
    }
}

/// Workloads the runner knows how to launch; the argv is derived here, never supplied.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RegisteredWorkload {
    FixedCompile,
}

impl RegisteredWorkload {
    pub fn label(self) -> &'static str {
        match self {
            Self::FixedCompile => "fixed-compile-v1",
        }
    }
}

/// Probe sets for [`Request::ExecuteRecording`]. One corpus set is used for every capture so
/// controls and positives are recorded identically.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RecordProbes {
    Core,
    Corpus { irq: Option<u32> },
}

/// The discriminant of a request, used in authorization grants.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum RunnerAction {
    Status,
    ExecuteLiveSuite,
    ExecuteSchedulerBurst,
    ExecuteRecording,
    ExecuteWorkloadOverhead,
    PrepareKernelCycle,
    SetNextBoot,
    Reboot,
    ResumeCampaign,
    CancelCampaign,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub enum Request {
    Status,
    ExecuteLiveSuite {
        suite: LiveSuite,
        /// Attempt number; the runner derives the output directory from it.
        attempt: u32,
        seconds: Option<u64>,
        rounds: Option<u32>,
    },
    ExecuteSchedulerBurst {
        cpu: u32,
        burst_ms: u64,
        period_ms: u64,
        seconds: u64,
    },
    /// Records a campaign-owned target process with a fixed, typed probe set.
    ExecuteRecording {
        attempt: u32,
        capture: u32,
        target_pid: u32,
        target_starttime_ticks: u64,
        seconds: u64,
        probes: RecordProbes,
    },
    /// The overhead suite against a *registered* workload; the command is derived by the runner.
    ExecuteWorkloadOverhead {
        workload: RegisteredWorkload,
        attempt: u32,
        seconds: u64,
        rounds: u32,
        work_units: u64,
    },
    PrepareKernelCycle {
        entry_id: String,
    },
    SetNextBoot {
        entry_id: String,
    },
    Reboot,
    ResumeCampaign,
    CancelCampaign,
}

impl Request {
    pub fn action(&self) -> RunnerAction {
        match self {
            Self::Status => RunnerAction::Status,
            Self::ExecuteLiveSuite { .. } => RunnerAction::ExecuteLiveSuite,
            Self::ExecuteSchedulerBurst { .. } => RunnerAction::ExecuteSchedulerBurst,
            Self::ExecuteRecording { .. } => RunnerAction::ExecuteRecording,
            Self::ExecuteWorkloadOverhead { .. } => RunnerAction::ExecuteWorkloadOverhead,
            Self::PrepareKernelCycle { .. } => RunnerAction::PrepareKernelCycle,
            Self::SetNextBoot { .. } => RunnerAction::SetNextBoot,
            Self::Reboot => RunnerAction::Reboot,
            Self::ResumeCampaign => RunnerAction::ResumeCampaign,
            Self::CancelCampaign => RunnerAction::CancelCampaign,
        }
    }

    pub fn suite(&self) -> Option<LiveSuite> {
        match self {
            Self::ExecuteLiveSuite { suite, .. } => Some(*suite),
            _ => None,
        }
    }
}

/// Every request carries the identity it claims; none of it is trusted over the socket's
/// peer credentials or the root-owned authorization.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub schema_version: u32,
    pub campaign_id: String,
    pub task_id: String,
    pub request_id: String,
    pub manifest_sha256: String,
    pub body: Request,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ErrorCode {
    UnauthorizedPeer,
    UnknownCampaign,
    ManifestMismatch,
    ActionNotPermitted,
    TaskNotPermitted,
    InvalidArgument,
    BootEntryNotAuthorized,
    RebootBudgetExhausted,
    CorruptState,
    Unsupported,
    ExecutionFailed,
    SchemaMismatch,
    Busy,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StructuredError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ResponseStatus {
    Ok,
    Rejected,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Response {
    pub schema_version: u32,
    pub request_id: String,
    pub status: ResponseStatus,
    /// Identifier of the privileged receipt written for this request, if any.
    pub receipt: Option<String>,
    pub error: Option<StructuredError>,
    pub data: Option<serde_json::Value>,
}

impl Response {
    pub fn ok(request_id: &str, receipt: Option<String>, data: serde_json::Value) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            request_id: request_id.to_owned(),
            status: ResponseStatus::Ok,
            receipt,
            error: None,
            data: Some(data),
        }
    }

    pub fn rejected(request_id: &str, code: ErrorCode, message: impl Into<String>) -> Self {
        Self::refused(request_id, ResponseStatus::Rejected, code, message)
    }

    pub fn failed(request_id: &str, code: ErrorCode, message: impl Into<String>) -> Self {
        Self::refused(request_id, ResponseStatus::Failed, code, message)
    }

    fn refused(
        request_id: &str,
        status: ResponseStatus,
        code: ErrorCode,
        message: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            request_id: request_id.to_owned(),
            status,
            receipt: None,
            error: Some(StructuredError {
                code,
                message: message.into(),
            }),
            data: None,
        }
    }
}

/// One permitted (task, action, suite) combination.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct TaskGrant {
    pub task_id: String,
    pub action: RunnerAction,
    /// Required for `ExecuteLiveSuite`; the grant names exactly one suite.
    pub suite: Option<LiveSuite>,
}

/// Root-owned authorization for one campaign. It names what may run; the caller never does.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Authorization {
    pub schema_version: u32,
    pub controller: ControllerKind,
    pub campaign_id: String,
    pub owner_uid: u32,
    pub owner_gid: u32,
    /// Canonical repository path; all executable and output paths derive from it.
    pub repository: String,
    pub manifest_sha256: String,
    pub runner_sha256: String,
    /// Digests of the pinned campaign binaries, verified before every launch.
    pub xtask_sha256: String,
    pub stutter_sha256: String,
    /// `PATH` the suites run with, captured from the bootstrapping environment.
    pub path_env: String,
    pub grants: BTreeSet<TaskGrant>,
    /// Hard cap on reboots; the installer lowers it to `min(candidates + reboot_slack, max_reboots)`.
    pub max_reboots: u32,
    pub reboot_slack: u32,
    pub original_boot_entry: Option<String>,
    pub allowed_boot_entries: Vec<String>,
    pub created_unix_ms: u64,
}
