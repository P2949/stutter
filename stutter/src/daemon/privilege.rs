use std::{
    fmt, fs,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::{FileTypeExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::{
    actions::{ActionState, RollbackToken, SafetyClass, TaskIdentity, runner::ActionRunPolicy},
    audit::{AuditEvent, append_audit_event_to_path},
    autotune::{
        apply::executor_for_candidate,
        candidate::{CandidateAction, CandidateDryRunRecord, CandidatePlanFile},
        objective::ObjectiveKind,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrivilegeCommandAllowlist;

impl PrivilegeCommandAllowlist {
    pub fn check(&self, request: &PrivilegeCommandRequest) -> PrivilegeDecision {
        let worker_required = request.operation.requires_privileged_worker();
        let audit_required = worker_required || request.operation.requires_apply_authorization();

        if request.caller_role == PrivilegeProcessRole::PrivilegedWorker {
            return allow(
                "privileged_worker_execution_allowed",
                worker_required,
                audit_required,
            );
        }

        if worker_required && request.caller_role != PrivilegeProcessRole::ControlPlane {
            return deny(
                "caller_role_cannot_request_privileged_operation",
                "only the control plane may request privileged worker operations",
                worker_required,
                audit_required,
            );
        }

        if !request.transport.is_local() && request.operation.requires_apply_authorization() {
            return deny(
                "non_local_privileged_operation",
                "privileged daemon operations must use a local control transport",
                worker_required,
                audit_required,
            );
        }

        if !request.authenticated {
            return deny(
                "missing_authentication",
                "privilege boundary request is not authenticated",
                worker_required,
                audit_required,
            );
        }

        if request.operation.requires_apply_authorization() && !request.apply_authorized {
            return deny(
                "missing_apply_authorization",
                "privileged daemon operation requires apply/control authorization",
                worker_required,
                audit_required,
            );
        }

        if worker_required {
            allow(
                "allowlisted_control_plane_worker_request",
                worker_required,
                audit_required,
            )
        } else {
            allow(
                "allowlisted_unprivileged_operation",
                worker_required,
                audit_required,
            )
        }
    }
}

pub fn privileged_operation_audit_event(
    operation: PrivilegedOperation,
    success: bool,
    message: impl Into<String>,
) -> AuditEvent {
    AuditEvent {
        schema_version: 1,
        unix_nanos: crate::audit::unix_nanos_now(),
        command: "daemon_privilege".to_owned(),
        action_id: Some(operation.audit_action_id().to_owned()),
        safety_class: Some(operation.minimum_safety_class()),
        dry_run: false,
        success,
        affected_tasks: 0,
        restore_path: None,
        action_phase: None,
        error_category: None,
        message: message.into(),
    }
}

#[derive(Clone, Debug)]
pub struct PrivilegeAuditSink {
    path: PathBuf,
}

impl Default for PrivilegeAuditSink {
    fn default() -> Self {
        Self {
            path: crate::audit::default_audit_log_path(),
        }
    }
}

impl PrivilegeAuditSink {
    pub fn to_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn record_boundary(&self, input: PrivilegeBoundaryAuditInput<'_>) -> anyhow::Result<()> {
        let mut message = format!(
            "stage={} verdict={} reason_code={} caller_role={} transport={} operation={} policy_intent={} request={} action_id={} error_category={} detail={}",
            input.stage,
            if input.success { "allow" } else { "deny" },
            input.reason_code,
            input.caller_role.as_str(),
            input.transport.as_str(),
            input.operation.as_str(),
            input
                .policy_intent
                .as_ref()
                .map(|intent| format!("{intent:?}"))
                .unwrap_or_else(|| "none".to_owned()),
            input.request_kind,
            input
                .descriptor
                .map(|descriptor| descriptor.action_id.0.as_str())
                .unwrap_or("none"),
            input.reason_code,
            input.detail
        );
        if let Some(descriptor) = input.descriptor {
            message.push_str(&format!(
                " action_kind={} safety_class={:?}",
                descriptor.action_kind, descriptor.safety_class
            ));
        }

        append_audit_event_to_path(
            &self.path,
            &AuditEvent {
                schema_version: 1,
                unix_nanos: crate::audit::unix_nanos_now(),
                command: "daemon_privilege".to_owned(),
                action_id: input
                    .descriptor
                    .map(|descriptor| descriptor.action_id.0.clone())
                    .or_else(|| Some(input.operation.audit_action_id().to_owned())),
                safety_class: input
                    .descriptor
                    .map(|descriptor| descriptor.safety_class.clone())
                    .or_else(|| Some(input.operation.minimum_safety_class())),
                dry_run: input.policy_intent == Some(PolicyIntent::DryRun),
                success: input.success,
                affected_tasks: input.affected_tasks,
                restore_path: None,
                action_phase: None,
                error_category: Some(input.reason_code.to_owned()),
                message,
            },
        )
    }
}

#[derive(Clone, Debug)]
struct PrivilegeBoundaryAuditInput<'a> {
    stage: &'static str,
    request_kind: &'static str,
    operation: PrivilegedOperation,
    policy_intent: Option<PolicyIntent>,
    caller_role: PrivilegeProcessRole,
    transport: PrivilegeTransport,
    descriptor: Option<&'a ActionDescriptor>,
    success: bool,
    reason_code: &'a str,
    detail: &'a str,
    affected_tasks: usize,
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

pub trait PrivilegedActionService: fmt::Debug + Send + Sync {
    fn dry_run_candidate(
        &self,
        request: CandidateApplyRequest,
    ) -> anyhow::Result<CandidateDryRunRecord>;
    fn apply_candidate(&self, request: CandidateApplyRequest) -> anyhow::Result<ApplyResult>;
    fn rollback(&self, request: RollbackRequest) -> anyhow::Result<RollbackResult>;
}

#[derive(Clone, Debug)]
pub struct InProcessPrivilegedActionService {
    proc_root: PathBuf,
    audit_sink: PrivilegeAuditSink,
}

impl Default for InProcessPrivilegedActionService {
    fn default() -> Self {
        Self {
            proc_root: PathBuf::from("/proc"),
            audit_sink: PrivilegeAuditSink::default(),
        }
    }
}

impl InProcessPrivilegedActionService {
    pub fn with_proc_root(proc_root: impl Into<PathBuf>) -> Self {
        Self {
            proc_root: proc_root.into(),
            audit_sink: PrivilegeAuditSink::default(),
        }
    }

    pub fn with_audit_path(audit_path: impl Into<PathBuf>) -> Self {
        Self {
            audit_sink: PrivilegeAuditSink::to_path(audit_path),
            ..Self::default()
        }
    }

    pub fn with_proc_root_and_audit_path(
        proc_root: impl Into<PathBuf>,
        audit_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            proc_root: proc_root.into(),
            audit_sink: PrivilegeAuditSink::to_path(audit_path),
        }
    }

    fn audit_candidate_boundary(
        &self,
        stage: &'static str,
        request_kind: &'static str,
        intent: PolicyIntent,
        descriptor: &ActionDescriptor,
        success: bool,
        reason_code: &str,
        detail: &str,
        affected_tasks: usize,
    ) {
        self.record_boundary_audit(PrivilegeBoundaryAuditInput {
            stage,
            request_kind,
            operation: PrivilegedOperation::ApplyAction,
            policy_intent: Some(intent),
            caller_role: PrivilegeProcessRole::PrivilegedWorker,
            transport: PrivilegeTransport::LocalCli,
            descriptor: Some(descriptor),
            success,
            reason_code,
            detail,
            affected_tasks,
        });
    }

    fn audit_rollback_boundary(
        &self,
        stage: &'static str,
        descriptor: &ActionDescriptor,
        success: bool,
        reason_code: &str,
        detail: &str,
        affected_tasks: usize,
    ) {
        self.record_boundary_audit(PrivilegeBoundaryAuditInput {
            stage,
            request_kind: "rollback",
            operation: PrivilegedOperation::RollbackAction,
            policy_intent: Some(PolicyIntent::Rollback),
            caller_role: PrivilegeProcessRole::PrivilegedWorker,
            transport: PrivilegeTransport::LocalCli,
            descriptor: Some(descriptor),
            success,
            reason_code,
            detail,
            affected_tasks,
        });
    }

    fn record_boundary_audit(&self, input: PrivilegeBoundaryAuditInput<'_>) {
        if let Err(err) = self.audit_sink.record_boundary(input) {
            log::warn!(
                "privilege_boundary_audit_failed path={} err={err:#}",
                self.audit_sink.path().display()
            );
        }
    }
}

impl PrivilegedActionService for InProcessPrivilegedActionService {
    fn dry_run_candidate(
        &self,
        request: CandidateApplyRequest,
    ) -> anyhow::Result<CandidateDryRunRecord> {
        let descriptor = request.plan.descriptor.clone();
        self.audit_candidate_boundary(
            "request_received",
            "dry_run",
            PolicyIntent::DryRun,
            &descriptor,
            true,
            "request_received",
            "privilege dry-run request received",
            0,
        );
        if let Err(err) = validate_candidate_plan_request(&request, PolicyIntent::DryRun) {
            let detail = format!("{err:#}");
            let reason_code = stable_error_reason_code(&detail);
            self.audit_candidate_boundary(
                "policy_validation",
                "dry_run",
                PolicyIntent::DryRun,
                &descriptor,
                false,
                &reason_code,
                &detail,
                0,
            );
            return Err(err);
        }
        self.audit_candidate_boundary(
            "policy_validation",
            "dry_run",
            PolicyIntent::DryRun,
            &descriptor,
            true,
            "policy_allowed",
            "candidate plan passed dry-run validation",
            0,
        );
        let executor = executor_for_candidate(request.plan.candidate)?;
        let result = executor.dry_run();
        match result {
            Ok(record) => {
                self.audit_candidate_boundary(
                    "dry_run_completed",
                    "dry_run",
                    PolicyIntent::DryRun,
                    &descriptor,
                    true,
                    "dry_run_completed",
                    "privileged dry-run completed",
                    record.affected_tasks,
                );
                Ok(record)
            }
            Err(err) => {
                let detail = format!("{err:#}");
                let reason_code = stable_error_reason_code(&detail);
                self.audit_candidate_boundary(
                    "dry_run_failed",
                    "dry_run",
                    PolicyIntent::DryRun,
                    &descriptor,
                    false,
                    &reason_code,
                    &detail,
                    0,
                );
                Err(err)
            }
        }
    }

    fn apply_candidate(&self, request: CandidateApplyRequest) -> anyhow::Result<ApplyResult> {
        let descriptor = request.plan.descriptor.clone();
        self.audit_candidate_boundary(
            "request_received",
            "apply",
            PolicyIntent::Apply,
            &descriptor,
            true,
            "request_received",
            "privilege apply request received",
            0,
        );
        if let Err(err) = validate_candidate_plan_request(&request, PolicyIntent::Apply) {
            let detail = format!("{err:#}");
            let reason_code = stable_error_reason_code(&detail);
            self.audit_candidate_boundary(
                "policy_validation",
                "apply",
                PolicyIntent::Apply,
                &descriptor,
                false,
                &reason_code,
                &detail,
                0,
            );
            return Err(err);
        }
        self.audit_candidate_boundary(
            "policy_validation",
            "apply",
            PolicyIntent::Apply,
            &descriptor,
            true,
            "policy_allowed",
            "candidate plan passed apply validation",
            0,
        );
        if let Err(err) = revalidate_candidate_targets(&request.plan.candidate, &self.proc_root) {
            let detail = format!("{err:#}");
            let reason_code = stable_error_reason_code(&detail);
            self.audit_candidate_boundary(
                "target_revalidation",
                "apply",
                PolicyIntent::Apply,
                &descriptor,
                false,
                &reason_code,
                &detail,
                0,
            );
            return Err(err);
        }
        self.audit_candidate_boundary(
            "apply_started",
            "apply",
            PolicyIntent::Apply,
            &descriptor,
            true,
            "apply_started",
            "privileged apply started",
            0,
        );
        let executor = executor_for_candidate(request.plan.candidate)?;
        let run_policy = ActionRunPolicy {
            policy: request.policy,
            context: request.context,
            max_affected_tasks: None,
            max_total_duration: None,
            dry_run: false,
        };
        let result = match executor.apply_with_audit(run_policy) {
            Ok(result) => result,
            Err(err) => {
                let detail = format!("{err:#}");
                let reason_code = stable_error_reason_code(&detail);
                self.audit_candidate_boundary(
                    "apply_failed",
                    "apply",
                    PolicyIntent::Apply,
                    &descriptor,
                    false,
                    &reason_code,
                    &detail,
                    0,
                );
                return Err(err);
            }
        };
        let rollback = match result.rollback {
            Some(rollback) => rollback,
            None => {
                let detail = "privileged apply completed without rollback token";
                self.audit_candidate_boundary(
                    "apply_failed",
                    "apply",
                    PolicyIntent::Apply,
                    &descriptor,
                    false,
                    "privileged_apply_missing_rollback",
                    detail,
                    result.state.affected_tasks,
                );
                anyhow::bail!("{detail}");
            }
        };
        self.audit_candidate_boundary(
            "apply_completed",
            "apply",
            PolicyIntent::Apply,
            &descriptor,
            true,
            "apply_completed",
            "privileged apply completed",
            result.state.affected_tasks,
        );
        Ok(ApplyResult {
            state: result.state,
            rollback,
        })
    }

    fn rollback(&self, request: RollbackRequest) -> anyhow::Result<RollbackResult> {
        let descriptor = request.candidate.descriptor();
        self.audit_rollback_boundary(
            "rollback_requested",
            &descriptor,
            true,
            "rollback_requested",
            "privilege rollback request received",
            request.token.affected_tasks(),
        );
        if let Err(err) = request.policy.check_action_with_context(
            PolicyIntent::Rollback,
            &descriptor,
            &request.context,
        ) {
            let detail = format!("{err:#}");
            let reason_code = stable_error_reason_code(&detail);
            self.audit_rollback_boundary(
                "policy_validation",
                &descriptor,
                false,
                &reason_code,
                &detail,
                request.token.affected_tasks(),
            );
            return Err(err.into());
        }
        self.audit_rollback_boundary(
            "policy_validation",
            &descriptor,
            true,
            "policy_allowed",
            "rollback policy validation passed",
            request.token.affected_tasks(),
        );
        let affected_tasks = request.token.affected_tasks();
        let executor = executor_for_candidate(request.candidate)?;
        if let Err(err) = executor.rollback(&request.token) {
            let detail = format!("{err:#}");
            let reason_code = stable_error_reason_code(&detail);
            self.audit_rollback_boundary(
                "rollback_failed",
                &descriptor,
                false,
                &reason_code,
                &detail,
                affected_tasks,
            );
            return Err(err);
        }
        self.audit_rollback_boundary(
            "rollback_completed",
            &descriptor,
            true,
            "rollback_completed",
            "privileged rollback completed",
            affected_tasks,
        );
        Ok(RollbackResult { affected_tasks })
    }
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

    fn into_plan_request(self) -> anyhow::Result<CandidatePlanRequest> {
        if self.schema_version != PRIVILEGED_WORKER_IPC_SCHEMA_VERSION {
            anyhow::bail!(
                "privileged_worker_unsupported_schema: got {} expected {}",
                self.schema_version,
                PRIVILEGED_WORKER_IPC_SCHEMA_VERSION
            );
        }

        if self.plan_file.schema_version != CandidatePlanFile::SCHEMA_VERSION {
            anyhow::bail!(
                "privileged_worker_unsupported_candidate_plan_schema: got {} expected {}",
                self.plan_file.schema_version,
                CandidatePlanFile::SCHEMA_VERSION
            );
        }

        if self.plan_file.descriptor.action_id != self.descriptor.action_id
            || self.plan_file.descriptor.action_kind != self.descriptor.action_kind
            || self.plan_file.descriptor.safety_class != self.descriptor.safety_class
            || self.plan_file.descriptor.effect_scope != self.descriptor.effect_scope
            || self.plan_file.objective != self.objective
        {
            anyhow::bail!(
                "privileged_worker_candidate_plan_metadata_mismatch: candidate plan metadata does not match worker request"
            );
        }

        let Some(executable) = self.plan_file.executable else {
            if let Some(reason) = self.plan_file.manual_only_reason.as_deref() {
                anyhow::bail!(
                    "privileged_worker_candidate_plan_manual_only: candidate '{}' action_kind={} reason={}",
                    self.plan_file.candidate.candidate_name,
                    self.plan_file.candidate.action_kind,
                    reason
                );
            }
            anyhow::bail!(
                "privileged_worker_candidate_plan_not_executable: candidate '{}' action_kind={} has no executable payload",
                self.plan_file.candidate.candidate_name,
                self.plan_file.candidate.action_kind
            );
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
    fn operation(&self) -> PrivilegedOperation {
        match self {
            Self::DryRun { .. } | Self::Apply { .. } => PrivilegedOperation::ApplyAction,
            Self::Rollback { .. } => PrivilegedOperation::RollbackAction,
            Self::Shutdown => PrivilegedOperation::ControlDaemon,
        }
    }

    fn request_name(&self) -> &'static str {
        match self {
            Self::DryRun { .. } => "dry_run",
            Self::Apply { .. } => "apply",
            Self::Rollback { .. } => "rollback",
            Self::Shutdown => "shutdown",
        }
    }

    fn policy_intent(&self) -> Option<PolicyIntent> {
        match self {
            Self::DryRun { .. } => Some(PolicyIntent::DryRun),
            Self::Apply { .. } => Some(PolicyIntent::Apply),
            Self::Rollback { .. } => Some(PolicyIntent::Rollback),
            Self::Shutdown => None,
        }
    }

    fn descriptor(&self) -> Option<&ActionDescriptor> {
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
    fn from_error(error: anyhow::Error) -> Self {
        let message = format!("{error:#}");
        let reason_code = stable_error_reason_code(&message);
        Self::Error {
            reason_code,
            message,
        }
    }

    fn into_error(self) -> anyhow::Error {
        match self {
            Self::Error {
                reason_code,
                message,
            } => anyhow::anyhow!("{reason_code}: {message}"),
            other => anyhow::anyhow!("privileged_worker_unexpected_response: {other:?}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct UnixSocketPrivilegedActionService {
    socket_path: PathBuf,
}

impl UnixSocketPrivilegedActionService {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    fn call_worker(
        &self,
        request: PrivilegedWorkerRequest,
    ) -> anyhow::Result<PrivilegedWorkerResponse> {
        let mut stream = UnixStream::connect(&self.socket_path).with_context(|| {
            format!(
                "failed to connect to privileged worker socket {}",
                self.socket_path.display()
            )
        })?;
        serde_json::to_writer(&mut stream, &request)?;
        stream.write_all(b"\n")?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).with_context(|| {
            format!(
                "failed to read privileged worker response from {}",
                self.socket_path.display()
            )
        })?;
        if line.trim().is_empty() {
            anyhow::bail!(
                "privileged_worker_empty_response: {} returned no response",
                self.socket_path.display()
            );
        }

        serde_json::from_str(&line).with_context(|| {
            format!(
                "failed to decode privileged worker response from {}",
                self.socket_path.display()
            )
        })
    }

    #[cfg(test)]
    pub fn request_shutdown_for_tests(&self) -> anyhow::Result<()> {
        match self.call_worker(PrivilegedWorkerRequest::Shutdown)? {
            PrivilegedWorkerResponse::Shutdown { .. } => Ok(()),
            response @ PrivilegedWorkerResponse::Error { .. } => Err(response.into_error()),
            other => anyhow::bail!("privileged_worker_unexpected_response: {other:?}"),
        }
    }
}

impl PrivilegedActionService for UnixSocketPrivilegedActionService {
    fn dry_run_candidate(
        &self,
        request: CandidateApplyRequest,
    ) -> anyhow::Result<CandidateDryRunRecord> {
        let response = self.call_worker(PrivilegedWorkerRequest::DryRun {
            plan: PrivilegedWorkerCandidatePlan::from_plan_request(&request.plan),
            policy: request.policy,
            context: request.context,
            max_plan_age_nanos: request.max_plan_age_nanos,
        })?;
        match response {
            PrivilegedWorkerResponse::DryRun { record } => Ok(record),
            response @ PrivilegedWorkerResponse::Error { .. } => Err(response.into_error()),
            other => anyhow::bail!("privileged_worker_unexpected_response: {other:?}"),
        }
    }

    fn apply_candidate(&self, request: CandidateApplyRequest) -> anyhow::Result<ApplyResult> {
        let response = self.call_worker(PrivilegedWorkerRequest::Apply {
            plan: PrivilegedWorkerCandidatePlan::from_plan_request(&request.plan),
            policy: request.policy,
            context: request.context,
            max_plan_age_nanos: request.max_plan_age_nanos,
        })?;
        match response {
            PrivilegedWorkerResponse::Apply { result } => Ok(result),
            response @ PrivilegedWorkerResponse::Error { .. } => Err(response.into_error()),
            other => anyhow::bail!("privileged_worker_unexpected_response: {other:?}"),
        }
    }

    fn rollback(&self, request: RollbackRequest) -> anyhow::Result<RollbackResult> {
        let response = self.call_worker(PrivilegedWorkerRequest::Rollback {
            plan: PrivilegedWorkerCandidatePlan::from_plan_request(
                &CandidatePlanRequest::from_candidate(
                    request.candidate,
                    crate::audit::unix_nanos_now(),
                ),
            ),
            token: request.token,
            policy: request.policy,
            context: request.context,
        })?;
        match response {
            PrivilegedWorkerResponse::Rollback { result } => Ok(result),
            response @ PrivilegedWorkerResponse::Error { .. } => Err(response.into_error()),
            other => anyhow::bail!("privileged_worker_unexpected_response: {other:?}"),
        }
    }
}

pub fn default_privileged_worker_socket_path() -> anyhow::Result<PathBuf> {
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(runtime_dir).join("stutter-privileged-worker.sock"));
    }

    let Some(home) = std::env::var_os("HOME") else {
        anyhow::bail!(
            "cannot choose default privileged worker socket path without XDG_RUNTIME_DIR or HOME"
        );
    };

    Ok(PathBuf::from(home)
        .join(".local")
        .join("state")
        .join("stutter")
        .join("privileged-worker.sock"))
}

pub fn run_privileged_worker(socket_path: &Path) -> anyhow::Result<()> {
    let service = InProcessPrivilegedActionService::default();
    run_privileged_worker_with_service(socket_path, &service)
}

pub fn run_privileged_worker_with_service(
    socket_path: &Path,
    service: &dyn PrivilegedActionService,
) -> anyhow::Result<()> {
    prepare_privileged_worker_socket_path(socket_path)?;
    let listener = UnixListener::bind(socket_path).with_context(|| {
        format!(
            "failed to bind privileged worker socket {}",
            socket_path.display()
        )
    })?;
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600)).with_context(|| {
        format!(
            "failed to set permissions on privileged worker socket {}",
            socket_path.display()
        )
    })?;

    log::info!(
        "privileged_worker_listening socket={} auth=filesystem_permissions mode=0600",
        socket_path.display()
    );

    let result = loop {
        let (stream, _) = listener.accept().with_context(|| {
            format!(
                "failed to accept privileged worker connection on {}",
                socket_path.display()
            )
        })?;
        match handle_privileged_worker_connection(stream, service) {
            Ok(true) => break Ok(()),
            Ok(false) => {}
            Err(err) => {
                log::warn!("privileged_worker_connection_failed err={err:#}");
            }
        }
    };

    let cleanup_result = fs::remove_file(socket_path).with_context(|| {
        format!(
            "failed to remove privileged worker socket {}",
            socket_path.display()
        )
    });
    result.and(cleanup_result)
}

fn prepare_privileged_worker_socket_path(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create privileged worker socket directory {}",
                parent.display()
            )
        })?;
    }

    if !path.exists() {
        return Ok(());
    }

    let metadata = fs::symlink_metadata(path).with_context(|| {
        format!(
            "failed to inspect existing privileged worker socket {}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_socket() {
        anyhow::bail!(
            "privileged_worker_socket_refusing_non_socket: refusing to replace {}",
            path.display()
        );
    }

    fs::remove_file(path).with_context(|| {
        format!(
            "failed to remove stale privileged worker socket {}",
            path.display()
        )
    })
}

fn handle_privileged_worker_connection(
    mut stream: UnixStream,
    service: &dyn PrivilegedActionService,
) -> anyhow::Result<bool> {
    let request = match read_privileged_worker_request(&stream) {
        Ok(request) => request,
        Err(err) => {
            let response = PrivilegedWorkerResponse::from_error(err);
            serde_json::to_writer(&mut stream, &response)?;
            stream.write_all(b"\n")?;
            stream.flush()?;
            return Ok(false);
        }
    };
    let shutdown = matches!(request, PrivilegedWorkerRequest::Shutdown);
    let response = execute_privileged_worker_request(request, service);
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(shutdown)
}

fn read_privileged_worker_request(stream: &UnixStream) -> anyhow::Result<PrivilegedWorkerRequest> {
    let reader_stream = stream.try_clone()?;
    let mut reader = BufReader::new(reader_stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.trim().is_empty() {
        anyhow::bail!("privileged_worker_empty_request: request body was empty");
    }

    serde_json::from_str(&line)
        .with_context(|| "privileged_worker_decode_request_failed: invalid worker request JSON")
}

fn execute_privileged_worker_request(
    request: PrivilegedWorkerRequest,
    service: &dyn PrivilegedActionService,
) -> PrivilegedWorkerResponse {
    let audit_sink = PrivilegeAuditSink::default();
    execute_privileged_worker_request_with_audit_sink(request, service, &audit_sink)
}

fn execute_privileged_worker_request_with_audit_sink(
    request: PrivilegedWorkerRequest,
    service: &dyn PrivilegedActionService,
    audit_sink: &PrivilegeAuditSink,
) -> PrivilegedWorkerResponse {
    let operation = request.operation();
    let request_name = request.request_name();
    let policy_intent = request.policy_intent();
    let descriptor = request.descriptor().cloned();
    let decision = PrivilegeCommandAllowlist.check(&PrivilegeCommandRequest {
        caller_role: PrivilegeProcessRole::ControlPlane,
        operation,
        transport: PrivilegeTransport::UnixSocket,
        authenticated: true,
        apply_authorized: true,
    });

    log::info!(
        "privileged_worker_boundary_decision request={} operation={} allowed={} reason_code={}",
        request_name,
        operation.as_str(),
        decision.allowed,
        decision.reason_code
    );

    record_worker_boundary_audit(
        audit_sink,
        PrivilegeBoundaryAuditInput {
            stage: "allowlist_decision",
            request_kind: request_name,
            operation,
            policy_intent: policy_intent.clone(),
            caller_role: PrivilegeProcessRole::ControlPlane,
            transport: PrivilegeTransport::UnixSocket,
            descriptor: descriptor.as_ref(),
            success: decision.allowed,
            reason_code: &decision.reason_code,
            detail: &decision.message,
            affected_tasks: 0,
        },
    );

    if !decision.allowed {
        return PrivilegedWorkerResponse::Error {
            reason_code: decision.reason_code,
            message: decision.message,
        };
    }

    let result = match request {
        PrivilegedWorkerRequest::DryRun {
            plan,
            policy,
            context,
            max_plan_age_nanos,
        } => plan.into_plan_request().and_then(|plan| {
            service
                .dry_run_candidate(CandidateApplyRequest {
                    plan,
                    policy,
                    context,
                    max_plan_age_nanos,
                })
                .map(|record| PrivilegedWorkerResponse::DryRun { record })
        }),
        PrivilegedWorkerRequest::Apply {
            plan,
            policy,
            context,
            max_plan_age_nanos,
        } => plan.into_plan_request().and_then(|plan| {
            service
                .apply_candidate(CandidateApplyRequest {
                    plan,
                    policy,
                    context,
                    max_plan_age_nanos,
                })
                .map(|result| PrivilegedWorkerResponse::Apply { result })
        }),
        PrivilegedWorkerRequest::Rollback {
            plan,
            token,
            policy,
            context,
        } => plan.into_plan_request().and_then(|plan| {
            service
                .rollback(RollbackRequest {
                    candidate: plan.candidate,
                    token,
                    policy,
                    context,
                })
                .map(|result| PrivilegedWorkerResponse::Rollback { result })
        }),
        PrivilegedWorkerRequest::Shutdown => Ok(PrivilegedWorkerResponse::Shutdown {
            message: "privileged worker shutdown requested".to_owned(),
        }),
    };

    let response = result.unwrap_or_else(PrivilegedWorkerResponse::from_error);
    let (success, stage, reason_code, detail, affected_tasks) = match &response {
        PrivilegedWorkerResponse::DryRun { record } => (
            true,
            "dry_run_completed",
            "dry_run_completed",
            "privileged worker dry-run completed",
            record.affected_tasks,
        ),
        PrivilegedWorkerResponse::Apply { result } => (
            true,
            "apply_completed",
            "apply_completed",
            "privileged worker apply completed",
            result.state.affected_tasks,
        ),
        PrivilegedWorkerResponse::Rollback { result } => (
            true,
            "rollback_completed",
            "rollback_completed",
            "privileged worker rollback completed",
            result.affected_tasks,
        ),
        PrivilegedWorkerResponse::Shutdown { .. } => (
            true,
            "shutdown_completed",
            "shutdown_completed",
            "privileged worker shutdown completed",
            0,
        ),
        PrivilegedWorkerResponse::Error {
            reason_code,
            message,
        } => (
            false,
            "request_failed",
            reason_code.as_str(),
            message.as_str(),
            0,
        ),
    };
    record_worker_boundary_audit(
        audit_sink,
        PrivilegeBoundaryAuditInput {
            stage,
            request_kind: request_name,
            operation,
            policy_intent,
            caller_role: PrivilegeProcessRole::ControlPlane,
            transport: PrivilegeTransport::UnixSocket,
            descriptor: descriptor.as_ref(),
            success,
            reason_code,
            detail,
            affected_tasks,
        },
    );

    response
}

fn record_worker_boundary_audit(
    audit_sink: &PrivilegeAuditSink,
    input: PrivilegeBoundaryAuditInput<'_>,
) {
    if let Err(err) = audit_sink.record_boundary(input) {
        log::warn!(
            "privileged_worker_boundary_audit_failed path={} err={err:#}",
            audit_sink.path().display()
        );
    }
}

fn stable_error_reason_code(message: &str) -> String {
    message
        .split_once(':')
        .map(|(code, _)| code.trim())
        .filter(|code| {
            !code.is_empty()
                && code
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        })
        .unwrap_or("privileged_worker_request_failed")
        .to_owned()
}

mod serde_u128_string {
    use serde::{Deserialize, Deserializer, Serializer, de};

    pub fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(de::Error::custom)
    }
}

fn validate_candidate_plan_request(
    request: &CandidateApplyRequest,
    intent: PolicyIntent,
) -> anyhow::Result<()> {
    let now = crate::audit::unix_nanos_now();
    if request.max_plan_age_nanos > 0
        && now.saturating_sub(request.plan.created_unix_nanos) > request.max_plan_age_nanos
    {
        anyhow::bail!("stale_candidate_plan: candidate plan timestamp is too old");
    }

    let live_descriptor = request.plan.candidate.descriptor();
    if live_descriptor.action_id != request.plan.descriptor.action_id
        || live_descriptor.action_kind != request.plan.descriptor.action_kind
        || live_descriptor.safety_class != request.plan.descriptor.safety_class
        || live_descriptor.effect_scope != request.plan.descriptor.effect_scope
    {
        anyhow::bail!(
            "candidate_plan_descriptor_mismatch: candidate plan descriptor does not match action payload"
        );
    }

    if request.plan.objective != request.plan.candidate.objective() {
        anyhow::bail!("candidate_plan_objective_mismatch: candidate objective changed");
    }

    if request.plan.evidence_count == 0
        && request.plan.candidate.evidence().is_empty()
        && request.plan.candidate.action_kind() != "cpu_affinity_profile"
    {
        anyhow::bail!("candidate_plan_missing_evidence: candidate plan has no evidence");
    }

    request
        .policy
        .check_action_with_context(intent, &request.plan.descriptor, &request.context)?;

    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetRevalidationError {
    MissingTid {
        tid: u32,
    },
    StarttimeMismatch {
        tid: u32,
        expected: u64,
        actual: Option<u64>,
    },
    ProcessPidMismatch {
        tid: u32,
        expected: u32,
        actual: Option<u32>,
    },
    CommMismatch {
        tid: u32,
        expected: String,
        actual: Option<String>,
    },
}

impl TargetRevalidationError {
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::MissingTid { .. } => "target_revalidation_missing_tid",
            Self::StarttimeMismatch { .. } => "target_revalidation_starttime_mismatch",
            Self::ProcessPidMismatch { .. } => "target_revalidation_process_pid_mismatch",
            Self::CommMismatch { .. } => "target_revalidation_comm_mismatch",
        }
    }
}

