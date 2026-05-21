//! Architecture test allowlists; this module owns temporary exception tables, not scanner logic.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::architecture_tests) struct ExpectedPublicModule {
    pub(in crate::architecture_tests) name: &'static str,
    pub(in crate::architecture_tests) reason: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::architecture_tests) struct OversizedRustFileAllowance {
    pub(in crate::architecture_tests) path: &'static str,
    pub(in crate::architecture_tests) max_lines: usize,
    pub(in crate::architecture_tests) reason: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::architecture_tests) struct ExistingProductionUnwrapExpectAllowance {
    pub(in crate::architecture_tests) path: &'static str,
    pub(in crate::architecture_tests) reason: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::architecture_tests) struct ExistingDirectPrintAllowance {
    pub(in crate::architecture_tests) path: &'static str,
    pub(in crate::architecture_tests) line_number: usize,
    pub(in crate::architecture_tests) macro_name: &'static str,
    pub(in crate::architecture_tests) reason: &'static str,
}

pub(in crate::architecture_tests) const OVERSIZED_RUST_FILE_ALLOWLIST:
    &[OversizedRustFileAllowance] = &[
    OversizedRustFileAllowance {
        path: "src/agent.rs",
        max_lines: 750,
        reason: "agent root is now a thin configuration/startup/auth boundary after route handlers moved to focused submodules",
    },
    OversizedRustFileAllowance {
        path: "src/agent/autotune.rs",
        max_lines: 1_245,
        reason: "autotune agent route handlers, remote policy helpers, explicit task reaping status, active record-level restore endpoint wiring, task reaping, and enum-mode apply-low-risk start behavior remain pending future policy/helper split",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/planning/tests.rs",
        max_lines: 1_514,
        reason: "existing broad candidate planning regression tests moved out of candidate.rs pending future test split",
    },
    OversizedRustFileAllowance {
        path: "src/diagnosis.rs",
        max_lines: 1_585,
        reason: "temporary extraction stage: owns diagnosis config/model/anchor/candidate/evidence orchestration; next split is diagnosis/model.rs",
    },
    OversizedRustFileAllowance {
        path: "src/cli/monitor.rs",
        max_lines: 1_287,
        reason: "temporary extraction stage: owns monitor CLI args/defaults/merge/validation logic; DMABUF and GPU-engine flags are staged here until cli/monitor/args.rs exists",
    },
    OversizedRustFileAllowance {
        path: "src/session/monitor_session.rs",
        max_lines: 1_545,
        reason: "temporary extraction stage: owns MonitorSession run loop and remaining tick handlers after facade split; DMABUF and GPU-engine ingestion are staged here until display-path tick handling is split",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/shutdown.rs",
        max_lines: 1_015,
        reason: "existing autotune shutdown and rollback-on-exit tests pending future split; exit rollback helpers now have no-allow regression coverage",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/live_experiment/mod.rs",
        max_lines: 2_046,
        reason: "existing live experiment manager implementation pending future split; rollback verification and simulated rollback coverage are staged here",
    },
    OversizedRustFileAllowance {
        path: "src/report/render/text.rs",
        max_lines: 1_685,
        reason: "existing text report renderer pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/config_file.rs",
        max_lines: 1_787,
        reason: "existing config file parser/model implementation pending future split; display topology, DMABUF, and GPU-engine config fields are staged here",
    },
    OversizedRustFileAllowance {
        path: "src/report/mod.rs",
        max_lines: 1_553,
        reason: "existing report public module and tests pending future split; display-path comparison entry point is staged here",
    },
    OversizedRustFileAllowance {
        path: "src/cli/report.rs",
        max_lines: 1_546,
        reason: "existing report CLI argument surface pending future split; display-path compare strict/expect parsing and rules Args-to-Input conversion are staged here",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/status.rs",
        max_lines: 1_426,
        reason: "existing autotune status rendering implementation pending future split; dry-run affected-task status output is staged here",
    },
    OversizedRustFileAllowance {
        path: "src/validation_corpus_tests.rs",
        max_lines: 1_468,
        reason: "existing validation corpus test module pending future split; display-path validation corpus cases are staged here",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/startup_recovery.rs",
        max_lines: 1_380,
        reason: "existing autotune startup recovery implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/emergency_restore.rs",
        max_lines: 1_658,
        reason: "existing autotune emergency restore implementation pending future split; restore input conversion, rollback summary wiring, and record-level restore outcome propagation are staged here",
    },
    OversizedRustFileAllowance {
        path: "src/tune/mod.rs",
        max_lines: 1_333,
        reason: "existing tune module implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/active_config.rs",
        max_lines: 1_815,
        reason: "existing active autotune config implementation pending future split; rollback baseline verification is staged here",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/controller.rs",
        max_lines: 1_294,
        reason: "existing autotune controller state implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/providers/mod.rs",
        max_lines: 1_211,
        reason: "existing autotune provider registry, calibration, and provider tests pending future split; public provider extension reexports are staged here",
    },
    OversizedRustFileAllowance {
        path: "src/metrics.rs",
        max_lines: 1_264,
        reason: "existing metrics model and tests pending future split; PSI delta and fallback-collision counter fields are staged here",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/rolling_window.rs",
        max_lines: 1_382,
        reason: "existing autotune rolling window implementation pending future split; memory PSI spike, IRQ missing-timestamp policy, interval ordering hardening, and invalid-frametime drop coverage are staged here",
    },
    OversizedRustFileAllowance {
        path: "src/config/effective.rs",
        max_lines: 1_348,
        reason: "existing effective config resolution implementation pending future split; prime display-path, display topology, DMABUF, and GPU-engine provenance are staged here",
    },
    OversizedRustFileAllowance {
        path: "src/recorder/session.rs",
        max_lines: 1_119,
        reason: "existing recorder session writer implementation pending future split; display metadata capture is staged here",
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
        max_lines: 1_740,
        reason: "existing cgroup action implementation pending future split; rollback handler registration, identity-verified restore checks, best-effort restore error handling, cpuset rollback coverage, restore-write classifier coverage, and transactional cgroup apply coverage are staged here",
    },
    OversizedRustFileAllowance {
        path: "src/cli/mod.rs",
        max_lines: 1_106,
        reason: "existing top-level CLI parser implementation pending future split; Clap command tree, compare expect/strict command coverage, dry-run-all-safe parsing, and agent Unix socket cap/timeout validation are staged here",
    },
    OversizedRustFileAllowance {
        path: "src/actions/cpu_power.rs",
        max_lines: 1_035,
        reason: "existing CPU power action implementation pending future split; rollback handler registration, medium-risk EPP classification, and policy-backed factory execution are staged here",
    },
    OversizedRustFileAllowance {
        path: "src/actions/gpu_power.rs",
        max_lines: 1_065,
        reason: "existing GPU power action implementation pending future split; rollback handler registration and medium-risk profile classification are staged here",
    },
    OversizedRustFileAllowance {
        path: "src/actions/ioprio.rs",
        max_lines: 1_080,
        reason: "existing I/O priority action implementation pending future split; transactional apply now prebuilds restore records before mutation and rollback verifies task identity plus best-effort write errors before restore",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/runtime.rs",
        max_lines: 1_005,
        reason: "existing autotune runtime implementation pending future split; restore, task-lifecycle, and display-path event ignore wiring are staged here",
    },
    OversizedRustFileAllowance {
        path: "src/actions/uclamp.rs",
        max_lines: 1_178,
        reason: "existing uclamp action implementation pending future split; transactional apply now prebuilds restore records before mutation and rollback verifies task identity plus best-effort write errors before restore",
    },
    OversizedRustFileAllowance {
        path: "src/actions/vm_knobs.rs",
        max_lines: 1_135,
        reason: "existing VM knob action implementation pending future split; rollback handler registration, safe-value medium-risk guards, and executable plan serialization are staged here",
    },
    OversizedRustFileAllowance {
        path: "src/doctor.rs",
        max_lines: 1_158,
        reason: "existing doctor diagnostics implementation pending future split",
    },
    OversizedRustFileAllowance {
        path: "src/session_io.rs",
        max_lines: 1_102,
        reason: "existing session artifact loader implementation pending future split; DRM fence data-quality warnings are staged here",
    },
];

