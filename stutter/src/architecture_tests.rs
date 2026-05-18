use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DependencyMatrixEntry {
    subsystem: &'static str,
    may_depend_on: &'static [&'static str],
    must_not_depend_on: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExpectedPublicModule {
    name: &'static str,
    reason: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OversizedRustFileAllowance {
    path: &'static str,
    max_lines: usize,
    reason: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RustFileLineCount {
    path: String,
    lines: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExistingProductionUnwrapExpectAllowance {
    path: &'static str,
    reason: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProductionUnwrapExpectCall {
    path: String,
    line_number: usize,
    call: &'static str,
    line: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExistingDirectPrintAllowance {
    path: &'static str,
    line_number: usize,
    macro_name: &'static str,
    reason: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DirectPrintMacroCall {
    path: String,
    line_number: usize,
    macro_name: &'static str,
    line: String,
}

const RUST_FILE_SIZE_LIMIT_LINES: usize = 1_000;

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
            "autotune::candidate",
            "autotune::objective",
            "autotune::observation",
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

const OVERSIZED_RUST_FILE_ALLOWLIST: &[OversizedRustFileAllowance] = &[
    OversizedRustFileAllowance {
        path: "src/agent.rs",
        max_lines: 5_049,
        reason: "existing monolithic agent/control API implementation pending future split, plus module-level line additions",
    },
    OversizedRustFileAllowance {
        path: "src/focus/mod.rs",
        max_lines: 4_030,
        reason: "existing focus snapshot, scoring, resolver, and classification module pending future split, plus module-level line additions",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/candidate.rs",
        max_lines: 3_744,
        reason: "existing autotune candidate model and tests pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/planner.rs",
        max_lines: 3_419,
        reason: "existing autotune planner and planner tests pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/ebpf_loader.rs",
        max_lines: 2_842,
        reason: "existing eBPF loader implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/daemon/policy.rs",
        max_lines: 2_657,
        reason: "existing daemon policy implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/runtime.rs",
        max_lines: 2_557,
        reason: "existing autotune runtime orchestration pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/regression_tests.rs",
        max_lines: 2_514,
        reason: "existing broad regression test module pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/architecture_tests.rs",
        max_lines: 2_416,
        reason: "Proposal 31 public API facade tests intentionally extend architecture gates pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/process_tree.rs",
        max_lines: 2_381,
        reason: "existing process tree scanner/classifier implementation pending future split, plus module-level line additions",
    },
    OversizedRustFileAllowance {
        path: "src/diagnosis.rs",
        max_lines: 2_351,
        reason: "existing diagnosis engine implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/daemon/privilege.rs",
        max_lines: 2_330,
        reason: "existing privileged daemon worker implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/foreground.rs",
        max_lines: 2_302,
        reason: "existing foreground provider implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/cli/monitor.rs",
        max_lines: 2_297,
        reason: "existing monitor CLI argument surface pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/profiles.rs",
        max_lines: 2_189,
        reason: "existing profile model and profile tests pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/session.rs",
        max_lines: 2_200,
        reason: "existing session model and lifecycle implementation pending future split, plus module-level line additions",
    },
    OversizedRustFileAllowance {
        path: "src/community_rules.rs",
        max_lines: 2_132,
        reason: "existing community rule model/import implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/test_fixture_builder.rs",
        max_lines: 2_146,
        reason: "existing shared test fixture builder pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/report/analysis.rs",
        max_lines: 2_235,
        reason: "existing report analysis implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/apply_low_risk.rs",
        max_lines: 1_815,
        reason: "existing low-risk autotune apply path pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/actions/runner.rs",
        max_lines: 1_746,
        reason: "existing audited action runner implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/live_experiment.rs",
        max_lines: 1_707,
        reason: "existing live experiment manager implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/report/render/text.rs",
        max_lines: 1_685,
        reason: "existing text report renderer pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/config_file.rs",
        max_lines: 1_649,
        reason: "existing config file parser/model implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/report/mod.rs",
        max_lines: 1_545,
        reason: "existing report public module and tests pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/cli/report.rs",
        max_lines: 1_435,
        reason: "existing report CLI argument surface pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/status.rs",
        max_lines: 1_419,
        reason: "existing autotune status rendering implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/validation_corpus_tests.rs",
        max_lines: 1_396,
        reason: "existing validation corpus test module pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/startup_recovery.rs",
        max_lines: 1_350,
        reason: "existing autotune startup recovery implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/emergency_restore.rs",
        max_lines: 1_348,
        reason: "existing autotune emergency restore implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/tune/mod.rs",
        max_lines: 1_333,
        reason: "existing tune module implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/active_config.rs",
        max_lines: 1_337,
        reason: "existing active autotune config implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/controller.rs",
        max_lines: 1_294,
        reason: "existing autotune controller state implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/metrics.rs",
        max_lines: 1_257,
        reason: "existing metrics model and tests pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/rolling_window.rs",
        max_lines: 1_162,
        reason: "existing autotune rolling window implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/config/effective.rs",
        max_lines: 1_162,
        reason: "existing effective config resolution implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/recorder/session.rs",
        max_lines: 1_076,
        reason: "existing recorder session writer implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/community_rules/importer.rs",
        max_lines: 1_051,
        reason: "existing community rules importer implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/daemon/state.rs",
        max_lines: 1_041,
        reason: "existing daemon state implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/actions/cgroup.rs",
        max_lines: 1_032,
        reason: "existing cgroup action implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/cli/mod.rs",
        max_lines: 1_042,
        reason: "existing top-level CLI parser implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/doctor.rs",
        max_lines: 1_003,
        reason: "existing doctor diagnostics implementation pending future split",
    },
];

const EXISTING_PRODUCTION_UNWRAP_EXPECT_FILE_ALLOWLIST:
    &[ExistingProductionUnwrapExpectAllowance] = &[
    ExistingProductionUnwrapExpectAllowance {
        path: "src/actions/fake_action.rs",
        reason: "existing fake action test-support implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/affinity.rs",
        reason: "existing affinity implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/agent.rs",
        reason: "existing agent implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/artifact_contract_tests.rs",
        reason: "existing artifact contract test module contains unwrap/expect calls outside cfg-test module blocks",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/artifacts.rs",
        reason: "existing artifact metadata implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/architecture_tests.rs",
        reason: "architecture tests intentionally contain unwrap/expect scanner fixtures and test-only panic helpers",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/autotune/candidate.rs",
        reason: "existing autotune candidate implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/autotune/shutdown.rs",
        reason: "existing autotune shutdown implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/cli/mod.rs",
        reason: "existing CLI parser tests live in source and contain unwrap/expect calls outside cfg-test module blocks",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/cli/monitor.rs",
        reason: "existing monitor CLI tests live in source and contain unwrap/expect calls outside cfg-test module blocks",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/cli/report.rs",
        reason: "existing report CLI tests live in source and contain unwrap/expect calls outside cfg-test module blocks",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/community_rules.rs",
        reason: "existing community rules implementation/tests contain production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/daemon/acceptance.rs",
        reason: "existing daemon acceptance test-support module contains unwrap/expect calls outside cfg-test module blocks",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/daemon/policy.rs",
        reason: "existing daemon policy implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/diagnosis.rs",
        reason: "existing diagnosis implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/ebpf_loader.rs",
        reason: "existing eBPF loader implementation/tests contain production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/events/interpret.rs",
        reason: "existing event interpretation implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/focus/mod.rs",
        reason: "existing focus implementation/tests contain production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/probe_registry.rs",
        reason: "existing probe registry implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/process_tree.rs",
        reason: "existing process tree implementation/tests contain production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/recording_fixture_tests.rs",
        reason: "existing recording fixture test module contains unwrap/expect calls outside cfg-test module blocks",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/regression_tests.rs",
        reason: "existing regression test module contains unwrap/expect calls outside cfg-test module blocks",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/report/analysis.rs",
        reason: "existing report analysis implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/report/render/text.rs",
        reason: "existing text report implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/runnable_depth_tests.rs",
        reason: "existing runnable depth test module contains unwrap/expect calls outside cfg-test module blocks",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/session.rs",
        reason: "existing session implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/tune/mod.rs",
        reason: "existing tune implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/validation_corpus_tests.rs",
        reason: "existing validation corpus test module contains unwrap/expect calls outside cfg-test module blocks",
    },
];

const EXISTING_DIRECT_PRINT_ALLOWLIST: &[ExistingDirectPrintAllowance] = &[
    ExistingDirectPrintAllowance {
        path: "src/agent.rs",
        line_number: 311,
        macro_name: "println!",
        reason: "existing agent startup recovery status output pending conversion to structured status/logging",
    },
    ExistingDirectPrintAllowance {
        path: "src/agent.rs",
        line_number: 322,
        macro_name: "eprintln!",
        reason: "existing agent startup recovery warning output pending conversion to structured status/logging",
    },
    ExistingDirectPrintAllowance {
        path: "src/agent.rs",
        line_number: 326,
        macro_name: "eprintln!",
        reason: "existing agent manual restore warning output pending conversion to structured status/logging",
    },
    ExistingDirectPrintAllowance {
        path: "src/agent.rs",
        line_number: 337,
        macro_name: "println!",
        reason: "existing agent startup recovery status output pending conversion to structured status/logging",
    },
    ExistingDirectPrintAllowance {
        path: "src/agent.rs",
        line_number: 346,
        macro_name: "println!",
        reason: "existing agent startup recovery status output pending conversion to structured status/logging",
    },
    ExistingDirectPrintAllowance {
        path: "src/agent.rs",
        line_number: 359,
        macro_name: "println!",
        reason: "existing agent non-loopback bind warning output pending conversion to structured status/logging",
    },
    ExistingDirectPrintAllowance {
        path: "src/agent.rs",
        line_number: 363,
        macro_name: "println!",
        reason: "existing agent non-loopback bind warning output pending conversion to structured status/logging",
    },
    ExistingDirectPrintAllowance {
        path: "src/agent.rs",
        line_number: 434,
        macro_name: "println!",
        reason: "existing agent Unix listener startup output pending conversion to structured status/logging",
    },
    ExistingDirectPrintAllowance {
        path: "src/agent.rs",
        line_number: 437,
        macro_name: "println!",
        reason: "existing agent TCP listener startup output pending conversion to structured status/logging",
    },
    ExistingDirectPrintAllowance {
        path: "src/autotune/mod.rs",
        line_number: 216,
        macro_name: "println!",
        reason: "existing autotune profile output pending conversion to command/rendering layer output",
    },
    ExistingDirectPrintAllowance {
        path: "src/autotune/mod.rs",
        line_number: 243,
        macro_name: "println!",
        reason: "existing autotune runtime start output pending conversion to command/rendering layer output",
    },
    ExistingDirectPrintAllowance {
        path: "src/autotune/mod.rs",
        line_number: 248,
        macro_name: "println!",
        reason: "existing autotune runtime finish output pending conversion to command/rendering layer output",
    },
    ExistingDirectPrintAllowance {
        path: "src/autotune/runtime.rs",
        line_number: 839,
        macro_name: "println!",
        reason: "existing autotune runtime stream output pending conversion to explicit rendering boundary",
    },
];

const EXPECTED_ROOT_PUBLIC_MODULES: &[ExpectedPublicModule] = &[ExpectedPublicModule {
    name: "api",
    reason: "single intentional public façade replacing direct public subsystem modules",
}];

const EXPECTED_API_PUBLIC_MODULES: &[ExpectedPublicModule] = &[
    ExpectedPublicModule {
        name: "error",
        reason: "public error and warning contracts returned by stable crate entry points",
    },
    ExpectedPublicModule {
        name: "actions",
        reason: "public action descriptors, safety classes, outcomes, and rollback contracts",
    },
    ExpectedPublicModule {
        name: "agent",
        reason: "public agent embedding and remote-control entry points",
    },
    ExpectedPublicModule {
        name: "alert",
        reason: "public alert payload and sender contracts",
    },
    ExpectedPublicModule {
        name: "artifacts",
        reason: "public artifact kind, selection, path, and stream metadata contracts",
    },
    ExpectedPublicModule {
        name: "autotune",
        reason: "public autotune command, planning, status, and data contracts",
    },
    ExpectedPublicModule {
        name: "config",
        reason: "public configuration model, source, and merge contracts",
    },
    ExpectedPublicModule {
        name: "daemon",
        reason: "public daemon policy, state, health, lifecycle, and runtime contracts",
    },
    ExpectedPublicModule {
        name: "daemon_policy",
        reason: "compatibility facade for daemon policy and explanation contracts",
    },
    ExpectedPublicModule {
        name: "events",
        reason: "public event decoding and interpretation contracts",
    },
    ExpectedPublicModule {
        name: "focus",
        reason: "public focus snapshot, classification, scoring, and resolution contracts",
    },
    ExpectedPublicModule {
        name: "presets",
        reason: "public preset names and default configuration contracts",
    },
    ExpectedPublicModule {
        name: "probe_activation",
        reason: "public probe activation planning contracts",
    },
    ExpectedPublicModule {
        name: "probe_registry",
        reason: "public probe registry contracts",
    },
    ExpectedPublicModule {
        name: "process_tree",
        reason: "public process tree snapshot, classifier, target diff, and scan contracts",
    },
    ExpectedPublicModule {
        name: "recorder",
        reason: "public recording artifact schema, live recorder, retention, and writer contracts",
    },
    ExpectedPublicModule {
        name: "report",
        reason: "public report loading, analysis, rendering, diffing, and regression contracts",
    },
    ExpectedPublicModule {
        name: "session",
        reason: "public monitor session runtime entry points",
    },
    ExpectedPublicModule {
        name: "session_events",
        reason: "public monitor event stream data contracts",
    },
    ExpectedPublicModule {
        name: "session_io",
        reason: "public offline session artifact loading and validation contracts",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
struct RustPathOccurrence {
    path: String,
    line_number: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ForbiddenRustPath {
    path: &'static str,
    boundary: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PathToken {
    kind: PathTokenKind,
    line_number: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PathTokenKind {
    Ident(String),
    ColonColon,
    Punct(char),
}

fn crate_src_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn autotune_src_root() -> PathBuf {
    crate_src_root().join("autotune")
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

fn public_modules_from_source(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| {
            let module = line.strip_prefix("pub mod ")?;
            let module = module.trim();
            let module = module
                .strip_suffix(';')
                .or_else(|| module.strip_suffix('{'))?;
            Some(module.trim().to_owned())
        })
        .collect()
}

fn root_public_modules_from_lib_rs(source: &str) -> Vec<String> {
    public_modules_from_source(source)
}

fn expected_root_public_module_names() -> Vec<String> {
    EXPECTED_ROOT_PUBLIC_MODULES
        .iter()
        .map(|module| module.name.to_owned())
        .collect()
}

fn expected_api_public_module_names() -> Vec<String> {
    EXPECTED_API_PUBLIC_MODULES
        .iter()
        .map(|module| module.name.to_owned())
        .collect()
}

fn rust_source_line_count(path: &Path) -> usize {
    fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
        .lines()
        .count()
}

fn relative_to_crate_root(path: &Path) -> String {
    path.strip_prefix(env!("CARGO_MANIFEST_DIR"))
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn rust_file_line_counts_under(path: &Path) -> Vec<RustFileLineCount> {
    rust_files_under(path)
        .into_iter()
        .map(|file| RustFileLineCount {
            path: relative_to_crate_root(&file),
            lines: rust_source_line_count(&file),
        })
        .collect()
}

fn allowlisted_file_size(path: &str) -> Option<&'static OversizedRustFileAllowance> {
    OVERSIZED_RUST_FILE_ALLOWLIST
        .iter()
        .find(|allowance| allowance.path == path)
}

fn largest_rust_files(counts: &[RustFileLineCount], limit: usize) -> Vec<String> {
    let mut largest = counts.to_vec();
    largest.sort_by(|left, right| {
        right
            .lines
            .cmp(&left.lines)
            .then_with(|| left.path.cmp(&right.path))
    });
    largest
        .into_iter()
        .take(limit)
        .map(|count| format!("{} lines {}", count.lines, count.path))
        .collect()
}

fn allowlisted_existing_production_unwrap_expect_file(
    path: &str,
) -> Option<&'static ExistingProductionUnwrapExpectAllowance> {
    EXISTING_PRODUCTION_UNWRAP_EXPECT_FILE_ALLOWLIST
        .iter()
        .find(|allowance| allowance.path == path)
}

fn production_unwrap_expect_calls_in_file(path: &Path) -> Vec<ProductionUnwrapExpectCall> {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    let relative_path = relative_to_crate_root(path);
    production_unwrap_expect_calls_in_source(&source, &relative_path)
}

fn production_unwrap_expect_calls_in_source(
    source: &str,
    path: &str,
) -> Vec<ProductionUnwrapExpectCall> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut calls = Vec::new();

    for (line_number, line) in production_code_lines_outside_cfg_test_modules(source) {
        let preceding_line_has_invariant = line_number
            .checked_sub(2)
            .and_then(|index| lines.get(index))
            .is_some_and(|line| line.contains("// invariant:"));

        for call in [".unwrap()", ".expect("] {
            if line.contains(call) && !preceding_line_has_invariant {
                calls.push(ProductionUnwrapExpectCall {
                    path: path.to_owned(),
                    line_number,
                    call,
                    line: line.trim().to_owned(),
                });
            }
        }
    }

    calls
}

fn production_code_lines_outside_cfg_test_modules(source: &str) -> Vec<(usize, &str)> {
    let mut lines = Vec::new();
    let mut cfg_test_pending = false;
    let mut skipped_test_module_brace_depth: Option<isize> = None;

    for (zero_based_line_number, line) in source.lines().enumerate() {
        let line_number = zero_based_line_number + 1;
        let trimmed = line.trim_start();

        if let Some(depth) = skipped_test_module_brace_depth {
            let next_depth = depth + brace_delta(line);
            if next_depth <= 0 {
                skipped_test_module_brace_depth = None;
            } else {
                skipped_test_module_brace_depth = Some(next_depth);
            }
            continue;
        }

        if trimmed.starts_with("#[cfg(test)]") {
            cfg_test_pending = true;
            if trimmed.contains("mod tests") && trimmed.contains('{') {
                let depth = brace_delta(line);
                if depth > 0 {
                    skipped_test_module_brace_depth = Some(depth);
                }
            }
            continue;
        }

        if cfg_test_pending && trimmed.starts_with("mod tests") && trimmed.contains('{') {
            cfg_test_pending = false;
            let depth = brace_delta(line);
            if depth > 0 {
                skipped_test_module_brace_depth = Some(depth);
            }
            continue;
        }

        if cfg_test_pending
            && !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && !trimmed.starts_with("//")
        {
            cfg_test_pending = false;
        }

        lines.push((line_number, line));
    }

    lines
}

fn brace_delta(line: &str) -> isize {
    line.chars().filter(|ch| *ch == '{').count() as isize
        - line.chars().filter(|ch| *ch == '}').count() as isize
}

fn direct_print_forbidden_files() -> Vec<PathBuf> {
    let root = crate_src_root();
    let mut files = vec![
        root.join("agent.rs"),
        root.join("autotune/mod.rs"),
        root.join("autotune/runtime.rs"),
        root.join("autotune/planner.rs"),
        root.join("report/analysis.rs"),
        root.join("process_tree.rs"),
    ];

    files.extend(rust_files_under(&root.join("actions")));
    files.extend(rust_files_under(&root.join("daemon")));
    files.extend(rust_files_under(&root.join("focus")));

    files.sort();
    files.dedup();
    files
}

fn allowlisted_direct_print_call(
    path: &str,
    line_number: usize,
    macro_name: &str,
) -> Option<&'static ExistingDirectPrintAllowance> {
    EXISTING_DIRECT_PRINT_ALLOWLIST.iter().find(|allowance| {
        allowance.path == path
            && allowance.line_number == line_number
            && allowance.macro_name == macro_name
    })
}

fn direct_print_macro_calls_in_file(path: &Path) -> Vec<DirectPrintMacroCall> {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    let relative_path = relative_to_crate_root(path);
    direct_print_macro_calls_in_source(&source, &relative_path)
}

fn direct_print_macro_calls_in_source(source: &str, path: &str) -> Vec<DirectPrintMacroCall> {
    let mut calls = Vec::new();

    for (line_number, line) in production_code_lines_outside_cfg_test_modules(source) {
        if line.trim_start().starts_with("//") {
            continue;
        }

        if line.contains("eprintln!") {
            calls.push(DirectPrintMacroCall {
                path: path.to_owned(),
                line_number,
                macro_name: "eprintln!",
                line: line.trim().to_owned(),
            });
        }

        if line.contains("println!") {
            let mut start = 0;
            let mut found_non_eprintln = false;
            while let Some(idx) = line[start..].find("println!") {
                let absolute_idx = start + idx;
                if absolute_idx == 0 || line.as_bytes()[absolute_idx - 1] != b'e' {
                    found_non_eprintln = true;
                    break;
                }
                start = absolute_idx + "println!".len();
            }
            if found_non_eprintln {
                calls.push(DirectPrintMacroCall {
                    path: path.to_owned(),
                    line_number,
                    macro_name: "println!",
                    line: line.trim().to_owned(),
                });
            }
        }
    }

    calls
}

fn allowed_direct_prints_summary() -> String {
    EXISTING_DIRECT_PRINT_ALLOWLIST
        .iter()
        .map(|allowance| {
            format!(
                "{}:{} {} -- {}",
                allowance.path, allowance.line_number, allowance.macro_name, allowance.reason
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn rust_files_under(path: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rust_files(path, &mut files);
    files.sort();
    files
}

fn collect_rust_files(path: &Path, files: &mut Vec<PathBuf>) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.is_file() {
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path.to_path_buf());
        }
        return;
    }

    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        collect_rust_files(&entry.path(), files);
    }
}

fn assert_sources_do_not_reference_paths(files: &[PathBuf], forbidden: &[ForbiddenRustPath]) {
    let mut violations = Vec::new();

    for file in files {
        let source = fs::read_to_string(file).unwrap_or_default();
        for occurrence in rust_path_occurrences(&source) {
            for forbidden_path in forbidden {
                if rust_path_matches_forbidden(&occurrence.path, forbidden_path.path) {
                    violations.push(format_architecture_violation(
                        file,
                        &occurrence,
                        forbidden_path,
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "architecture boundary violations:\n{}",
        violations.join("\n")
    );
}

fn format_architecture_violation(
    file: &Path,
    occurrence: &RustPathOccurrence,
    forbidden: &ForbiddenRustPath,
) -> String {
    format!(
        "{}:{}: boundary '{}' forbids '{}', found '{}'",
        file.display(),
        occurrence.line_number,
        forbidden.boundary,
        forbidden.path,
        occurrence.path
    )
}

fn rust_path_matches_forbidden(path: &str, forbidden: &str) -> bool {
    if path == forbidden || path.starts_with(&format!("{forbidden}::")) {
        return true;
    }

    if let Some(stripped) = path.strip_prefix("crate::")
        && (stripped == forbidden || stripped.starts_with(&format!("{forbidden}::")))
    {
        return true;
    }

    if !forbidden.contains("::") {
        return path.split("::").any(|component| component == forbidden);
    }

    false
}

fn rust_path_occurrences(source: &str) -> Vec<RustPathOccurrence> {
    let sanitized = sanitize_rust_source(source);
    let tokens = lex_rust_path_tokens(&sanitized);
    let mut occurrences = Vec::new();

    collect_use_tree_paths(&tokens, &mut occurrences);
    collect_qualified_paths(&tokens, &mut occurrences);
    collect_bare_ident_paths(&tokens, &mut occurrences);
    dedupe_path_occurrences(occurrences)
}

fn dedupe_path_occurrences(occurrences: Vec<RustPathOccurrence>) -> Vec<RustPathOccurrence> {
    let mut deduped = Vec::new();
    for occurrence in occurrences {
        if !deduped.iter().any(|existing: &RustPathOccurrence| {
            existing.path == occurrence.path && existing.line_number == occurrence.line_number
        }) {
            deduped.push(occurrence);
        }
    }
    deduped
}

fn collect_bare_ident_paths(tokens: &[PathToken], occurrences: &mut Vec<RustPathOccurrence>) {
    for token in tokens {
        let PathTokenKind::Ident(ident) = &token.kind else {
            continue;
        };
        if !is_rust_keyword(ident) {
            occurrences.push(RustPathOccurrence {
                path: ident.clone(),
                line_number: token.line_number,
            });
        }
    }
}

fn collect_qualified_paths(tokens: &[PathToken], occurrences: &mut Vec<RustPathOccurrence>) {
    for index in 0..tokens.len() {
        if matches!(tokens[index].kind, PathTokenKind::ColonColon) {
            if let Some((path, line_number)) = parse_qualified_path_from(tokens, index + 1) {
                occurrences.push(RustPathOccurrence { path, line_number });
            }
            continue;
        }

        if !matches!(tokens[index].kind, PathTokenKind::Ident(_)) {
            continue;
        }

        if matches!(
            tokens.get(index + 1).map(|token| &token.kind),
            Some(PathTokenKind::ColonColon)
        ) && let Some((path, line_number)) = parse_qualified_path_from(tokens, index)
        {
            occurrences.push(RustPathOccurrence { path, line_number });
        }
    }
}

fn parse_qualified_path_from(tokens: &[PathToken], start: usize) -> Option<(String, usize)> {
    let first = tokens.get(start)?;
    let PathTokenKind::Ident(first_ident) = &first.kind else {
        return None;
    };

    let mut parts = vec![first_ident.clone()];
    let line_number = first.line_number;
    let mut cursor = start + 1;

    while matches!(
        tokens.get(cursor).map(|token| &token.kind),
        Some(PathTokenKind::ColonColon)
    ) {
        let Some(next) = tokens.get(cursor + 1) else {
            break;
        };
        let PathTokenKind::Ident(next_ident) = &next.kind else {
            break;
        };
        parts.push(next_ident.clone());
        cursor += 2;
    }

    (parts.len() > 1).then(|| (parts.join("::"), line_number))
}

fn collect_use_tree_paths(tokens: &[PathToken], occurrences: &mut Vec<RustPathOccurrence>) {
    let mut index = 0;
    while index < tokens.len() {
        if !token_is_ident(&tokens[index], "use") {
            index += 1;
            continue;
        }

        let mut cursor = index + 1;
        cursor = parse_use_tree(tokens, cursor, &[], occurrences);
        while cursor < tokens.len() && !matches!(tokens[cursor].kind, PathTokenKind::Punct(';')) {
            cursor += 1;
        }
        index = cursor.saturating_add(1);
    }
}

fn parse_use_group(
    tokens: &[PathToken],
    mut index: usize,
    prefix: &[String],
    occurrences: &mut Vec<RustPathOccurrence>,
) -> usize {
    while index < tokens.len() {
        match &tokens[index].kind {
            PathTokenKind::Punct('}') => return index + 1,
            PathTokenKind::Punct(',') => index += 1,
            _ => index = parse_use_tree(tokens, index, prefix, occurrences),
        }
    }
    index
}

fn parse_use_tree(
    tokens: &[PathToken],
    mut index: usize,
    prefix: &[String],
    occurrences: &mut Vec<RustPathOccurrence>,
) -> usize {
    let mut path_parts = prefix.to_vec();

    loop {
        let Some(token) = tokens.get(index) else {
            return index;
        };

        match &token.kind {
            PathTokenKind::Ident(ident) if ident == "self" => {
                if !path_parts.is_empty() {
                    occurrences.push(RustPathOccurrence {
                        path: path_parts.join("::"),
                        line_number: token.line_number,
                    });
                }
                return skip_use_alias(tokens, index + 1);
            }
            PathTokenKind::Ident(ident) => {
                let ident_line_number = token.line_number;
                path_parts.push(ident.clone());
                index += 1;

                if matches!(
                    tokens.get(index).map(|token| &token.kind),
                    Some(PathTokenKind::ColonColon)
                ) {
                    index += 1;
                    if matches!(
                        tokens.get(index).map(|token| &token.kind),
                        Some(PathTokenKind::Punct('{'))
                    ) {
                        return parse_use_group(tokens, index + 1, &path_parts, occurrences);
                    }
                    continue;
                }

                occurrences.push(RustPathOccurrence {
                    path: path_parts.join("::"),
                    line_number: ident_line_number,
                });
                return skip_use_alias(tokens, index);
            }
            PathTokenKind::Punct('*') => {
                path_parts.push("*".to_owned());
                occurrences.push(RustPathOccurrence {
                    path: path_parts.join("::"),
                    line_number: token.line_number,
                });
                return index + 1;
            }
            PathTokenKind::Punct('{') => {
                return parse_use_group(tokens, index + 1, &path_parts, occurrences);
            }
            _ => return index + 1,
        }
    }
}

fn skip_use_alias(tokens: &[PathToken], mut index: usize) -> usize {
    if token_is_ident_at(tokens, index, "as") {
        index += 1;
        if matches!(
            tokens.get(index).map(|token| &token.kind),
            Some(PathTokenKind::Ident(_))
        ) {
            index += 1;
        }
    }
    index
}

fn token_is_ident(token: &PathToken, expected: &str) -> bool {
    matches!(&token.kind, PathTokenKind::Ident(ident) if ident == expected)
}

fn token_is_ident_at(tokens: &[PathToken], index: usize, expected: &str) -> bool {
    tokens
        .get(index)
        .is_some_and(|token| token_is_ident(token, expected))
}

fn lex_rust_path_tokens(source: &str) -> Vec<PathToken> {
    let mut tokens = Vec::new();
    let mut chars = source.chars().peekable();
    let mut line_number = 1;

    while let Some(ch) = chars.next() {
        if ch == '\n' {
            line_number += 1;
            continue;
        }

        if is_ident_start(ch) {
            let mut ident = String::from(ch);
            while let Some(next) = chars.peek().copied() {
                if is_ident_continue(next) {
                    ident.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(PathToken {
                kind: PathTokenKind::Ident(ident),
                line_number,
            });
            continue;
        }

        if ch == ':' && chars.peek() == Some(&':') {
            chars.next();
            tokens.push(PathToken {
                kind: PathTokenKind::ColonColon,
                line_number,
            });
            continue;
        }

        if matches!(
            ch,
            '{' | '}' | '(' | ')' | '[' | ']' | ',' | ';' | '*' | '<' | '>'
        ) {
            tokens.push(PathToken {
                kind: PathTokenKind::Punct(ch),
                line_number,
            });
        }
    }

    tokens
}

fn sanitize_rust_source(source: &str) -> String {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum State {
        Normal,
        LineComment,
        BlockComment(usize),
        String { escaped: bool },
        RawString { hashes: usize },
    }

    let mut output = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut state = State::Normal;

    while let Some(ch) = chars.next() {
        match state {
            State::Normal => {
                if ch == 'r'
                    && let Some(hashes) = try_start_raw_string(ch, &mut chars, &mut output)
                {
                    state = State::RawString { hashes };
                } else if ch == '/' && chars.peek() == Some(&'/') {
                    chars.next();
                    output.push(' ');
                    output.push(' ');
                    state = State::LineComment;
                } else if ch == '/' && chars.peek() == Some(&'*') {
                    chars.next();
                    output.push(' ');
                    output.push(' ');
                    state = State::BlockComment(1);
                } else if ch == '"' {
                    output.push(' ');
                    state = State::String { escaped: false };
                } else {
                    output.push(ch);
                }
            }
            State::LineComment => {
                if ch == '\n' {
                    output.push('\n');
                    state = State::Normal;
                } else {
                    output.push(' ');
                }
            }
            State::BlockComment(depth) => {
                if ch == '\n' {
                    output.push('\n');
                } else if ch == '/' && chars.peek() == Some(&'*') {
                    chars.next();
                    output.push(' ');
                    output.push(' ');
                    state = State::BlockComment(depth + 1);
                } else if ch == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    output.push(' ');
                    output.push(' ');
                    if depth == 1 {
                        state = State::Normal;
                    } else {
                        state = State::BlockComment(depth - 1);
                    }
                } else {
                    output.push(' ');
                }
            }
            State::String { escaped } => {
                if ch == '\n' {
                    output.push('\n');
                    state = State::Normal;
                } else {
                    output.push(' ');
                    if escaped {
                        state = State::String { escaped: false };
                    } else if ch == '\\' {
                        state = State::String { escaped: true };
                    } else if ch == '"' {
                        state = State::Normal;
                    }
                }
            }
            State::RawString { hashes } => {
                if ch == '\n' {
                    output.push('\n');
                } else if ch == '"' && raw_string_hashes_close(&mut chars, hashes, &mut output) {
                    output.push(' ');
                    state = State::Normal;
                } else {
                    output.push(' ');
                }
            }
        }
    }

    output
}

fn try_start_raw_string(
    first_char: char,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    output: &mut String,
) -> Option<usize> {
    if first_char != 'r' {
        return None;
    }

    let mut lookahead = chars.clone();
    let mut hashes = 0;
    while lookahead.peek() == Some(&'#') {
        hashes += 1;
        lookahead.next();
    }
    if lookahead.peek() != Some(&'"') {
        return None;
    }

    output.push(' ');
    for _ in 0..hashes {
        chars.next();
        output.push(' ');
    }
    chars.next();
    output.push(' ');
    Some(hashes)
}

fn raw_string_hashes_close(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    hashes: usize,
    output: &mut String,
) -> bool {
    let mut lookahead = chars.clone();
    for _ in 0..hashes {
        if lookahead.next() != Some('#') {
            return false;
        }
    }

    for _ in 0..hashes {
        chars.next();
        output.push(' ');
    }
    true
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn is_rust_keyword(ident: &str) -> bool {
    matches!(
        ident,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
    )
}

fn dependency_matrix_entry(subsystem: &str) -> &'static DependencyMatrixEntry {
    ARCHITECTURE_DEPENDENCY_MATRIX
        .iter()
        .find(|entry| entry.subsystem == subsystem)
        .unwrap_or_else(|| panic!("missing architecture dependency matrix entry for {subsystem}"))
}

#[test]
fn rust_path_extractor_finds_imports_qualified_paths_and_line_numbers() {
    let source = r#"
use crate::{cli, commands::{self, AppCommand}};
use clap::Parser;
fn demo() {
    let _ = crate::daemon::DaemonPolicy::default();
    let _ = super::helper::Thing::new();
    let _ignored = "crate::report::HtmlReportModel";
    // crate::actions::runner::run_audited_action
}
"#;

    let occurrences = rust_path_occurrences(source);

    for (path, line_number) in [
        ("crate::cli", 2),
        ("crate::commands", 2),
        ("crate::commands::AppCommand", 2),
        ("clap::Parser", 3),
        ("crate::daemon::DaemonPolicy::default", 5),
        ("super::helper::Thing::new", 6),
    ] {
        assert!(
            occurrences
                .iter()
                .any(|occurrence| occurrence.path == path && occurrence.line_number == line_number),
            "missing parsed path {path} at line {line_number}; got {occurrences:?}"
        );
    }

    assert!(
        !occurrences
            .iter()
            .any(|occurrence| occurrence.path == "crate::report::HtmlReportModel"),
        "paths inside strings must not be reported"
    );
    assert!(
        !occurrences
            .iter()
            .any(|occurrence| occurrence.path == "crate::actions::runner::run_audited_action"),
        "paths inside comments must not be reported"
    );
}

#[test]
fn architecture_violation_message_includes_boundary_path_file_and_line() {
    let file = Path::new("src/actions/mod.rs");
    let occurrence = RustPathOccurrence {
        path: "crate::commands::AppCommand".to_owned(),
        line_number: 17,
    };
    let forbidden = ForbiddenRustPath {
        path: "crate::commands",
        boundary: "actions must not depend on command parsing",
    };

    let message = format_architecture_violation(file, &occurrence, &forbidden);

    assert!(message.contains("src/actions/mod.rs:17"));
    assert!(message.contains("actions must not depend on command parsing"));
    assert!(message.contains("crate::commands"));
    assert!(message.contains("crate::commands::AppCommand"));
}

#[test]
fn direct_print_scanner_finds_print_macros_and_ignores_cfg_test_modules() {
    let source = r#"
fn runtime_stdout() {
    println!("runtime should not print directly");
}

fn runtime_stderr() {
    eprintln!("runtime should not print directly");
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_prints_are_ignored() {
        println!("test output is allowed");
        eprintln!("test output is allowed");
    }
}
"#;

    let calls = direct_print_macro_calls_in_source(source, "src/domain.rs");

    assert_eq!(
        calls,
        vec![
            DirectPrintMacroCall {
                path: "src/domain.rs".to_owned(),
                line_number: 3,
                macro_name: "println!",
                line: "println!(\"runtime should not print directly\");".to_owned(),
            },
            DirectPrintMacroCall {
                path: "src/domain.rs".to_owned(),
                line_number: 7,
                macro_name: "eprintln!",
                line: "eprintln!(\"runtime should not print directly\");".to_owned(),
            },
        ]
    );
}

#[test]
fn runtime_domain_and_service_modules_do_not_print_directly() {
    for allowance in EXISTING_DIRECT_PRINT_ALLOWLIST {
        assert!(
            !allowance.reason.trim().is_empty(),
            "direct print allowlist entry '{}:{} {}' must have a reason",
            allowance.path,
            allowance.line_number,
            allowance.macro_name
        );
    }

    let files = direct_print_forbidden_files();
    let calls = files
        .iter()
        .flat_map(|file| direct_print_macro_calls_in_file(file))
        .collect::<Vec<_>>();

    let mut violations = Vec::new();

    for allowance in EXISTING_DIRECT_PRINT_ALLOWLIST {
        if !calls.iter().any(|call| {
            call.path == allowance.path
                && call.line_number == allowance.line_number
                && call.macro_name == allowance.macro_name
        }) {
            violations.push(format!(
                "allowlisted direct print no longer exists or moved: {}:{} {} -- update EXISTING_DIRECT_PRINT_ALLOWLIST",
                allowance.path, allowance.line_number, allowance.macro_name
            ));
        }
    }

    for call in &calls {
        if allowlisted_direct_print_call(&call.path, call.line_number, call.macro_name).is_none() {
            violations.push(format!(
                "{}:{} uses {} outside approved CLI/rendering/test boundaries: {}",
                call.path, call.line_number, call.macro_name, call.line
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "direct printing architecture guard failed:\n{}\n\nallowed existing direct prints:\n{}",
        violations.join("\n"),
        allowed_direct_prints_summary()
    );
}

#[test]
fn production_unwrap_expect_scanner_ignores_cfg_test_modules_and_invariant_comments() {
    let source = r#"
fn bad_unwrap(value: Option<u8>) -> u8 {
    value.unwrap()
}

fn bad_expect(value: Option<u8>) -> u8 {
    value.expect("value must exist")
}

fn documented_unwrap(value: Option<u8>) -> u8 {
    // invariant: value was checked by the caller
    value.unwrap()
}

fn documented_expect(value: Option<u8>) -> u8 {
    // invariant: value was checked by the caller
    value.expect("value must exist")
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_unwraps_are_ignored() {
        Some(1).unwrap();
        Some(1).expect("present");
    }
}
"#;

    let calls = production_unwrap_expect_calls_in_source(source, "src/new_module.rs");

    assert_eq!(
        calls,
        vec![
            ProductionUnwrapExpectCall {
                path: "src/new_module.rs".to_owned(),
                line_number: 3,
                call: ".unwrap()",
                line: "value.unwrap()".to_owned(),
            },
            ProductionUnwrapExpectCall {
                path: "src/new_module.rs".to_owned(),
                line_number: 7,
                call: ".expect(",
                line: "value.expect(\"value must exist\")".to_owned(),
            },
        ]
    );
}

#[test]
fn new_production_unwrap_expect_calls_require_invariant_or_allowlist() {
    for allowance in EXISTING_PRODUCTION_UNWRAP_EXPECT_FILE_ALLOWLIST {
        assert!(
            !allowance.reason.trim().is_empty(),
            "existing production unwrap/expect allowlist entry '{}' must have a reason",
            allowance.path
        );
    }

    let mut violations = Vec::new();

    for file in rust_files_under(&crate_src_root()) {
        let relative_path = relative_to_crate_root(&file);
        if allowlisted_existing_production_unwrap_expect_file(&relative_path).is_some() {
            continue;
        }

        for call in production_unwrap_expect_calls_in_file(&file) {
            violations.push(format!(
                "{}:{} uses {} without preceding '// invariant:' comment: {}",
                call.path, call.line_number, call.call, call.line
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "production unwrap/expect guard failed:\n{}",
        violations.join("\n")
    );
}

#[test]
fn rust_source_file_sizes_do_not_grow_without_architecture_allowlist() {
    for allowance in OVERSIZED_RUST_FILE_ALLOWLIST {
        assert!(
            !allowance.reason.trim().is_empty(),
            "oversized Rust file '{}' must have an allowlist reason",
            allowance.path
        );
    }

    let counts = rust_file_line_counts_under(&crate_src_root());
    let largest_files = largest_rust_files(&counts, 20).join("\n");
    let mut violations = Vec::new();

    for allowance in OVERSIZED_RUST_FILE_ALLOWLIST {
        match counts.iter().find(|count| count.path == allowance.path) {
            Some(count) if count.lines > allowance.max_lines => violations.push(format!(
                "{} has {} lines, exceeding allowlisted maximum {} lines; split the file or update OVERSIZED_RUST_FILE_ALLOWLIST with an explicit reason",
                count.path, count.lines, allowance.max_lines
            )),
            Some(_) => {}
            None => violations.push(format!(
                "allowlisted oversized Rust file '{}' no longer exists; remove or update its allowlist entry",
                allowance.path
            )),
        }
    }

    for count in &counts {
        if count.lines > RUST_FILE_SIZE_LIMIT_LINES && allowlisted_file_size(&count.path).is_none()
        {
            violations.push(format!(
                "{} has {} lines, exceeding {} lines without an allowlist entry",
                count.path, count.lines, RUST_FILE_SIZE_LIMIT_LINES
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "Rust source file size gate failed:\n{}\n\nlargest Rust files:\n{}",
        violations.join("\n"),
        largest_files
    );
}

#[test]
fn root_public_modules_are_intentional() {
    for module in EXPECTED_ROOT_PUBLIC_MODULES {
        assert!(
            !module.reason.trim().is_empty(),
            "public module '{}' must have a reason explaining why it is exported",
            module.name
        );
    }

    let lib_rs_path = crate_src_root().join("lib.rs");
    let lib_rs = fs::read_to_string(&lib_rs_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", lib_rs_path.display()));

    let actual = root_public_modules_from_lib_rs(&lib_rs);
    let expected = expected_root_public_module_names();

    assert_eq!(
        actual, expected,
        "root public module exports changed; update EXPECTED_ROOT_PUBLIC_MODULES with the intentional public module list and a reason for every exported module"
    );
}

#[test]
fn api_public_modules_are_intentional() {
    for module in EXPECTED_API_PUBLIC_MODULES {
        assert!(
            !module.reason.trim().is_empty(),
            "api public module '{}' must have a reason explaining why it is exported",
            module.name
        );
    }

    let api_rs_path = crate_src_root().join("api.rs");
    let api_rs = fs::read_to_string(&api_rs_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", api_rs_path.display()));

    let actual = public_modules_from_source(&api_rs);
    let expected = expected_api_public_module_names();

    assert_eq!(
        actual, expected,
        "api public module exports changed; update EXPECTED_API_PUBLIC_MODULES with the intentional public facade module list and a reason for every exported section"
    );
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
        "autotune::candidate",
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
