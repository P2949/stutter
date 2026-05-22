use std::path::Path;
use anyhow::Context;
use crate::{audit::{AuditEvent, append_audit_event_to_path}, actions::RollbackToken, autotune::history::*};
use super::{manual_command::*};

pub(super) fn write_emergency_restore_audit_event(
    audit_path: &Path,
    action_id: &str,
    rollback_token: &RollbackToken,
    success: bool,
    affected_tasks: usize,
    message: String,
) -> anyhow::Result<()> {
    let event = AuditEvent {
        schema_version: 1,
        unix_nanos: crate::audit::unix_nanos_now(),
        command: "autotune emergency restore".to_owned(),
        action_id: Some(action_id.to_owned()),
        safety_class: Some(safety_class_for_rollback_token(rollback_token)),
        dry_run: false,
        success,
        affected_tasks,
        restore_path: rollback_token.restore_path().cloned(),
        action_phase: None,
        error_category: None,
        message,
    };

    append_audit_event_to_path(audit_path, &event).with_context(|| {
        format!(
            "failed to write emergency restore audit event to {}",
            audit_path.display()
        )
    })
}

pub(super) struct EmergencyRestoreHistoryEventInput<'a> {
    pub(super) history_path: &'a Path,
    pub(super) phase: ControllerPhase,
    pub(super) decision: &'a str,
    pub(super) experiment_id: &'a str,
    pub(super) action_id: &'a str,
    pub(super) rollback_token: &'a RollbackToken,
    pub(super) rollback_performed: bool,
    pub(super) reason: String,
}

pub(super) fn write_emergency_restore_history_event(
    input: EmergencyRestoreHistoryEventInput<'_>,
) -> anyhow::Result<()> {
    let event = AutotuneHistoryEvent::new(AutotuneHistoryEventInput {
        controller_id: "emergency-restore".to_owned(),
        phase: input.phase,
        mode: AutotuneMode::ApplyLowRisk,
        target: None,
        situation: SituationKind::Unknown,
        observation_summary: empty_observation_summary(),
        decision: AutotuneDecisionSummary {
            decision: input.decision.to_owned(),
            candidate_name: candidate_name_from_action_id(input.action_id),
            action_kind: Some(action_kind_from_action_id(input.action_id)),
            safety_class: Some(safety_class_for_rollback_token(input.rollback_token)),
            eligible: input.rollback_performed,
            rollback_policy: "emergency-restore".to_owned(),
        },
        reason: input.reason,
    })
    .with_experiment_id(input.experiment_id.to_owned())
    .with_action_id(input.action_id.to_owned())
    .with_rollback_performed(input.rollback_performed);

    append_autotune_history_event(input.history_path, &event).with_context(|| {
        format!(
            "failed to write emergency restore history event to {}",
            input.history_path.display()
        )
    })
}

pub(super) fn empty_observation_summary() -> ObservationSummary {
    ObservationSummary {
        target_present: false,
        active_target_count: 0,
        scored_task_count: 0,
        interval_count: 0,
        scored_samples: 0,
        score_total: 0,
        over_1ms: 0,
        over_2ms: 0,
        over_5ms: 0,
        frame_p99_ms: 0.0,
        frame_max_ms: 0.0,
        drop_counter_total: 0,
        data_quality: "Unknown".to_owned(),
    }
}
