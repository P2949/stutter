use serde::{Deserialize, Serialize};

use super::{Diagnosis, SpikeClusterSource, SpikePoint, WakeGraphEdge};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpikeClusterAnalysis {
    pub source: SpikeClusterSource,
    pub source_count: usize,
    pub clusters: Vec<SpikeCluster>,
}

pub const MIN_CLUSTER_TASKS: usize = 3;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpikeCluster {
    pub points: Vec<SpikePoint>,
    pub distinct_tasks: usize,
    pub min_switch_ns: u64,
    pub max_switch_ns: u64,
    pub max_latency_ns: u64,
    pub diagnosis: Option<Diagnosis>,
    pub wake_graph: Vec<WakeGraphEdge>,
}
