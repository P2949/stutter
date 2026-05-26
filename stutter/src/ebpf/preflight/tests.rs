use stutter_common::BPF_MAX_TRACKED_CPUS;

use super::cpu::{cpu_tracking_limit_warning, parse_cpu_range_list_max_id};

#[test]
fn cpu_range_list_max_id_parses_single_range() {
    assert_eq!(parse_cpu_range_list_max_id("0-7"), Some(7));
}

#[test]
fn cpu_range_list_max_id_parses_sparse_ranges() {
    assert_eq!(parse_cpu_range_list_max_id("0-3,8,16-19"), Some(19));
}

#[test]
fn cpu_range_list_max_id_rejects_malformed_ranges() {
    assert_eq!(parse_cpu_range_list_max_id("7-0"), None);
    assert_eq!(parse_cpu_range_list_max_id("0-"), None);
    assert_eq!(parse_cpu_range_list_max_id(""), None);
}

#[test]
fn cpu_tracking_limit_warning_allows_highest_valid_cpu_id() {
    assert!(cpu_tracking_limit_warning(BPF_MAX_TRACKED_CPUS - 1).is_none());
}

#[test]
fn cpu_tracking_limit_warning_triggers_at_first_untracked_cpu_id() {
    let warning = cpu_tracking_limit_warning(BPF_MAX_TRACKED_CPUS)
        .expect("first untracked CPU id should warn");

    assert!(warning.contains("DROP_CPU_ACCOUNTING_UNTRACKED"));
    assert!(warning.contains(&BPF_MAX_TRACKED_CPUS.to_string()));
}
