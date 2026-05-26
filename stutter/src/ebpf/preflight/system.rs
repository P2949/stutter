use std::path::Path;

use super::cpu::{CPU_POSSIBLE_PATH, cpu_tracking_limit_warning_from_possible_path};

pub(super) fn push_system_warnings(warnings: &mut Vec<String>) {
    if let Some(warning) =
        cpu_tracking_limit_warning_from_possible_path(Path::new(CPU_POSSIBLE_PATH))
    {
        warnings.push(warning);
    }
}

pub(super) fn log_system_warnings() {
    if let Some(warning) =
        cpu_tracking_limit_warning_from_possible_path(Path::new(CPU_POSSIBLE_PATH))
    {
        log::warn!("{warning}");
    }
}
