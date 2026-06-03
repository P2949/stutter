use super::{
    AutotuneDryRunPlanFileSummary, AutotuneRuntime, AutotuneRuntimePhase,
    planning::{select_best_candidate_for_situation, simulated_dry_run_records},
};
use crate::{
    autotune::{
        observation::AutotuneObservation,
        planner::{CandidatePlanner, PlanResult, PlannerInput},
        planning::{candidate::CandidateAction, plan_io, policy::policy_intent_for_mode},
    },
    daemon::policy::DaemonMode,
};

impl AutotuneRuntime {
    pub(super) fn select_candidate_for_observation(
        &mut self,
        observation: &AutotuneObservation,
    ) -> anyhow::Result<Option<CandidateAction>> {
        self.last_dry_run_plan_files.clear();

        if self.config.mode() == DaemonMode::Observe {
            self.last_plan_result = Some(PlanResult {
                selected: None,
                evaluations: Vec::new(),
                no_action_reason: Some("observe mode does not suggest or apply".to_owned()),
            });
            return Ok(None);
        }

        if observation.data_quality.blocks_action()
            || observation.focus_is_idle_or_unknown()
            || observation.focus_has_critical_realtime_warning()
            || observation.focus_confidence < self.controller.policy.min_focus_confidence
        {
            self.last_plan_result = Some(PlanResult {
                selected: None,
                evaluations: Vec::new(),
                no_action_reason: Some(
                    "quality, focus, realtime, or confidence gate blocked planning".to_owned(),
                ),
            });
            return Ok(None);
        }

        let Some(tree_pid) = observation.target_root_pid.or(self.config.tree_pid()) else {
            return Ok(None);
        };

        if !self.config.simulated_candidates.is_empty() {
            let candidates = self.config.simulated_candidates.clone();
            let records = simulated_dry_run_records(&candidates, observation.active_target_count);
            let selected = select_best_candidate_for_situation(
                &candidates,
                &records,
                observation,
                self.controller.policy.max_safety_class.clone(),
                &self.controller.state,
            );
            self.last_plan_result = Some(PlanResult {
                selected: selected.clone(),
                evaluations: Vec::new(),
                no_action_reason: selected
                    .is_none()
                    .then(|| "no simulated candidate selected".to_owned()),
            });
            return Ok(selected);
        }

        let mut observation = observation.clone();
        if observation.target_root_pid.is_none() {
            observation.target_root_pid = Some(tree_pid);
        }
        let planner = CandidatePlanner::default_for_policy(&self.config.daemon_policy);
        if let Some(err) = self.config.workload_policy_error.as_ref() {
            self.last_plan_result = Some(PlanResult {
                selected: None,
                evaluations: Vec::new(),
                no_action_reason: Some(format!("invalid workload policy configuration: {err}")),
            });
            return Ok(None);
        }
        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &self.config.daemon_policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &self.controller.state,
            active_profile_state: Some(&self.controller.active_profile_state),
            workload_policy: &self.config.workload_policy,
            profiles: &self.config.profiles,
        });
        let selected = result.selected.clone();
        if self.config.dry_run_all_safe {
            self.prepare_runtime_phase_for_dry_run()?;
            self.transition_runtime_phase(
                AutotuneRuntimePhase::DryRun,
                "writing dry-run plan files for eligible candidates",
            )?;
            self.last_dry_run_plan_files = self.write_dry_run_plan_files_for_plan(&result)?;
        }
        self.last_plan_result = Some(result);
        Ok(selected)
    }

    fn write_dry_run_plan_files_for_plan(
        &self,
        plan: &PlanResult,
    ) -> anyhow::Result<Vec<AutotuneDryRunPlanFileSummary>> {
        let plan_dir = self
            .config
            .dry_run_plan_dir
            .clone()
            .unwrap_or_else(plan_io::default_candidate_plan_dir);
        let mut written = Vec::new();

        for evaluation in &plan.evaluations {
            let Some(dry_run) = evaluation.dry_run.as_ref() else {
                continue;
            };
            let path = plan_io::candidate_plan_path(&evaluation.candidate, &plan_dir);
            plan_io::write_candidate_plan_file_with_policy(
                &path,
                &evaluation.candidate,
                Some(dry_run.affected_tasks),
                &self.config.daemon_policy,
                policy_intent_for_mode(self.config.daemon_policy.mode),
            )?;
            written.push(AutotuneDryRunPlanFileSummary {
                candidate_name: evaluation.candidate_name.clone(),
                action_kind: evaluation.action_kind.clone(),
                path,
                affected_tasks: dry_run.affected_tasks,
                safety_class: evaluation.descriptor.safety_class.clone(),
                eligible: evaluation.eligible,
                deny_reasons: evaluation.deny_reasons.clone(),
            });
        }

        Ok(written)
    }

    fn prepare_runtime_phase_for_dry_run(&mut self) -> anyhow::Result<()> {
        match self.runtime_phase() {
            AutotuneRuntimePhase::Idle => {
                self.transition_runtime_phase(
                    AutotuneRuntimePhase::ObservingBaseline,
                    "preparing direct dry run from idle runtime",
                )?;
                self.transition_runtime_phase(
                    AutotuneRuntimePhase::Planning,
                    "preparing direct dry run from idle runtime",
                )
            }
            AutotuneRuntimePhase::ObservingBaseline => self.transition_runtime_phase(
                AutotuneRuntimePhase::Planning,
                "preparing dry run after observation",
            ),
            _ => Ok(()),
        }
    }
}
