#![allow(dead_code)]

pub struct MonitorRuntime;

impl MonitorRuntime {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MonitorRuntime {
    fn default() -> Self {
        Self::new()
    }
}
