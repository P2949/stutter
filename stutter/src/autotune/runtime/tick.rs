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
        live_experiment::{
            LiveExperimentActionExecutor, LiveExperimentEvent, LiveExperimentManagerInput,
            LiveExperimentRuntimeState, RuntimeLiveExperimentActionExecutor,
        },
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
            MonitorEvent::WorkloadProgress { record } => {
                self.controller.window.push_workload_progress(*record);
                // The subsequent interval tick supplies complete scheduler and
                // foreground telemetry before deciding whether a build improved.
            }
            MonitorEvent::TargetSnapshot { active_targets, .. } => {
                self.update_target_snapshot(&active_targets);
            }
            MonitorEvent::Interval {
                records,
                drop_counters,
                ..
            } => {
                // A suspended gap must clear the window before these intervals join it.
                self.poll_lifecycle_clock()?;
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
                self.target_state.root_pid = self
                    .config
                    .tree_pid()
                    .or_else(|| root_pids.first().copied());
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

    pub(super) fn prepare_runtime_phase_for_evaluation(&mut self) -> anyhow::Result<()> {
        let next = if self.live_experiments.has_active_experiment() {
            AutotuneRuntimePhase::MeasuringCandidate
        } else {
            AutotuneRuntimePhase::ObservingBaseline
        };
        self.transition_runtime_phase(next, "evaluating monitor event")
    }

    /// An explicitly named process tree whose tasks are a game is the focused workload: the user
    /// chose it, so no desktop focus inference is needed. Other explicit targets keep their
    /// previous (unfocused) behaviour until they have their own contract.
    fn explicit_target_focus(&self) -> Option<AutotuneObservationFocus> {
        let root = self.config.tree_pid()?;
        if self.latest_focus.is_some()
            || !self
                .target_state
                .active_tasks
                .values()
                .any(|task| crate::scorer::class_contributes_to_score(task.class))
        {
            return None;
        }
        let mut members: Vec<u32> = self
            .target_state
            .active_tasks
            .values()
            .map(|task| task.process_pid.as_u32())
            .collect();
        members.sort_unstable();
        members.dedup();
        Some(AutotuneObservationFocus {
            kind: crate::focus::FocusGroupKind::Game,
            root_pids: vec![root],
            member_pids: members,
            confidence: 1.0,
            situation: crate::autotune::situation::SituationKind::GameFocused,
            reasons: vec!["explicit target (--tree-pid) is a game".to_owned()],
        })
    }

    /// Background CPU consumers of the target's owner, sampled only while a game is the focused
    /// workload (the only workload whose protection an offender can matter for today).
    fn sample_offenders(
        &self,
        focus: Option<&AutotuneObservationFocus>,
    ) -> Vec<crate::autotune::offenders::OffenderProcess> {
        use std::os::unix::fs::MetadataExt;
        if focus.is_none_or(|focus| focus.kind != crate::focus::FocusGroupKind::Game) {
            return Vec::new();
        }
        let Some(root) = self.config.tree_pid().or(self.target_state.root_pid) else {
            return Vec::new();
        };
        let proc_root = Path::new("/proc");
        let Ok(owner) = std::fs::metadata(proc_root.join(root.to_string())).map(|m| m.uid()) else {
            return Vec::new();
        };
        let exclude: std::collections::BTreeSet<u32> = self
            .target_state
            .active_tasks
            .values()
            .map(|task| task.process_id().as_u32())
            .chain([root, std::process::id()])
            .collect();
        match self.offender_sampler.lock() {
            Ok(mut sampler) => sampler.sample(
                proc_root,
                crate::syscall::clock_ticks_per_second(),
                owner,
                &exclude,
            ),
            Err(_) => Vec::new(),
        }
    }

    pub(super) fn build_observation(&self) -> AutotuneObservation {
        let explicit_focus = self.explicit_target_focus();
        let focus = self.latest_focus.as_ref().or(explicit_focus.as_ref());
        let offenders = self.sample_offenders(focus);
        AutotuneObservationBuilder::build(AutotuneObservationBuilderInput {
            window: &self.controller.window,
            online_data_quality_policy: &self.config.online_data_quality_policy,
            focus,
            offenders,
            // Preserve the distinction between a configured process-tree root
            // and a focus-derived root, so focus group helpers stay in scope.
            root_pid: self.config.tree_pid().or(self
                .target_state
                .root_pid
                .filter(|_| self.latest_focus.is_none())),
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
        self.apply_decision_side_effects_with_executor(
            observation,
            decision,
            reason,
            &mut RuntimeLiveExperimentActionExecutor,
        )
    }

    pub(super) fn apply_decision_side_effects_with_executor<
        E: LiveExperimentActionExecutor + ?Sized,
    >(
        &mut self,
        observation: &AutotuneObservation,
        decision: &AutotuneDecision,
        reason: &str,
        executor: &mut E,
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
            audit_path: None,
            #[cfg(test)]
            exit_rollback_registry: None,
            privileged_action_service: privileged_action_service.as_service(),
        };

        let outcome = match self
            .live_experiments
            .apply_decision_side_effects_with_executor(
                input,
                LiveExperimentRuntimeState {
                    controller_state: &mut self.controller.state,
                    active_profile_state: &mut self.controller.active_profile_state,
                },
                observation,
                decision,
                reason,
                executor,
            ) {
            Ok(outcome) => outcome,
            Err(err) => {
                let failure_reason = format!("runtime decision side effect failed: {err:#}");
                self.fault_runtime_phase(failure_reason.clone());
                self.pending_history_context = self
                    .live_experiments
                    .take_failed_apply_context()
                    .map(Into::into);
                if let Err(report_error) =
                    self.persist_side_effect_fault(observation, &failure_reason)
                {
                    return Err(err.context(format!(
                        "failed to persist side-effect fault report: {report_error:#}"
                    )));
                }
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

    pub(super) fn prepare_runtime_phase_for_apply_candidate(&mut self) -> anyhow::Result<()> {
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
