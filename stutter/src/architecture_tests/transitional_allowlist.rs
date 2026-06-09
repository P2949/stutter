//! Inventory of temporary migration markers; scanner policy lives in `transitional`.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::architecture_tests) struct MigrationModuleAllowance {
    pub(in crate::architecture_tests) path: &'static str,
    pub(in crate::architecture_tests) reason: &'static str,
    pub(in crate::architecture_tests) exit_criteria: &'static str,
}

pub(in crate::architecture_tests) const MAX_MIGRATION_MARKER_MODULES: usize = 0;

pub(in crate::architecture_tests) const MIGRATION_MODULE_ALLOWLIST: &[MigrationModuleAllowance] =
    &[];
