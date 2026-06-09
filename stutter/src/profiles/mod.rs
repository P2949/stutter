use crate::{
    actions::ioprio::IoPrioValue,
    affinity::CpuMask,
    process_tree::{CompiledPattern, TaskClass},
};

mod action_decision;
mod apply;
mod evaluate;
pub(crate) mod explain;
mod ioprio;
mod matching;
pub(crate) mod parse;
mod plan;
pub(crate) mod render;
mod summary;
mod verify;
pub(crate) mod warnings;

#[cfg(test)]
mod tests;

pub use apply::{
    apply_managed_profile_to_tree, apply_managed_profile_to_tree_cached, apply_profile_to_tree,
    profile_uses_priority_actions,
};
pub use evaluate::{ProfileEvaluationInput, evaluate_profile_for_tasks};
#[cfg(test)]
pub(crate) use matching::{profile_matched_task_count, profile_matches_task};
pub use matching::{
    profile_matched_task_count_for_tree, profile_matched_task_count_from_snapshots,
};
#[cfg(test)]
pub(crate) use parse::parse_profiles;
pub use parse::{load_profiles, load_selected_profile};
pub use plan::ProfileApplyCache;
#[cfg(test)]
pub(crate) use plan::planned_profile_apply_with_readers;
pub use render::{generate_topology_template, render_profiles_toml};
#[cfg(test)]
pub(crate) use summary::ProfileApplySummary;
pub use summary::{ProfileApplyResult, profile_apply_summary_for_tree};
#[cfg(test)]
pub(crate) use summary::{profile_apply_summary_with_reader, profile_apply_summary_with_readers};
#[cfg(test)]
pub(crate) use warnings::{profile_offline_cpu_warnings, profile_rule_overlap_warnings};

#[derive(Clone, Debug)]
pub struct Profile {
    pub name: String,
    pub rules: Vec<ProfileRule>,
}

#[derive(Clone, Debug)]
pub struct ProfileRule {
    pub affinity: Option<CpuMask>,
    pub nice: Option<i32>,
    pub ionice: Option<IoPrioValue>,
    pub match_class: Vec<TaskClass>,
    pub match_comm: Vec<CompiledPattern>,
}

#[cfg(test)]
use crate::process_tree::{self, TaskInfo};
