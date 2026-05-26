use std::{fs, path::Path};

use stutter_common::BPF_MAX_TRACKED_CPUS;

pub(super) const CPU_POSSIBLE_PATH: &str = "/sys/devices/system/cpu/possible";

pub(super) fn parse_cpu_range_list_max_id(value: &str) -> Option<u32> {
    let mut max_id = None;

    for raw_part in value.trim().split(',') {
        let part = raw_part.trim();
        if part.is_empty() {
            return None;
        }

        let part_max = if let Some((start, end)) = part.split_once('-') {
            let start = start.parse::<u32>().ok()?;
            let end = end.parse::<u32>().ok()?;
            if start > end {
                return None;
            }
            end
        } else {
            part.parse::<u32>().ok()?
        };

        max_id = Some(max_id.map_or(part_max, |current: u32| current.max(part_max)));
    }

    max_id
}

pub(super) fn cpu_tracking_limit_warning(max_possible_cpu_id: u32) -> Option<String> {
    if max_possible_cpu_id < BPF_MAX_TRACKED_CPUS {
        return None;
    }

    Some(format!(
        "possible CPU id {max_possible_cpu_id} exceeds eBPF CPU accounting limit {}; \
         runnable-depth and target-pending-wakeup accounting will be skipped for CPU ids >= {} \
         and counted via DROP_CPU_ACCOUNTING_UNTRACKED",
        BPF_MAX_TRACKED_CPUS - 1,
        BPF_MAX_TRACKED_CPUS
    ))
}

pub(super) fn cpu_tracking_limit_warning_from_possible_path(path: &Path) -> Option<String> {
    let possible = fs::read_to_string(path).ok()?;
    let max_possible_cpu_id = parse_cpu_range_list_max_id(&possible)?;
    cpu_tracking_limit_warning(max_possible_cpu_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cpu_range_list_max_id_handles_single_cpu() {
        assert_eq!(parse_cpu_range_list_max_id("0"), Some(0));
        assert_eq!(parse_cpu_range_list_max_id("  0  \n"), Some(0));
    }

    #[test]
    fn parse_cpu_range_list_max_id_handles_contiguous_range() {
        assert_eq!(parse_cpu_range_list_max_id("0-3"), Some(3));
        assert_eq!(parse_cpu_range_list_max_id("0-15\n"), Some(15));
    }

    #[test]
    fn parse_cpu_range_list_max_id_handles_sparse_lists() {
        assert_eq!(parse_cpu_range_list_max_id("0,2"), Some(2));
        assert_eq!(parse_cpu_range_list_max_id("0, 2, 4"), Some(4));
        assert_eq!(parse_cpu_range_list_max_id("0-1,3-7"), Some(7));
        assert_eq!(parse_cpu_range_list_max_id("0,2-4,6"), Some(6));
        assert_eq!(parse_cpu_range_list_max_id("10-15,20,25-30"), Some(30));
    }

    #[test]
    fn parse_cpu_range_list_max_id_handles_invalid_input() {
        assert_eq!(parse_cpu_range_list_max_id(""), None);
        assert_eq!(parse_cpu_range_list_max_id("a"), None);
        assert_eq!(parse_cpu_range_list_max_id("0-a"), None);
        assert_eq!(parse_cpu_range_list_max_id("3-0"), None);
        assert_eq!(parse_cpu_range_list_max_id("0,,"), None);
    }
}
