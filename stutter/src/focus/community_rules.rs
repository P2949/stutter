//! Community rules classification bridge.
//!
//! Maps generic community classification results to internal process classes.

use super::classify::ProcessIdentity;
use crate::process_tree::TaskClass as SystemTaskClass;

#[cfg(test)]
pub(super) fn try_community_rules_classification(
    reasons: &mut Vec<String>,
    identity: &ProcessIdentity<'_>,
    cgroup_path: &str,
) -> Option<(SystemTaskClass, f32)> {
    if let Some(hit) = crate::community_rules::classify_process_identity(
        &crate::community_rules::CommunityProcessIdentity {
            thread_comm: identity.comm,
            process_comm: identity.comm,
            cmdline: identity.cmdline,
            exe_path: identity.exe_path.unwrap_or_default(),
            cgroup_path,
        },
    ) && let Some(class) = system_class_for_community_task_class(hit.class)
    {
        reasons.push(hit.reason);
        return Some((class, hit.confidence));
    }
    None
}

#[cfg(not(test))]
pub(super) fn try_community_rules_classification(
    _reasons: &mut Vec<String>,
    _identity: &ProcessIdentity<'_>,
    _cgroup_path: &str,
) -> Option<(SystemTaskClass, f32)> {
    None
}

#[cfg(test)]
fn system_class_for_community_task_class(
    class: crate::process_tree::TaskClass,
) -> Option<SystemTaskClass> {
    match class {
        crate::process_tree::TaskClass::Game => Some(SystemTaskClass::Game),
        _ => None,
    }
}
