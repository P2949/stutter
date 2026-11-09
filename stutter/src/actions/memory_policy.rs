//! Process-scoped memory policy: what can be observed and what is refused (package W11c).
//!
//! `set_mempolicy`, `mbind` and `set_mempolicy_home_node` act on the *calling* thread or on ranges
//! of the *calling* process's own address space. There is no call that sets another process's
//! policy from outside, so Stutter has no operation that could, and this module states that as a
//! typed refusal with the reason rather than leaving it as a missing feature. What can be done to
//! a foreign process's memory is page migration, a different contract (`NumaMigrationAction`,
//! irreversible, acknowledged, manual) that moves pages without changing the policy.
//!
//! The policy a process currently has is observable read-only from `/proc/<pid>/numa_maps`.
use std::{collections::BTreeSet, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryPolicyCall {
    SetMempolicy,
    Mbind,
    SetMempolicyHomeNode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryPolicyRefusal {
    /// The call only affects its own caller; there is no cross-process form.
    ForeignProcessCannotBeTargeted { call: MemoryPolicyCall },
}

impl MemoryPolicyRefusal {
    pub fn reason(&self) -> String {
        match self {
            Self::ForeignProcessCannotBeTargeted { call } => format!(
                "{call:?} applies to the calling thread or the caller's own address space, so it \
                 cannot set another process's memory policy; migrating that process's pages is the \
                 separate, irreversible NumaMigration contract and does not change its policy"
            ),
        }
    }
}

/// Always refuses: there is no foreign-process form of any memory-policy call.
pub fn refuse_foreign_policy(call: MemoryPolicyCall) -> MemoryPolicyRefusal {
    MemoryPolicyRefusal::ForeignProcessCannotBeTargeted { call }
}

/// The policies one process's mappings use, summarized from `numa_maps`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObservedPolicy {
    pub mappings: usize,
    pub default_mappings: usize,
    pub bind_mappings: usize,
    pub interleave_mappings: usize,
    pub prefer_mappings: usize,
    /// Every node named by a bind, interleave or prefer policy.
    pub named_nodes: BTreeSet<u32>,
}

/// Summarizes the policy column (the token after the address) of `numa_maps` text.
pub fn parse_policy(numa_maps: &str) -> ObservedPolicy {
    let mut observed = ObservedPolicy::default();
    for line in numa_maps.lines() {
        let mut tokens = line.split_whitespace();
        let (Some(address), Some(policy)) = (tokens.next(), tokens.next()) else {
            continue;
        };
        if !address.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        observed.mappings += 1;
        let (kind, nodes) = policy.split_once(':').unwrap_or((policy, ""));
        match kind {
            "default" | "local" => observed.default_mappings += 1,
            "bind" => observed.bind_mappings += 1,
            "interleave" => observed.interleave_mappings += 1,
            "prefer" | "prefer (many)" => observed.prefer_mappings += 1,
            _ => {
                observed.mappings -= 1;
                continue;
            }
        }
        if let Ok(list) = crate::topology::parse_cpu_list(nodes) {
            observed.named_nodes.extend(list);
        }
    }
    observed
}

/// Reads one process's policy summary; `None` when its `numa_maps` is unreadable.
pub fn observe_policy(proc_root: &Path, task: u32) -> Option<ObservedPolicy> {
    let text = std::fs::read_to_string(proc_root.join(task.to_string()).join("numa_maps")).ok()?;
    Some(parse_policy(&text))
}

#[cfg(test)]
#[path = "memory_policy_tests.rs"]
mod tests;
