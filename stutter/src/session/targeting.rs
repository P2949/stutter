#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::{
    process_tree::{ProcessCache, TaskInfo},
    watch::WatchProcessState,
};

pub struct TargetController {
    pub tree_pids: Vec<u32>,
    pub watch_state: WatchProcessState,
    pub tree_root_starttimes: BTreeMap<u32, Option<u64>>,
    pub process_cache: ProcessCache,
    pub active_targets: BTreeMap<u32, TaskInfo>,
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
            active_targets: BTreeMap::new(),
        }
    }
}
