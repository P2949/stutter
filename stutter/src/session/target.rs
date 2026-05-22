#![allow(unused_imports)] // Transitional target-stage facade while targeting.rs remains the active implementation.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

pub(crate) use super::targeting::{TargetController, TargetPolicy};
