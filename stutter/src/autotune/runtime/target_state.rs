//! Runtime target snapshot state shared by controller and daemon-state views.

use crate::process_tree::TaskMap;

#[derive(Clone, Debug)]
pub struct RuntimeTargetState {
    pub root_pid: Option<u32>,
    pub active_targets: usize,
    pub target_comm: Option<String>,
    pub active_tasks: TaskMap,
}

impl RuntimeTargetState {
    pub(crate) fn new(root_pid: Option<u32>) -> Self {
        Self {
            root_pid,
            active_targets: 0,
            target_comm: None,
            active_tasks: TaskMap::new(),
        }
    }
}
