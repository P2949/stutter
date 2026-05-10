use std::collections::BTreeMap;

use crate::{
    process_tree::ProcessCache,
    tasks::TaskTracker,
    watch::{WatchProcessState, capture_tree_root_starttimes},
};

pub struct TargetController {
    pub tree_pids: Vec<u32>,
    pub watch_state: WatchProcessState,
    pub tree_root_starttimes: BTreeMap<u32, Option<u64>>,
    pub process_cache: ProcessCache,
    pub tasks: TaskTracker,
}

impl TargetController {
    pub fn new(
        tree_pids: Vec<u32>,
        watch_state: WatchProcessState,
        tree_root_starttimes: BTreeMap<u32, Option<u64>>,
    ) -> Self {
        Self {
            tree_pids,
            watch_state,
            tree_root_starttimes,
            process_cache: ProcessCache::default(),
            tasks: TaskTracker::default(),
        }
    }

    pub fn from_config_parts(
        tree_pids: Vec<u32>,
        watch_state: WatchProcessState,
        tree_root_starttimes: BTreeMap<u32, Option<u64>>,
    ) -> Self {
        Self::new(tree_pids, watch_state, tree_root_starttimes)
    }

    pub fn replace_tree_roots(&mut self, tree_pids: Vec<u32>) {
        self.tree_pids = tree_pids;
        self.tree_root_starttimes = capture_tree_root_starttimes(&self.tree_pids);
    }

    pub fn clear_tree_roots(&mut self) {
        self.tree_pids.clear();
        self.tree_root_starttimes.clear();
    }

    pub fn has_tree_roots(&self) -> bool {
        !self.tree_pids.is_empty()
    }
}
