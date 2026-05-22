//! Architecture dependency matrix coverage tests; this module owns dependency policy tables.

use std::path::PathBuf;

use super::{
    autotune_src_root, crate_src_root,
    scanners::{ForbiddenRustPath, assert_sources_do_not_reference_paths, rust_files_under},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DependencyMatrixEntry {
    subsystem: &'static str,
    may_depend_on: &'static [&'static str],
    must_not_depend_on: &'static [&'static str],
}

const KNOWN_TOP_LEVEL_ARCHITECTURE_MODULES: &[&str] = &[
    "actions",
    "agent",
    "autotune",
    "cli",
    "commands",
    "config",
    "daemon",
    "events",
    "focus",
    "process_tree",
    "recorder",
    "report",
    "system",
];

const ARCHITECTURE_DEPENDENCY_MATRIX: &[DependencyMatrixEntry] = &[
    DependencyMatrixEntry {
        subsystem: "cli",
        may_depend_on: &[
            "commands",
            "commands::input",
            "config",
            "daemon",
            "service",
            "validate",
        ],
        must_not_depend_on: &[
            "actions::runner",
            "autotune::live_experiment",
            "daemon::runtime",
            "recorder::live",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "commands",
        may_depend_on: &[
            "actions",
            "agent",
            "artifacts",
            "autotune",
            "config",
            "daemon",
            "doctor",
            "events",
            "presets",
            "probe_activation",
            "probe_registry",
            "process_tree",
            "recorder",
            "release",
            "remote",
            "report",
            "scenario",
            "service",
            "session",
            "session_io",
            "system",
            "validate",
        ],
        must_not_depend_on: &[
            "actions::runner without daemon policy",
            "autotune provider mutation",
            "undocumented persistence paths",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "agent",
        may_depend_on: &[
            "artifacts",
            "autotune",
            "config",
            "daemon",
            "recorder",
            "remote",
            "report",
            "service",
            "session",
            "session_io",
        ],
        must_not_depend_on: &[
            "actions::runner",
            "cli",
            "commands",
            "direct privileged mutation from remote requests",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "daemon",
        may_depend_on: &[
            "actions",
            "autotune",
            "config",
            "daemon::capabilities",
            "daemon::health",
            "daemon::lifecycle",
            "daemon::policy",
            "daemon::privilege",
            "daemon::state",
            "daemon::store",
            "daemon::watchdog",
            "process_tree",
            "recorder",
            "session",
            "system",
        ],
        must_not_depend_on: &[
            "cli",
            "commands",
            "clap",
            "direct action mutation without DaemonPolicy",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "autotune",
        may_depend_on: &[
            "actions",
            "autotune::objective",
            "autotune::observation",
            "autotune::planning::candidate",
            "autotune::planning::dry_run",
            "autotune::planning::plan_io",
            "autotune::planning::profile_candidates",
            "autotune::planning::suggestion",
            "autotune::planner",
            "autotune::providers",
            "config",
            "daemon::policy",
            "focus",
            "process_tree",
            "recorder",
            "report",
            "system",
        ],
        must_not_depend_on: &[
            "cli",
            "commands",
            "direct sysfs mutation",
            "provider mutation",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "actions",
        may_depend_on: &[
            "affinity",
            "audit",
            "daemon::policy",
            "hwmon",
            "irq_inspect",
            "process_tree",
            "procfs",
            "profile_restore",
            "system",
            "system_inventory",
            "task_class",
            "tasks",
            "topology",
        ],
        must_not_depend_on: &[
            "agent",
            "autotune::planner",
            "cli",
            "commands",
            "daemon::runtime",
            "recorder::live",
            "report",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "report",
        may_depend_on: &[
            "autotune::report_overlay",
            "diagnosis",
            "metrics",
            "recorder::event_types",
            "runtime_slices",
            "session_io",
            "spike",
            "summary",
        ],
        must_not_depend_on: &[
            "actions::runner",
            "agent",
            "autotune::providers",
            "daemon::runtime",
            "recorder::live",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "focus",
        may_depend_on: &[
            "community_rules",
            "config",
            "foreground",
            "metrics",
            "process_tree",
            "task_class",
        ],
        must_not_depend_on: &["actions", "agent", "daemon"],
    },
    DependencyMatrixEntry {
        subsystem: "events",
        may_depend_on: &[
            "metrics",
            "recorder::event_types",
            "runtime_slices",
            "stutter_common",
        ],
        must_not_depend_on: &[
            "actions",
            "agent",
            "autotune",
            "commands",
            "daemon",
            "recorder::live",
            "report",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "recorder",
        may_depend_on: &[
            "config",
            "events",
            "foreground",
            "metrics",
            "runtime_slices",
            "session_io",
        ],
        must_not_depend_on: &[
            "actions::runner",
            "agent",
            "autotune::providers",
            "daemon::policy",
            "report",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "config",
        may_depend_on: &[
            "config::effective",
            "config::layer",
            "config::merge",
            "config::model",
            "config::schema",
            "config::source",
            "config::types",
        ],
        must_not_depend_on: &[
            "actions", "agent", "autotune", "cli", "commands", "daemon", "recorder", "report",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "process_tree",
        may_depend_on: &[
            "community_rules",
            "config",
            "procfs",
            "task_class",
            "task_filter",
            "tasks",
        ],
        must_not_depend_on: &[
            "actions::runner",
            "agent",
            "autotune::controller",
            "cli",
            "commands",
            "daemon::runtime",
            "recorder::live",
            "report",
        ],
    },
    DependencyMatrixEntry {
        subsystem: "system",
        may_depend_on: &[
            "config",
            "foreground",
            "hwmon",
            "irq_inspect",
            "kernel_event",
            "mangohud",
            "perf_counters",
            "process_tree",
            "psi",
            "sched_state",
            "scx",
            "stutter_common",
            "system_inventory",
            "topology",
        ],
        must_not_depend_on: &[
            "actions::runner",
            "agent",
            "autotune::providers",
            "cli",
            "commands",
            "daemon::runtime",
            "recorder::live",
            "report",
        ],
    },
];

fn dependency_matrix_entry(subsystem: &str) -> &'static DependencyMatrixEntry {
    ARCHITECTURE_DEPENDENCY_MATRIX
        .iter()
        .find(|entry| entry.subsystem == subsystem)
        .unwrap_or_else(|| panic!("missing architecture dependency matrix entry for {subsystem}"))
}

#[test]
fn dependency_matrix_covers_known_top_level_modules() {
    let mut matrix_subsystems = ARCHITECTURE_DEPENDENCY_MATRIX
        .iter()
        .map(|entry| entry.subsystem)
        .collect::<Vec<_>>();
    matrix_subsystems.sort_unstable();

    let mut unique_subsystems = matrix_subsystems.clone();
    unique_subsystems.dedup();
    assert_eq!(
        matrix_subsystems, unique_subsystems,
        "architecture dependency matrix contains duplicate subsystem entries"
    );

    let mut expected_subsystems = KNOWN_TOP_LEVEL_ARCHITECTURE_MODULES.to_vec();
    expected_subsystems.sort_unstable();

    assert_eq!(
        matrix_subsystems, expected_subsystems,
        "architecture dependency matrix must cover exactly the known top-level modules"
    );

    assert!(
        dependency_matrix_entry("cli")
            .may_depend_on
            .contains(&"commands"),
        "cli must be allowed to depend on commands"
    );

    let commands = dependency_matrix_entry("commands");
    assert!(
        commands.may_depend_on.contains(&"service"),
        "commands must be allowed to depend on service modules"
    );
    assert!(
        commands.may_depend_on.contains(&"daemon"),
        "commands must be allowed to dispatch to daemon application modules"
    );
    assert!(
        commands.may_depend_on.contains(&"actions"),
        "commands must be allowed to dispatch to action-backed application modules"
    );

    let agent = dependency_matrix_entry("agent");
    for dependency in ["daemon", "autotune", "recorder", "config", "remote"] {
        assert!(
            agent.may_depend_on.contains(&dependency),
            "agent must be allowed to depend on {dependency}"
        );
    }

    let daemon = dependency_matrix_entry("daemon");
    for dependency in ["daemon::policy", "daemon::state", "actions", "autotune"] {
        assert!(
            daemon.may_depend_on.contains(&dependency),
            "daemon must encode allowed dependency on {dependency}"
        );
    }

    let autotune = dependency_matrix_entry("autotune");
    for dependency in [
        "autotune::observation",
        "autotune::planning::candidate",
        "autotune::planning::dry_run",
        "autotune::planning::plan_io",
        "autotune::planning::profile_candidates",
        "autotune::planning::suggestion",
        "autotune::providers",
        "autotune::objective",
    ] {
        assert!(
            autotune.may_depend_on.contains(&dependency),
            "autotune planning must encode allowed dependency on {dependency}"
        );
    }

    let actions = dependency_matrix_entry("actions");
    for dependency in ["affinity", "process_tree", "system"] {
        assert!(
            actions.may_depend_on.contains(&dependency),
            "actions must encode allowed low-level system dependency on {dependency}"
        );
    }

    let report = dependency_matrix_entry("report");
    for dependency in [
        "session_io",
        "summary",
        "diagnosis",
        "recorder::event_types",
    ] {
        assert!(
            report.may_depend_on.contains(&dependency),
            "report must encode allowed dependency on {dependency}"
        );
    }

    let focus = dependency_matrix_entry("focus");
    for dependency in ["process_tree", "config", "foreground", "community_rules"] {
        assert!(
            focus.may_depend_on.contains(&dependency),
            "focus must encode allowed dependency on {dependency}"
        );
    }
    for forbidden_dependency in ["actions", "daemon", "agent"] {
        assert!(
            focus.must_not_depend_on.contains(&forbidden_dependency),
            "focus must encode forbidden dependency on {forbidden_dependency}"
        );
    }
}

fn autotune_non_mutation_forbidden_paths(boundary: &'static str) -> [ForbiddenRustPath; 9] {
    [
        ForbiddenRustPath {
            path: "crate::actions::runner",
            boundary,
        },
        ForbiddenRustPath {
            path: "actions::runner",
            boundary,
        },
        ForbiddenRustPath {
            path: "TuningAction",
            boundary,
        },
        ForbiddenRustPath {
            path: "ActionRunPolicy",
            boundary,
        },
        ForbiddenRustPath {
            path: "AuditedActionResult",
            boundary,
        },
        ForbiddenRustPath {
            path: "ActionHooks",
            boundary,
        },
        ForbiddenRustPath {
            path: "run_audited_action",
            boundary,
        },
        ForbiddenRustPath {
            path: "run_audited_action_with_audit_path",
            boundary,
        },
        ForbiddenRustPath {
            path: "run_audited_action_with_hooks",
            boundary,
        },
    ]
}

fn autotune_observation_quality_and_selection_files() -> Vec<PathBuf> {
    let root = autotune_src_root();
    [
        "observation.rs",
        "observation_builder.rs",
        "rolling_window.rs",
        "quality.rs",
        "system_context.rs",
        "target_selection.rs",
    ]
    .into_iter()
    .map(|file| root.join(file))
    .collect()
}

#[test]
fn focus_does_not_depend_on_control_or_mutation_layers() {
    let files = rust_files_under(&crate_src_root().join("focus"));

    assert_sources_do_not_reference_paths(
        &files,
        &[
            ForbiddenRustPath {
                path: "crate::agent",
                boundary: "focus must not depend on the agent control layer",
            },
            ForbiddenRustPath {
                path: "crate::daemon",
                boundary: "focus must not depend on the daemon control layer",
            },
            ForbiddenRustPath {
                path: "crate::actions",
                boundary: "focus must not depend on action mutation layers",
            },
            ForbiddenRustPath {
                path: "crate::commands",
                boundary: "focus must not depend on command dispatch",
            },
            ForbiddenRustPath {
                path: "crate::cli",
                boundary: "focus must not depend on CLI parsing",
            },
        ],
    );
}

#[test]
fn report_does_not_depend_on_live_runtime_or_control_layers() {
    let files = rust_files_under(&crate_src_root().join("report"));

    assert_sources_do_not_reference_paths(
        &files,
        &[
            ForbiddenRustPath {
                path: "crate::agent",
                boundary: "report must not depend on the agent control layer",
            },
            ForbiddenRustPath {
                path: "crate::daemon::runtime",
                boundary: "report must not depend on daemon live runtime",
            },
            ForbiddenRustPath {
                path: "crate::autotune::runtime",
                boundary: "report must not depend on autotune live runtime",
            },
            ForbiddenRustPath {
                path: "crate::actions::runner",
                boundary: "report must not depend on action runner mutation paths",
            },
            ForbiddenRustPath {
                path: "crate::cli",
                boundary: "report must not depend on CLI parsing",
            },
            ForbiddenRustPath {
                path: "crate::commands",
                boundary: "report must not depend on command dispatch",
            },
        ],
    );
}

#[test]
fn autotune_planner_does_not_import_action_execution() {
    let files = vec![autotune_src_root().join("planner.rs")];
    let forbidden = autotune_non_mutation_forbidden_paths(
        "autotune planner must not import action execution APIs",
    );

    assert_sources_do_not_reference_paths(&files, &forbidden);
}

#[test]
fn autotune_observation_quality_and_selection_modules_do_not_import_action_execution() {
    let files = autotune_observation_quality_and_selection_files();
    let forbidden = autotune_non_mutation_forbidden_paths(
        "autotune observation, quality, rolling-window, system-context, and target-selection modules must not import action execution APIs",
    );

    assert_sources_do_not_reference_paths(&files, &forbidden);
}

#[test]
fn autotune_providers_do_not_import_action_execution() {
    let files = rust_files_under(&autotune_src_root().join("providers"));
    let forbidden = autotune_non_mutation_forbidden_paths(
        "autotune providers must not import action execution APIs",
    );

    assert_sources_do_not_reference_paths(&files, &forbidden);
}

#[test]
fn actions_do_not_depend_on_cli_or_command_parsing() {
    let root = crate_src_root().join("actions");
    let files = rust_files_under(&root);

    assert_sources_do_not_reference_paths(
        &files,
        &[
            ForbiddenRustPath {
                path: "crate::cli",
                boundary: "actions must not depend on CLI parsing",
            },
            ForbiddenRustPath {
                path: "crate::commands",
                boundary: "actions must not depend on command parsing",
            },
            ForbiddenRustPath {
                path: "AppCommand",
                boundary: "actions must not depend on command DTOs",
            },
            ForbiddenRustPath {
                path: "clap",
                boundary: "actions must not depend on Clap parsing",
            },
        ],
    );
}

#[test]
fn daemon_internals_do_not_depend_on_cli_or_command_parsing() {
    let root = crate_src_root().join("daemon");
    let files = rust_files_under(&root);

    assert_sources_do_not_reference_paths(
        &files,
        &[
            ForbiddenRustPath {
                path: "crate::cli",
                boundary: "daemon internals must not depend on CLI parsing",
            },
            ForbiddenRustPath {
                path: "crate::commands",
                boundary: "daemon internals must not depend on command parsing",
            },
            ForbiddenRustPath {
                path: "AppCommand",
                boundary: "daemon internals must not depend on command DTOs",
            },
            ForbiddenRustPath {
                path: "clap",
                boundary: "daemon internals must not depend on Clap parsing",
            },
        ],
    );
}

#[test]
fn event_decode_module_does_not_depend_on_recording() {
    let files = vec![crate_src_root().join("events/decode.rs")];

    assert_sources_do_not_reference_paths(
        &files,
        &[
            ForbiddenRustPath {
                path: "crate::recorder",
                boundary: "events/decode.rs must not depend on recording",
            },
            ForbiddenRustPath {
                path: "recorder",
                boundary: "events/decode.rs must not depend on recording",
            },
            ForbiddenRustPath {
                path: "LiveRecorder",
                boundary: "events/decode.rs must not depend on live recording",
            },
        ],
    );
}

#[test]
fn policy_module_does_not_mutate_persistent_daemon_state() {
    let files = vec![crate_src_root().join("daemon/policy.rs")];

    assert_sources_do_not_reference_paths(
        &files,
        &[
            ForbiddenRustPath {
                path: "DaemonStateStore",
                boundary: "daemon policy must not mutate persistent daemon state",
            },
            ForbiddenRustPath {
                path: "DaemonStateSnapshotWriter",
                boundary: "daemon policy must not mutate persistent daemon state",
            },
            ForbiddenRustPath {
                path: "load_daemon_state",
                boundary: "daemon policy must not load persistent daemon state",
            },
            ForbiddenRustPath {
                path: "default_daemon_state_snapshot_path",
                boundary: "daemon policy must not know persistent daemon state paths",
            },
        ],
    );
}