impl fmt::Display for TargetRevalidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTid { tid } => {
                write!(f, "{}: tid={tid} is missing", self.reason_code())
            }
            Self::StarttimeMismatch {
                tid,
                expected,
                actual,
            } => write!(
                f,
                "{}: tid={tid} expected_starttime={expected} actual_starttime={actual:?}",
                self.reason_code()
            ),
            Self::ProcessPidMismatch {
                tid,
                expected,
                actual,
            } => write!(
                f,
                "{}: tid={tid} expected_process_pid={expected} actual_process_pid={actual:?}",
                self.reason_code()
            ),
            Self::CommMismatch {
                tid,
                expected,
                actual,
            } => write!(
                f,
                "{}: tid={tid} expected_comm={expected:?} actual_comm={actual:?}",
                self.reason_code()
            ),
        }
    }
}

impl std::error::Error for TargetRevalidationError {}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LiveTaskIdentity {
    tid: u32,
    process_pid: Option<u32>,
    comm: Option<String>,
    starttime_ticks: Option<u64>,
}

fn revalidate_candidate_targets(
    candidate: &CandidateAction,
    proc_root: &Path,
) -> anyhow::Result<()> {
    for target in candidate_task_identities(candidate) {
        revalidate_task_identity(&target, proc_root)
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    }

    Ok(())
}

