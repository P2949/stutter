use std::path::Path;

use super::*;
use crate::{
    autotune::{
        activity::ActivityLevel,
        observation::AutotuneObservation, state::SituationKind,
        quality::OnlineDataQualityPolicy,
        rolling_window::RollingWindow,
        system_context::{SystemContextSnapshotInput, collect_system_context},
    },
    diagnosis::LiveDiagnosisEntry,
    ebpf_loader::DropCountersSnapshot,
    focus::FocusGroupKind,
    process_tree::TaskMap,
};

pub struct AutotuneObservationBuilder;

#[derive(Clone, Debug)]
pub struct AutotuneObservationFocus {
    pub kind: FocusGroupKind,
    pub root_pids: Vec<u32>,
    pub member_pids: Vec<u32>,
    pub confidence: f32,
    pub situation: SituationKind,
    pub reasons: Vec<String>,
}

pub struct AutotuneObservationBuilderInput<'a> {
    pub window: &'a RollingWindow,
    pub online_data_quality_policy: &'a OnlineDataQualityPolicy,
    pub focus: Option<&'a AutotuneObservationFocus>,
    pub root_pid: Option<u32>,
    pub active_target_count: usize,
    pub active_tasks: &'a TaskMap,
    pub recent_diagnoses: Vec<LiveDiagnosisEntry>,
    pub drop_counters: DropCountersSnapshot,
    pub proc_root: &'a Path,
    pub sys_root: &'a Path,
    pub activity_level: ActivityLevel,
}

#[derive(Clone, Debug)]
pub struct AutotuneObservationBuildOutput {
    pub observation: AutotuneObservation,
}

impl AutotuneObservationBuilder {
    pub fn build(input: AutotuneObservationBuilderInput<'_>) -> AutotuneObservationBuildOutput {
        let window_score = input
            .window
            .score_with_quality_policy(input.online_data_quality_policy);
        let focus = input.focus;
        let focus_kind = focus.map(|focus| focus.kind);
        let focus_confidence = focus.map(|focus| focus.confidence).unwrap_or(0.0);
        let focus_roots = focus
            .map(|focus| focus.root_pids.clone())
            .unwrap_or_default();
        let focus_reasons = focus.map(|focus| focus.reasons.clone()).unwrap_or_default();
        let primary_situation = focus
            .map(|focus| focus.situation)
            .unwrap_or(SituationKind::Unknown);
        let system_health = crate::daemon::health::evaluate_system_health(
            crate::daemon::health::SystemHealthInputs {
                ebpf_dropped_events: input.drop_counters.total(),
                ..crate::daemon::health::SystemHealthInputs::default()
            },
            &crate::daemon::health::SystemHealthThresholds {
                max_ebpf_dropped_events: input.online_data_quality_policy.max_drop_counter_total,
                ..crate::daemon::health::SystemHealthThresholds::default()
            },
        );
        let active_tasks =
            active_task_snapshots_from_active_tasks(input.proc_root, input.active_tasks);
        let protected_tasks = protected_tasks_from_active_tasks(input.active_tasks);
        let system_context = collect_system_context(SystemContextSnapshotInput {
            proc_root: input.proc_root,
            sys_root: input.sys_root,
            active_tasks: &active_tasks,
            health: system_health,
            sampled_at_unix_nanos: crate::audit::unix_nanos_now(),
        });
        let capabilities = system_context.capabilities.clone();
        let topology_signature = system_context.inventory.inventory_hash.clone();
        let active_config_snapshot = system_context.active_config.clone();
        let target_root_pid = input.root_pid.or_else(|| focus_roots.first().copied());
        let workload_identity = workload_identity_from_runtime_context(
            input.proc_root,
            target_root_pid,
            focus_kind,
            input.active_tasks,
        );
        let mut objective_signals = input.window.objective_signals();
        apply_focus_gpu_resolution(
            &mut objective_signals,
            input.proc_root,
            target_root_pid,
            &active_tasks,
            &system_context.inventory,
        );

        let mut observation = AutotuneObservation {
            now_unix_nanos: system_context.sampled_at_unix_nanos,
            elapsed_ms: input.window.latest_elapsed_ms().unwrap_or(0),
            target_present: input.active_target_count > 0 || window_score.scored_samples > 0,
            target_root_pid,
            active_target_count: input.active_target_count,
            scored_task_count: window_score.scored_task_count,
            interval_count: window_score.interval_count,
            scored_samples: window_score.scored_samples,
            score: stutter_score_from_runtime_window_score(&window_score),
            data_quality: window_score.data_quality.clone(),
            activity_level: input.activity_level,
            objective_signals,
            primary_situation,
            situation: Default::default(),
            focus_kind,
            focus_confidence,
            focus_roots,
            focus_reasons,
            recent_diagnoses: input.recent_diagnoses,
            system_health: system_context.health.clone(),
            capabilities,
            topology_signature: Some(topology_signature),
            workload_identity,
            active_tasks,
            protected_tasks,
            active_config_snapshot: Some(active_config_snapshot),
            system_context: Some(system_context),
            frame_count: window_score.frame_count,
            frame_p99_ms: window_score.frame_p99_ms,
            frame_max_ms: window_score.frame_max_ms,
            drop_counter_total: input.drop_counters.total(),
        };
        observation.refresh_situation_classification();

        AutotuneObservationBuildOutput { observation }
    }
}