pub(in crate::architecture_tests) fn allowlisted_file_size(
    path: &str,
) -> Option<&'static OversizedRustFileAllowance> {
    OVERSIZED_RUST_FILE_ALLOWLIST
        .iter()
        .find(|allowance| allowance.path == path)
}

pub(in crate::architecture_tests) const EXISTING_PRODUCTION_UNWRAP_EXPECT_FILE_ALLOWLIST:
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
        path: "src/architecture_tests/unwrap_expect.rs",
        reason: "architecture unwrap/expect scanner tests intentionally contain unwrap/expect fixture strings",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/autotune/planner_tests/support.rs",
        reason: "autotune planner test support is cfg-test-only through planner.rs and contains synthetic fixture unwraps",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/autotune/planning/tests.rs",
        reason: "candidate planning regression tests are cfg-test-only through planning/mod.rs and contain test fixture unwraps",
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
        path: "src/daemon/acceptance.rs",
        reason: "existing daemon acceptance test-support module contains unwrap/expect calls outside cfg-test module blocks",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/diagnosis.rs",
        reason: "existing diagnosis implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/events/interpret.rs",
        reason: "existing event interpretation implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/focus/test_support.rs",
        reason: "cfg(test)-only focus test support helpers contain unwrap/expect calls for synthetic fixture setup",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/probe_registry.rs",
        reason: "existing probe registry implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/recording_fixture_tests.rs",
        reason: "existing recording fixture test module contains unwrap/expect calls outside cfg-test module blocks",
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
        path: "src/tune/mod.rs",
        reason: "existing tune implementation contains production unwrap/expect calls",
    },
    ExistingProductionUnwrapExpectAllowance {
        path: "src/validation_corpus_tests.rs",
        reason: "existing validation corpus test module contains unwrap/expect calls outside cfg-test module blocks",
    },
];

pub(in crate::architecture_tests) const EXISTING_DIRECT_PRINT_ALLOWLIST:
    &[ExistingDirectPrintAllowance] = &[];

pub(in crate::architecture_tests) const EXPECTED_ROOT_PUBLIC_MODULES: &[ExpectedPublicModule] =
    &[ExpectedPublicModule {
        name: "api",
        reason: "single intentional public façade replacing direct public subsystem modules",
    }];

pub(in crate::architecture_tests) const EXPECTED_API_PUBLIC_MODULES: &[ExpectedPublicModule] = &[
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

pub(in crate::architecture_tests) fn allowlisted_existing_production_unwrap_expect_file(
    path: &str,
) -> Option<&'static ExistingProductionUnwrapExpectAllowance> {
    EXISTING_PRODUCTION_UNWRAP_EXPECT_FILE_ALLOWLIST
        .iter()
        .find(|allowance| allowance.path == path)
}

pub(in crate::architecture_tests) fn allowlisted_direct_print_call(
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

pub(in crate::architecture_tests) fn allowed_direct_prints_summary() -> String {
    EXISTING_DIRECT_PRINT_ALLOWLIST
        .iter()
        .map(|allowance| {
            format!(
                "{}:{} {} -- {}",
                allowance.path, allowance.line_number, allowance.macro_name, allowance.reason
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}