fn revalidate_task_identity(
    target: &TaskIdentity,
    proc_root: &Path,
) -> Result<(), TargetRevalidationError> {
    let live = read_live_task_identity(proc_root, target)
        .ok_or(TargetRevalidationError::MissingTid { tid: target.tid })?;

    if let Some(expected_pid) = target.process_pid
        && live.process_pid != Some(expected_pid)
    {
        return Err(TargetRevalidationError::ProcessPidMismatch {
            tid: target.tid,
            expected: expected_pid,
            actual: live.process_pid,
        });
    }

    if let Some(expected_starttime) = target.starttime_ticks
        && live.starttime_ticks != Some(expected_starttime)
    {
        return Err(TargetRevalidationError::StarttimeMismatch {
            tid: target.tid,
            expected: expected_starttime,
            actual: live.starttime_ticks,
        });
    }

    if let Some(expected_comm) = target.comm.as_deref()
        && live.comm.as_deref() != Some(expected_comm)
    {
        return Err(TargetRevalidationError::CommMismatch {
            tid: target.tid,
            expected: expected_comm.to_owned(),
            actual: live.comm,
        });
    }

    Ok(())
}

fn candidate_task_identities(candidate: &CandidateAction) -> Vec<TaskIdentity> {
    match candidate {
        CandidateAction::Nice { plan } => plan.action.targets.clone(),
        CandidateAction::IoPrio { plan } => plan.action.targets.clone(),
        CandidateAction::Uclamp { plan } => plan.action.targets.clone(),
        CandidateAction::CgroupPlacement { plan } => plan
            .action
            .targets
            .iter()
            .map(|target| target.identity.clone())
            .collect(),
        CandidateAction::CpuAffinityProfile { plan } => vec![TaskIdentity {
            tid: plan.tree_pid,
            process_pid: Some(plan.tree_pid),
            comm: None,
            starttime_ticks: None,
        }],
        _ => Vec::new(),
    }
}

