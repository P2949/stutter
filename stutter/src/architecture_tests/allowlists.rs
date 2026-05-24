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
    &[OversizedRustFileAllowance] = &[];

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
