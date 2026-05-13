use serde::{Deserialize, Serialize};

use crate::{
    actions::{RollbackToken, SafetyClass},
    daemon::policy::DaemonMode,
};

pub const DAEMON_STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DaemonPhase {
    Disabled,
    Observing,
    Planning,
    Applying,
    Measuring,
    Keeping,
    Reverting,
    Cooldown,
    Faulted,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonState {
    pub schema_version: u32,
    pub mode: DaemonMode,
    pub phase: DaemonPhase,
    pub active_target: Option<DaemonTargetState>,
    pub active_experiment: Option<DaemonExperimentState>,
    pub active_rollback: Option<DaemonRollbackState>,
    pub last_decision: Option<DaemonDecisionState>,
    pub degraded: Vec<DaemonDegradedStatus>,
    pub faulted: Option<DaemonFaultState>,
}

impl Default for DaemonState {
    fn default() -> Self {
        Self {
            schema_version: DAEMON_STATE_SCHEMA_VERSION,
            mode: DaemonMode::Observe,
            phase: DaemonPhase::Disabled,
            active_target: None,
            active_experiment: None,
            active_rollback: None,
            last_decision: None,
            degraded: Vec::new(),
            faulted: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonTargetState {
    pub root_pid: Option<u32>,
    pub active_targets: usize,
    pub comm: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonExperimentState {
    pub experiment_id: String,
    pub action_id: String,
    pub candidate_name: Option<String>,
    pub safety_class: SafetyClass,
    pub started_unix_nanos: Option<u128>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonRollbackState {
    pub action_id: String,
    pub rollback_available: bool,
    pub token: Option<RollbackToken>,
    pub manual_restore_command: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonDecisionState {
    pub decision: String,
    pub reason: String,
    pub unix_nanos: Option<u128>,
    pub score_total: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonDegradedStatus {
    pub category: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonFaultState {
    pub reason: String,
    pub manual_restore_command: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_state_default_serializes_with_schema_version() {
        let state = DaemonState::default();

        let json = serde_json::to_string(&state).unwrap();

        assert!(json.contains("\"schema_version\":1"));
        assert!(json.contains("\"mode\":\"observe\""));
        assert!(json.contains("\"phase\":\"disabled\""));
    }

    #[test]
    fn daemon_state_can_store_live_runtime_fields() {
        let state = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Measuring,
            active_target: Some(DaemonTargetState {
                root_pid: Some(1234),
                active_targets: 12,
                comm: Some("game".to_owned()),
            }),
            active_experiment: Some(DaemonExperimentState {
                experiment_id: "experiment-1".to_owned(),
                action_id: "cpu-affinity-profile:game".to_owned(),
                candidate_name: Some("game".to_owned()),
                safety_class: SafetyClass::ReversibleLowRisk,
                started_unix_nanos: Some(100),
            }),
            active_rollback: Some(DaemonRollbackState {
                action_id: "cpu-affinity-profile:game".to_owned(),
                rollback_available: true,
                token: None,
                manual_restore_command: Some("stutter autotune restore".to_owned()),
            }),
            last_decision: Some(DaemonDecisionState {
                decision: "candidate_applied".to_owned(),
                reason: "candidate passed gates".to_owned(),
                unix_nanos: Some(200),
                score_total: Some(300),
            }),
            degraded: vec![DaemonDegradedStatus {
                category: "data_quality".to_owned(),
                message: "low scored samples".to_owned(),
            }],
            faulted: None,
            ..DaemonState::default()
        };

        let decoded: DaemonState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();

        assert_eq!(decoded.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(decoded.phase, DaemonPhase::Measuring);
        assert_eq!(
            decoded
                .active_target
                .as_ref()
                .and_then(|target| target.root_pid),
            Some(1234)
        );
        assert!(decoded.active_rollback.unwrap().rollback_available);
        assert_eq!(decoded.degraded.len(), 1);
    }
}
