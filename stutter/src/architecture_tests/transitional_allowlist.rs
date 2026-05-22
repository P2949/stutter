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

pub(in crate::architecture_tests) const MAX_MIGRATION_MARKER_MODULES: usize = 3;

pub(in crate::architecture_tests) const MIGRATION_MODULE_ALLOWLIST: &[MigrationModuleAllowance] = &[
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