fn read_live_task_identity(proc_root: &Path, target: &TaskIdentity) -> Option<LiveTaskIdentity> {
    if let Some(process_pid) = target.process_pid {
        let task_stat = proc_root
            .join(process_pid.to_string())
            .join("task")
            .join(target.tid.to_string())
            .join("stat");
        if let Some(identity) = read_identity_from_stat(&task_stat, target.tid, Some(process_pid)) {
            return Some(identity);
        }

        let actual = read_top_level_task_identity(proc_root, target.tid);
        if actual.is_some() {
            return actual;
        }

        return None;
    }

    read_top_level_task_identity(proc_root, target.tid)
}

fn read_top_level_task_identity(proc_root: &Path, tid: u32) -> Option<LiveTaskIdentity> {
    let stat_path = proc_root.join(tid.to_string()).join("stat");
    let mut identity = read_identity_from_stat(&stat_path, tid, None)?;
    identity.process_pid = read_tgid_from_status(&proc_root.join(tid.to_string()).join("status"));
    Some(identity)
}

fn read_identity_from_stat(
    path: &Path,
    expected_tid: u32,
    process_pid: Option<u32>,
) -> Option<LiveTaskIdentity> {
    let stat = std::fs::read_to_string(path).ok()?;
    let (stat_tid, comm) = parse_proc_stat_tid_and_comm(&stat)?;
    if stat_tid != expected_tid {
        return None;
    }

    Some(LiveTaskIdentity {
        tid: expected_tid,
        process_pid,
        comm: Some(comm),
        starttime_ticks: crate::process_tree::parse_proc_stat_starttime(&stat),
    })
}

