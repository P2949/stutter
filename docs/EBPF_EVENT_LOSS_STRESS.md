# eBPF Event-Loss Stress Recipe

Use this recipe after changes to eBPF maps, ring-buffer sizing, wakeup handling,
or userspace event ingestion. It is intentionally empirical and should be run on
a Linux machine where eBPF loading is permitted.

## Setup

Build the normal userspace binary and BPF object first:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo build -p stutter
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- preflight
```

Create a high-churn target tree:

```bash
scripts/smoke/sleep_tree_churn.sh
```

If you need a manual target PID, run a shell loop that repeatedly forks short
tasks and export the parent PID:

```bash
(while true; do sleep 0.01 & wait $!; done) &
export PID=$!
```

## Small Ring Buffer

Force a small ring buffer to confirm drop counters and data-quality warnings are
visible:

```bash
sudo stutter monitor --duration 10s --target-pid "$PID" --ebpf-ringbuf-size-kb 64
```

Expected result: if event pressure exceeds the small ring buffer, reports should
show nonzero drop counters or degraded data quality instead of silent success.

## Large Ring Buffer

Repeat with a larger buffer to verify losses disappear or materially decrease:

```bash
sudo stutter monitor --duration 10s --target-pid "$PID" --ebpf-ringbuf-size-kb 4096
```

Expected result: `DROP_RINGBUF_RESERVE_FAILED` should be zero or lower than the
small-buffer run under the same workload.

## Many Target Threads

Stress map sizing with many target tasks:

```bash
sudo stutter monitor --duration 10s --tree-pid "$PID" --target-max-tasks 1024
```

Expected result: wakeup-data, runnable-task, and previous-fault maps should not
drop state under the configured target count. If they do, increase the relevant
map factor and rerun.

## CPU ID Accounting Check

Check CPU id coverage before interpreting runnable-depth data:

```bash
cat /sys/devices/system/cpu/possible
sudo stutter doctor
```

If the maximum possible CPU id is outside `BPF_MAX_TRACKED_CPUS`, reports should
warn and count skipped CPU accounting with `DROP_CPU_ACCOUNTING_UNTRACKED`.

## Evidence to Save

Save both run directories and note:

- `metadata.json` and `session.json`
- `interval.json`
- `spike_events.json`
- nonzero drop counters
- tracepoint preflight warnings
- kernel version and `/sys/devices/system/cpu/possible`
