use std::path::Path;

use super::{
    AutotuneDecisionStreamEntry, AutotuneRuntime, AutotuneRuntimePhase,
    DAEMON_EMERGENCY_RESTORE_COMMAND, DEFAULT_RECENT_DIAGNOSIS_LIMIT,
    decision_view::decision_reason, emit_decision_stream_entry,
};
use crate::{
    autotune::{
        controller::decide_autotune_transition,
        decision::AutotuneDecision,
        live_experiment::{LiveExperimentEvent, LiveExperimentManagerInput},
        observation::AutotuneObservation,
        observation_builder::{
            AutotuneObservationBuilder, AutotuneObservationBuilderInput, AutotuneObservationFocus,
        },
    },
    daemon::{
        policy::DaemonMode,
        privilege::{
            InProcessPrivilegedActionService, PrivilegedActionService,
            UnixSocketPrivilegedActionService, default_privileged_worker_socket_path,
        },
    },
    diagnosis::LiveDiagnosisEntry,
    process_tree::TaskMap,
    session_events::MonitorEvent,
};

struct ResolvedPrivilegedActionService {
    socket_service: Option<UnixSocketPrivilegedActionService>,
    in_process_service: Option<InProcessPrivilegedActionService>,
}

impl ResolvedPrivilegedActionService {
    fn as_service(&self) -> Option<&dyn PrivilegedActionService> {
        self.socket_service
            .as_ref()
            .map(|service| service as &dyn PrivilegedActionService)
            .or_else(|| {
                self.in_process_service
                    .as_ref()
                    .map(|service| service as &dyn PrivilegedActionService)
            })
    }
}

fn resolve_privileged_action_service(
    config: &super::AutotuneRuntimeConfig,
) -> anyhow::Result<ResolvedPrivilegedActionService> {
    let use_privileged_service =
        config.mode() == DaemonMode::ApplyMediumRisk && !config.simulate_action_effects;
    if !use_privileged_service {
        return Ok(ResolvedPrivilegedActionService {
            socket_service: None,
            in_process_service: None,
        });
    }

    if config
        .daemon_config
        .autotune
        .unsafe_in_process_privileged_worker
    {
        return Ok(ResolvedPrivilegedActionService {
            socket_service: None,
            in_process_service: Some(InProcessPrivilegedActionService::default()),
        });
    }

    let socket_path = config
        .daemon_config
        .autotune
        .privileged_worker_socket
        .clone()
        .map(Ok)
        .unwrap_or_else(default_privileged_worker_socket_path)?;
    Ok(ResolvedPrivilegedActionService {
        socket_service: Some(UnixSocketPrivilegedActionService::new(socket_path)),
        in_process_service: None,
    })
}

impl AutotuneRuntime {
    pub fn on_event(
        &mut self,
        event: MonitorEvent,
    ) -> anyhow::Result<Option<AutotuneDecisionStreamEntry>> {
        match event {
            MonitorEvent::TargetSnapshot { active_targets, .. } => {
                self.update_target_snapshot(&active_targets);
            }
            MonitorEvent::Interval {
                records,
                drop_counters,
                ..
            } => {
                let scored_samples = records
                    .iter()
                    .map(|record| record.samples)
                    .fold(0_u64, u64::saturating_add);
                self.activity_classifier.push_interval(scored_samples);
                self.latest_drop_counters = drop_counters;
                self.controller.window.push_intervals(records);
                return self.evaluate_and_emit(None);
            }
            MonitorEvent::Frame { event } => {
                self.controller.window.push_frame(*event);
            }
            MonitorEvent::IrqEvent { event } => {
                self.controller.window.push_irq_event(*event);
            }
            MonitorEvent::IoEvent { event } => {
                self.controller.window.push_block_io_event(*event);
            }
            MonitorEvent::GpuSample { sample } => {
                self.controller.window.push_gpu_sample(*sample);
            }
            MonitorEvent::GpuEngineSample { .. } => {}
            MonitorEvent::CpuFreqSample { event } => {
                self.controller.window.push_cpu_freq_event(*event);
            }
            MonitorEvent::ForegroundEvent { event } => {
                self.controller.window.push_foreground_event(*event);
            }
            MonitorEvent::LiveDiagnosis { entry } => {
                self.push_diagnosis(*entry);
            }
            MonitorEvent::FocusChanged {
                new_kind,
                root_pids,
                member_pids,
                confidence,
                situation,
                reasons,
                ..
            } => {
                self.controller.window.clear();
                if self.live_experiments.has_active_experiment() {
                    self.rollback_live_experiment(
                        crate::audit::unix_nanos_now(),
                        "focus changed during active experiment",
                    )?;
                }
                self.latest_focus = Some(AutotuneObservationFocus {
                    kind: new_kind,
                    root_pids: root_pids.clone(),
                    member_pids,
                    confidence,
                    situation,
                    reasons,
                });
                self.target_state.root_pid = root_pids.first().copied().or(self.config.tree_pid());
                return self.evaluate_and_emit(Some("focus changed; measurement window reset"));
            }
            MonitorEvent::FocusCleared { reason, .. } => {
                self.controller.window.clear();
                if self.live_experiments.has_active_experiment() {
                    self.rollback_live_experiment(
                        crate::audit::unix_nanos_now(),
                        "focus cleared during active experiment",
                    )?;
                }
                self.latest_focus = None;
                self.target_state.root_pid = self.config.tree_pid();
                return self.evaluate_and_emit(Some(&reason));
            }
            MonitorEvent::DataQualityWarning { message } => {
                return self.evaluate_and_emit(Some(&message));
            }
            MonitorEvent::Finished { reason } => {
                return self.evaluate_and_emit(Some(&reason));
            }
            MonitorEvent::Alert { .. }
            | MonitorEvent::MigrationEvent { .. }
            | MonitorEvent::SchedulerSample { .. }
            | MonitorEvent::Spike { .. }
            | MonitorEvent::ScxEvent { .. }
            | MonitorEvent::KmsFlipEvent { .. }
            | MonitorEvent::DrmFenceEvent { .. }
            | MonitorEvent::WaylandPresentationEvent { .. }
            | MonitorEvent::DmaBufEvent { .. }
            | MonitorEvent::Exec { .. } => {}
        }

        Ok(None)
    }

