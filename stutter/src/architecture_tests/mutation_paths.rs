//! Architecture guard for privileged mutation boundaries.

use std::{fs, path::Path};

use super::{
    crate_src_root, relative_to_crate_root,
    scanners::{production_code_lines_outside_cfg_test_modules, rust_files_under},
};

#[derive(Clone, Copy, Debug)]
struct MutationPattern {
    needle: &'static str,
    boundary: &'static str,
    allowed_paths: &'static [&'static str],
}

const MUTATION_PATTERNS: &[MutationPattern] = &[
    MutationPattern {
        needle: "libc::setpriority",
        boundary: "nice mutations must use action syscall wrappers or emergency restore",
        allowed_paths: &[
            "src/actions/syscalls.rs",
            "src/autotune/emergency_restore/executors.rs",
        ],
    },
    MutationPattern {
        needle: "libc::sched_setaffinity",
        boundary: "CPU affinity mutation must stay in the affinity syscall boundary",
        allowed_paths: &["src/affinity/syscall.rs"],
    },
    MutationPattern {
        needle: "libc::SYS_ioprio_set",
        boundary: "I/O priority mutations must use action syscall wrappers or emergency restore",
        allowed_paths: &[
            "src/actions/syscalls.rs",
            "src/autotune/emergency_restore/executors.rs",
            "src/autotune/emergency_restore/manual_command.rs",
        ],
    },
    MutationPattern {
        needle: "libc::SYS_sched_setattr",
        boundary: "uclamp mutations must use action syscall wrappers or emergency restore",
        allowed_paths: &[
            "src/actions/syscalls.rs",
            "src/autotune/emergency_restore/executors.rs",
        ],
    },
    MutationPattern {
        needle: "\"cgroup.procs\"",
        boundary: "cgroup task movement must stay in cgroup action/process/emergency modules",
        allowed_paths: &[
            "src/actions/cgroup/",
            "src/actions/rollback.rs",
            "src/autotune/emergency_restore/",
            "src/autotune/providers/mod.rs",
            "src/process/cgroup.rs",
        ],
    },
    MutationPattern {
        needle: "\"cpuset.cpus\"",
        boundary: "cpuset mutation must stay in cgroup action/emergency modules",
        allowed_paths: &[
            "src/actions/cgroup/",
            "src/autotune/emergency_restore/",
            "src/autotune/providers/cgroup.rs",
        ],
    },
    MutationPattern {
        needle: "\"/proc/sys/vm/",
        boundary: "VM knob mutation must stay in the VM knob action boundary",
        allowed_paths: &["src/actions/vm_knobs.rs"],
    },
];

fn is_test_or_fixture_path(path: &str) -> bool {
    path == "src/architecture_tests.rs"
        || path == "src/artifact_contract_tests.rs"
        || path == "src/recording_fixture_tests.rs"
        || path == "src/test_fixture_builder.rs"
        || path == "src/test_support.rs"
        || path.contains("/architecture_tests/")
        || path.contains("/planner_tests/")
        || path.contains("/regression_tests/")
        || path.contains("/test_fixture_builder/")
        || path.contains("/tests/")
        || path.ends_with("/tests.rs")
        || path.ends_with("_tests.rs")
}

fn path_is_allowed(path: &str, allowed_paths: &[&str]) -> bool {
    allowed_paths.iter().any(|allowed| {
        if allowed.ends_with('/') {
            path.starts_with(allowed)
        } else {
            path == *allowed
        }
    })
}

fn matching_lines(path: &Path, needle: &str) -> Vec<usize> {
    let Ok(source) = fs::read_to_string(path) else {
        return Vec::new();
    };

    production_code_lines_outside_cfg_test_modules(&source)
        .into_iter()
        .filter_map(|(line_number, line)| line.contains(needle).then_some(line_number))
        .collect()
}

#[test]
fn privileged_mutation_paths_stay_inside_policy_boundaries() {
    let mut violations = Vec::new();

    for file in rust_files_under(&crate_src_root()) {
        let relative_path = relative_to_crate_root(&file);
        if is_test_or_fixture_path(&relative_path) {
            continue;
        }

        for pattern in MUTATION_PATTERNS {
            if path_is_allowed(&relative_path, pattern.allowed_paths) {
                continue;
            }

            for line_number in matching_lines(&file, pattern.needle) {
                violations.push(format!(
                    "{}:{} contains '{}' outside boundary '{}'",
                    relative_path, line_number, pattern.needle, pattern.boundary
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "privileged mutation boundary violations:\n{}",
        violations.join("\n")
    );
}

#[cfg(test)]
mod tests {
    use super::{is_test_or_fixture_path, path_is_allowed};

    #[test]
    fn allowed_path_prefixes_match_whole_module_trees() {
        assert!(path_is_allowed(
            "src/actions/cgroup/mod.rs",
            &["src/actions/cgroup/"]
        ));
        assert!(!path_is_allowed(
            "src/actions/cgroup_extra.rs",
            &["src/actions/cgroup/"]
        ));
    }

    #[test]
    fn test_fixture_paths_are_exempt_from_mutation_scan() {
        assert!(is_test_or_fixture_path("src/actions/cgroup/tests.rs"));
        assert!(is_test_or_fixture_path("src/test_support.rs"));
        assert!(!is_test_or_fixture_path("src/actions/cgroup/mod.rs"));
    }
}
