#![allow(dead_code)] // Transitional procfs reader trait while process_tree I/O splits.

use crate::{error::ProcfsError, process_tree::TaskInfo};

pub(crate) trait ProcfsReader {
    fn read_task(&self, tid: u32) -> Result<TaskInfo, ProcfsError>;
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LinuxProcfsReader;
