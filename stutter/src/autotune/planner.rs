use serde::{Deserialize, Serialize};

use crate::{
    actions::ActionState,
    autotune::{
        candidate::{
            CandidateAction, CandidateDryRunRecord, CandidateEvidence, dry_run_candidates,
        },
        controller::ControllerRuntimeState,
        kept::ActiveProfileState,
        objective::ObjectiveKind,
        observation::AutotuneObservation,
        providers::{CandidateProposal, CandidateProviderInput, CandidateProviderRegistry},
        workload_policy::workload_policy_for_situation,
    },
    daemon::{DaemonCapabilities, DaemonMode, DaemonPolicy, SystemHealthSnapshot},
    daemon_policy::{ActionDescriptor, DaemonPolicyContext, PolicyIntent},
    profiles::Profile,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateDenyReason {
    DisabledFamily,
    DeniedFamily,
    SafetyClassTooHigh,
    EffectScopeTooBroad,
    CapabilityMissing,
    DataQualityLow,
    HealthDegraded,
    NoExplicitTarget,
    FocusLowConfidence,
    CriticalRealtimeWarning,
    CooldownActive,
    ConflictWithActiveAction,
    ConflictWithKeptAction,
    CgroupTargetNotAllowlisted,
    WorkloadPolicyBlocked,
    NotAutonomousForWorkload,
    ObjectiveNotAllowedForWorkload,
    ObjectiveSignalMissing,
    DryRunFailed,
    DryRunMatchedZeroTasks,
    PolicyRejected,
}

#[derive(Clone, Debug)]
pub struct CandidateEvaluation {
    pub candidate_name: String,
    pub action_kind: String,
    pub descriptor: ActionDescriptor,
    pub provider: String,
    pub eligible: bool,
    pub deny_reasons: Vec<CandidateDenyReason>,
    pub deny_messages: Vec<String>,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
    pub rank: Option<u32>,
    pub dry_run: Option<ActionState>,
    pub candidate: CandidateAction,
}

#[derive(Clone, Debug, Default)]
pub struct PlanResult {
    pub selected: Option<CandidateAction>,
    pub evaluations: Vec<CandidateEvaluation>,
    pub no_action_reason: Option<String>,
}

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
        if input.daemon_policy.mode == crate::daemon::DaemonMode::Observe {
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

        let provider_input = CandidateProviderInput {
            observation: input.observation,
            daemon_policy: input.daemon_policy,
            capabilities: input.capabilities,
            system_health: input.system_health,
            controller_state: input.controller_state,
            profiles: input.profiles,
        };
        let proposals = self.registry.propose(&provider_input);
        let mut evaluations = evaluate_proposals(input, proposals);

        evaluations.sort_by_key(|evaluation| {
            (
                !evaluation.eligible,
                evaluation.rank.unwrap_or(u32::MAX),
                evaluation.candidate_name.clone(),
            )
        });

        let selected = evaluations
            .iter()
            .find(|evaluation| evaluation.eligible)
            .map(|evaluation| evaluation.candidate.clone());

        let no_action_reason = if selected.is_none() {
            Some(
                evaluations
                    .first()
                    .and_then(|evaluation| evaluation.deny_messages.first().cloned())
                    .unwrap_or_else(|| {
                        "no candidate providers produced an eligible candidate".to_owned()
                    }),
            )
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
    pub profiles: &'a [Profile],
}

fn evaluate_proposals(
    input: PlannerInput<'_>,
    proposals: Vec<CandidateProposal>,
) -> Vec<CandidateEvaluation> {
    let candidates = proposals
        .iter()
        .map(|proposal| proposal.candidate.clone())
        .collect::<Vec<_>>();
    let dry_runs = dry_run_candidates(&candidates);

    evaluate_proposals_with_dry_runs(input, proposals, &dry_runs)
}

fn evaluate_proposals_with_dry_runs(
    input: PlannerInput<'_>,
    proposals: Vec<CandidateProposal>,
    dry_runs: &[CandidateDryRunRecord],
) -> Vec<CandidateEvaluation> {
    proposals
        .into_iter()
        .map(|proposal| {
            let descriptor = proposal.candidate.descriptor();
            let dry_run = dry_runs
                .iter()
                .find(|record| record.candidate_name == proposal.candidate.candidate_name());
            let mut deny_reasons = Vec::new();
            let mut deny_messages = proposal.deny_reasons.clone();

            let policy_context = policy_context_for_input(input);
            if let Err(rejection) = input.daemon_policy.check_action_with_context(
                PolicyIntent::Suggest,
                &descriptor,
                &policy_context,
            ) {
                deny_reasons.push(deny_reason_from_policy(rejection.reason_code()));
                deny_messages.push(rejection.to_string());
            }

            if input.observation.focus_confidence < input.daemon_policy.min_confidence {
                deny_reasons.push(CandidateDenyReason::FocusLowConfidence);
                deny_messages.push(format!(
                    "focus confidence {:.3} below policy minimum {:.3}",
                    input.observation.focus_confidence, input.daemon_policy.min_confidence
                ));
            }

            if input.observation.focus_has_critical_realtime_warning() {
                deny_reasons.push(CandidateDenyReason::CriticalRealtimeWarning);
                deny_messages.push("critical realtime focus warning blocks mutation".to_owned());
            }

            let workload_policy =
                workload_policy_for_situation(input.observation.primary_situation);
            if !workload_policy.allows_candidate(&proposal.candidate) {
                deny_reasons.push(CandidateDenyReason::WorkloadPolicyBlocked);
                deny_messages.push(format!(
                    "workload policy for {:?} does not allow action family {}",
                    input.observation.primary_situation,
                    proposal.candidate.action_kind()
                ));
            }
            if mode_requires_autonomous_workload_family(input.daemon_policy.mode)
                && !workload_policy.allows_autonomous_candidate(&proposal.candidate)
            {
                deny_reasons.push(CandidateDenyReason::NotAutonomousForWorkload);
                deny_messages.push(format!(
                    "workload policy for {:?} does not allow autonomous apply for action family {}; autonomous_families={:?}",
                    input.observation.primary_situation,
                    proposal.candidate.action_kind(),
                    workload_policy.autonomous_families
                ));
            }
            if !workload_policy.allows_objective(proposal.objective) {
                deny_reasons.push(CandidateDenyReason::ObjectiveNotAllowedForWorkload);
                deny_messages.push(format!(
                    "workload policy for {:?} does not allow objective {:?}",
                    input.observation.primary_situation, proposal.objective
                ));
            }

            if input
                .controller_state
                .candidate_memory
                .cooldown_remaining_for_action(
                    &proposal.candidate.action_id(),
                    input.observation.now_unix_nanos,
                )
                .is_some()
            {
                deny_reasons.push(CandidateDenyReason::CooldownActive);
                deny_messages.push("candidate action is cooling down".to_owned());
            }

            if let Some(active_experiment) = input.controller_state.active_experiment.as_ref()
                && proposal
                    .candidate
                    .conflicts_with(&active_experiment.candidate)
            {
                deny_reasons.push(CandidateDenyReason::ConflictWithActiveAction);
                deny_messages.push(format!(
                    "candidate conflict group {:?} conflicts with active experiment {} ({:?})",
                    proposal.candidate.conflict_group(),
                    active_experiment.candidate.candidate_name(),
                    active_experiment.candidate.conflict_group()
                ));
            }

            if let Some(kept) = input
                .active_profile_state
                .and_then(|state| state.current.as_ref())
                && proposal.candidate.conflicts_with(&kept.candidate)
            {
                deny_reasons.push(CandidateDenyReason::ConflictWithKeptAction);
                deny_messages.push(format!(
                    "candidate conflict group {:?} conflicts with kept action {} ({:?})",
                    proposal.candidate.conflict_group(),
                    kept.candidate.candidate_name(),
                    kept.candidate.conflict_group()
                ));
            }

            if let Some(cgroup_target) = proposal.candidate.cgroup_target_path()
                && !input
                    .daemon_policy
                    .cgroup_targets
                    .contains_path(cgroup_target)
            {
                deny_reasons.push(CandidateDenyReason::CgroupTargetNotAllowlisted);
                deny_messages.push(format!(
                    "cgroup target {} is not allowlisted by daemon policy",
                    cgroup_target.display()
                ));
            }

            let dry_run_state = dry_run.map(|record| ActionState {
                applied: false,
                affected_tasks: record.affected_tasks,
                checked_tasks: record.affected_tasks,
                pending_changes: record.affected_tasks,
                warnings: record.warnings.clone(),
            });

            if let Some(record) = dry_run {
                if !record.eligible {
                    deny_reasons.push(if record.affected_tasks == 0 {
                        CandidateDenyReason::DryRunMatchedZeroTasks
                    } else {
                        CandidateDenyReason::DryRunFailed
                    });
                    if let Some(reason) = &record.reason {
                        deny_messages.push(reason.clone());
                    }
                }
                if record.safety_class > input.daemon_policy.max_safety_class {
                    deny_reasons.push(CandidateDenyReason::SafetyClassTooHigh);
                    deny_messages.push(format!(
                        "candidate safety {:?} exceeds mode maximum {:?}",
                        record.safety_class, input.daemon_policy.max_safety_class
                    ));
                }
            }

            deny_reasons.sort_by_key(|reason| format!("{reason:?}"));
            deny_reasons.dedup();
            deny_messages.sort();
            deny_messages.dedup();

            CandidateEvaluation {
                candidate_name: proposal.candidate.candidate_name().to_owned(),
                action_kind: proposal.candidate.action_kind().to_owned(),
                descriptor,
                provider: proposal.provider.to_owned(),
                eligible: deny_reasons.is_empty(),
                deny_reasons,
                deny_messages,
                evidence: proposal.candidate.evidence().to_vec(),
                objective: proposal.objective,
                rank: Some(proposal.rank_hint),
                dry_run: dry_run_state,
                candidate: proposal.candidate,
            }
        })
        .collect()
}

fn mode_requires_autonomous_workload_family(mode: DaemonMode) -> bool {
    matches!(
        mode,
        DaemonMode::ApplyLowRisk | DaemonMode::ApplyMediumRisk | DaemonMode::ApplyHighRisk
    )
}

fn policy_context_for_input(input: PlannerInput<'_>) -> DaemonPolicyContext {
    DaemonPolicyContext {
        data_quality_ok: !input.observation.data_quality.blocks_action(),
        data_quality_reason_code: input
            .observation
            .data_quality
            .reason_code_strings()
            .first()
            .cloned(),
        system_health_ok: input.system_health.ok_for_apply,
        system_health_reason_code: input.system_health.reason_code.clone(),
        workload_stable: input.observation.workload_identity.is_some(),
        cooldown_active: input
            .controller_state
            .cooldown_until_unix_nanos
            .is_some_and(|until| until > input.observation.now_unix_nanos),
        rollback_pending: input.controller_state.active_experiment.is_some(),
        capabilities: Some(input.capabilities.clone()),
    }
}

fn deny_reason_from_policy(reason_code: &str) -> CandidateDenyReason {
    match reason_code {
        "action_family_not_enabled" => CandidateDenyReason::DisabledFamily,
        "action_family_denied" => CandidateDenyReason::DeniedFamily,
        "safety_class_too_high" => CandidateDenyReason::SafetyClassTooHigh,
        "effect_scope_not_allowed" | "system_wide_action_blocked" => {
            CandidateDenyReason::EffectScopeTooBroad
        }
        "capability_unavailable" => CandidateDenyReason::CapabilityMissing,
        "data_quality_blocked" => CandidateDenyReason::DataQualityLow,
        "system_health_blocked" => CandidateDenyReason::HealthDegraded,
        "explicit_target_required" => CandidateDenyReason::NoExplicitTarget,
        "confidence_too_low" => CandidateDenyReason::FocusLowConfidence,
        "cooldown_active" => CandidateDenyReason::CooldownActive,
        _ => CandidateDenyReason::PolicyRejected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        actions::{
            RollbackToken, SafetyClass, TaskIdentity,
            cgroup::{CgroupPlacementAction, CgroupPlacementTarget},
            ioprio::{IoPrioAction, IoPrioPolicy, IoPrioValue},
            irq_affinity::{IrqAffinityAction, IrqAffinityEvidence, IrqAffinityRisk},
            nice::{NiceAction, NicePolicy},
            uclamp::{UclampAction, UclampValues},
        },
        affinity::CpuMask,
        autotune::{
            candidate::{
                CgroupPlacementActionPlan, IoPrioActionPlan, IrqAffinityActionPlan, NiceActionPlan,
                UclampActionPlan,
            },
            controller::ActiveExperiment,
            experiment::{ExperimentId, WindowScore},
            kept::{ActiveProfileState, KeptCandidateState},
            observation::{ActiveTaskSnapshot, AutotuneObservation},
            providers::{CandidateProvider, CandidateProviderInput, CandidateProviderRegistry},
            quality::OnlineDataQuality,
            state::SituationKind,
        },
        daemon::{ActionSource, DaemonMode},
        daemon_policy::{DaemonPolicyBuildInput, build_daemon_policy},
        focus::FocusGroupKind,
        process_tree::TaskClass,
        profiles::{Profile, ProfileRule},
        scorer::StutterScore,
    };

    fn policy(mode: DaemonMode) -> DaemonPolicy {
        let config = crate::autotune::runtime::daemon_config_for_runtime_mode(
            mode,
            ActionSource::AutotuneRuntime,
            Some(1234),
            None,
        );
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }

    fn suggest_policy_with_compile_cgroup() -> DaemonPolicy {
        let mut config = crate::autotune::runtime::daemon_config_for_runtime_mode(
            DaemonMode::Suggest,
            ActionSource::AutotuneRuntime,
            Some(1234),
            None,
        );
        config.safety.cgroup_targets.compile_cgroup = Some(std::path::PathBuf::from(
            "/user.slice/stutter-compile.slice",
        ));
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }

    fn observation() -> AutotuneObservation {
        let mut observation = AutotuneObservation {
            now_unix_nanos: 1_000,
            target_present: true,
            target_root_pid: Some(1234),
            active_target_count: 1,
            scored_task_count: 1,
            interval_count: 3,
            scored_samples: 100,
            score: StutterScore {
                total: 100,
                over_1ms: 10,
                ..StutterScore::default()
            },
            data_quality: OnlineDataQuality::High,
            primary_situation: SituationKind::GameCpuSchedulerPressure,
            focus_kind: Some(FocusGroupKind::Game),
            focus_confidence: 0.95,
            focus_roots: vec![1234],
            ..AutotuneObservation::default()
        };
        observation.refresh_situation_classification();
        observation
    }

    #[derive(Clone)]
    struct StaticProvider {
        candidate: CandidateAction,
    }

    impl CandidateProvider for StaticProvider {
        fn family(&self) -> &'static str {
            self.candidate.action_kind()
        }

        fn propose(&self, _input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
            vec![CandidateProposal {
                candidate: self.candidate.clone(),
                provider: self.family(),
                confidence: 1.0,
                deny_reasons: Vec::new(),
                objective: self.candidate.objective(),
                rank_hint: 1,
            }]
        }
    }

    fn cpu_affinity_candidate(name: &str) -> CandidateAction {
        CandidateAction::cpu_affinity_profile(
            Profile {
                name: name.to_owned(),
                rules: vec![ProfileRule {
                    affinity: Some(CpuMask::parse("0").unwrap()),
                    nice: None,
                    ionice: None,
                    match_class: vec![TaskClass::Game],
                    match_comm: Vec::new(),
                }],
            },
            1234,
        )
    }

    fn cgroup_candidate(target_cgroup: &str) -> CandidateAction {
        CandidateAction::CgroupPlacement {
            plan: CgroupPlacementActionPlan {
                name: format!("cgroup-to-{target_cgroup}"),
                action: CgroupPlacementAction {
                    cgroup_root: std::path::PathBuf::from("/sys/fs/cgroup"),
                    target_cgroup: std::path::PathBuf::from(target_cgroup),
                    targets: vec![CgroupPlacementTarget {
                        identity: TaskIdentity {
                            tid: 1234,
                            process_pid: Some(1234),
                            comm: Some("rustc".to_owned()),
                            starttime_ticks: None,
                        },
                        class: TaskClass::Compiler,
                    }],
                    cpuset_cpus: None,
                    cpuset_mems: None,
                },
                target_root_pid: Some(1234),
                evidence: Vec::new(),
                objective: ObjectiveKind::CompileThroughputWithForegroundProtection,
            },
        }
    }

    fn nice_candidate() -> CandidateAction {
        CandidateAction::Nice {
            plan: NiceActionPlan {
                name: "nice-background-compile".to_owned(),
                action: NiceAction {
                    targets: vec![TaskIdentity {
                        tid: 1234,
                        process_pid: Some(1234),
                        comm: Some("rustc".to_owned()),
                        starttime_ticks: None,
                    }],
                    nice: 5,
                    policy: NicePolicy::default(),
                },
                target_root_pid: Some(1234),
                evidence: Vec::new(),
                objective: ObjectiveKind::DesktopInteractivity,
            },
        }
    }

    fn task_identity() -> TaskIdentity {
        TaskIdentity {
            tid: 1234,
            process_pid: Some(1234),
            comm: Some("stutter-test".to_owned()),
            starttime_ticks: None,
        }
    }

    fn uclamp_candidate(name: &str) -> CandidateAction {
        CandidateAction::Uclamp {
            plan: UclampActionPlan {
                name: name.to_owned(),
                action: UclampAction {
                    targets: vec![task_identity()],
                    values: UclampValues {
                        sched_util_min: Some(128),
                        sched_util_max: Some(1024),
                    },
                },
                target_root_pid: Some(1234),
                evidence: Vec::new(),
                objective: ObjectiveKind::DesktopInteractivity,
            },
        }
    }

    fn ioprio_candidate(name: &str) -> CandidateAction {
        CandidateAction::IoPrio {
            plan: IoPrioActionPlan {
                name: name.to_owned(),
                action: IoPrioAction {
                    targets: vec![task_identity()],
                    ioprio: IoPrioValue::idle(),
                    policy: IoPrioPolicy {
                        allow_ioprio_changes: true,
                        require_strong_block_io_evidence: false,
                        strong_block_io_evidence: true,
                        ..IoPrioPolicy::default()
                    },
                },
                target_root_pid: Some(1234),
                evidence: Vec::new(),
                objective: ObjectiveKind::IoLatency,
            },
        }
    }

    fn irq_affinity_candidate(name: &str) -> CandidateAction {
        let evidence = IrqAffinityEvidence {
            strong_irq_evidence: true,
            stable_irq_identity: true,
            known_device_mapping: true,
            observed_irq: Some(42),
            observed_device_hint: Some("test-device".to_owned()),
            reason: "test IRQ evidence".to_owned(),
        };

        CandidateAction::IrqAffinity {
            plan: IrqAffinityActionPlan {
                name: name.to_owned(),
                action: IrqAffinityAction::new(
                    42,
                    "test-device".to_owned(),
                    "1".to_owned(),
                    IrqAffinityRisk::ReversibleMediumRisk,
                    evidence,
                ),
                evidence: Vec::new(),
                objective: ObjectiveKind::IrqOverlapReduction,
            },
        }
    }

    fn observation_for_situation(
        situation: SituationKind,
        focus_kind: FocusGroupKind,
    ) -> AutotuneObservation {
        let mut observation = observation();
        observation.primary_situation = situation;
        observation.focus_kind = Some(focus_kind);
        observation
    }

    fn enable_capability_for_candidate(
        observation: &mut AutotuneObservation,
        candidate: &CandidateAction,
    ) {
        match candidate.action_kind() {
            "uclamp" => observation.capabilities.uclamp_available = true,
            "ionice" => observation.capabilities.ionice_available = true,
            "irq_affinity" => observation.capabilities.irq_affinity_available = true,
            "gpu_power" => observation.capabilities.gpu_sysfs_available = true,
            _ => {}
        }
    }

    fn eligible_dry_run(candidate: &CandidateAction) -> CandidateDryRunRecord {
        CandidateDryRunRecord {
            candidate_name: candidate.candidate_name().to_owned(),
            affected_tasks: 1,
            warnings: Vec::new(),
            safety_class: candidate.safety_class(),
            eligible: true,
            reason: None,
        }
    }

    fn evaluate_static_candidate(
        mode: DaemonMode,
        situation: SituationKind,
        focus_kind: FocusGroupKind,
        candidate: CandidateAction,
    ) -> CandidateEvaluation {
        let policy = policy(mode);
        let mut observation = observation_for_situation(situation, focus_kind);
        enable_capability_for_candidate(&mut observation, &candidate);
        let dry_runs = vec![eligible_dry_run(&candidate)];
        let proposals = vec![CandidateProposal {
            candidate: candidate.clone(),
            provider: candidate.action_kind(),
            confidence: 1.0,
            deny_reasons: Vec::new(),
            objective: candidate.objective(),
            rank_hint: 1,
        }];

        let mut evaluations = evaluate_proposals_with_dry_runs(
            PlannerInput {
                observation: &observation,
                daemon_policy: &policy,
                capabilities: &observation.capabilities,
                system_health: &observation.system_health,
                controller_state: &ControllerRuntimeState::default(),
                active_profile_state: None,
                profiles: &[],
            },
            proposals,
            &dry_runs,
        );

        assert_eq!(evaluations.len(), 1);
        evaluations.remove(0)
    }

    fn assert_autonomous_denial_mentions_context(
        evaluation: &CandidateEvaluation,
        situation: SituationKind,
        action_kind: &str,
    ) {
        assert!(!evaluation.eligible);
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::NotAutonomousForWorkload)
        );
        assert!(
            evaluation.deny_messages.iter().any(|message| {
                message.contains(&format!("{situation:?}"))
                    && message.contains(action_kind)
                    && message.contains("autonomous_families=")
            }),
            "missing autonomous denial context in {:?}",
            evaluation.deny_messages
        );
    }

    fn window_score(total: u64) -> WindowScore {
        WindowScore {
            started_unix_nanos: 1,
            finished_unix_nanos: 2,
            interval_count: 5,
            scored_samples: 100,
            scored_task_count: 1,
            score: StutterScore {
                total,
                ..StutterScore::default()
            },
        }
    }

    fn rollback_token() -> RollbackToken {
        RollbackToken::CpuAffinityRestoreFile {
            path: std::path::PathBuf::from("/tmp/stutter-restore.json"),
            affected_tasks: 1,
        }
    }

    #[test]
    fn observe_mode_returns_no_suggestions() {
        let policy = policy(DaemonMode::Observe);
        let planner = CandidatePlanner::default_for_policy(&policy);
        let observation = observation();

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: None,
            profiles: &[],
        });

        assert!(result.selected.is_none());
        assert!(result.evaluations.is_empty());
    }

    #[test]
    fn low_risk_policy_denies_medium_risk_fake_candidate() {
        let policy = policy(DaemonMode::ApplyLowRisk);
        let candidate = CandidateAction::Fake {
            action_id: crate::actions::ActionId("fake-medium".to_owned()),
            safety_class: SafetyClass::ReversibleMediumRisk,
        };

        assert!(candidate.safety_class() > policy.max_safety_class);
    }

    #[test]
    fn cgroup_provider_candidate_is_denied_when_cgroup_v2_capability_missing() {
        let policy = suggest_policy_with_compile_cgroup();
        let planner = CandidatePlanner::default_for_policy(&policy);
        let mut observation = observation();
        observation.primary_situation = SituationKind::CompileCpuBound;
        observation.focus_kind = Some(FocusGroupKind::Compile);
        observation.active_tasks = vec![ActiveTaskSnapshot {
            tid: 1234,
            process_pid: 1234,
            comm: "rustc".to_owned(),
            class: TaskClass::Compiler,
            process_starttime_ticks: Some(10),
            task_starttime_ticks: Some(10),
            cgroup_path: Some("/user.slice/app.scope".to_owned()),
        }];
        observation.refresh_situation_classification();

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: None,
            profiles: &[],
        });

        let cgroup = result
            .evaluations
            .iter()
            .find(|evaluation| evaluation.action_kind == "cgroup_placement")
            .expect("expected cgroup evaluation");
        assert!(
            cgroup
                .deny_reasons
                .contains(&CandidateDenyReason::CapabilityMissing)
        );
    }

    #[test]
    fn planner_denies_cgroup_candidate_outside_allowlist() {
        let policy = suggest_policy_with_compile_cgroup();
        let mut registry = CandidateProviderRegistry::default();
        registry.register(Box::new(StaticProvider {
            candidate: cgroup_candidate("/user.slice/not-allowlisted.slice"),
        }));
        let planner = CandidatePlanner::new(registry);
        let observation = observation();

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: None,
            profiles: &[],
        });

        assert!(
            result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::CgroupTargetNotAllowlisted)
        );
    }

    #[test]
    fn active_cpu_placement_experiment_blocks_cgroup_placement_candidate() {
        let policy = suggest_policy_with_compile_cgroup();
        let mut registry = CandidateProviderRegistry::default();
        registry.register(Box::new(StaticProvider {
            candidate: cgroup_candidate("/user.slice/stutter-compile.slice"),
        }));
        let planner = CandidatePlanner::new(registry);
        let observation = observation();
        let mut capabilities = observation.capabilities.clone();
        capabilities.cgroup_v2_available = true;
        let controller_state = ControllerRuntimeState {
            active_experiment: Some(ActiveExperiment {
                experiment_id: ExperimentId::new("active-cpu-placement"),
                candidate: cpu_affinity_candidate("game-main"),
                baseline_score_total: 100,
            }),
            ..ControllerRuntimeState::default()
        };

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &capabilities,
            system_health: &observation.system_health,
            controller_state: &controller_state,
            active_profile_state: None,
            profiles: &[],
        });

        assert!(
            result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::ConflictWithActiveAction)
        );
    }

    #[test]
    fn independent_nice_candidate_does_not_conflict_with_kept_cpu_placement() {
        let policy = policy(DaemonMode::Suggest);
        let mut registry = CandidateProviderRegistry::default();
        registry.register(Box::new(StaticProvider {
            candidate: nice_candidate(),
        }));
        let planner = CandidatePlanner::new(registry);
        let observation = observation();
        let kept = KeptCandidateState::new(
            ExperimentId::new("kept-cpu-placement"),
            cpu_affinity_candidate("game-main"),
            window_score(100),
            window_score(80),
            rollback_token(),
            10,
            "kept test candidate",
        );
        let active_profile_state = ActiveProfileState {
            current: Some(kept),
            history: Vec::new(),
        };

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: Some(&active_profile_state),
            profiles: &[],
        });

        assert!(
            !result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::ConflictWithKeptAction)
        );
    }

    #[test]
    fn recording_situation_blocks_game_only_cpu_affinity_candidate() {
        let policy = policy(DaemonMode::Suggest);
        let mut registry = CandidateProviderRegistry::default();
        registry.register(Box::new(StaticProvider {
            candidate: cpu_affinity_candidate("game-main"),
        }));
        let planner = CandidatePlanner::new(registry);
        let mut observation = observation();
        observation.primary_situation = SituationKind::Recording;
        observation.focus_kind = Some(FocusGroupKind::Recording);
        observation.refresh_situation_classification();

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: None,
            profiles: &[],
        });

        assert!(
            result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::WorkloadPolicyBlocked)
        );
    }

    #[test]
    fn game_cpu_affinity_profile_stays_apply_low_risk_eligible() {
        let evaluation = evaluate_static_candidate(
            DaemonMode::ApplyLowRisk,
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            cpu_affinity_candidate("game-main"),
        );

        assert!(
            evaluation.eligible,
            "expected cpu_affinity_profile to remain apply-low-risk eligible; deny_reasons={:?} deny_messages={:?}",
            evaluation.deny_reasons, evaluation.deny_messages
        );
        assert!(
            !evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::NotAutonomousForWorkload)
        );
    }

    #[test]
    fn game_uclamp_is_suggest_eligible_but_not_autonomous_for_apply() {
        let candidate = uclamp_candidate("game-uclamp");
        let suggest = evaluate_static_candidate(
            DaemonMode::Suggest,
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            candidate.clone(),
        );

        assert!(
            suggest.eligible,
            "expected game uclamp to remain suggest eligible; deny_reasons={:?} deny_messages={:?}",
            suggest.deny_reasons, suggest.deny_messages
        );

        let apply = evaluate_static_candidate(
            DaemonMode::ApplyMediumRisk,
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            candidate,
        );

        assert_autonomous_denial_mentions_context(
            &apply,
            SituationKind::GameCpuSchedulerPressure,
            "uclamp",
        );
    }

    #[test]
    fn compile_nice_is_suggest_eligible_but_not_autonomous_for_apply() {
        let candidate = nice_candidate();
        let suggest = evaluate_static_candidate(
            DaemonMode::Suggest,
            SituationKind::CompileCpuBound,
            FocusGroupKind::Compile,
            candidate.clone(),
        );

        assert!(
            suggest.eligible,
            "expected compile nice candidate to remain suggest eligible; deny_reasons={:?} deny_messages={:?}",
            suggest.deny_reasons, suggest.deny_messages
        );

        let apply = evaluate_static_candidate(
            DaemonMode::ApplyMediumRisk,
            SituationKind::CompileCpuBound,
            FocusGroupKind::Compile,
            candidate,
        );

        assert_autonomous_denial_mentions_context(&apply, SituationKind::CompileCpuBound, "nice");
    }

    #[test]
    fn empty_autonomous_families_block_browser_recording_irq_and_io_apply() {
        let cases = vec![
            (
                "browser nice",
                SituationKind::BrowserCpuPressure,
                FocusGroupKind::Browser,
                nice_candidate(),
            ),
            (
                "recording uclamp",
                SituationKind::Recording,
                FocusGroupKind::Recording,
                uclamp_candidate("recording-uclamp"),
            ),
            (
                "irq affinity",
                SituationKind::IrqPressure,
                FocusGroupKind::Desktop,
                irq_affinity_candidate("irq-affinity"),
            ),
            (
                "io ionice",
                SituationKind::IoPressure,
                FocusGroupKind::Desktop,
                ioprio_candidate("io-ionice"),
            ),
        ];

        for (label, situation, focus_kind, candidate) in cases {
            let action_kind = candidate.action_kind();
            let suggest = evaluate_static_candidate(
                DaemonMode::Suggest,
                situation,
                focus_kind,
                candidate.clone(),
            );

            assert!(
                suggest.eligible,
                "expected {label} to remain suggest eligible; deny_reasons={:?} deny_messages={:?}",
                suggest.deny_reasons, suggest.deny_messages
            );

            let apply = evaluate_static_candidate(
                DaemonMode::ApplyMediumRisk,
                situation,
                focus_kind,
                candidate,
            );

            assert_autonomous_denial_mentions_context(&apply, situation, action_kind);
        }
    }

    #[test]
    fn browser_foreground_blocks_compile_throughput_optimization() {
        let policy = suggest_policy_with_compile_cgroup();
        let mut registry = CandidateProviderRegistry::default();
        registry.register(Box::new(StaticProvider {
            candidate: cgroup_candidate("/user.slice/stutter-compile.slice"),
        }));
        let planner = CandidatePlanner::new(registry);
        let mut observation = observation();
        observation.primary_situation = SituationKind::BrowserFocused;
        observation.focus_kind = Some(FocusGroupKind::Browser);
        observation.refresh_situation_classification();
        let mut capabilities = observation.capabilities.clone();
        capabilities.cgroup_v2_available = true;

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: None,
            profiles: &[],
        });

        assert!(
            result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::WorkloadPolicyBlocked)
        );
        assert!(
            result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::ObjectiveNotAllowedForWorkload)
        );
    }
}
