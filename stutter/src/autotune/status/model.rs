use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{actions::ActionId, autotune::planner::PlannerSummary};

#[derive(Clone, Debug)]
pub struct AutotuneStatusCommandInput {
    pub json: bool,
    pub history_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AutotuneStatus {
    pub phase: String,
    pub mode: String,
    pub target: Option<StatusTarget>,
    pub focus_group: Option<String>,
    pub current_score: Option<u64>,
    pub active_profile: Option<String>,
    pub active_candidate: Option<String>,
    pub kept_actions: Vec<StatusKeptAction>,
    pub last_decision: String,
    pub rollback_available: bool,
    pub last_rollback_path: Option<String>,
    pub cooldown_remaining_seconds: Option<u64>,
    pub planner: Option<PlannerSummary>,
    pub data_quality: Option<String>,
    pub last_fault: Option<String>,
    pub manual_restore_command: String,
    pub history_path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusTarget {
    pub comm: String,
    pub pid: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusKeptAction {
    pub candidate_name: String,
    pub action_id: ActionId,
    pub action_kind: String,
    pub safety_class: String,
    pub kept_unix_nanos: u128,
}
