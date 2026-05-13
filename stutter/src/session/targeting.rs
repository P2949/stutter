use std::{collections::BTreeMap, path::PathBuf};

use crate::{
    community_rules::CommunityRulesDb,
    config::model::MonitorConfig,
    error::ConfigError,
    process_tree::{CompiledPattern, ProcessCache, TargetSnapshotInput, TaskFilters},
    tasks::TaskTracker,
    watch::{
        WatchProcessConfig, WatchProcessState, add_watch_tree_pid, capture_tree_root_starttimes,
        process_root_starttime, remove_stale_tree_roots, remove_watch_tree_pid,
    },
};

pub struct TargetPolicy {
    pub manual_pids: Vec<u32>,
    pub configured_tree_pids: Vec<u32>,
    pub cgroupv2: Option<PathBuf>,
    pub exclude_tree_pids: Vec<u32>,
    pub include_comm: Vec<String>,
    pub exclude_comm: Vec<String>,
    pub keep_missing_pid: bool,
    pub max_tasks: usize,
    pub compiled_filters: TaskFilters,
}

impl TargetPolicy {
    pub fn from_monitor_config(config: &MonitorConfig) -> Result<Self, ConfigError> {
        let compiled_filters =
            compile_target_filters(&config.target.include_comm, &config.target.exclude_comm)?;

        Ok(Self {
            manual_pids: config.target.target_pids.clone(),
            configured_tree_pids: config.target.tree_pids.clone(),
            cgroupv2: config.target.cgroupv2.clone(),
            exclude_tree_pids: config.target.exclude_tree_pids.clone(),
            include_comm: config.target.include_comm.clone(),
            exclude_comm: config.target.exclude_comm.clone(),
            keep_missing_pid: config.target.keep_missing_pid,
            max_tasks: config.target.max_tasks,
            compiled_filters,
        })
    }
}

fn compile_target_filters(
    include_comm: &[String],
    exclude_comm: &[String],
) -> Result<TaskFilters, ConfigError> {
    Ok(TaskFilters {
        include_comm: compile_target_patterns("include_comm", include_comm)?,
        exclude_comm: compile_target_patterns("exclude_comm", exclude_comm)?,
    })
}

fn compile_target_patterns(
    field: &'static str,
    patterns: &[String],
) -> Result<Vec<CompiledPattern>, ConfigError> {
    patterns
        .iter()
        .map(|pattern| {
            CompiledPattern::new(pattern.clone()).map_err(|source| {
                ConfigError::InvalidTargetFilter {
                    field,
                    pattern: pattern.clone(),
                    source,
                }
            })
        })
        .collect()
}

pub struct TargetController {
    pub policy: TargetPolicy,
    pub watch_config: WatchProcessConfig,
    pub dynamic_tree_pids: Vec<u32>,
    pub watch_state: WatchProcessState,
    pub tree_root_starttimes: BTreeMap<u32, Option<u64>>,
    pub process_cache: ProcessCache,
    pub tasks: TaskTracker,
}

impl TargetController {
    pub fn new(
        policy: TargetPolicy,
        watch_config: WatchProcessConfig,
        dynamic_tree_pids: Vec<u32>,
        watch_state: WatchProcessState,
        tree_root_starttimes: BTreeMap<u32, Option<u64>>,
    ) -> Self {
        Self {
            policy,
            watch_config,
            dynamic_tree_pids,
            watch_state,
            tree_root_starttimes,
            process_cache: ProcessCache::default(),
            tasks: TaskTracker::default(),
        }
    }

    pub fn from_policy_parts(
        policy: TargetPolicy,
        watch_config: WatchProcessConfig,
        dynamic_tree_pids: Vec<u32>,
        watch_state: WatchProcessState,
        tree_root_starttimes: BTreeMap<u32, Option<u64>>,
    ) -> Self {
        Self::new(
            policy,
            watch_config,
            dynamic_tree_pids,
            watch_state,
            tree_root_starttimes,
        )
    }

    pub fn effective_tree_pids(&self) -> &[u32] {
        &self.dynamic_tree_pids
    }

    pub fn replace_dynamic_tree_roots(&mut self, roots: Vec<u32>) {
        self.dynamic_tree_pids = roots;
        self.tree_root_starttimes = capture_tree_root_starttimes(&self.dynamic_tree_pids);
    }

    pub fn clear_dynamic_tree_roots(&mut self) {
        self.dynamic_tree_pids.clear();
        self.tree_root_starttimes.clear();
    }

    pub fn add_watch_root(&mut self, pid: u32) {
        add_watch_tree_pid(&mut self.dynamic_tree_pids, pid);
        self.tree_root_starttimes
            .insert(pid, process_root_starttime(pid));
    }

    pub fn remove_watch_root(&mut self, pid: u32) {
        remove_watch_tree_pid(&mut self.dynamic_tree_pids, pid);
        self.tree_root_starttimes.remove(&pid);
    }

    pub fn remove_stale_dynamic_tree_roots(&mut self) -> Vec<u32> {
        remove_stale_tree_roots(
            &mut self.dynamic_tree_pids,
            &mut self.tree_root_starttimes,
            self.watch_state.running_pid(),
        )
    }

    pub fn has_tree_roots(&self) -> bool {
        !self.dynamic_tree_pids.is_empty()
    }

    pub fn target_snapshot_input<'a>(
        &'a mut self,
        community_rules: Option<&'a CommunityRulesDb>,
    ) -> TargetSnapshotInput<'a> {
        Self::target_snapshot_input_from_parts(
            &self.policy,
            &self.dynamic_tree_pids,
            &mut self.process_cache,
            community_rules,
        )
    }

    pub fn target_snapshot_input_from_parts<'a>(
        policy: &'a TargetPolicy,
        dynamic_tree_pids: &'a [u32],
        process_cache: &'a mut ProcessCache,
        community_rules: Option<&'a CommunityRulesDb>,
    ) -> TargetSnapshotInput<'a> {
        TargetSnapshotInput::default()
            .manual_pids(&policy.manual_pids)
            .tree_pids(dynamic_tree_pids)
            .cgroup_path(policy.cgroupv2.as_deref())
            .exclude_tree_pids(&policy.exclude_tree_pids)
            .filters(&policy.compiled_filters)
            .keep_missing_pid(policy.keep_missing_pid)
            .cache(process_cache)
            .community_rules(community_rules)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_policy_rejects_invalid_include_comm_regex() {
        let mut config = MonitorConfig::default();
        config.target.include_comm = vec!["/[unclosed/".to_owned()];

        let result = TargetPolicy::from_monitor_config(&config);

        match result {
            Err(ConfigError::InvalidTargetFilter { field, pattern, .. }) => {
                assert_eq!(field, "include_comm");
                assert_eq!(pattern, "/[unclosed/");
            }
            Err(other) => panic!("unexpected error: {other}"),
            Ok(_) => panic!("expected invalid include_comm regex to fail"),
        }
    }
}
