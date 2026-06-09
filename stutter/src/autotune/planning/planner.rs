//! Candidate planner orchestration and input wiring; this module owns provider dispatch, not evaluation details.

use crate::{
    autotune::{
        controller::ControllerRuntimeState,
        kept::ActiveProfileState,
        observation::AutotuneObservation,
        planning::{
            dry_run::{CandidateDryRunner, RealCandidateDryRunner},
            evaluate::evaluate_proposals_with_runner,
            model::PlanResult,
            ranking::{no_action_reason_for_evaluations, sort_candidate_evaluations},
        },
        providers::{CandidateProviderInput, CandidateProviderRegistry},
        system_context::SystemContextSnapshot,
        workload_policy::WorkloadPolicyMatrix,
    },
    daemon::{DaemonPolicy, capabilities::DaemonCapabilities, health::SystemHealthSnapshot},
    profiles::Profile,
};

pub struct CandidatePlanner {
    registry: CandidateProviderRegistry,
}

impl CandidatePlanner {
    pub fn new(registry: CandidateProviderRegistry) -> Self {
        Self { registry }
    }

    pub fn default_for_policy(policy: &DaemonPolicy) -> Self {
        Self::new(CandidateProviderRegistry::default_for_policy(policy))
    }

    pub fn plan(&self, input: PlannerInput<'_>) -> PlanResult {
        let mut dry_runner = RealCandidateDryRunner;
        self.plan_with_dry_runner(input, &mut dry_runner)
    }

    pub(crate) fn plan_with_dry_runner<R: CandidateDryRunner>(
        &self,
        input: PlannerInput<'_>,
        dry_runner: &mut R,
    ) -> PlanResult {
        if input.daemon_policy.mode == crate::daemon::policy::DaemonMode::Observe {
            return PlanResult {
                selected: None,
                evaluations: Vec::new(),
                no_action_reason: Some("observe mode does not suggest or apply".to_owned()),
            };
        }

        if input.observation.focus_is_idle_or_unknown() {
            return PlanResult {
                selected: None,
                evaluations: Vec::new(),
                no_action_reason: Some("focus is idle or unknown".to_owned()),
            };
        }

        let fallback_system_context;
        let system_context = if let Some(system_context) = input.observation.system_context.as_ref()
        {
            system_context
        } else {
            fallback_system_context = SystemContextSnapshot::from_observation(input.observation);
            &fallback_system_context
        };

        let provider_input = CandidateProviderInput {
            observation: input.observation,
            daemon_policy: input.daemon_policy,
            capabilities: input.capabilities,
            system_health: input.system_health,
            system_context,
            controller_state: input.controller_state,
            profiles: input.profiles,
        };
        let proposals = self.registry.propose(&provider_input);
        let mut evaluations = evaluate_proposals_with_runner(input, proposals, dry_runner);

        sort_candidate_evaluations(&mut evaluations);

        let selected = evaluations
            .iter()
            .find(|evaluation| evaluation.eligible)
            .map(|evaluation| evaluation.candidate.clone());

        let no_action_reason = if selected.is_none() {
            Some(no_action_reason_for_evaluations(&evaluations))
        } else {
            None
        };

        PlanResult {
            selected,
            evaluations,
            no_action_reason,
        }
    }
}

#[derive(Clone, Copy)]
pub struct PlannerInput<'a> {
    pub observation: &'a AutotuneObservation,
    pub daemon_policy: &'a DaemonPolicy,
    pub capabilities: &'a DaemonCapabilities,
    pub system_health: &'a SystemHealthSnapshot,
    pub controller_state: &'a ControllerRuntimeState,
    pub active_profile_state: Option<&'a ActiveProfileState>,
    pub workload_policy: &'a WorkloadPolicyMatrix,
    pub profiles: &'a [Profile],
}
