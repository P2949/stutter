#![allow(unused_imports)] // Transitional import namespace around the existing importer module.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

pub(crate) mod parse;
pub(crate) mod report;
pub(crate) mod validate;

pub(crate) use crate::community_rules::importer::{
    ImportInput, ImportedCommunityRules, import_ananicy_rules,
};
