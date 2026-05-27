//! Privileged action and worker IPC data models.

use serde::{Deserialize, Serialize};

use super::{PrivilegedWorkerError, serde_u128_string, stable_error_reason_code};
use crate::{
    actions::{ActionState, RollbackToken, SafetyClass},
    autotune::{
        objective::ObjectiveKind,
        planning::{
            candidate::CandidateAction, dry_run::CandidateDryRunRecord, plan_io::CandidatePlanFile,
        },
    },
    daemon_policy::{ActionDescriptor, DaemonPolicy, DaemonPolicyContext, PolicyIntent},
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrivilegeProcessRole {
    PrivilegedWorker,
    ControlPlane,
    UiClient,
}

impl PrivilegeProcessRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PrivilegedWorker => "privileged_worker",
            Self::ControlPlane => "control_plane",
            Self::UiClient => "ui_client",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrivilegeTransport {
    LocalCli,
    UnixSocket,
    LoopbackTcp,
    RemoteTcp,
}

impl PrivilegeTransport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalCli => "local_cli",
            Self::UnixSocket => "unix_socket",
            Self::LoopbackTcp => "loopback_tcp",
            Self::RemoteTcp => "remote_tcp",
        }
    }

    pub fn is_local(self) -> bool {
        matches!(self, Self::LocalCli | Self::UnixSocket | Self::LoopbackTcp)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrivilegedOperation {
    ReadStatus,
    ExplainPolicy,
    ProbeCapabilities,
    LoadEbpf,
    AttachProbe,
    StartRecording,
    StopRecording,
    ControlDaemon,
    ApplyAction,
    RollbackAction,
    WriteProtectedState,
}

impl PrivilegedOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadStatus => "read_status",
            Self::ExplainPolicy => "explain_policy",
            Self::ProbeCapabilities => "probe_capabilities",
            Self::LoadEbpf => "load_ebpf",
            Self::AttachProbe => "attach_probe",
            Self::StartRecording => "start_recording",
            Self::StopRecording => "stop_recording",
            Self::ControlDaemon => "control_daemon",
            Self::ApplyAction => "apply_action",
            Self::RollbackAction => "rollback_action",
            Self::WriteProtectedState => "write_protected_state",
        }
    }

    pub fn requires_privileged_worker(self) -> bool {
        matches!(
            self,
            Self::LoadEbpf
                | Self::AttachProbe
                | Self::StartRecording
                | Self::StopRecording
                | Self::ApplyAction
                | Self::RollbackAction
                | Self::WriteProtectedState
        )
    }

    pub fn requires_apply_authorization(self) -> bool {
        !matches!(
            self,
            Self::ReadStatus | Self::ExplainPolicy | Self::ProbeCapabilities
        )
    }

    pub fn audit_action_id(self) -> &'static str {
        match self {
            Self::ReadStatus => "privilege-read-status",
            Self::ExplainPolicy => "privilege-explain-policy",
            Self::ProbeCapabilities => "privilege-probe-capabilities",
            Self::LoadEbpf => "privilege-load-ebpf",
            Self::AttachProbe => "privilege-attach-probe",
            Self::StartRecording => "privilege-start-recording",
            Self::StopRecording => "privilege-stop-recording",
            Self::ControlDaemon => "privilege-control-daemon",
            Self::ApplyAction => "privilege-apply-action",
            Self::RollbackAction => "privilege-rollback-action",
            Self::WriteProtectedState => "privilege-write-protected-state",
        }
    }

    pub fn minimum_safety_class(self) -> SafetyClass {
        match self {
            Self::ReadStatus | Self::ExplainPolicy | Self::ProbeCapabilities => {
                SafetyClass::ObserveOnly
            }
            Self::LoadEbpf
            | Self::AttachProbe
            | Self::StartRecording
            | Self::StopRecording
            | Self::ControlDaemon
            | Self::RollbackAction
            | Self::WriteProtectedState => SafetyClass::ReversibleLowRisk,
            Self::ApplyAction => SafetyClass::ReversibleLowRisk,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivilegeCommandRequest {
    pub caller_role: PrivilegeProcessRole,
    pub operation: PrivilegedOperation,
    pub transport: PrivilegeTransport,
    pub authenticated: bool,
    pub apply_authorized: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivilegeDecision {
    pub allowed: bool,
    pub reason_code: String,
    pub message: String,
    pub privileged_worker_required: bool,
    pub audit_required: bool,
}

#[derive(Clone, Debug)]
pub struct CandidatePlanRequest {
    pub candidate: CandidateAction,
    pub descriptor: ActionDescriptor,
    pub objective: ObjectiveKind,
    pub evidence_count: usize,
    pub created_unix_nanos: u128,
}

impl CandidatePlanRequest {
    pub fn from_candidate(candidate: CandidateAction, created_unix_nanos: u128) -> Self {
        Self {
            descriptor: candidate.descriptor(),
            objective: candidate.objective(),
            evidence_count: candidate.evidence().len(),
            candidate,
            created_unix_nanos,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CandidateApplyRequest {
    pub plan: CandidatePlanRequest,
    pub policy: DaemonPolicy,
    pub context: DaemonPolicyContext,
    pub max_plan_age_nanos: u128,
}

#[derive(Clone, Debug)]
pub struct RollbackRequest {
    pub candidate: CandidateAction,
    pub token: RollbackToken,
    pub policy: DaemonPolicy,
    pub context: DaemonPolicyContext,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApplyResult {
    pub state: ActionState,
    pub rollback: RollbackToken,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RollbackResult {
    pub affected_tasks: usize,
}

pub const PRIVILEGED_WORKER_IPC_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrivilegedWorkerCandidatePlan {
    pub schema_version: u32,
    pub plan_file: CandidatePlanFile,
    pub descriptor: ActionDescriptor,
    pub objective: ObjectiveKind,
    pub evidence_count: usize,
    #[serde(with = "serde_u128_string")]
    pub created_unix_nanos: u128,
}

impl PrivilegedWorkerCandidatePlan {
    pub fn from_plan_request(request: &CandidatePlanRequest) -> Self {
        Self {
            schema_version: PRIVILEGED_WORKER_IPC_SCHEMA_VERSION,
            plan_file: CandidatePlanFile::from_candidate(&request.candidate, None),
            descriptor: request.descriptor.clone(),
            objective: request.objective,
            evidence_count: request.evidence_count,
            created_unix_nanos: request.created_unix_nanos,
        }
    }

    pub(crate) fn into_plan_request(self) -> anyhow::Result<CandidatePlanRequest> {
        if self.schema_version != PRIVILEGED_WORKER_IPC_SCHEMA_VERSION {
            return Err(PrivilegedWorkerError::UnsupportedSchema {
                got: self.schema_version,
                expected: PRIVILEGED_WORKER_IPC_SCHEMA_VERSION,
            }
            .into());
        }

        if self.plan_file.schema_version != CandidatePlanFile::SCHEMA_VERSION {
            return Err(PrivilegedWorkerError::UnsupportedCandidatePlanSchema {
                got: self.plan_file.schema_version,
                expected: CandidatePlanFile::SCHEMA_VERSION,
            }
            .into());
        }

        if self.plan_file.descriptor.action_id != self.descriptor.action_id
            || self.plan_file.descriptor.action_kind != self.descriptor.action_kind
            || self.plan_file.descriptor.safety_class != self.descriptor.safety_class
            || self.plan_file.descriptor.effect_scope != self.descriptor.effect_scope
            || self.plan_file.objective != self.objective
        {
            return Err(PrivilegedWorkerError::CandidatePlanMetadataMismatch.into());
        }

        let Some(executable) = self.plan_file.executable else {
            if let Some(reason) = self.plan_file.manual_only_reason {
                return Err(PrivilegedWorkerError::CandidatePlanManualOnly {
                    candidate_name: self.plan_file.candidate.candidate_name,
                    action_kind: self.plan_file.candidate.action_kind,
                    reason,
                }
                .into());
            }

            return Err(PrivilegedWorkerError::CandidatePlanNotExecutable {
                candidate_name: self.plan_file.candidate.candidate_name,
                action_kind: self.plan_file.candidate.action_kind,
            }
            .into());
        };

        let candidate = executable.into_candidate();
        Ok(CandidatePlanRequest {
            candidate,
            descriptor: self.descriptor,
            objective: self.objective,
            evidence_count: self.evidence_count,
            created_unix_nanos: self.created_unix_nanos,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum PrivilegedWorkerRequest {
    DryRun {
        plan: PrivilegedWorkerCandidatePlan,
        policy: DaemonPolicy,
        context: DaemonPolicyContext,
        #[serde(with = "serde_u128_string")]
        max_plan_age_nanos: u128,
    },
    Apply {
        plan: PrivilegedWorkerCandidatePlan,
        policy: DaemonPolicy,
        context: DaemonPolicyContext,
        #[serde(with = "serde_u128_string")]
        max_plan_age_nanos: u128,
    },
    Rollback {
        plan: PrivilegedWorkerCandidatePlan,
        token: RollbackToken,
        policy: DaemonPolicy,
        context: DaemonPolicyContext,
    },
    Shutdown,
}

impl PrivilegedWorkerRequest {
    pub(crate) fn operation(&self) -> PrivilegedOperation {
        match self {
            Self::DryRun { .. } | Self::Apply { .. } => PrivilegedOperation::ApplyAction,
            Self::Rollback { .. } => PrivilegedOperation::RollbackAction,
            Self::Shutdown => PrivilegedOperation::ControlDaemon,
        }
    }

    pub(crate) fn request_name(&self) -> &'static str {
        match self {
            Self::DryRun { .. } => "dry_run",
            Self::Apply { .. } => "apply",
            Self::Rollback { .. } => "rollback",
            Self::Shutdown => "shutdown",
        }
    }

    pub(crate) fn policy_intent(&self) -> Option<PolicyIntent> {
        match self {
            Self::DryRun { .. } => Some(PolicyIntent::DryRun),
            Self::Apply { .. } => Some(PolicyIntent::Apply),
            Self::Rollback { .. } => Some(PolicyIntent::Rollback),
            Self::Shutdown => None,
        }
    }

    pub(crate) fn descriptor(&self) -> Option<&ActionDescriptor> {
        match self {
            Self::DryRun { plan, .. } | Self::Apply { plan, .. } | Self::Rollback { plan, .. } => {
                Some(&plan.descriptor)
            }
            Self::Shutdown => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PrivilegedWorkerResponse {
    DryRun {
        record: CandidateDryRunRecord,
    },
    Apply {
        result: ApplyResult,
    },
    Rollback {
        result: RollbackResult,
    },
    Shutdown {
        message: String,
    },
    Error {
        reason_code: String,
        message: String,
    },
}

impl PrivilegedWorkerResponse {
    pub(crate) fn from_error(error: anyhow::Error) -> Self {
        let reason_code = error
            .downcast_ref::<PrivilegedWorkerError>()
            .map(|error| error.reason_code().to_owned());

        let message = format!("{error:#}");
        let reason_code = reason_code.unwrap_or_else(|| stable_error_reason_code(&message));

        Self::Error {
            reason_code,
            message,
        }
    }

    pub(crate) fn into_error(self) -> anyhow::Error {
        match self {
            Self::Error {
                reason_code,
                message,
            } => anyhow::anyhow!("{reason_code}: {message}"),
            other => anyhow::anyhow!("privileged_worker_unexpected_response: {other:?}"),
        }
    }
}
