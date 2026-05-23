//! Architecture tests for the eBPF crate layout.
//!
//! These checks keep the tracepoint entrypoint file from silently absorbing
//! more unrelated responsibilities after helper modules have been extracted.

use std::{fs, path::PathBuf};

fn ebpf_src_root() -> PathBuf {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = crate_root
        .parent()
        .unwrap_or_else(|| panic!("stutter crate should have a workspace parent"));
    workspace_root.join("stutter-ebpf").join("src")
}

fn line_count(relative_path: &str) -> usize {
    fs::read_to_string(ebpf_src_root().join(relative_path))
        .unwrap_or_else(|err| panic!("failed to read eBPF source {relative_path}: {err}"))
        .lines()
        .count()
}

fn ebpf_source(relative_path: &str) -> String {
    fs::read_to_string(ebpf_src_root().join(relative_path))
        .unwrap_or_else(|err| panic!("failed to read eBPF source {relative_path}: {err}"))
}

#[test]
fn ebpf_main_keeps_extracted_layout_helpers_out_of_entrypoint_file() {
    let main = ebpf_source("main.rs");

    assert!(
        main.contains("mod trace_offsets;") && main.contains("mod trace_read;"),
        "stutter-ebpf/src/main.rs must keep tracepoint offset globals and field readers in helper modules",
    );
    assert!(
        !main.contains("static mut BLOCK_RQ_KEY_OFFSET"),
        "tracepoint offset tunables belong in trace_offsets.rs, not main.rs",
    );
    assert!(
        !main.contains("fn read_sequence_field"),
        "tracepoint field readers belong in trace_read.rs, not main.rs",
    );
    assert!(
        line_count("main.rs") <= 1_650,
        "stutter-ebpf/src/main.rs grew beyond the post-split ceiling; extract another tracepoint family before adding more logic",
    );
    assert!(
        line_count("trace_offsets.rs") <= 160,
        "trace_offsets.rs should stay a small offset/export table",
    );
    assert!(
        line_count("trace_read.rs") <= 100,
        "trace_read.rs should stay a small tracepoint field-reader module",
    );
}

#[test]
fn ebpf_helpers_do_not_return_aggregate_shapes() {
    for relative_path in ["main.rs", "trace_read.rs"] {
        let source = ebpf_source(relative_path);

        assert!(
            !source.contains("-> Result<"),
            "eBPF helper functions in {relative_path} must not return Result; bpf-linker rejects aggregate returns",
        );
        assert!(
            !source.contains("-> Option<"),
            "eBPF helper functions in {relative_path} must not return Option; use bool plus out-parameters instead",
        );
    }
}
