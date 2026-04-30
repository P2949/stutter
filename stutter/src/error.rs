use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StutterError {
    #[error("eBPF load failed: {0}")]
    EbpfLoad(String),
    #[error("tracepoint offset mismatch: {0}")]
    TracepointOffsetMismatch(String),
    #[error("record write failed: {0}")]
    RecordWrite(#[source] io::Error),
}