fn parse_proc_stat_tid_and_comm(stat: &str) -> Option<(u32, String)> {
    let open = stat.find('(')?;
    let close = stat.rfind(") ")?;
    let tid = stat[..open].trim().parse().ok()?;
    Some((tid, stat[open + 1..close].to_owned()))
}

fn read_tgid_from_status(path: &Path) -> Option<u32> {
    let status = std::fs::read_to_string(path).ok()?;
    status.lines().find_map(|line| {
        let value = line.strip_prefix("Tgid:")?;
        value.trim().parse().ok()
    })
}

fn allow(
    reason_code: &'static str,
    privileged_worker_required: bool,
    audit_required: bool,
) -> PrivilegeDecision {
    PrivilegeDecision {
        allowed: true,
        reason_code: reason_code.to_owned(),
        message: "operation is allowed by the privilege boundary".to_owned(),
        privileged_worker_required,
        audit_required,
    }
}

fn deny(
    reason_code: &'static str,
    message: &'static str,
    privileged_worker_required: bool,
    audit_required: bool,
) -> PrivilegeDecision {
    PrivilegeDecision {
        allowed: false,
        reason_code: reason_code.to_owned(),
        message: message.to_owned(),
        privileged_worker_required,
        audit_required,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::{Arc, Mutex},
        thread,
        time::Duration,
    };

    use super::*;

    fn check(request: PrivilegeCommandRequest) -> PrivilegeDecision {
        PrivilegeCommandAllowlist.check(&request)
    }

    #[test]
    fn control_plane_can_request_privileged_worker_operation_over_unix_socket() {
        let decision = check(PrivilegeCommandRequest {
            caller_role: PrivilegeProcessRole::ControlPlane,
            operation: PrivilegedOperation::ApplyAction,
            transport: PrivilegeTransport::UnixSocket,
            authenticated: true,
            apply_authorized: true,
        });

        assert!(decision.allowed);
        assert!(decision.privileged_worker_required);
        assert!(decision.audit_required);
        assert_eq!(
            decision.reason_code,
            "allowlisted_control_plane_worker_request"
        );
    }

    #[test]
    fn ui_client_cannot_request_privileged_worker_operation() {
        let decision = check(PrivilegeCommandRequest {
            caller_role: PrivilegeProcessRole::UiClient,
            operation: PrivilegedOperation::RollbackAction,
            transport: PrivilegeTransport::UnixSocket,
            authenticated: true,
            apply_authorized: true,
        });

        assert!(!decision.allowed);
        assert_eq!(
            decision.reason_code,
            "caller_role_cannot_request_privileged_operation"
        );
    }

    #[test]
    fn remote_tcp_cannot_request_privileged_operation_even_with_apply_auth() {
        let decision = check(PrivilegeCommandRequest {
            caller_role: PrivilegeProcessRole::ControlPlane,
            operation: PrivilegedOperation::StartRecording,
            transport: PrivilegeTransport::RemoteTcp,
            authenticated: true,
            apply_authorized: true,
        });

        assert!(!decision.allowed);
        assert_eq!(decision.reason_code, "non_local_privileged_operation");
    }

    #[test]
    fn loopback_tcp_privileged_operation_requires_apply_authorization() {
        let decision = check(PrivilegeCommandRequest {
            caller_role: PrivilegeProcessRole::ControlPlane,
            operation: PrivilegedOperation::ControlDaemon,
            transport: PrivilegeTransport::LoopbackTcp,
            authenticated: true,
            apply_authorized: false,
        });

        assert!(!decision.allowed);
        assert_eq!(decision.reason_code, "missing_apply_authorization");
    }

    #[test]
    fn status_read_is_unprivileged_but_still_requires_authenticated_boundary_context() {
        let denied = check(PrivilegeCommandRequest {
            caller_role: PrivilegeProcessRole::UiClient,
            operation: PrivilegedOperation::ReadStatus,
            transport: PrivilegeTransport::RemoteTcp,
            authenticated: false,
            apply_authorized: false,
        });
        let allowed = check(PrivilegeCommandRequest {
            caller_role: PrivilegeProcessRole::UiClient,
            operation: PrivilegedOperation::ReadStatus,
            transport: PrivilegeTransport::RemoteTcp,
            authenticated: true,
            apply_authorized: false,
        });

        assert!(!denied.allowed);
        assert_eq!(denied.reason_code, "missing_authentication");
        assert!(allowed.allowed);
        assert!(!allowed.privileged_worker_required);
        assert!(!allowed.audit_required);
    }

    #[test]
    fn privileged_worker_can_execute_allowlisted_worker_operations() {
        let decision = check(PrivilegeCommandRequest {
            caller_role: PrivilegeProcessRole::PrivilegedWorker,
            operation: PrivilegedOperation::LoadEbpf,
            transport: PrivilegeTransport::LocalCli,
            authenticated: false,
            apply_authorized: false,
        });

        assert!(decision.allowed);
        assert_eq!(decision.reason_code, "privileged_worker_execution_allowed");
        assert!(decision.privileged_worker_required);
    }

    #[test]
    fn privileged_operation_audit_event_uses_stable_action_id() {
        let event = privileged_operation_audit_event(
            PrivilegedOperation::RollbackAction,
            false,
            "rollback denied",
        );

        assert_eq!(event.command, "daemon_privilege");
        assert_eq!(
            event.action_id.as_deref(),
            Some("privilege-rollback-action")
        );
        assert_eq!(event.safety_class, Some(SafetyClass::ReversibleLowRisk));
        assert!(!event.success);
        assert_eq!(event.message, "rollback denied");
    }

    fn fake_apply_request() -> CandidateApplyRequest {
        let candidate = CandidateAction::Fake {
            action_id: crate::actions::ActionId("fake:privilege".to_owned()),
            safety_class: SafetyClass::ReversibleLowRisk,
        };
        CandidateApplyRequest {
            plan: CandidatePlanRequest::from_candidate(candidate, crate::audit::unix_nanos_now()),
            policy: DaemonPolicy::apply_low_risk(crate::daemon_policy::ActionSource::Test),
            context: DaemonPolicyContext::default(),
            max_plan_age_nanos: 1_000_000_000,
        }
    }

    fn nice_candidate(target: TaskIdentity) -> CandidateAction {
        CandidateAction::Nice {
            plan: crate::autotune::candidate::NiceActionPlan {
                name: format!("nice-{}", target.tid),
                action: crate::actions::nice::NiceAction {
                    targets: vec![target],
                    nice: 5,
                    policy: crate::actions::nice::NicePolicy::default(),
                },
                target_root_pid: Some(100),
                evidence: vec![crate::autotune::candidate::CandidateEvidence::new(
                    "test",
                    "target revalidation test",
                    1.0,
                )],
                objective: ObjectiveKind::DesktopInteractivity,
            },
        }
    }

    fn nice_apply_request(candidate: CandidateAction) -> CandidateApplyRequest {
        CandidateApplyRequest {
            plan: CandidatePlanRequest::from_candidate(candidate, crate::audit::unix_nanos_now()),
            policy: DaemonPolicy::apply_medium_risk(crate::daemon_policy::ActionSource::Test),
            context: DaemonPolicyContext::default(),
            max_plan_age_nanos: 1_000_000_000,
        }
    }

    fn target(tid: u32, process_pid: u32, comm: &str, starttime: u64) -> TaskIdentity {
        TaskIdentity {
            tid,
            process_pid: Some(process_pid),
            comm: Some(comm.to_owned()),
            starttime_ticks: Some(starttime),
        }
    }

    fn temp_proc_root(name: &str) -> PathBuf {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "stutter-target-revalidation-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn temp_audit_path(name: &str) -> PathBuf {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "stutter-privilege-audit-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&root).unwrap();
        root.join("audit.jsonl")
    }

    fn read_audit_events(path: &std::path::Path) -> Vec<AuditEvent> {
        crate::audit::read_audit_tail(path, 100).unwrap()
    }

    fn proc_stat(tid: u32, comm: &str, starttime: u64) -> String {
        let mut fields = vec!["S".to_owned()];
        fields.extend((0..18).map(|_| "0".to_owned()));
        fields.push(starttime.to_string());
        fields.extend((0..24).map(|_| "0".to_owned()));
        format!("{tid} ({comm}) {}\n", fields.join(" "))
    }

    fn write_expected_task(
        proc_root: &std::path::Path,
        process_pid: u32,
        tid: u32,
        comm: &str,
        starttime: u64,
    ) {
        let task_dir = proc_root
            .join(process_pid.to_string())
            .join("task")
            .join(tid.to_string());
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(task_dir.join("stat"), proc_stat(tid, comm, starttime)).unwrap();
    }

    fn write_top_level_task(
        proc_root: &std::path::Path,
        tgid: u32,
        tid: u32,
        comm: &str,
        starttime: u64,
    ) {
        let task_dir = proc_root.join(tid.to_string());
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(task_dir.join("stat"), proc_stat(tid, comm, starttime)).unwrap();
        fs::write(
            task_dir.join("status"),
            format!("Name:\t{comm}\nTgid:\t{tgid}\n"),
        )
        .unwrap();
    }

    #[derive(Debug, Default)]
    struct FakeWorkerService {
        dry_run_calls: Mutex<usize>,
        apply_calls: Mutex<usize>,
        rollback_calls: Mutex<usize>,
    }

    impl FakeWorkerService {
        fn calls(&self, field: &Mutex<usize>) -> usize {
            *field.lock().unwrap()
        }
    }

    impl PrivilegedActionService for FakeWorkerService {
        fn dry_run_candidate(
            &self,
            request: CandidateApplyRequest,
        ) -> anyhow::Result<CandidateDryRunRecord> {
            *self.dry_run_calls.lock().unwrap() += 1;
            Ok(CandidateDryRunRecord {
                candidate_name: request.plan.candidate.candidate_name().to_owned(),
                affected_tasks: 2,
                warnings: Vec::new(),
                safety_class: request.plan.candidate.safety_class(),
                eligible: true,
                reason: None,
            })
        }

        fn apply_candidate(&self, _request: CandidateApplyRequest) -> anyhow::Result<ApplyResult> {
            *self.apply_calls.lock().unwrap() += 1;
            Ok(ApplyResult {
                state: ActionState {
                    applied: true,
                    affected_tasks: 2,
                    checked_tasks: 2,
                    pending_changes: 2,
                    warnings: Vec::new(),
                },
                rollback: RollbackToken::NiceRestore {
                    records: Vec::new(),
                },
            })
        }

        fn rollback(&self, request: RollbackRequest) -> anyhow::Result<RollbackResult> {
            *self.rollback_calls.lock().unwrap() += 1;
            Ok(RollbackResult {
                affected_tasks: request.token.affected_tasks(),
            })
        }
    }

    fn temp_socket_path(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "stutter-privileged-worker-{name}-{}-{}.sock",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        path
    }

    fn wait_for_socket(path: &std::path::Path) {
        for _ in 0..100 {
            if path.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for socket {}", path.display());
    }

    #[test]
    fn unix_socket_privileged_worker_round_trips_apply_and_rollback() {
        let socket = temp_socket_path("round-trip");
        let service = Arc::new(FakeWorkerService::default());
        let worker_service = Arc::clone(&service);
        let worker_socket = socket.clone();
        let handle = thread::spawn(move || {
            run_privileged_worker_with_service(&worker_socket, worker_service.as_ref())
        });
        wait_for_socket(&socket);

        let metadata = fs::metadata(&socket).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);

        let client = UnixSocketPrivilegedActionService::new(&socket);
        let candidate = nice_candidate(target(200, 100, "worker", 12345));
        let apply = client
            .apply_candidate(nice_apply_request(candidate.clone()))
            .unwrap();
        assert_eq!(apply.state.affected_tasks, 2);

        let rollback = client
            .rollback(RollbackRequest {
                candidate,
                token: apply.rollback,
                policy: DaemonPolicy::apply_medium_risk(crate::daemon_policy::ActionSource::Test),
                context: DaemonPolicyContext::default(),
            })
            .unwrap();
        assert_eq!(rollback.affected_tasks, 0);

        client.request_shutdown_for_tests().unwrap();
        handle.join().unwrap().unwrap();

        assert_eq!(service.calls(&service.apply_calls), 1);
        assert_eq!(service.calls(&service.rollback_calls), 1);
        assert!(!socket.exists());
    }

    #[test]
    fn privileged_worker_rejects_non_executable_candidate_plan() {
        let request = PrivilegedWorkerRequest::Apply {
            plan: PrivilegedWorkerCandidatePlan::from_plan_request(&fake_apply_request().plan),
            policy: DaemonPolicy::apply_low_risk(crate::daemon_policy::ActionSource::Test),
            context: DaemonPolicyContext::default(),
            max_plan_age_nanos: 1_000_000_000,
        };

        let response = execute_privileged_worker_request(request, &FakeWorkerService::default());

        let PrivilegedWorkerResponse::Error { reason_code, .. } = response else {
            panic!("expected worker error");
        };
        assert_eq!(
            reason_code,
            "privileged_worker_candidate_plan_not_executable"
        );
    }

    #[test]
    fn privilege_audit_records_stale_plan_denial() {
        let audit_path = temp_audit_path("stale");
        let service = InProcessPrivilegedActionService::with_audit_path(&audit_path);
        let mut request = fake_apply_request();
        request.plan.created_unix_nanos = 1;
        request.max_plan_age_nanos = 1;

        let _ = service.apply_candidate(request).unwrap_err();

        let events = read_audit_events(&audit_path);
        assert!(events.iter().any(|event| {
            event.message.contains("stage=request_received")
                && event.message.contains("policy_intent=Apply")
        }));
        assert!(events.iter().any(|event| {
            !event.success
                && event.error_category.as_deref() == Some("stale_candidate_plan")
                && event.message.contains("stage=policy_validation")
        }));
        fs::remove_dir_all(audit_path.parent().unwrap()).ok();
    }

    #[test]
    fn privilege_audit_records_descriptor_mismatch_denial() {
        let audit_path = temp_audit_path("descriptor");
        let service = InProcessPrivilegedActionService::with_audit_path(&audit_path);
        let mut request = fake_apply_request();
        request.plan.descriptor.action_kind = "nice".to_owned();

        let _ = service.apply_candidate(request).unwrap_err();

        let events = read_audit_events(&audit_path);
        assert!(events.iter().any(|event| {
            !event.success
                && event.error_category.as_deref() == Some("candidate_plan_descriptor_mismatch")
        }));
        fs::remove_dir_all(audit_path.parent().unwrap()).ok();
    }

    #[test]
    fn privilege_audit_records_missing_evidence_denial() {
        let audit_path = temp_audit_path("missing-evidence");
        let service = InProcessPrivilegedActionService::with_audit_path(&audit_path);
        let mut request = fake_apply_request();
        request.plan.evidence_count = 0;

        let _ = service.apply_candidate(request).unwrap_err();

        let events = read_audit_events(&audit_path);
        assert!(events.iter().any(|event| {
            !event.success
                && event.error_category.as_deref() == Some("candidate_plan_missing_evidence")
        }));
        fs::remove_dir_all(audit_path.parent().unwrap()).ok();
    }

    #[test]
    fn privilege_audit_records_successful_worker_apply_and_rollback() {
        let audit_path = temp_audit_path("worker-success");
        let audit_sink = PrivilegeAuditSink::to_path(&audit_path);
        let service = FakeWorkerService::default();
        let candidate = nice_candidate(target(200, 100, "worker", 12345));
        let apply_request = PrivilegedWorkerRequest::Apply {
            plan: PrivilegedWorkerCandidatePlan::from_plan_request(
                &nice_apply_request(candidate.clone()).plan,
            ),
            policy: DaemonPolicy::apply_medium_risk(crate::daemon_policy::ActionSource::Test),
            context: DaemonPolicyContext::default(),
            max_plan_age_nanos: 1_000_000_000,
        };
        let apply_response =
            execute_privileged_worker_request_with_audit_sink(apply_request, &service, &audit_sink);
        let rollback_token = match apply_response {
            PrivilegedWorkerResponse::Apply { result } => result.rollback,
            other => panic!("expected apply response, got {other:?}"),
        };

        let rollback_request = PrivilegedWorkerRequest::Rollback {
            plan: PrivilegedWorkerCandidatePlan::from_plan_request(
                &CandidatePlanRequest::from_candidate(candidate, crate::audit::unix_nanos_now()),
            ),
            token: rollback_token,
            policy: DaemonPolicy::apply_medium_risk(crate::daemon_policy::ActionSource::Test),
            context: DaemonPolicyContext::default(),
        };
        let rollback_response = execute_privileged_worker_request_with_audit_sink(
            rollback_request,
            &service,
            &audit_sink,
        );
        assert!(matches!(
            rollback_response,
            PrivilegedWorkerResponse::Rollback { .. }
        ));

        let events = read_audit_events(&audit_path);
        assert!(events.iter().any(|event| {
            event.success
                && event.error_category.as_deref() == Some("apply_completed")
                && event.message.contains("stage=apply_completed")
        }));
        assert!(events.iter().any(|event| {
            event.success
                && event.error_category.as_deref() == Some("rollback_completed")
                && event.message.contains("stage=rollback_completed")
        }));
        fs::remove_dir_all(audit_path.parent().unwrap()).ok();
    }

    #[test]
    fn privileged_action_service_rejects_stale_candidate_plan_before_execution() {
        let service = InProcessPrivilegedActionService::default();
        let mut request = fake_apply_request();
        request.plan.created_unix_nanos = 1;
        request.max_plan_age_nanos = 1;

        let err = service.apply_candidate(request).unwrap_err().to_string();

        assert!(err.contains("stale_candidate_plan"));
    }

    #[test]
    fn privileged_action_service_rechecks_descriptor_integrity() {
        let service = InProcessPrivilegedActionService::default();
        let mut request = fake_apply_request();
        request.plan.descriptor.action_kind = "nice".to_owned();

        let err = service.apply_candidate(request).unwrap_err().to_string();

        assert!(err.contains("candidate_plan_descriptor_mismatch"));
    }

    #[test]
    fn privileged_action_service_rejects_payload_without_evidence() {
        let service = InProcessPrivilegedActionService::default();
        let mut request = fake_apply_request();
        request.plan.evidence_count = 0;

        let err = service.apply_candidate(request).unwrap_err().to_string();

        assert!(err.contains("candidate_plan_missing_evidence"));
    }

    #[test]
    fn target_revalidation_accepts_valid_task_identity() {
        let proc_root = temp_proc_root("valid");
        write_expected_task(&proc_root, 100, 200, "worker", 12345);
        let candidate = nice_candidate(target(200, 100, "worker", 12345));

        revalidate_candidate_targets(&candidate, &proc_root).unwrap();

        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn target_revalidation_rejects_missing_tid() {
        let proc_root = temp_proc_root("missing");
        let candidate = nice_candidate(target(200, 100, "worker", 12345));

        let err = revalidate_candidate_targets(&candidate, &proc_root)
            .unwrap_err()
            .to_string();

        assert!(err.contains("target_revalidation_missing_tid"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn target_revalidation_rejects_reused_tid_starttime() {
        let proc_root = temp_proc_root("reused");
        write_expected_task(&proc_root, 100, 200, "worker", 99999);
        let candidate = nice_candidate(target(200, 100, "worker", 12345));

        let err = revalidate_candidate_targets(&candidate, &proc_root)
            .unwrap_err()
            .to_string();

        assert!(err.contains("target_revalidation_starttime_mismatch"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn target_revalidation_rejects_process_pid_mismatch() {
        let proc_root = temp_proc_root("pid-mismatch");
        write_top_level_task(&proc_root, 201, 200, "worker", 12345);
        let candidate = nice_candidate(target(200, 100, "worker", 12345));

        let err = revalidate_candidate_targets(&candidate, &proc_root)
            .unwrap_err()
            .to_string();

        assert!(err.contains("target_revalidation_process_pid_mismatch"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn target_revalidation_rejects_comm_mismatch() {
        let proc_root = temp_proc_root("comm-mismatch");
        write_expected_task(&proc_root, 100, 200, "other", 12345);
        let candidate = nice_candidate(target(200, 100, "worker", 12345));

        let err = revalidate_candidate_targets(&candidate, &proc_root)
            .unwrap_err()
            .to_string();

        assert!(err.contains("target_revalidation_comm_mismatch"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn privileged_apply_revalidates_targets_before_execution() {
        let proc_root = temp_proc_root("service-comm-mismatch");
        write_expected_task(&proc_root, 100, 200, "other", 12345);
        let service = InProcessPrivilegedActionService::with_proc_root(&proc_root);
        let request = nice_apply_request(nice_candidate(target(200, 100, "worker", 12345)));

        let err = service.apply_candidate(request).unwrap_err().to_string();

        assert!(err.contains("target_revalidation_comm_mismatch"));
        fs::remove_dir_all(proc_root).ok();
    }
}
