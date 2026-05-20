//! Privilege boundary allowlist decisions.

use super::{PrivilegeCommandRequest, PrivilegeDecision, PrivilegeProcessRole};

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
