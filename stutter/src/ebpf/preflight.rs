#![allow(dead_code)] // Transitional eBPF split: preflight reporting migrates from ebpf_loader.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EbpfPreflightReport {
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}
