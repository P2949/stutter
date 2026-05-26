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
RUSTUP_TOOLCHAIN=nightly cargo xtask ebpf-smoke
```

## Release Privileged Smoke Recipe

Before a release or after eBPF/action-facing refactors, run a real-machine smoke
pass in addition to the skip-aware CI tests:

```bash
RUSTUP_TOOLCHAIN=nightly cargo xtask privileged-ebpf-smoke
sudo stutter doctor
sudo stutter monitor --duration 5s --target-pid "$PID"
```

Use a short-lived target process for `$PID`, such as a shell `sleep` command or a
known game/test workload. Save the generated run directory if any probe reports
missing tracepoints, nonzero drop counters, or degraded evidence.
