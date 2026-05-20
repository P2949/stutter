//! Community-rules facade.
//!
//! Owns the module boundary and stable crate-private re-exports for community rule loading,
//! command handling, classification, and data models.

pub mod importer;
pub mod loader;
pub mod paths;

pub mod classify;
pub mod commands;
pub mod db;
pub mod import;
pub mod load;
pub mod model;
pub mod normalize;
pub mod render;

#[cfg(test)]
pub(crate) use classify::classify_process_identity;
pub(crate) use classify::{
    CommunityProcessIdentity, classify_process_identity_with_db, rule_requires_context,
};
pub use commands::rules_command;
#[cfg(test)]
pub(crate) use commands::{
    rules_check_generated_command, rules_check_source_command, rules_import_command,
};
pub(crate) use db::CommunityRulesDb;
#[cfg(test)]
pub(crate) use importer::ImportReport;
#[cfg(test)]
pub(crate) use load::load_community_rules;
pub(crate) use load::{
    load_community_rules_db, load_community_rules_file, load_community_rules_status,
};
pub(crate) use loader::load_rules_file;
pub(crate) use model::{
    CommunityRule, CommunityRulesConfig, CommunityRulesFile, CommunityRulesMetadataFile,
    CommunityRulesSource, CommunityRulesSourceKind, CommunityRulesStatus,
};
pub(crate) use normalize::{is_guarded_community_rule_name, normalize_process_name};
pub(crate) use paths::{default_community_rules_dir, default_user_rules_dir};
#[cfg(test)]
pub(crate) use render::render_import_report;

#[cfg(test)]
#[path = "community_rules/tests/commands.rs"]
mod rules_command_tests;

#[cfg(test)]
#[path = "community_rules/tests/classification.rs"]
mod tests;
