use std::{collections::BTreeSet, collections::BTreeMap, path::Path};
use crate::{
    actions::uclamp::UclampValues,
    affinity::CpuMask,
    autotune::observation::CpuPolicyRuntimeState,
};

pub(super) fn cpu_mask_strings_match(current: &str, requested: &str) -> bool {
    match (CpuMask::parse(current), CpuMask::parse(requested)) {
        (Ok(current), Ok(requested)) => current == requested,
        _ => current.trim() == requested.trim(),
    }
}

pub(super) fn uclamp_matches_request(current: UclampValues, requested: UclampValues) -> bool {
    requested
        .sched_util_min
        .is_none_or(|value| current.sched_util_min == Some(value))
        && requested
            .sched_util_max
            .is_none_or(|value| current.sched_util_max == Some(value))
}

pub(super) fn normalize_cgroup_path(path: &Path) -> String {
    normalize_cgroup_str(&path.to_string_lossy())
}

pub(super) fn normalize_cgroup_str(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        "/".to_owned()
    } else if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    }
}

pub(super) fn cpu_policy_for_cpu(
    policies: &[CpuPolicyRuntimeState],
    cpu: u32,
) -> Option<&CpuPolicyRuntimeState> {
    policies.iter().find(|policy| {
        policy
            .related_cpus
            .as_deref()
            .is_some_and(|related| cpu_list_contains(related, cpu))
            || policy.policy == format!("policy{cpu}")
    })
}

fn cpu_list_contains(list: &str, cpu: u32) -> bool {
    list.split(',').any(|part| {
        let part = part.trim();
        if let Some((start, end)) = part.split_once('-') {
            let Ok(start) = start.trim().parse::<u32>() else {
                return false;
            };
            let Ok(end) = end.trim().parse::<u32>() else {
                return false;
            };
            (start..=end).contains(&cpu)
        } else {
            part.parse::<u32>().is_ok_and(|value| value == cpu)
        }
    })
}

pub(super) fn vm_knob_keys_for_change(root: &Path, path: &Path) -> Vec<String> {
    let mut keys = BTreeSet::new();
    keys.insert(path.to_string_lossy().trim_start_matches('/').to_owned());

    if let Ok(relative) = path.strip_prefix(root) {
        keys.insert(
            relative
                .to_string_lossy()
                .trim_start_matches('/')
                .to_owned(),
        );
    }

    if let Ok(relative) = path.strip_prefix("/proc") {
        keys.insert(
            relative
                .to_string_lossy()
                .trim_start_matches('/')
                .to_owned(),
        );
    }

    keys.into_iter().collect()
}

pub(super) fn vm_knob_active_value<'a>(
    knobs: &'a BTreeMap<String, String>,
    keys: &[String],
) -> Option<&'a String> {
    keys.iter().find_map(|key| knobs.get(key))
}
