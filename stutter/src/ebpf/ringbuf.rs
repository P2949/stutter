#![allow(dead_code)] // Transitional eBPF split: ring/perf buffer setup migrates from ebpf_loader.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EventBufferPlan {
    pub ring_buffer_bytes: u32,
}
