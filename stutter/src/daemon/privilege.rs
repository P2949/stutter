use serde::{Deserialize, Serialize};

use crate::{actions::SafetyClass, audit::AuditEvent};

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
}
