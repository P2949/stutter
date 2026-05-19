#![allow(dead_code)] // Transitional eBPF split: attach logic migrates from ebpf_loader.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AttachPlan {
    pub tracepoints: Vec<&'static str>,
}
