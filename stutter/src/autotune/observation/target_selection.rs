#![allow(unused_imports)] // Transitional target-selection facade while old paths are preserved.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

pub(crate) use crate::autotune::target_selection::{
    MutableTaskSelection, TargetSelectionMode, TaskTargetSelector,
    mutable_cgroup_targets_for_observation_with_mode,
    mutable_task_snapshots_for_observation_with_mode,
    mutable_task_targets_for_observation_with_mode,
};
