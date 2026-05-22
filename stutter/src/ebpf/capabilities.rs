#![allow(dead_code)] // Transitional eBPF split: capability probing migrates from ebpf_loader.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EbpfCapabilityReport {
    pub btf_available: bool,
    pub tracefs_available: bool,
}
