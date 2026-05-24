# eBPF Loader Testing

The real eBPF loader tests are visible in normal test runs, but they skip unless
explicitly enabled. This keeps ordinary development and CI from requiring root,
tracefs, or BPF permissions.

Run the local skip-only check with:

```bash
RUSTUP_TOOLCHAIN=nightly cargo test -p stutter privileged_ -- --nocapture
```

Run the real privileged suite on a Linux system with tracefs and eBPF
permissions:

```bash
STUTTER_RUN_PRIVILEGED_EBPF_TESTS=1 \
RUSTUP_TOOLCHAIN=nightly \
cargo test -p stutter privileged_ -- --nocapture
```

The suite covers default load/attach/drop, load/drop/load again, custom map
sizing, and optional latency probe degradation.

The xtask wrapper builds the BPF object first and then runs the privileged
suite:

```bash
RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- ebpf-smoke
```
