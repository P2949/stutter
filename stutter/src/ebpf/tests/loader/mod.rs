//! Generic tests for eBPF loader configuration and object loading behavior.
//!
//! Owns loader regression tests and test-only fixtures. Does not own production object loading,
//! tracepoint attach, map sizing, or preflight logic.

use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use stutter_common::tracepoint_offsets::{TracepointFieldSpec, TracepointName};

// tokio::time::sleep removed as unused
use super::*;
use crate::ebpf::load::{map_init_context, missing_map_context};
fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    dir
}

mod tracepoint_parser;

mod block_request_key;

mod tracepoint_validation;

mod irq_tracepoints;

mod preflight;

mod map_sizing;

mod bpf_object;
