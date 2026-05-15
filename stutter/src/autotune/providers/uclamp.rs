use crate::{
    actions::{
        TaskIdentity,
        uclamp::{UclampAction, UclampValues},
    },
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, UclampActionPlan},
        objective::ObjectiveKind,
        protection::mutation_allowed_for_pid,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
    },
};

#[derive(Default)]
pub struct UclampProvider;

impl CandidateProvider for UclampProvider {
    fn family(&self) -> &'static str {
        "uclamp"
    }

    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        if !input.capabilities.uclamp_available || !input.system_health.ok_for_apply {
            return Vec::new();
        }
        let Some(root_pid) = input.observation.target_root_pid else {
            return Vec::new();
        };
        if !mutation_allowed_for_pid(root_pid, self.family(), input.observation).is_allowed() {
            return Vec::new();
        }

        let (values, objective, name_suffix) = match input.observation.primary_situation {
            SituationKind::GameCpuSchedulerPressure | SituationKind::CompositorPressure => (
                UclampValues {
                    sched_util_min: Some(128),
                    sched_util_max: None,
                },
                ObjectiveKind::GameRunnableLatency,
                "min-128",
            ),
            SituationKind::CompileLoad | SituationKind::CompileCpuBound => (
                UclampValues {
                    sched_util_min: None,
                    sched_util_max: Some(768),
                },
                ObjectiveKind::DesktopInteractivity,
                "max-768",
            ),
            _ => return Vec::new(),
        };

        let action = UclampAction {
            targets: vec![TaskIdentity {
                tid: root_pid,
                process_pid: Some(root_pid),
                comm: None,
                starttime_ticks: input
                    .observation
                    .workload_identity
                    .as_ref()
                    .and_then(|identity| identity.process_starttime_ticks),
            }],
            values,
        };
        let candidate = CandidateAction::Uclamp {
            plan: UclampActionPlan {
                name: format!("uclamp-root-{root_pid}-{name_suffix}"),
                action,
                target_root_pid: Some(root_pid),
                evidence: vec![CandidateEvidence::new(
                    "situation",
                    format!("{:?}", input.observation.primary_situation),
                    input.observation.situation.confidence,
                )],
                objective,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence: input.observation.situation.confidence,
            deny_reasons: Vec::new(),
            objective,
            rank_hint: 25,
        }]
    }
}
