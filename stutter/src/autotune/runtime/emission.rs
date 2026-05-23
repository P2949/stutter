use super::{
    AutotuneRuntime,
    daemon_state::{history_mode, history_phase, history_situation},
    decision_view::{
        data_quality_label, decision_action_kind, decision_candidate_name, decision_is_eligible,
        decision_label, decision_safety_class, rollback_policy_for_decision,
    },
    history::{LifecycleHistoryEventInput, RuntimeHistoryContext},
};
use crate::autotune::{
    decision::AutotuneDecision,
    history::{
        AutotuneDecisionSummary, AutotuneHistoryEvent, AutotuneHistoryEventInput,
        ObservationSummary, TargetIdentity, append_autotune_history_event,
    },
    observation::AutotuneObservation,
    planner::PlanResult,
    state::ControllerPhase,
};

impl AutotuneRuntime {
    pub(super) fn append_history(
        &mut self,
        observation: &AutotuneObservation,
        decision: &AutotuneDecision,
        reason: &str,
    ) -> anyhow::Result<()> {
        let Some(path) = self.config.history_log.clone() else {
            return Ok(());
        };

        let score = observation.to_window_score();

        let target = self.target_identity_from_observation(observation);
        let context = self.pending_history_context.take();

        let candidate_name = context
            .as_ref()
            .map(|context| context.candidate_name.clone())
            .or_else(|| decision_candidate_name(decision));

        let action_kind = context
            .as_ref()
            .map(|context| context.action_kind.clone())
            .or_else(|| decision_action_kind(decision));

        let rollback_policy = context
            .as_ref()
            .map(RuntimeHistoryContext::rollback_policy_with_metadata)
            .unwrap_or_else(|| rollback_policy_for_decision(decision).to_owned());
        let event_mode = context
            .as_ref()
            .map(|context| context.mode)
            .unwrap_or_else(|| self.config.mode());
        let safety_class = context
            .as_ref()
            .map(|context| context.safety_class.clone())
            .or_else(|| decision_safety_class(decision));

        let planner_summary = self.last_plan_result.as_ref().map(PlanResult::summary);

        let mut history = AutotuneHistoryEvent::new(AutotuneHistoryEventInput {
            controller_id: self.config.controller_id.clone(),
            phase: history_phase(self.controller.state.phase),
            mode: history_mode(event_mode),
            target,
            situation: history_situation(observation.primary_situation),
            observation_summary: ObservationSummary {
                target_present: observation.target_present,
                active_target_count: observation.active_target_count,
                scored_task_count: observation.scored_task_count,
                interval_count: observation.interval_count,
                scored_samples: observation.scored_samples,
                diagnostic_score_total: observation.score.total,
                over_1ms: observation.score.over_1ms,
                over_2ms: observation.score.over_2ms,
                over_5ms: observation.score.over_5ms,
                frame_p99_ms: observation.frame_p99_ms,
                frame_max_ms: observation.frame_max_ms,
                drop_counter_total: observation.drop_counter_total,
                data_quality: data_quality_label(&observation.data_quality),
            },
            decision: AutotuneDecisionSummary {
                decision: decision_label(decision),
                candidate_name,
                action_kind,
                safety_class,
                eligible: decision_is_eligible(decision),
                rollback_policy,
            },
            reason: reason.to_owned(),
        })
        .with_planner(planner_summary);

        if let Some(context) = context.as_ref() {
            history = history
                .with_experiment_id(context.experiment_id.clone())
                .with_action_id(context.action_id.clone())
                .with_scores(context.score_before.clone(), context.score_after.clone())
                .with_rollback_performed(context.rollback_performed);
        } else {
            history = history.with_scores(None, Some(score));
        }

        append_autotune_history_event(&path, &history)?;

        self.append_followup_history_events(
            &path,
            observation,
            decision,
            reason,
            context.as_ref(),
        )?;

        Ok(())
    }

