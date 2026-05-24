//! Focused eBPF loader module namespace.
//!
//! The current loader façade remains `crate::ebpf_loader` while internals migrate here.

pub(crate) mod attach;
pub(crate) mod errors;
pub(crate) mod load;
pub(crate) mod load_plan;
pub(crate) mod maps;
pub(crate) mod memlock;
pub(crate) mod memory;
pub(crate) mod model;
pub(crate) mod object;
pub(crate) mod preflight;
pub(crate) mod tracepoint_format;
pub(crate) mod tracepoints;

pub(crate) use errors::EbpfLoadError;