    fn update_target_snapshot(&mut self, active_targets: &TaskMap) {
        self.target_state.active_targets = active_targets.len();
        self.target_state.active_tasks = active_targets.clone();

        if self.target_state.root_pid.is_none() {
            self.target_state.root_pid = self
                .latest_focus
                .as_ref()
                .and_then(|focus| focus.root_pids.first().copied())
                .or(self.config.tree_pid());
        }

        if let Some(root_pid) = self.target_state.root_pid {
            self.target_state.target_comm = active_targets
                .get(&root_pid)
                .map(|task| task.comm.to_string())
                .or_else(|| {
                    active_targets
                        .values()
                        .next()
                        .map(|task| task.comm.to_string())
                });
        } else {
            self.target_state.target_comm = active_targets
                .values()
                .next()
                .map(|task| task.comm.to_string());
        }
    }

    fn push_diagnosis(&mut self, diagnosis: LiveDiagnosisEntry) {
        self.controller.window.push_diagnosis(diagnosis.clone());
        self.recent_diagnoses.push_back(diagnosis);
        while self.recent_diagnoses.len() > DEFAULT_RECENT_DIAGNOSIS_LIMIT {
            self.recent_diagnoses.pop_front();
        }
    }

    fn evaluate_and_emit(
        &mut self,
        forced_reason: Option<&str>,
    ) -> anyhow::Result<Option<AutotuneDecisionStreamEntry>> {
        self.prepare_runtime_phase_for_evaluation()?;

        let observation = self.build_observation();
        self.last_observation = observation.clone();

        let decision =
            if let Some(decision) = self.live_experiments.active_window_decision(&observation) {
                decision
            } else if self.live_experiments.has_active_experiment() {
                self.active_experiment_external_mutation_decision(&observation)
                    .unwrap_or_else(|| {
                        decide_autotune_transition(
                            &self.controller.policy,
                            &self.controller.state,
                            &observation,
                            None,
                        )
                    })
            } else {
                self.transition_runtime_phase(
                    AutotuneRuntimePhase::Planning,
                    "planning candidates for observation",
                )?;
                let candidate = self.select_candidate_for_observation(&observation)?;
                self.plan_external_mutation_recovery_decision()
                    .unwrap_or_else(|| {
                        decide_autotune_transition(
                            &self.controller.policy,
                            &self.controller.state,
                            &observation,
                            candidate,
                        )
                    })
            };

        let reason = forced_reason
            .map(str::to_owned)
            .unwrap_or_else(|| decision_reason(&decision));

        self.apply_decision_side_effects(&observation, &decision, &reason)?;

        let stream_entry = self.stream_entry_from_decision(&observation, &decision, reason.clone());

        self.append_decision_log(&stream_entry)?;
        self.append_history(&observation, &decision, &reason)?;

        emit_decision_stream_entry(&stream_entry)?;

        self.last_decision = Some(stream_entry.clone());

        Ok(Some(stream_entry))
    }

    fn prepare_runtime_phase_for_evaluation(&mut self) -> anyhow::Result<()> {
        let next = if self.live_experiments.has_active_experiment() {
            AutotuneRuntimePhase::MeasuringCandidate
        } else {
            AutotuneRuntimePhase::ObservingBaseline
        };
        self.transition_runtime_phase(next, "evaluating monitor event")
    }

    pub(super) fn build_observation(&self) -> AutotuneObservation {
        AutotuneObservationBuilder::build(AutotuneObservationBuilderInput {
            window: &self.controller.window,
            online_data_quality_policy: &self.config.online_data_quality_policy,
            focus: self.latest_focus.as_ref(),
            root_pid: self.target_state.root_pid,
            active_target_count: self.target_state.active_targets,
            active_tasks: &self.target_state.active_tasks,
            recent_diagnoses: self.recent_diagnoses.iter().cloned().collect(),
            drop_counters: self.latest_drop_counters.clone(),
            proc_root: Path::new("/proc"),
            sys_root: Path::new("/sys"),
            activity_level: self.activity_classifier.classify(),
        })
        .observation
    }

