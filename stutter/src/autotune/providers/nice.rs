use crate::{
    actions::nice::{NiceAction, NicePolicy},
    autotune::{
        objective::ObjectiveKind,
        planning::{
            candidate::{CandidateAction, CandidateEvidence},
            executable_plan::NiceActionPlan,
        },
        protection::mutation_allowed_for_pid,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
        target_selection::{
            TargetSelectionMode, TaskTargetSelector, mutable_task_targets_for_observation_with_mode,
        },
    },
};

/// The niceness a demoted background offender receives.
pub const OFFENDER_DEMOTED_NICE: i32 = 10;
/// At most this many tasks of offenders are demoted by one candidate.
const MAX_OFFENDER_TASKS: usize = 64;

#[derive(Default)]
pub struct NiceProvider;

impl NiceProvider {
    /// Under scheduler pressure on a game, lowers the priority of sustained background CPU
    /// consumers of the same user. The identity of every task comes from the observation, so the
    /// action's targets are exactly the tasks that were measured competing.
    fn propose_offender_demotion(
        &self,
        input: &CandidateProviderInput<'_>,
        root_pid: u32,
    ) -> Vec<CandidateProposal> {
        let offenders: Vec<_> = input
            .observation
            .background_offenders
            .iter()
            .filter(|offender| offender.nice < OFFENDER_DEMOTED_NICE)
            .collect();
        if offenders.is_empty() {
            return Vec::new();
        }
        let targets: Vec<_> = offenders
            .iter()
            .flat_map(|offender| offender.tasks.iter().cloned())
            .take(MAX_OFFENDER_TASKS)
            .collect();
        let pids = offenders
            .iter()
            .map(|offender| offender.pid.to_string())
            .collect::<Vec<_>>()
            .join("+");
        let mut evidence = vec![CandidateEvidence::new(
            "situation",
            format!("{:?}", input.observation.primary_situation),
            input.observation.situation.confidence,
        )];
        evidence.extend(offenders.iter().map(|offender| {
            CandidateEvidence::new(
                "background_cpu_offender",
                format!(
                    "pid={} comm={} cpu={:.0}% sustained_samples={} nice={}",
                    offender.pid,
                    offender.comm,
                    offender.cpu_percent,
                    offender.sustained_samples,
                    offender.nice
                ),
                1.0,
            )
        }));
        let candidate = CandidateAction::Nice {
            plan: NiceActionPlan {
                name: format!("nice-offenders-{pids}-to-{OFFENDER_DEMOTED_NICE}"),
                action: NiceAction {
                    targets,
                    nice: OFFENDER_DEMOTED_NICE,
                    policy: NicePolicy {
                        allow_nice_changes: true,
                        min_nice: -1,
                        max_nice: 19,
                    },
                },
                target_root_pid: Some(root_pid),
                evidence,
                objective: ObjectiveKind::GameRunnableLatency,
            },
        };
        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence: input.observation.situation.confidence,
            deny_reasons: Vec::new(),
            rank_hint: 30,
        }]
    }
}

impl CandidateProvider for NiceProvider {
    fn family(&self) -> &'static str {
        "nice"
    }

    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        let Some(root_pid) = input.observation.target_root_pid else {
            return Vec::new();
        };
        if !mutation_allowed_for_pid(root_pid, self.family(), input.observation).is_allowed() {
            return Vec::new();
        }

        if input.observation.primary_situation == SituationKind::GameCpuSchedulerPressure {
            return self.propose_offender_demotion(input, root_pid);
        }

        let (nice, objective, _reason) = match input.observation.primary_situation {
            SituationKind::CompileLoad
            | SituationKind::CompileCpuBound
            | SituationKind::CompileLinkerPressure => (
                5,
                ObjectiveKind::DesktopInteractivity,
                "compile/background work can be lowered to protect interactivity",
            ),
            SituationKind::BrowserCpuPressure | SituationKind::BrowserFocused => (
                -1,
                ObjectiveKind::BrowserInteractivity,
                "browser foreground CPU pressure",
            ),
            _ => return Vec::new(),
        };

        let selector = match input.observation.primary_situation {
            SituationKind::CompileLoad
            | SituationKind::CompileCpuBound
            | SituationKind::CompileLinkerPressure => TaskTargetSelector::CompilerAndLinker,
            SituationKind::BrowserCpuPressure | SituationKind::BrowserFocused => {
                TaskTargetSelector::BrowserRenderersAndHelpers
            }
            _ => TaskTargetSelector::ForegroundRootOnly,
        };
        let selection = mutable_task_targets_for_observation_with_mode(
            input.observation,
            selector,
            TargetSelectionMode::from_daemon_mode(input.daemon_policy.mode),
        );
        let target_selection_denies = selection.deny_reasons();
        let used_fallback_root = selection.used_fallback_root;
        let targets = selection.items;
        if targets.is_empty() {
            return Vec::new();
        }

        let action = NiceAction {
            targets,
            nice,
            policy: NicePolicy {
                allow_nice_changes: true,
                min_nice: -1,
                max_nice: 19,
            },
        };
        let candidate = CandidateAction::Nice {
            plan: NiceActionPlan {
                name: format!("nice-root-{root_pid}-to-{nice}"),
                action,
                target_root_pid: Some(root_pid),
                evidence: {
                    let mut evidence = vec![CandidateEvidence::new(
                        "situation",
                        format!("{:?}", input.observation.primary_situation),
                        input.observation.situation.confidence,
                    )];
                    if used_fallback_root {
                        evidence.push(CandidateEvidence::new(
                            "target_selection_fallback_root",
                            format!("root_pid={root_pid} active_task_snapshots=missing"),
                            0.0,
                        ));
                    }
                    evidence
                },
                objective,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence: input.observation.situation.confidence,
            deny_reasons: target_selection_denies,
            rank_hint: 30,
        }]
    }
}
