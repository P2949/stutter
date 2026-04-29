use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum StutterError {
    #[error("eBPF load failed: {0}")]
    EbpfLoad(String),
    #[error("eBPF map is full: {0}")]
    MapFull(String),
    #[error("tracepoint offset mismatch: {0}")]
    TracepointOffsetMismatch(String),
    #[error("affinity denied")]
    AffinityDenied(#[source] io::Error),
    #[error("record write failed")]
    RecordWrite(#[source] io::Error),
    #[error("profile parse failed: {0}")]
    ProfileParse(String),
}
