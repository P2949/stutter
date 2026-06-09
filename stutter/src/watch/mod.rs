use std::time::Duration;

mod apply;
mod policy;
mod process_match;
pub(crate) mod profile_explain_render;
mod resolve;
mod restore;
mod tree_roots;

#[cfg(test)]
mod tests;

pub const PROFILE_WATCH_VERIFY_MS: u64 = 10_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatchProcessState {
    None,
    Waiting,
    Running(u32),
}

impl WatchProcessState {
    pub fn running_pid(&self) -> Option<u32> {
        match self {
            WatchProcessState::Running(pid) => Some(*pid),
            _ => None,
        }
    }

    pub fn should_poll(&self) -> bool {
        matches!(self, WatchProcessState::Waiting | WatchProcessState::None)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchProcessConfig {
    pub pattern: Option<String>,
    pub persistent: bool,
    pub poll_ms: u64,
    pub timeout: Option<Duration>,
}

impl WatchProcessConfig {
    pub fn from_monitor_config(config: &crate::config::model::MonitorConfig) -> Self {
        Self {
            pattern: config.target.watch_process.clone(),
            persistent: config.target.persistent,
            poll_ms: config.watch.poll_ms,
            timeout: config.watch.timeout,
        }
    }

    pub fn is_active(&self) -> bool {
        self.pattern.is_some()
    }
}

pub use apply::{
    ApplyProfileCommandInput, ProfilePlanCommandInput, apply_profile_command,
    apply_profile_to_tree_blocking, apply_profile_to_tree_cached_blocking, profile_plan_command,
};
pub use policy::profile_apply_policy;
#[cfg(test)]
pub(super) use policy::{
    force_for_watch_apply, validate_apply_profile_mode, validate_apply_profile_policy,
};
pub use process_match::find_process_by_pattern_at_with_cache;
#[cfg(test)]
pub use process_match::{find_process_by_pattern_at, process_match_score};
pub use resolve::resolve_watch_process;
pub use tree_roots::{
    add_watch_tree_pid, capture_tree_root_starttimes, process_root_starttime,
    remove_stale_tree_roots, remove_watch_tree_pid, tree_root_is_stale,
};
