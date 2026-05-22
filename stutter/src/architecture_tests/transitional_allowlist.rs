//! Inventory of temporary migration markers; scanner policy lives in `transitional`.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::architecture_tests) struct MigrationModuleAllowance {
    pub(in crate::architecture_tests) path: &'static str,
    pub(in crate::architecture_tests) reason: &'static str,
    pub(in crate::architecture_tests) exit_criteria: &'static str,
}

macro_rules! migration_module {
    ($path:literal, $reason:literal, $exit_criteria:literal) => {
        MigrationModuleAllowance {
            path: $path,
            reason: $reason,
            exit_criteria: $exit_criteria,
        }
    };
}

pub(in crate::architecture_tests) const MAX_MIGRATION_MARKER_MODULES: usize = 18;

pub(in crate::architecture_tests) const MIGRATION_MODULE_ALLOWLIST: &[MigrationModuleAllowance] = &[
    migration_module!(
        "src/autotune/domain/mod.rs",
        "autotune domain facade keeps pure type extraction stable during import migration",
        "remove once callers import the domain submodules directly and no facade imports remain"
    ),
    migration_module!(
        "src/autotune/human_output.rs",
        "human output renderer remains available while callers migrate through autotune::output",
        "remove once all presentation call sites use the final output module boundary"
    ),
    migration_module!(
        "src/autotune/observation/builder.rs",
        "observation builder facade preserves old paths during observation extraction",
        "remove once old observation builder paths are unused"
    ),
    migration_module!(
        "src/autotune/observation/observation.rs",
        "observation facade preserves old paths during observation extraction",
        "remove once old observation paths are unused"
    ),
    migration_module!(
        "src/autotune/observation/quality.rs",
        "quality facade preserves old paths during observation extraction",
        "remove once old quality paths are unused"
    ),
    migration_module!(
        "src/autotune/observation/rolling_window.rs",
        "rolling-window facade preserves old paths during observation extraction",
        "remove once old rolling-window paths are unused"
    ),
    migration_module!(
        "src/autotune/observation/system_context.rs",
        "system-context facade preserves old paths during observation extraction",
        "remove once old system-context paths are unused"
    ),
    migration_module!(
        "src/autotune/observation/target_selection.rs",
        "target-selection facade preserves old paths during observation extraction",
        "remove once old target-selection paths are unused"
    ),
    migration_module!(
        "src/autotune/output/mod.rs",
        "output facade keeps presentation module movement source-compatible",
        "remove once callers import concrete output modules directly"
    ),
    migration_module!(
        "src/autotune/planning/candidate.rs",
        "planning candidate wrapper remains while providers migrate from CandidateAction",
        "remove once providers emit the final planning candidate type directly"
    ),
    migration_module!(
        "src/autotune/runtime/controller.rs",
        "runtime controller facade preserves old paths during runtime split",
        "remove once old runtime controller paths are unused"
    ),
    migration_module!(
        "src/autotune/runtime/emergency_restore.rs",
        "runtime emergency-restore facade preserves old paths during runtime split",
        "remove once old runtime emergency-restore paths are unused"
    ),
    migration_module!(
        "src/autotune/runtime/journal.rs",
        "runtime journal facade preserves old paths during runtime split",
        "remove once old runtime journal paths are unused"
    ),
    migration_module!(
        "src/autotune/runtime/shutdown.rs",
        "runtime shutdown facade preserves old paths during runtime split",
        "remove once old runtime shutdown paths are unused"
    ),
    migration_module!(
        "src/autotune/runtime/startup_recovery.rs",
        "runtime startup-recovery facade preserves old paths during runtime split",
        "remove once old runtime startup-recovery paths are unused"
    ),
    migration_module!(
        "src/events/domain.rs",
        "event domain wrappers remain while raw decoders migrate path by path",
        "remove once callers use final event domain modules directly"
    ),
    migration_module!(
        "src/session.rs",
        "session-stage context remains while tick extraction adopts shared context incrementally",
        "remove once tick extraction owns the context and no local dead code allowance is needed"
    ),
    migration_module!(
        "src/session/ticks/mod.rs",
        "tick-context namespace exists during session stage extraction",
        "remove once tick contexts are fully owned by concrete tick modules"
    ),
];
