//! `NumaMigratePages` as a manual, policy-gated action (package W11c).
//!
//! Moving a process's pages between NUMA nodes is **not exactly reversible**: pages reclaimed,
//! pinned or faulted in afterwards do not return where they were. The action is therefore HighRisk,
//! system-wide and manual, its token is manual-recovery only and says so, and it is never an
//! executable autotune plan. It verifies the target by live start time, the nodes against the
//! online node list, and accounts for what actually moved from `numa_maps` before and after. Moving
//! CPU affinity is not moving memory; this is the separate, explicit memory operation.
use std::path::{Path, PathBuf};

use super::{
    ActionBoundaryError, ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass,
    TuningAction,
};
use crate::autotune::sensors::parse_numa_maps;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NumaMigrationRecord {
    pub task: u32,
    pub start_ticks: u64,
    pub from_nodes: Vec<u32>,
    pub to_nodes: Vec<u32>,
    /// Pages per node before and after, as read from `numa_maps`.
    pub before: Vec<(u32, u64)>,
    pub after: Vec<(u32, u64)>,
    /// Pages the kernel reported it could not move.
    pub not_moved: u64,
}

/// The kernel call, replaceable in tests.
pub trait PageMigrator {
    fn migrate(&self, task: u32, from_mask: u64, to_mask: u64) -> std::io::Result<u64>;
}

pub struct RealMigrator;

impl PageMigrator for RealMigrator {
    fn migrate(&self, task: u32, from_mask: u64, to_mask: u64) -> std::io::Result<u64> {
        super::syscalls::migrate_pages(task, from_mask, to_mask)
    }
}

#[derive(Clone, Debug, Default)]
pub struct NumaMigrationPolicy {
    /// Nothing is moved unless this is set explicitly.
    pub allow: bool,
    /// Migration is irreversible in effect and must be acknowledged.
    pub acknowledge_irreversible: bool,
}

#[derive(Clone, Debug)]
pub struct NumaMigrationAction {
    pub proc_root: PathBuf,
    pub node_root: PathBuf,
    pub task: u32,
    pub start_ticks: u64,
    pub from_nodes: Vec<u32>,
    pub to_nodes: Vec<u32>,
}

fn mask(nodes: &[u32]) -> Option<u64> {
    nodes
        .iter()
        .try_fold(0u64, |acc, node| (*node < 64).then(|| acc | (1 << node)))
}

fn online_nodes(node_root: &Path) -> Option<Vec<u32>> {
    let text = std::fs::read_to_string(node_root.join("online")).ok()?;
    crate::topology::parse_cpu_list(text.trim())
        .ok()
        .map(crate::topology::sorted_unique)
}

fn pages(proc_root: &Path, task: u32) -> Option<Vec<(u32, u64)>> {
    let text = std::fs::read_to_string(proc_root.join(task.to_string()).join("numa_maps")).ok()?;
    Some(parse_numa_maps(&text).pages_by_node.into_iter().collect())
}

impl NumaMigrationAction {
    pub fn new(task: u32, start_ticks: u64, from_nodes: Vec<u32>, to_nodes: Vec<u32>) -> Self {
        Self {
            proc_root: PathBuf::from("/proc"),
            node_root: PathBuf::from("/sys/devices/system/node"),
            task,
            start_ticks,
            from_nodes,
            to_nodes,
        }
    }

    fn denied(requirement: &'static str) -> anyhow::Error {
        ActionBoundaryError::PolicyDenied {
            action_kind: "numa_migrate_pages",
            requirement,
        }
        .into()
    }

