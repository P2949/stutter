#![allow(dead_code, unused_imports)] // Transitional CLI split: parse_app_command remains in cli/mod.rs during staged migration.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

pub(crate) use super::{parse_app_command, parse_app_command_from};
