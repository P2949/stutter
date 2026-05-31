//! Privilege boundary audit event construction and sinks.

use std::path::{Path, PathBuf};

use super::{PrivilegeProcessRole, PrivilegeTransport, PrivilegedOperation};
use crate::{
    audit::{AuditEvent, append_audit_event_to_path},
    daemon_policy::{ActionDescriptor, PolicyIntent},
};

pub fn privileged_operation_audit_event(
    operation: PrivilegedOperation,
    success: bool,
    message: impl Into<String>,
) -> AuditEvent {
    AuditEvent {
        schema_version: 1,
        unix_nanos: crate::audit::unix_nanos_now(),
        command: "daemon_privilege".to_owned(),
        action_id: Some(crate::actions::ActionId::new(operation.audit_action_id())),
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
        Self::to_path(crate::audit::default_audit_log_path())
    }
}

impl PrivilegeAuditSink {
    pub fn to_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn record_boundary(
        &self,
        input: PrivilegeBoundaryAuditInput<'_>,
    ) -> anyhow::Result<()> {
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
                .map(|descriptor| descriptor.action_id.as_str())
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
                    .map(|descriptor| descriptor.action_id.clone())
                    .or_else(|| {
                        Some(crate::actions::ActionId::new(
                            input.operation.audit_action_id(),
                        ))
                    }),
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
pub(crate) struct PrivilegeBoundaryAuditInput<'a> {
    pub(crate) stage: &'static str,
    pub(crate) request_kind: &'static str,
    pub(crate) operation: PrivilegedOperation,
    pub(crate) policy_intent: Option<PolicyIntent>,
    pub(crate) caller_role: PrivilegeProcessRole,
    pub(crate) transport: PrivilegeTransport,
    pub(crate) descriptor: Option<&'a ActionDescriptor>,
    pub(crate) success: bool,
    pub(crate) reason_code: &'a str,
    pub(crate) detail: &'a str,
    pub(crate) affected_tasks: usize,
}

pub(crate) struct CandidateBoundaryAuditInput<'a> {
    pub(crate) stage: &'static str,
    pub(crate) request_kind: &'static str,
    pub(crate) intent: PolicyIntent,
    pub(crate) descriptor: &'a ActionDescriptor,
    pub(crate) success: bool,
    pub(crate) reason_code: &'a str,
    pub(crate) detail: &'a str,
    pub(crate) affected_tasks: usize,
}
