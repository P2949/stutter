#![allow(dead_code)] // Transitional eBPF split: map setup migrates from ebpf_loader.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MapSetupPlan {
    pub map_names: Vec<&'static str>,
}
