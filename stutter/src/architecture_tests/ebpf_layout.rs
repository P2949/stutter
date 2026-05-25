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

fn source_between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start_index = source
        .find(start)
        .unwrap_or_else(|| panic!("failed to find source marker {start}"));
    let source_after_start = &source[start_index..];
    let end_index = source_after_start
        .find(end)
        .unwrap_or_else(|| panic!("failed to find source marker {end}"));
    &source_after_start[..end_index]
}

#[test]
fn ebpf_main_keeps_extracted_layout_helpers_out_of_entrypoint_file() {
    let main = ebpf_source("main.rs");

    assert!(
        main.contains("mod block_io;")
            && main.contains("mod kms_emit;")
            && main.contains("mod trace_offsets;")
            && main.contains("mod trace_read;"),
        "stutter-ebpf/src/main.rs must keep block I/O, KMS event emission, tracepoint offset globals, and field readers in helper modules",
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
        line_count("main.rs") <= 1_550,
        "stutter-ebpf/src/main.rs grew beyond the post-block-I/O-split ceiling; extract another tracepoint family before adding more logic",
    );
    assert!(
        line_count("block_io.rs") <= 200,
        "block_io.rs should stay a focused block request correlation module",
    );
    assert!(
        line_count("kms_emit.rs") <= 120,
        "kms_emit.rs should stay a focused KMS flip event emission module",
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
fn kms_sequence_offsets_uses_exhaustive_typed_provider_and_event_dispatch() {
    let main = ebpf_source("main.rs");
    let offsets = source_between(
        &main,
        "fn kms_sequence_offsets(",
        "\n#[inline(always)]\nfn fill_kms_flip_key",
    );

    assert!(
        main.contains("enum KmsProvider") && main.contains("enum KmsCompletionEvent"),
        "KMS sequence dispatch must classify raw provider and event constants before matching",
    );
    assert!(
        offsets.contains("provider: KmsProvider,")
            && offsets.contains("completion_event: KmsCompletionEvent,"),
        "kms_sequence_offsets must match typed provider and completion-event enums, not raw u32 constants",
    );
    assert!(
        !offsets.contains("_ =>"),
        "kms_sequence_offsets must not use a wildcard arm; future KMS providers/events should force an explicit sequence-offset decision",
    );
    assert!(
        offsets.contains("(KmsProvider::I915, KmsCompletionEvent::Vblank)")
            && offsets.contains("(KmsProvider::Unknown, KmsCompletionEvent::Unknown) => false"),
        "unsupported provider/event combinations must be listed explicitly instead of hidden behind a wildcard",
    );
}

#[test]
fn ebpf_helpers_do_not_return_aggregate_shapes() {
    for relative_path in ["main.rs", "block_io.rs", "kms_emit.rs", "trace_read.rs"] {
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

#[test]
fn ebpf_ringbuf_events_are_emitted_from_struct_literals() {
    for relative_path in ["main.rs", "block_io.rs", "kms_emit.rs"] {
        let source = ebpf_source(relative_path);

        assert!(
            !source.contains("(*event)."),
            "eBPF event emission in {relative_path} must build a complete event value and submit it through emit_ringbuf_event!, not mutate raw event pointers field-by-field",
        );
    }

    let main = ebpf_source("main.rs");
    assert!(
        main.contains("core::ptr::write(entry.as_mut_ptr(), $event)"),
        "emit_ringbuf_event! must keep the single raw ring-buffer write site centralized",
    );
}
