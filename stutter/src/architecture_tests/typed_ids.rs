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
fn action_restore_models_use_typed_ids_at_json_boundaries() {
    let actions_model = source("actions/model.rs");
    assert!(
        actions_model.contains("pub use stutter_core::ids::{ActionId, Pid, Tid};"),
        "actions/model.rs must import typed process/task IDs for rollback-token identity models"
    );

    for expected in [
        "pub struct TaskIdentity {\n    pub tid: Tid,",
        "pub struct TaskIdentity {\n    pub tid: Tid,\n    pub process_pid: Option<Pid>,",
        "pub struct TaskRestoreIdentity {\n    pub tid: Tid,",
        "pub struct TaskRestoreIdentity {\n    pub tid: Tid,\n    #[serde(default)]\n    pub process_pid: Option<Pid>,",
        "pub struct NiceRestoreRecord {\n    #[serde(default = \"zero_tid\")]\n    pub tid: Tid,",
        "pub struct UclampRestoreRecord {\n    #[serde(default = \"zero_tid\")]\n    pub tid: Tid,",
        "pub struct IoPrioRestoreRecord {\n    #[serde(default = \"zero_tid\")]\n    pub tid: Tid,",
        "pub struct CgroupRestoreRecord {\n    #[serde(default = \"zero_tid\")]\n    pub tid: Tid,",
    ] {
        assert!(
            actions_model.contains(expected),
            "actions/model.rs rollback identity models must use typed IDs while preserving numeric serde shape; missing expected marker: {expected}"
        );
    }

    assert!(
        !actions_model.contains("pub tid: u32")
            && !actions_model.contains("pub process_pid: Option<u32>"),
        "actions/model.rs must not expose raw public task/process ID fields; convert to u32 only at procfs/syscall or persisted compatibility accessors"
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

#[test]
fn remaining_task_process_identity_models_use_typed_ids() {
    for relative_path in [
        "focus/classify.rs",
        "affinity.rs",
        "profile_restore.rs",
        "autotune/washout.rs",
        "profiles.rs",
    ] {
        let source = source(relative_path);
        assert!(
            !source.contains("pub tid: u32")
                && !source.contains("pub process_pid: u32")
                && !source.contains("pub process_pid: Option<u32>")
                && !source.contains("pub pid: u32")
                && !source.contains("pub ppid: u32"),
            "{relative_path} must use Tid/Pid for public or persisted task/process identity fields"
        );
    }
}

#[test]
fn recorder_artifact_task_process_ids_use_typed_ids_but_common_abi_stays_raw() {
    let recorder = source("recorder/event_types.rs");
    assert!(
        recorder.contains("use stutter_core::ids::{Pid, Tid};")
            && recorder.contains("pub tid: Tid")
            && recorder.contains("pub process_pid: Pid")
            && recorder.contains("pub process_pid: Option<Pid>")
            && recorder.contains("pub task: Tid")
            && recorder.contains("pub waker_tid: Tid")
            && recorder.contains("pub client_pid: Option<Pid>"),
        "recorder artifact DTOs should keep Rust-side task/process IDs typed while serde preserves numeric JSON"
    );
}
