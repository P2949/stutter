//! Focused eBPF loader module namespace.
//!
//! The current loader façade remains `crate::ebpf_loader` while internals migrate here.

pub(crate) mod attach;
pub(crate) mod capabilities;
pub(crate) mod errors;
pub(crate) mod maps;
pub(crate) mod object;
pub(crate) mod preflight;
pub(crate) mod ringbuf;
pub(crate) mod tracepoint_format;

pub(crate) use errors::EbpfLoadError;
