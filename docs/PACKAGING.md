# Packaging Guide

## Default Build

Building stutter builds both the userspace binary and the eBPF object locally:

```bash
cargo build --release
```

This requires a Rust nightly toolchain with `rust-src` and the `bpf-linker` tool.

## Prebuilt eBPF Object

Packagers can provide a prebuilt eBPF object to avoid requiring the eBPF build
toolchain during userspace compilation. This is useful for distribution packages,
containers, or CI environments where the eBPF toolchain is not available.

### Build-Time Override

Set `STUTTER_USE_PREBUILT_BPF=1` and `STUTTER_BPF_OBJECT` to embed a prebuilt
object during the userspace build:

```bash
STUTTER_USE_PREBUILT_BPF=1 \
STUTTER_BPF_OBJECT=/usr/lib/stutter/stutter.bpf.o \
cargo build --release
```

When `STUTTER_USE_PREBUILT_BPF=1` is set:

- `STUTTER_BPF_OBJECT` must point to a valid, non-empty eBPF object file.
- The build script copies that object into `OUT_DIR` and skips local eBPF
  compilation entirely.
- No eBPF toolchain, `bpf-linker`, or `rust-src` is required.

If `STUTTER_USE_PREBUILT_BPF=1` is set without `STUTTER_BPF_OBJECT`, the build
will fail with a clear error.

### Runtime Override

Developers can also override the eBPF object at runtime without rebuilding:

```bash
STUTTER_BPF_OBJECT=/usr/lib/stutter/stutter.bpf.o stutter monitor --pid 1234
```

When `STUTTER_BPF_OBJECT` is set at runtime:

- The loader reads the specified file instead of using the embedded object.
- If the file is missing, unreadable, or empty, stutter fails with a clear
  error. It does **not** silently fall back to the embedded object.
- All runtime tracepoint validation remains active and equally strict.

## ABI and Kernel Compatibility

A prebuilt eBPF object must match the userspace ABI — specifically the ring
buffer event structures, map names, and program entry points expected by the
loader. Mismatched objects will fail at load time.

Runtime validation still decides whether the object is compatible with the
current kernel. The loader validates tracepoint formats and patches dynamic
offsets into BPF globals at load time, exactly as it does for locally-built
objects. A prebuilt object that passes validation on one kernel may fail on
another if the kernel's tracepoint layout differs.

> **This is an advanced packaging escape hatch.** For most users, the default
> local build produces the correct object automatically.

## Release Artifacts

The GitHub release workflow automatically uploads the eBPF object alongside the
binary. Packagers can download `stutter.bpf.o` from the release artifacts and
use it with the build-time or runtime override.

## Environment Variable Reference

| Variable | Phase | Description |
|---|---|---|
| `STUTTER_USE_PREBUILT_BPF` | Build | Set to `1` to skip local eBPF compilation and use `STUTTER_BPF_OBJECT` |
| `STUTTER_BPF_OBJECT` | Build / Runtime | Path to a prebuilt eBPF object file |
