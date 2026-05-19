#![allow(dead_code)] // Transitional eBPF split: ring/perf buffer setup migrates from ebpf_loader.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EventBufferPlan {
    pub ring_buffer_bytes: u32,
}
