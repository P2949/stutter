#![allow(dead_code)] // Transitional eBPF split: capability probing migrates from ebpf_loader.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EbpfCapabilityReport {
    pub btf_available: bool,
    pub tracefs_available: bool,
}