    pub(super) fn apply_decision_side_effects(
        &mut self,
        observation: &AutotuneObservation,
        decision: &AutotuneDecision,
        reason: &str,
    ) -> anyhow::Result<()> {
        if self.config.dry_run_all_safe
            && matches!(decision, AutotuneDecision::StartExperiment { .. })
        {
            anyhow::bail!("dry-run-all-safe mode refused to start a live experiment");
        }

        self.transition_for_decision(decision, reason)?;

        let privileged_action_service = resolve_privileged_action_service(&self.config)?;
        let input = LiveExperimentManagerInput {
            mode: self.config.mode(),
            daemon_policy: self.config.daemon_policy.clone(),
            controller_policy: self.controller.policy.clone(),
            simulate_action_effects: self.config.simulate_action_effects,
            washout: self.config.washout.clone(),
            candidate_window_seconds: self.config.candidate_window_seconds,
            manual_restore_command: DAEMON_EMERGENCY_RESTORE_COMMAND,
            controller_journal_path: self.config.controller_journal_path.clone(),
            #[cfg(test)]
            exit_rollback_registry: None,
            privileged_action_service: privileged_action_service.as_service(),
        };

        let outcome = match self.live_experiments.apply_decision_side_effects(
            input,
            &mut self.controller.state,
            &mut self.controller.active_profile_state,
            observation,
            decision,
            reason,
        ) {
            Ok(outcome) => outcome,
            Err(err) => {
                self.fault_runtime_phase(format!("runtime decision side effect failed: {err}"));
                return Err(err);
            }
        };

        log::debug!(
            "autotune_live_experiment_outcome event={} clear_measurement_window={} history_context={}",
            outcome.event.as_str(),
            outcome.clear_measurement_window,
            outcome.history_context.is_some()
        );

        if outcome.clear_measurement_window {
            self.controller.window.clear();
        }

        if let Some(context) = outcome.history_context {
            self.pending_history_context = Some(context.into());
        }

        self.transition_for_outcome(outcome.event)?;

        Ok(())
    }

    fn transition_for_decision(
        &mut self,
        decision: &AutotuneDecision,
        reason: &str,
    ) -> anyhow::Result<()> {
        let next = match decision {
            AutotuneDecision::StartExperiment { .. } => AutotuneRuntimePhase::ApplyingCandidate,
            AutotuneDecision::KeepCurrent { .. } => AutotuneRuntimePhase::KeepingCandidate,
            AutotuneDecision::Revert { .. } => AutotuneRuntimePhase::RevertingCandidate,
            AutotuneDecision::Fault { .. } => AutotuneRuntimePhase::Faulted,
            AutotuneDecision::Noop { .. }
            | AutotuneDecision::Suggest { .. }
            | AutotuneDecision::EnterCooldown { .. } => return Ok(()),
        };

        if next == AutotuneRuntimePhase::ApplyingCandidate {
            self.prepare_runtime_phase_for_apply_candidate()?;
        }

        self.transition_runtime_phase(next, format!("applying decision side effect: {reason}"))
    }

    fn prepare_runtime_phase_for_apply_candidate(&mut self) -> anyhow::Result<()> {
        match self.runtime_phase() {
            AutotuneRuntimePhase::Idle => {
                self.transition_runtime_phase(
                    AutotuneRuntimePhase::ObservingBaseline,
                    "preparing direct candidate apply from idle runtime",
                )?;
                self.transition_runtime_phase(
                    AutotuneRuntimePhase::Planning,
                    "preparing direct candidate apply from idle runtime",
                )
            }
            AutotuneRuntimePhase::ObservingBaseline => self.transition_runtime_phase(
                AutotuneRuntimePhase::Planning,
                "preparing candidate apply after observation",
            ),
            _ => Ok(()),
        }
    }

    fn transition_for_outcome(&mut self, event: LiveExperimentEvent) -> anyhow::Result<()> {
        let next = match event {
            LiveExperimentEvent::Started => AutotuneRuntimePhase::MeasuringCandidate,
            LiveExperimentEvent::Kept
            | LiveExperimentEvent::Reverted
            | LiveExperimentEvent::CooldownEntered => AutotuneRuntimePhase::ObservingBaseline,
            LiveExperimentEvent::Faulted => AutotuneRuntimePhase::Faulted,
            LiveExperimentEvent::Noop => {
                if self.live_experiments.has_active_experiment() {
                    AutotuneRuntimePhase::MeasuringCandidate
                } else {
                    AutotuneRuntimePhase::ObservingBaseline
                }
            }
        };

        self.transition_runtime_phase(next, format!("live experiment outcome {}", event.as_str()))
    }
}
