#![allow(unused_imports)] // Transitional system-context facade while old paths are preserved.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

pub(crate) use crate::autotune::system_context::{
    SystemContextSnapshot, SystemContextSnapshotInput, collect_system_context,
};