    fn target_identity_from_observation(
        &self,
        observation: &AutotuneObservation,
    ) -> Option<TargetIdentity> {
        observation.target_root_pid.map(|root_pid| TargetIdentity {
            root_pid,
            process_comm: self
                .target_state
                .target_comm
                .clone()
                .or_else(|| self.config.watch_process().map(str::to_owned))
                .unwrap_or_else(|| "unknown".to_owned()),
            process_starttime_ticks: None,
            exe_dev: None,
            exe_ino: None,
            active_task_count: observation.active_target_count,
        })
    }

    fn append_followup_history_events(
        &self,
        path: &std::path::Path,
        observation: &AutotuneObservation,
        decision: &AutotuneDecision,
        reason: &str,
        context: Option<&RuntimeHistoryContext>,
    ) -> anyhow::Result<()> {
        if let (AutotuneDecision::StartExperiment { .. }, Some(context)) = (decision, context) {
            let applied = self.lifecycle_history_event(LifecycleHistoryEventInput {
                observation,
                context,
                phase: ControllerPhase::Measuring,
                decision: "candidate_applied",
                eligible: true,
                rollback_performed: false,
                reason: "candidate was applied and rollback token was written to controller journal",
            });
            append_autotune_history_event(path, &applied)?;
        }

        if matches!(
            decision,
            AutotuneDecision::KeepCurrent { .. }
                | AutotuneDecision::Revert { .. }
                | AutotuneDecision::EnterCooldown { .. }
        ) && let Some(context) = context
        {
            let cooldown = self.lifecycle_history_event(LifecycleHistoryEventInput {
                observation,
                context,
                phase: ControllerPhase::Cooldown,
                decision: "cooldown_entered",
                eligible: true,
                rollback_performed: context.rollback_performed,
                reason,
            });
            append_autotune_history_event(path, &cooldown)?;
        }

        if matches!(decision, AutotuneDecision::Fault { .. })
            && let Some(context) = context
        {
            let faulted = self.lifecycle_history_event(LifecycleHistoryEventInput {
                observation,
                context,
                phase: ControllerPhase::Faulted,
                decision: "faulted",
                eligible: false,
                rollback_performed: context.rollback_performed,
                reason,
            });
            append_autotune_history_event(path, &faulted)?;
        }

        Ok(())
    }

    fn lifecycle_history_event(
        &self,
        input: LifecycleHistoryEventInput<'_>,
    ) -> AutotuneHistoryEvent {
        AutotuneHistoryEvent::new(AutotuneHistoryEventInput {
            controller_id: self.config.controller_id.clone(),
            phase: history_phase(input.phase),
            mode: history_mode(input.context.mode),
            target: self.target_identity_from_observation(input.observation),
            situation: history_situation(input.observation.primary_situation),
            observation_summary: ObservationSummary {
                target_present: input.observation.target_present,
                active_target_count: input.observation.active_target_count,
                scored_task_count: input.observation.scored_task_count,
                interval_count: input.observation.interval_count,
                scored_samples: input.observation.scored_samples,
                diagnostic_score_total: input.observation.score.total,
                over_1ms: input.observation.score.over_1ms,
                over_2ms: input.observation.score.over_2ms,
                over_5ms: input.observation.score.over_5ms,
                frame_p99_ms: input.observation.frame_p99_ms,
                frame_max_ms: input.observation.frame_max_ms,
                drop_counter_total: input.observation.drop_counter_total,
                data_quality: data_quality_label(&input.observation.data_quality),
            },
            decision: AutotuneDecisionSummary {
                decision: input.decision.to_owned(),
                candidate_name: Some(input.context.candidate_name.clone()),
                action_kind: Some(input.context.action_kind.clone()),
                safety_class: Some(input.context.safety_class.clone()),
                eligible: input.eligible,
                rollback_policy: input.context.rollback_policy_with_metadata(),
            },
            reason: input.reason.to_owned(),
        })
        .with_experiment_id(input.context.experiment_id.clone())
        .with_action_id(input.context.action_id.clone())
        .with_scores(
            input.context.score_before.clone(),
            input.context.score_after.clone(),
        )
        .with_rollback_performed(input.rollback_performed)
    }
}
