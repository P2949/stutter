//! Runtime-to-daemon/history state mapping helpers; this module owns enum and profile DTO translation.

use super::{DAEMON_EMERGENCY_RESTORE_COMMAND, data_quality_label};
use crate::{
    actions::SafetyClass,
    autotune::{
        candidate_memory::CandidateMemoryResult,
        history::{
            AutotuneMode as HistoryAutotuneMode, ControllerPhase as HistoryControllerPhase,
            SituationKind as HistorySituationKind,
        },
        runtime::AutotuneRuntime,
        state::{ControllerPhase, SituationKind},
    },
    daemon::{
        health::{
            SystemHealthInputs, SystemHealthSnapshot, SystemHealthThresholds,
            evaluate_system_health,
        },
        policy::DaemonMode,
        state::{
            DAEMON_STATE_SCHEMA_VERSION, DaemonDecisionState, DaemonDegradedStatus,
            DaemonFaultState, DaemonPhase, DaemonProfileEnvironment, DaemonProfileMemory,
            DaemonProfilePartition, DaemonState, DaemonTargetState, DaemonWorkloadProfile,
            daemon_profile_stable_hash,
        },
    },
};

impl AutotuneRuntime {
    pub fn daemon_state_snapshot(&self) -> DaemonState {
        let active_experiment = self.live_experiments.daemon_experiment_state();

        let active_rollback = self
            .live_experiments
            .daemon_rollback_state(DAEMON_EMERGENCY_RESTORE_COMMAND);

        let active_target = self.daemon_active_target_snapshot();
        let profile_memory = self.daemon_profile_memory_snapshot(active_target.as_ref());

        DaemonState {
            schema_version: DAEMON_STATE_SCHEMA_VERSION,
            mode: self.config.mode(),
            phase: daemon_phase_from_controller_phase(self.controller.state.phase),
            cooldown_until_unix_nanos: self.controller.state.cooldown_until_unix_nanos,
            active_target,
            active_experiment,
            active_rollback,
            last_decision: self
                .last_decision
                .as_ref()
                .map(|decision| DaemonDecisionState {
                    decision: decision.decision.clone(),
                    reason: decision.reason.clone(),
                    unix_nanos: Some(decision.unix_nanos),
                    diagnostic_score_total: Some(decision.diagnostic_score_total),
                    candidate_count: Some(decision.candidate_count),
                    top_denied_reason: decision.top_denied_reason.clone(),
                    planner: decision.planner.clone(),
                    situation: Some(decision.situation.clone()),
                    focus_kind: decision.focus_kind.clone(),
                }),
            health: self.daemon_health_snapshot(),
            degraded: self.daemon_degraded_statuses(),
            faulted: self.daemon_fault_state(),
            profile_memory,
        }
    }

    fn daemon_profile_memory_snapshot(
        &self,
        active_target: Option<&DaemonTargetState>,
    ) -> DaemonProfileMemory {
        let environment = DaemonProfileEnvironment::current();
        let workload_identity_hash = daemon_profile_workload_identity_hash(active_target);
        let workload_label = active_target
            .and_then(|target| target.comm.clone())
            .or_else(|| self.target_state.target_comm.clone());
        let profiles = self
            .controller
            .state
            .candidate_memory
            .records
            .iter()
            .filter(|record| record.result == CandidateMemoryResult::Kept)
            .map(|record| {
                let (action_kind, safety_class) =
                    daemon_profile_action_kind_and_safety_class(record.action_id.as_str());
                DaemonWorkloadProfile {
                    workload_identity_hash: workload_identity_hash.clone(),
                    workload_label: workload_label.clone(),
                    candidate_name: record.candidate_name.clone(),
                    action_id: record.action_id.as_str().to_owned(),
                    action_kind,
                    safety_class,
                    kept_unix_nanos: record.last_tried_unix_nanos,
                    last_validated_unix_nanos: Some(record.last_tried_unix_nanos),
                    diagnostic_baseline_raw_score_total: None,
                    diagnostic_candidate_diagnostic_score_total: None,
                    score_delta: record.score_delta,
                    confidence_milli: daemon_profile_confidence_milli(record.score_delta),
                    environment: environment.clone(),
                    partition: DaemonProfilePartition {
                        scheduler_label: environment.scheduler_label.clone(),
                        ..DaemonProfilePartition::default()
                    },
                }
            })
            .collect();

        DaemonProfileMemory { profiles }
    }

    fn daemon_health_snapshot(&self) -> SystemHealthSnapshot {
        let thresholds = SystemHealthThresholds {
            max_ebpf_dropped_events: self
                .config
                .online_data_quality_policy
                .max_drop_counter_total,
            ..SystemHealthThresholds::default()
        };

        evaluate_system_health(
            SystemHealthInputs {
                ebpf_dropped_events: self.latest_drop_counters.total(),
                ..SystemHealthInputs::default()
            },
            &thresholds,
        )
    }

    fn daemon_active_target_snapshot(&self) -> Option<DaemonTargetState> {
        let focus = self.latest_focus.as_ref();
        let root_pid = self
            .target_state
            .root_pid
            .or_else(|| focus.and_then(|focus| focus.root_pids.first().copied()));
        let active_targets = if self.target_state.active_targets > 0 {
            self.target_state.active_targets
        } else {
            focus.map(|focus| focus.member_pids.len()).unwrap_or(0)
        };
        let comm = self.target_state.target_comm.clone();

        if root_pid.is_none() && active_targets == 0 && comm.is_none() {
            return None;
        }

        Some(DaemonTargetState {
            root_pid,
            active_targets,
            comm,
        })
    }

