//! Profile warning models and warning detectors.
//!
//! Owns offline-CPU and first-match rule-overlap warning logic. Does not own profile parsing,
//! action planning/application, or TOML rendering.

use anyhow::Context;

use super::{Profile, ProfileRule};
use crate::{
    affinity::CpuMask,
    process_tree::{CompiledPattern, TaskClass},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileCpuWarning {
    pub rule_index: usize,
    pub requested: String,
    pub online: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRuleOverlapWarning {
    pub earlier_rule: usize,
    pub later_rule: usize,
}

fn class_dimension_may_overlap(a: &[TaskClass], b: &[TaskClass]) -> bool {
    a.is_empty() || b.is_empty() || a.iter().any(|left| b.contains(left))
}

fn comm_dimension_may_overlap(a: &[CompiledPattern], b: &[CompiledPattern]) -> bool {
    a.is_empty()
        || b.is_empty()
        || a.iter()
            .any(|left| b.iter().any(|right| left.raw() == right.raw()))
}

fn rule_may_overlap(earlier: &ProfileRule, later: &ProfileRule) -> bool {
    let class_overlap = class_dimension_may_overlap(&earlier.match_class, &later.match_class);
    let comm_overlap = comm_dimension_may_overlap(&earlier.match_comm, &later.match_comm);

    class_overlap && comm_overlap
}

pub fn profile_rule_overlap_warnings(rules: &[ProfileRule]) -> Vec<ProfileRuleOverlapWarning> {
    let mut warnings = Vec::new();

    for earlier_rule in 0..rules.len() {
        for later_rule in (earlier_rule + 1)..rules.len() {
            if rule_may_overlap(&rules[earlier_rule], &rules[later_rule]) {
                warnings.push(ProfileRuleOverlapWarning {
                    earlier_rule,
                    later_rule,
                });
            }
        }
    }

    warnings
}

pub fn profile_offline_cpu_warnings(profile: &Profile, online: &CpuMask) -> Vec<ProfileCpuWarning> {
    profile
        .rules
        .iter()
        .enumerate()
        .filter_map(|(idx, rule)| {
            let affinity = rule.affinity.as_ref()?;
            (!affinity.is_subset_of(online)).then(|| ProfileCpuWarning {
                rule_index: idx,
                requested: affinity.to_range_string(),
                online: online.to_range_string(),
            })
        })
        .collect()
}

pub(super) fn warn_profile_offline_cpus(profile: &Profile) -> anyhow::Result<()> {
    let online =
        CpuMask::online_cpus().context("failed to read online CPU mask before applying profile")?;

    for warning in profile_offline_cpu_warnings(profile, &online) {
        log::warn!(
            "profile_rule_offline_cpus rule={} requested={} online={}",
            warning.rule_index,
            warning.requested,
            warning.online
        );
    }

    Ok(())
}
