//! Architecture tests for gradual typed identifier adoption.

use std::fs;

use super::crate_src_root;

fn source(relative_path: &str) -> String {
    fs::read_to_string(crate_src_root().join(relative_path))
        .unwrap_or_else(|err| panic!("failed to read {relative_path}: {err}"))
}

#[test]
fn core_typed_ids_are_load_bearing_in_process_and_metrics_models() {
    let process_model = source("process/model.rs");
    assert!(
        process_model.contains("use stutter_core::ids::{Pid, Tid};")
            && process_model.contains("pub tid: Tid")
            && process_model.contains("pub process_pid: Pid")
            && process_model.contains("pub process_ppid: Pid")
            && process_model.contains("pub fn task_id(&self) -> Tid")
            && process_model.contains("pub fn process_id(&self) -> Pid")
            && process_model.contains("pub fn parent_process_id(&self) -> Pid"),
        "TaskInfo must store task/process identifiers as stutter-core typed IDs, not raw u32 plus accessor wrappers",
    );

    let metrics = source("metrics.rs");
    assert!(
        metrics.contains("use stutter_core::ids::{CpuId, Pid, Tid};")
            && metrics.contains("pub task: Tid")
            && metrics.contains("pub process_pid: Option<Pid>")
            && metrics.contains("pub fn cpu_id(&self) -> CpuId")
            && metrics.contains("pub fn task_id(&self) -> Tid")
            && metrics.contains("pub fn process_id(&self) -> Option<Pid>"),
        "TaskStats must store task/process identifiers as stutter-core typed IDs while converting to raw u32 only at external DTO/eBPF boundaries",
    );
}

#[test]
fn process_comm_runtime_models_no_longer_depend_on_serializable_arc_str() {
    for relative_path in [
        "process/model.rs",
        "metrics.rs",
        "recorder/event_types.rs",
        "recorder/session_files.rs",
    ] {
        let source = source(relative_path);
        assert!(
            !source.contains("Arc<str>") && !source.contains("std::sync::Arc::from"),
            "{relative_path} must keep process_comm as owned strings at persistence and process-model boundaries",
        );
    }
}
