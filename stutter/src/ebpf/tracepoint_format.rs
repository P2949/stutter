#![allow(dead_code)] // Transitional eBPF split: tracepoint validation migrates from ebpf_loader.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedTracepointField {
    pub name: &'static str,
    pub offset: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExpectedTracepointFormat {
    pub name: &'static str,
    pub fields: &'static [ExpectedTracepointField],
}
