#![allow(dead_code)] // Transitional cgroup façade.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CgroupPath(pub PathBuf);
