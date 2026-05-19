#![allow(dead_code)] // Transitional eBPF split: loader façade still owns object loading.

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(crate) struct EbpfObjectLoadInput {
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub(crate) struct EbpfObjectLoadOutput {
    pub source: EbpfObjectSource,
    pub bytes_len: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EbpfObjectSource {
    Embedded,
    External(PathBuf),
}
