use serde::{Deserialize, Serialize};
use stutter_core::ids::{CpuId, Pid, Tid};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpikePoint {
    pub task: Tid,
    pub class: String,
    pub process_pid: Option<Pid>,
    pub comm: String,
    pub cpu: CpuId,
    pub wakeup_target_cpu: CpuId,
    pub latency_ns: u64,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    pub target_pending_wakeups: u32,
    pub observed_runnable_depth: u32,
    pub switch_prev_pid: Tid,
    pub switch_prev_state: i64,
    pub switch_prev_state_label: String,
    pub scx_ops: Option<String>,
    pub primary_cause: Option<String>,
    pub cause_tags: Vec<String>,
}
