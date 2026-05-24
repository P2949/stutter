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

#[test]
fn action_restore_raw_ids_are_explicit_serialization_boundary_fields() {
    let actions_model = source("actions/model.rs");
    assert!(
        actions_model.contains("JSON rollback-token serialization boundaries"),
        "actions/model.rs raw task IDs must carry an explicit serialization-boundary note"
    );

    let expected_structs = [
        "pub struct TaskIdentity {\n    pub tid: u32,",
        "pub struct TaskRestoreIdentity {\n    pub tid: u32,",
        "pub struct NiceRestoreRecord {\n    #[serde(default)]\n    pub tid: u32,",
        "pub struct UclampRestoreRecord {\n    #[serde(default)]\n    pub tid: u32,",
        "pub struct IoPrioRestoreRecord {\n    #[serde(default)]\n    pub tid: u32,",
        "pub struct CgroupRestoreRecord {\n    #[serde(default)]\n    pub tid: u32,",
    ];

    for expected in expected_structs {
        assert!(
            actions_model.contains(expected),
            "actions/model.rs raw `pub tid: u32` is allowed only in known rollback-token serialization-boundary structs; missing expected marker: {expected}"
        );
    }

    assert_eq!(
        actions_model.matches("pub tid: u32").count(),
        expected_structs.len(),
        "new raw `pub tid: u32` fields in actions/model.rs need an explicit typed-ID or serialization-boundary decision"
    );
}

#[test]
fn autotune_observation_task_snapshots_use_typed_ids() {
    let observation = source("autotune/observation.rs");
    assert!(
        observation.contains("use stutter_core::ids::{Pid, Tid};")
            && observation.contains("pub struct ProtectedTask")
            && observation.contains("pub struct ActiveTaskSnapshot")
            && observation.matches("pub tid: Tid").count() >= 2
            && observation.matches("pub process_pid: Pid").count() >= 2,
        "autotune observation task snapshots must keep internal task/process IDs typed and convert to raw u32 only at external boundaries",
    );
}