    fn daemon_degraded_statuses(&self) -> Vec<DaemonDegradedStatus> {
        let mut degraded = Vec::new();

        if self.last_observation.data_quality.is_low() {
            let data_quality_reason_codes =
                self.last_observation.data_quality.reason_code_strings();
            let message = if data_quality_reason_codes.is_empty() {
                data_quality_label(&self.last_observation.data_quality)
            } else {
                format!(
                    "{} reason_codes={}",
                    data_quality_label(&self.last_observation.data_quality),
                    data_quality_reason_codes.join(",")
                )
            };
            degraded.push(DaemonDegradedStatus {
                category: "data_quality".to_owned(),
                message,
            });
        }

        let drop_counter_total = self.latest_drop_counters.total();
        if drop_counter_total > 0 {
            degraded.push(DaemonDegradedStatus {
                category: "drop_counters".to_owned(),
                message: format!("drop counters reported {drop_counter_total} lost events"),
            });
        }

        if let Some(cooldown_until_unix_nanos) = self.controller.state.cooldown_until_unix_nanos {
            degraded.push(DaemonDegradedStatus {
                category: "cooldown".to_owned(),
                message: format!("cooldown_until_unix_nanos={cooldown_until_unix_nanos}"),
            });
        }

        for diagnosis in &self.recent_diagnoses {
            degraded.push(DaemonDegradedStatus {
                category: "recent_diagnosis".to_owned(),
                message: format!(
                    "{:?} {:?} anchored on {}",
                    diagnosis.confidence, diagnosis.cause, diagnosis.anchor_comm
                ),
            });
        }

        degraded
    }

    fn daemon_fault_state(&self) -> Option<DaemonFaultState> {
        if self.controller.state.phase != ControllerPhase::Faulted {
            return None;
        }

        Some(DaemonFaultState {
            reason: self
                .last_decision
                .as_ref()
                .map(|decision| decision.reason.clone())
                .filter(|reason| !reason.trim().is_empty())
                .unwrap_or_else(|| "controller is faulted".to_owned()),
            manual_restore_command: Some(DAEMON_EMERGENCY_RESTORE_COMMAND.to_owned()),
        })
    }
}

pub fn daemon_phase_from_controller_phase(phase: ControllerPhase) -> DaemonPhase {
    match phase {
        ControllerPhase::Disabled => DaemonPhase::Disabled,
        ControllerPhase::Observing => DaemonPhase::Observe,
        ControllerPhase::Planning => DaemonPhase::Decide,
        ControllerPhase::Applying => DaemonPhase::Apply,
        ControllerPhase::Measuring => DaemonPhase::Measure,
        ControllerPhase::Keeping => DaemonPhase::Keep,
        ControllerPhase::Reverting => DaemonPhase::Rollback,
        ControllerPhase::Cooldown => DaemonPhase::Cooldown,
        ControllerPhase::Faulted => DaemonPhase::Faulted,
    }
}

pub(crate) fn daemon_profile_workload_identity_hash(
    active_target: Option<&DaemonTargetState>,
) -> String {
    let root_pid = active_target
        .and_then(|target| target.root_pid)
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "unknown".to_owned());
    let active_targets = active_target
        .map(|target| target.active_targets.to_string())
        .unwrap_or_else(|| "0".to_owned());
    let comm = active_target
        .and_then(|target| target.comm.as_deref())
        .unwrap_or("unknown");

    daemon_profile_stable_hash([root_pid.as_str(), active_targets.as_str(), comm])
}

pub(crate) fn daemon_profile_action_kind_and_safety_class(
    action_id: &str,
) -> (String, SafetyClass) {
    if action_id.starts_with("cpu-affinity-profile:") {
        (
            "cpu_affinity_profile".to_owned(),
            SafetyClass::ReversibleLowRisk,
        )
    } else {
        ("unknown".to_owned(), SafetyClass::ObserveOnly)
    }
}

pub(crate) fn daemon_profile_confidence_milli(score_delta: i64) -> u16 {
    if score_delta < 0 {
        900
    } else if score_delta == 0 {
        600
    } else {
        350
    }
}

pub(crate) fn history_phase(phase: ControllerPhase) -> HistoryControllerPhase {
    match phase {
        ControllerPhase::Disabled => HistoryControllerPhase::Disabled,
        ControllerPhase::Observing => HistoryControllerPhase::Observing,
        ControllerPhase::Planning => HistoryControllerPhase::Planning,
        ControllerPhase::Applying => HistoryControllerPhase::Applying,
        ControllerPhase::Measuring => HistoryControllerPhase::Measuring,
        ControllerPhase::Keeping => HistoryControllerPhase::Keeping,
        ControllerPhase::Reverting => HistoryControllerPhase::Reverting,
        ControllerPhase::Cooldown => HistoryControllerPhase::Cooldown,
        ControllerPhase::Faulted => HistoryControllerPhase::Faulted,
    }
}

pub(crate) fn history_mode(mode: DaemonMode) -> HistoryAutotuneMode {
    match mode {
        DaemonMode::Observe => HistoryAutotuneMode::Observe,
        DaemonMode::Suggest => HistoryAutotuneMode::Suggest,
        DaemonMode::ApplyLowRisk => HistoryAutotuneMode::ApplyLowRisk,
        DaemonMode::ApplyMediumRisk => HistoryAutotuneMode::ApplyMediumRisk,
        DaemonMode::ApplyHighRisk => HistoryAutotuneMode::ApplyHighRisk,
    }
}

pub(crate) fn history_situation(situation: SituationKind) -> HistorySituationKind {
    situation
}
