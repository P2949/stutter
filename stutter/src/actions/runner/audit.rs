//! Audit event update helpers for the audited action runner.
//!
//! Owns mutation of runner audit event snapshots and append failure logging. It must not evaluate
//! policy, perform rollback, or execute action phases.

use std::path::{Path, PathBuf};

use crate::{
    actions::ActionPhase,
    audit::{AuditEvent, append_audit_event_to_path, unix_nanos_now},
};

pub(super) struct RunnerAuditEventUpdate {
    phase: Option<ActionPhase>,
    success: bool,
    affected_tasks: usize,
    restore_path: Option<PathBuf>,
    error_category: Option<String>,
    message: String,
}

impl RunnerAuditEventUpdate {
    pub(super) fn new(
        phase: Option<ActionPhase>,
        success: bool,
        affected_tasks: usize,
        restore_path: Option<PathBuf>,
        error_category: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            phase,
            success,
            affected_tasks,
            restore_path,
            error_category,
            message: message.into(),
        }
    }
}

pub(super) fn append_runner_audit_event(
    audit_path: &Path,
    base_event: &AuditEvent,
    update: RunnerAuditEventUpdate,
) {
    let mut event = base_event.clone();
    event.unix_nanos = unix_nanos_now();
    event.action_phase = update.phase;
    event.success = update.success;
    event.affected_tasks = update.affected_tasks;
    event.restore_path = update.restore_path;
    event.error_category = update.error_category;
    event.message = update.message;

    if let Err(audit_err) = append_audit_event_to_path(audit_path, &event) {
        log::warn!(
            "failed to write audit event to {}: {audit_err:#}",
            audit_path.display()
        );
    }
}
