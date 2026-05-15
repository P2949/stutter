use crate::{
    autotune::{
        candidate::{
            CandidateAction, generate_profile_candidates,
            generate_topology_aware_profile_candidates,
        },
        objective::ObjectiveKind,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
    },
    topology::TopologyModel,
};

#[derive(Default)]
pub struct CpuAffinityProvider;

impl CandidateProvider for CpuAffinityProvider {
    fn family(&self) -> &'static str {
        "cpu_affinity_profile"
    }

    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        let Some(tree_pid) = input.observation.target_root_pid else {
            return Vec::new();
        };

        let candidates = if input.profiles.is_empty() {
            TopologyModel::read()
                .map(|topology| generate_topology_aware_profile_candidates(&topology, tree_pid))
                .unwrap_or_default()
        } else {
            generate_profile_candidates(input.profiles, tree_pid, None)
        };

        candidates
            .into_iter()
            .filter_map(|candidate| {
                let rank_hint =
                    rank_for_candidate(&candidate, input.observation.primary_situation)?;
                Some(CandidateProposal {
                    candidate,
                    provider: self.family(),
                    confidence: input.observation.focus_confidence,
                    deny_reasons: Vec::new(),
                    objective: ObjectiveKind::StutterScore,
                    rank_hint: u32::from(rank_hint),
                })
            })
            .collect()
    }
}

fn rank_for_candidate(candidate: &CandidateAction, situation: SituationKind) -> Option<u8> {
    let name = candidate.candidate_name().to_ascii_lowercase();

    match situation {
        SituationKind::GameCpuSchedulerPressure | SituationKind::GameFocused => {
            if name.contains("game-isolate-render") {
                Some(0)
            } else if name.contains("avoid-smt-contention") {
                Some(1)
            } else if name.contains("wine-server-dedicated") {
                Some(2)
            } else if name.contains("helper-spread") {
                Some(3)
            } else if name.contains("game") || name.contains("wine") || name.contains("helper") {
                Some(10)
            } else {
                None
            }
        }
        SituationKind::CompositorPressure => {
            if name.contains("game-compositor-separate") {
                Some(0)
            } else if name.contains("compositor") {
                Some(1)
            } else {
                None
            }
        }
        SituationKind::CpuPressure => {
            if name.contains("avoid-smt-contention") {
                Some(0)
            } else if name.contains("helper-spread") {
                Some(1)
            } else {
                None
            }
        }
        _ => None,
    }
}