    /// Everything checked before the call; nothing is written.
    fn validate(&self, policy: &NumaMigrationPolicy) -> anyhow::Result<(u64, u64)> {
        if !policy.allow {
            return Err(Self::denied("allow_numa_migration"));
        }
        if !policy.acknowledge_irreversible {
            return Err(Self::denied("acknowledge_irreversible"));
        }
        let live = crate::telemetry::proc_view::live_process(&self.proc_root, self.task)
            .ok_or_else(|| Self::denied("target_process_alive"))?;
        if live.start_ticks != self.start_ticks {
            return Err(Self::denied("target_identity"));
        }
        let online = online_nodes(&self.node_root).ok_or_else(|| Self::denied("online_nodes"))?;
        let from = mask(&self.from_nodes).ok_or_else(|| Self::denied("node_range"))?;
        let to = mask(&self.to_nodes).ok_or_else(|| Self::denied("node_range"))?;
        let valid = |nodes: &[u32]| !nodes.is_empty() && nodes.iter().all(|n| online.contains(n));
        if !valid(&self.from_nodes) || !valid(&self.to_nodes) {
            return Err(Self::denied("nodes_online"));
        }
        if from & to != 0 {
            return Err(Self::denied("disjoint_node_sets"));
        }
        Ok((from, to))
    }

    pub fn preflight_with_policy(
        &self,
        policy: &NumaMigrationPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.validate(policy)?;
        Ok(vec![ActionWarning {
            message: "page migration is not exactly reversible: pages reclaimed, pinned or faulted in later are not returned; recovery is manual".to_owned(),
        }])
    }

    pub fn apply_with_policy(
        &self,
        policy: &NumaMigrationPolicy,
        migrator: &dyn PageMigrator,
    ) -> anyhow::Result<RollbackToken> {
        let (from, to) = self.validate(policy)?;
        let before =
            pages(&self.proc_root, self.task).ok_or_else(|| Self::denied("numa_maps_readable"))?;
        let not_moved = migrator.migrate(self.task, from, to)?;
        let after = pages(&self.proc_root, self.task).unwrap_or_default();
        Ok(RollbackToken::NumaMigration {
            records: vec![NumaMigrationRecord {
                task: self.task,
                start_ticks: self.start_ticks,
                from_nodes: self.from_nodes.clone(),
                to_nodes: self.to_nodes.clone(),
                before,
                after,
                not_moved,
            }],
        })
    }
}

impl TuningAction for NumaMigrationAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!(
            "numa:migrate:task={}:{:?}->{:?}",
            self.task, self.from_nodes, self.to_nodes
        ))
    }

    fn describe(&self) -> String {
        format!(
            "migrate one process's pages from NUMA nodes {:?} to {:?} (irreversible in effect)",
            self.from_nodes, self.to_nodes
        )
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::HighRisk
    }

    fn descriptor(&self) -> crate::daemon_policy::ActionDescriptor {
        crate::daemon_policy::ActionDescriptor {
            action_kind: "numa_migrate_pages".to_owned(),
            action_id: self.id(),
            safety_class: self.safety_class(),
            effect_scope: crate::daemon_policy::ActionEffectScope::SystemWide,
            rollback: crate::daemon_policy::RollbackRequirement::RequiredBeforeApply,
            persistent_effect: true,
            touches_system_wide_state: true,
            requires_explicit_target: true,
            confidence: None,
        }
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_with_policy(&NumaMigrationPolicy::default())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.validate(&NumaMigrationPolicy::default())
            .map(|_| ActionState {
                applied: false,
                affected_tasks: 1,
                checked_tasks: 1,
                pending_changes: 1,
                warnings: Vec::new(),
            })
    }

    fn apply(&self) -> crate::actions::ApplyResult {
        self.apply_with_policy(&NumaMigrationPolicy::default(), &RealMigrator)
            .map_err(Into::into)
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        let after = pages(&self.proc_root, self.task).unwrap_or_default();
        let on_from: u64 = after
            .iter()
            .filter(|(node, _)| self.from_nodes.contains(node))
            .map(|(_, count)| *count)
            .sum();
        Ok(ActionState {
            applied: on_from == 0,
            affected_tasks: 1,
            checked_tasks: 1,
            pending_changes: usize::from(on_from > 0),
            warnings: Vec::new(),
        })
    }

    fn rollback(&self, _token: &RollbackToken) -> anyhow::Result<()> {
        Err(ActionBoundaryError::restore_failed(
            "numa_migrate_pages",
            "page migration cannot be exactly undone; the recorded before and after counts show what moved".to_owned(),
        )
        .into())
    }
}

#[cfg(test)]
#[path = "numa_migration_tests.rs"]
mod tests;
