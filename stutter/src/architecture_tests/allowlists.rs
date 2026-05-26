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
pub(in crate::architecture_tests) struct ExistingUnwrapExpectAllowance {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::architecture_tests) struct ExistingProductionPanicAllowance {
    pub(in crate::architecture_tests) path: &'static str,
    pub(in crate::architecture_tests) line_number: usize,
    pub(in crate::architecture_tests) macro_name: &'static str,
    pub(in crate::architecture_tests) reason: &'static str,
}

pub(in crate::architecture_tests) const OVERSIZED_RUST_FILE_ALLOWLIST:
    &[OversizedRustFileAllowance] = &[
    OversizedRustFileAllowance {
        path: "src/tune/mod.rs",
        max_lines: 938,
        reason: "pending Step 32.5 cleanup; baseline pinned for the 800-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/service.rs",
        max_lines: 825,
        reason: "pending service command split; baseline pinned for the 800-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/providers/irq_affinity.rs",
        max_lines: 809,
        reason: "pending IRQ-affinity provider split; baseline pinned for the 800-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/tune/comparability.rs",
        max_lines: 804,
        reason: "pending tune comparability split; baseline pinned for the 800-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/recommend.rs",
        max_lines: 792,
        reason: "pending recommend split; baseline pinned for the 700-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/live_experiment/mod.rs",
        max_lines: 790,
        reason: "pending live experiment split; baseline pinned for the 700-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/doctor.rs",
        max_lines: 763,
        reason: "pending doctor split; baseline pinned for the 700-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/status/mod.rs",
        max_lines: 766,
        reason: "pending autotune status split; baseline pinned for the 700-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/summary.rs",
        max_lines: 758,
        reason: "pending summary split; baseline pinned for the 700-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/remote.rs",
        max_lines: 755,
        reason: "pending remote split; baseline pinned for the 700-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/scenario.rs",
        max_lines: 755,
        reason: "pending scenario split; baseline pinned for the 700-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/observation_builder.rs",
        max_lines: 751,
        reason: "pending observation builder split; baseline pinned for the 700-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/actions/uclamp.rs",
        max_lines: 741,
        reason: "pending uclamp split; baseline pinned for the 700-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/daemon/config.rs",
        max_lines: 738,
        reason: "pending daemon config split; baseline pinned for the 700-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/advisor.rs",
        max_lines: 721,
        reason: "pending advisor split; baseline pinned for the 700-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/probe_registry.rs",
        max_lines: 721,
        reason: "pending probe registry split; baseline pinned for the 700-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/rolling_window.rs",
        max_lines: 718,
        reason: "pending rolling window split; baseline pinned for the 700-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/session/sinks.rs",
        max_lines: 717,
        reason: "pending session sinks split; baseline pinned for the 700-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/autotune/quality.rs",
        max_lines: 714,
        reason: "pending autotune quality split; baseline pinned for the 700-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/profile_restore.rs",
        max_lines: 708,
        reason: "pending profile restore split; baseline pinned for the 700-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/actions/ioprio.rs",
        max_lines: 701,
        reason: "pending ioprio split; baseline pinned for the 700-line production gate",
    },
    OversizedRustFileAllowance {
        path: "src/daemon/soak.rs",
        max_lines: 801,
        reason: "pending daemon soak-test split; baseline pinned for the 800-line production gate",
    },
];

pub(in crate::architecture_tests) fn allowlisted_file_size(
    path: &str,
) -> Option<&'static OversizedRustFileAllowance> {
    OVERSIZED_RUST_FILE_ALLOWLIST
        .iter()
        .find(|allowance| allowance.path == path)
}

pub(in crate::architecture_tests) const EXISTING_RUNTIME_UNWRAP_EXPECT_FILE_ALLOWLIST:
    &[ExistingUnwrapExpectAllowance] = &[];

pub(in crate::architecture_tests) const CFG_TEST_OR_FIXTURE_UNWRAP_EXPECT_FILE_ALLOWLIST:
    &[ExistingUnwrapExpectAllowance] = &[
    ExistingUnwrapExpectAllowance {
        path: "src/actions/fake_action.rs",
        reason: "fake action test-support implementation is compiled in source form and contains fixture unwrap/expect calls",
    },
    ExistingUnwrapExpectAllowance {
        path: "src/artifact_contract_tests.rs",
        reason: "artifact contract test module contains unwrap/expect calls outside cfg-test module blocks",
    },
    ExistingUnwrapExpectAllowance {
        path: "src/architecture_tests.rs",
        reason: "architecture tests intentionally contain unwrap/expect scanner fixtures and test-only panic helpers",
    },
    ExistingUnwrapExpectAllowance {
        path: "src/architecture_tests/unwrap_expect.rs",
        reason: "architecture unwrap/expect scanner tests intentionally contain unwrap/expect fixture strings",
    },
    ExistingUnwrapExpectAllowance {
        path: "src/autotune/planner_tests/support.rs",
        reason: "autotune planner test support is cfg-test-only through planner.rs and contains synthetic fixture unwraps",
    },
    ExistingUnwrapExpectAllowance {
        path: "src/daemon/acceptance.rs",
        reason: "daemon acceptance test-support module contains unwrap/expect calls outside cfg-test module blocks",
    },
    ExistingUnwrapExpectAllowance {
        path: "src/focus/test_support.rs",
        reason: "cfg(test)-only focus test support helpers contain unwrap/expect calls for synthetic fixture setup",
    },
    ExistingUnwrapExpectAllowance {
        path: "src/recording_fixture_tests.rs",
        reason: "recording fixture test module contains unwrap/expect calls outside cfg-test module blocks",
    },
    ExistingUnwrapExpectAllowance {
        path: "src/runnable_depth_tests.rs",
        reason: "runnable depth test module contains unwrap/expect calls outside cfg-test module blocks",
    },
];

pub(in crate::architecture_tests) const EXISTING_DIRECT_PRINT_ALLOWLIST:
    &[ExistingDirectPrintAllowance] = &[];

pub(in crate::architecture_tests) const EXISTING_PRODUCTION_PANIC_ALLOWLIST:
    &[ExistingProductionPanicAllowance] = &[];

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::architecture_tests) enum UnwrapExpectAllowanceCategory {
    Runtime,
    CfgTestOrFixture,
}

impl UnwrapExpectAllowanceCategory {
    pub(in crate::architecture_tests) fn label(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::CfgTestOrFixture => "cfg-test-or-fixture",
        }
    }
}

pub(in crate::architecture_tests) fn allowlisted_existing_unwrap_expect_file(
    path: &str,
) -> Option<(
    UnwrapExpectAllowanceCategory,
    &'static ExistingUnwrapExpectAllowance,
)> {
    if let Some(allowance) = EXISTING_RUNTIME_UNWRAP_EXPECT_FILE_ALLOWLIST
        .iter()
        .find(|allowance| allowance.path == path)
    {
        return Some((UnwrapExpectAllowanceCategory::Runtime, allowance));
    }

    CFG_TEST_OR_FIXTURE_UNWRAP_EXPECT_FILE_ALLOWLIST
        .iter()
        .find(|allowance| allowance.path == path)
        .map(|allowance| (UnwrapExpectAllowanceCategory::CfgTestOrFixture, allowance))
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

pub(in crate::architecture_tests) fn allowlisted_production_panic_call(
    path: &str,
    line_number: usize,
    macro_name: &str,
) -> Option<&'static ExistingProductionPanicAllowance> {
    EXISTING_PRODUCTION_PANIC_ALLOWLIST
        .iter()
        .find(|allowance| {
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
