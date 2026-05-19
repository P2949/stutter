#![allow(dead_code)] // Transitional cgroup façade.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CgroupPath(pub PathBuf);
