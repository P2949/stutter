#![allow(unused_imports)] // Transitional system façade while root low-level readers migrate.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

pub(crate) mod cgroup;
pub(crate) mod command;
pub(crate) mod hwmon;
pub(crate) mod irq;
pub(crate) mod perf;
pub(crate) mod procfs;
pub(crate) mod psi;
pub(crate) mod sched;
pub(crate) mod sysfs;
pub(crate) mod topology;
