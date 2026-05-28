use serde::{Deserialize, Serialize};
use stutter_core::ids::Tid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WakeGraphEdge {
    pub waker_tid: Tid,
    pub waker_comm: String,
    pub wakee_tid: Tid,
    pub wakee_comm: String,
    pub count: u64,
    pub max_latency_ns: u64,
}
