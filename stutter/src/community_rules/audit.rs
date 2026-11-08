//! Conflict audit of a community rule set.
//!
//! A rule is a *role hint*: a process name mapped to a task class with a confidence, never an
//! operation. When one name is mapped to two different classes the hint is contradictory, and the
//! audit reports it exactly so an operator can fix the source. It reads rules only; it changes none,
//! and a conflicting name is never resolved by picking a winner.
use std::collections::{BTreeMap, BTreeSet};

use super::{CommunityRulesFile, normalize_process_name};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClassConflict {
    pub(crate) name: String,
    /// The distinct classes the name is mapped to, in sorted order.
    pub(crate) classes: Vec<String>,
    /// The source paths that contributed the conflicting mappings, sorted and deduplicated.
    pub(crate) sources: Vec<String>,
}

/// Names mapped to more than one class, sorted by name.
pub(crate) fn class_conflicts(file: &CommunityRulesFile) -> Vec<ClassConflict> {
    let mut by_name: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    for rule in &file.rules {
        let name = if rule.normalized_name.trim().is_empty() {
            normalize_process_name(&rule.name).unwrap_or_else(|| rule.name.clone())
        } else {
            rule.normalized_name.clone()
        };
        let entry = by_name.entry(name).or_default();
        entry.0.insert(rule.stutter_class.clone());
        entry.1.insert(rule.source_path.clone());
    }
    by_name
        .into_iter()
        .filter(|(_, (classes, _))| classes.len() > 1)
        .map(|(name, (classes, sources))| ClassConflict {
            name,
            classes: classes.into_iter().collect(),
            sources: sources.into_iter().collect(),
        })
        .collect()
}
