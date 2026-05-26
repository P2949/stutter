//! Agent audit-event helpers.

use super::*;

pub(crate) fn audit_agent_event(
    action_id: &'static str,
    success: bool,
    affected_tasks: usize,
    message: String,
) {
    audit_agent_event_with_safety(
        action_id,
        SafetyClass::ObserveOnly,
        success,
        affected_tasks,
        message,
    );
}

pub(crate) fn audit_agent_event_with_safety(
    action_id: &'static str,
    safety_class: SafetyClass,
    success: bool,
    affected_tasks: usize,
    message: String,
) {
    crate::audit::audit_or_warn(&crate::audit::AuditEvent {
        schema_version: 1,
        unix_nanos: crate::audit::unix_nanos_now(),
        command: "agent".to_owned(),
        action_id: Some(action_id.to_owned()),
        safety_class: Some(safety_class),
        dry_run: false,
        success,
        affected_tasks,
        restore_path: None,
        action_phase: None,
        error_category: None,
        message,
    });
}
