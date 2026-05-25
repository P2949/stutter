# eBPF Capacity and Sizing Guide

This document is the central reference for the eBPF capacity constants that are
shared between the BPF object, the userspace loader, and diagnostics. Keep it in
sync when changing `stutter-common/src/lib.rs`, `stutter-ebpf/src/map_limits.rs`,
or `stutter/src/ebpf/maps.rs`.

## Shared ABI constants

| Constant | Location | Default | Controls | Memory impact | Safe adjustment rule |
| --- | --- | ---: | --- | --- | --- |
| `BPF_MAX_TRACKED_CPUS` | `stutter-common/src/lib.rs` | `16_384` | CPU-indexed BPF arrays used for runnable-depth and pending-wakeup accounting. | One slot per tracked CPU for each CPU-indexed array, plus kernel map metadata. | Do not lower below the highest CPU id expected on supported systems. The eBPF object asserts this stays at least `1024`. Out-of-range CPUs are skipped and counted with `DROP_CPU_ACCOUNTING_UNTRACKED`. |
| `BPF_DEFAULT_EVENTS_RINGBUF_BYTES` | `stutter-common/src/lib.rs` | `256 KiB` | Fallback `EVENTS` ring-buffer size baked into the BPF object. | Locked kernel memory equal to the ring-buffer byte size, rounded by kernel/page constraints. | Prefer userspace auto-sizing or `--ringbuf-size-kb`; change the fallback only for loader-bypass scenarios. Keep it inside the userspace clamp. |
| `DROP_COUNTERS_MAX` | `stutter-common/src/lib.rs` | `9` | Number of slots in the shared per-CPU drop-counter ABI. | One `u64` per counter slot per possible CPU, plus kernel map metadata. | Must be greater than the highest `DROP_*` index. Increment it whenever a new drop reason is added. |

Startup diagnostics read `/sys/devices/system/cpu/possible` and warn when the
maximum possible CPU id is outside `BPF_MAX_TRACKED_CPUS`. This is intentionally
a warning rather than a hard failure: runnable-latency events remain safe, while
CPU-indexed runnable-depth and target-pending-wakeup accounting are skipped for
untracked CPU ids and counted through `DROP_CPU_ACCOUNTING_UNTRACKED`.

## BPF object map multipliers

These constants live in `stutter-ebpf/src/map_limits.rs`. They are deliberately
separate from `main.rs` so the tracepoint program stays focused on event logic.

| Constant | Default | Controls | Rationale |
| --- | ---: | --- | --- |
| `TARGET_PIDS_MAP_MAX_ENTRIES` | `1_024` | BPF-side target PID gate. | Mirrors the userspace target cap. |
| `WAKEUP_DATA_PER_TARGET_MULTIPLIER` | `128` | Baked wakeup-state slots per target slot. | Wakeup records churn faster than target slots and can be replaced before `sched_switch` consumes them. |
| `RUNNABLE_TASK_CPU_PER_TARGET_MULTIPLIER` | `64` | Runnable CPU-state slots per target slot. | Tracks target TID churn and migration bookkeeping, not CPU count. |
| `PREV_FAULTS_PER_TARGET_MULTIPLIER` | `128` | Previous page-fault counter slots per target slot. | Mirrors wakeup headroom so page-fault deltas are not the first state lost under churn. |

The derived BPF object capacities are:

```text
WAKEUP_DATA_MAP_MAX_ENTRIES       = TARGET_PIDS_MAP_MAX_ENTRIES * WAKEUP_DATA_PER_TARGET_MULTIPLIER
RUNNABLE_TASK_CPU_MAP_MAX_ENTRIES = TARGET_PIDS_MAP_MAX_ENTRIES * RUNNABLE_TASK_CPU_PER_TARGET_MULTIPLIER
PREV_FAULTS_MAP_MAX_ENTRIES       = TARGET_PIDS_MAP_MAX_ENTRIES * PREV_FAULTS_PER_TARGET_MULTIPLIER
```

Do not reduce these multipliers without also updating the drop-counter model,
regression fixtures, and operator guidance. A lower multiplier can make map
pressure look like scheduler latency loss because the BPF program will stop
retaining the state needed to emit complete events.

## Default BPF object capacities and loader-time overrides

The default values live in `stutter-common/src/ebpf_capacity.rs` and are aliased
by `stutter-ebpf/src/map_limits.rs` for the BPF object. Userspace can override
the maps below before load; changes require restarting or reloading the monitor
because BPF map capacities are fixed when the object is loaded.

| Map | Default | Config-file field | CLI field | Increase when | Risk of increasing |
| --- | ---: | --- | --- | --- | --- |
| `EVENTS` | `256 KiB` fallback, dynamically sized | `ebpf_sizing.ringbuf_size_kb` | `--ebpf-ringbuf-size-kb` | Ring-buffer reserve drops appear during bursty recordings. | More locked kernel memory. |
| `WAKEUP_DATA`, `WAKEUP_CONSUMED` | dynamically sized | `ebpf_sizing.wakeup_map_factor` | `--ebpf-wakeup-map-factor` | Wakeup insert/replacement drops appear under target churn. | More locked kernel memory across both wakeup maps. |
| `TARGET_PIDS` | `1_024` | `ebpf_sizing.target_pids_entries` | none | `target.max_tasks` needs to grow for very large monitored process trees. | Larger target gate and larger derived runnable/fault maps. |
| `TARGET_CGROUP_IDS` | `64` | `ebpf_sizing.target_cgroup_ids_entries` | none | Native cgroup filtering is enabled and many cgroup ids must be allowed. | Extra locked memory for a currently experimental path. |
| `TARGET_IRQS` | `64` | `ebpf_sizing.target_irqs_entries` | none | Monitoring many explicit IRQs. | Extra locked memory for the IRQ allowlist. |
| `RUNNABLE_TASK_CPU` | `target_pids_entries * 64` | `ebpf_sizing.runnable_task_cpu_factor` | none | Runnable CPU-state churn causes incomplete migration/wakeup accounting. | More locked memory in an LRU map keyed by TID. |
| `PREV_FAULTS` | `target_pids_entries * 128` | `ebpf_sizing.prev_faults_factor` | none | Page-fault delta tracking loses state under target churn. | More locked memory in a hash map keyed by TID. |
| `IRQ_START_TIMES` | `1_024` | `ebpf_sizing.irq_start_entries` | none | IRQ entry/exit pairing drops during high IRQ concurrency. | Extra locked memory for short-lived IRQ correlation records. |
| `BLOCK_START` | `16_384` | `ebpf_sizing.block_start_entries` | `--ebpf-block-start-entries` | Block I/O start insert drops or fallback-key collisions appear. | More locked memory; stale in-flight block requests retain slots longer. |
| `KMS_FLIP_STARTS` | `4_096` | `ebpf_sizing.kms_flip_start_entries` | none | KMS request/done matching loses starts under heavy display activity. | More locked memory for display correlation. |
| `FENCE_WAIT_STARTS` | `4_096` | `ebpf_sizing.drm_fence_wait_start_entries` | `--ebpf-drm-fence-wait-start-entries` | Fence wait intervals lose wait-start records. | More locked memory for fence wait correlation. |
| `FENCE_SIGNAL_TIMES` | `4_096` | `ebpf_sizing.drm_fence_signal_entries` | `--ebpf-drm-fence-signal-entries` | Wait/signal correlation misses recent signal records. | More locked memory for recent fence signal history. |

Example:

```toml
[ebpf_sizing]
ringbuf_size_kb = 1024
wakeup_map_factor = 64
target_irqs_entries = 256
block_start_entries = 32768
drm_fence_wait_start_entries = 8192
drm_fence_signal_entries = 8192
```

## Userspace dynamic sizing clamps

These constants live in `stutter/src/ebpf/maps.rs` and apply when the loader
sizes maps before loading the BPF object.

| Constant | Default | Controls | Tuning surface |
| --- | ---: | --- | --- |
| `MIN_EVENTS_RINGBUF_BYTES` | `64 KiB` | Minimum event ring-buffer size. | Lower bound for automatic sizing and `--ringbuf-size-kb`. |
| `MAX_EVENTS_RINGBUF_BYTES` | `16 MiB` | Maximum event ring-buffer size. | Upper bound for automatic sizing and `--ringbuf-size-kb`. |
| `MIN_WAKEUP_DATA_ENTRIES` | `4_096` | Minimum wakeup-map entries after userspace sizing. | Lower bound for automatic sizing and `--wakeup-map-factor`. |
| `MAX_WAKEUP_DATA_ENTRIES` | `1_048_576` | Maximum wakeup-map entries after userspace sizing. | Upper bound for automatic sizing and `--wakeup-map-factor`. |
| `WAKEUP_DATA_MAP_ENTRY_BUDGET_BYTES` | `128` | Conservative userspace budget per logical wakeup slot. | Includes both wakeup maps, kernel metadata, alignment, and margin. |

The automatic sizing budget ratios are intentionally conservative:

| Constant | Default | Controls | Rationale |
| --- | ---: | --- | --- |
| `DEFAULT_AVAILABLE_MEMORY_BYTES` | `1 GiB` | Fallback memory input when MemAvailable is unknown. | Avoids assuming unlimited memory on unusual systems. |
| `AVAILABLE_MEMORY_BUDGET_DIVISOR` | `64` | Fraction of MemAvailable eligible for automatic eBPF map sizing. | Caps automatic sizing at 1/64 of available memory; manual overrides remain available after observing drops. |
| `MEMLOCK_BUDGET_NUMERATOR` / `MEMLOCK_BUDGET_DENOMINATOR` | `3 / 4` | Fraction of finite RLIMIT_MEMLOCK eligible for stutter maps. | Leaves margin for page rounding, other BPF objects, and kernel accounting overhead. |
| `EVENTS_BUDGET_NUMERATOR` / `EVENTS_BUDGET_DENOMINATOR` | `2 / 5` | Share of the computed budget reserved for the ring buffer. | Keeps roughly 40% for bursty event delivery and leaves the rest for wakeup/cursor maps. |

Automatic sizing uses a conservative fraction of available memory and memlock.
Manual overrides are escape hatches for recordings that show drops or map
pressure:

```bash
stutter monitor --pid 1234 --ebpf-ringbuf-size-kb 8192
stutter monitor --pid 1234 --ebpf-wakeup-map-factor 4
```

Use larger ring buffers for bursty event loss. Use a larger wakeup-map factor
for target churn or wakeup insert failures. Both consume locked kernel memory,
so prefer the smallest value that eliminates the observed drop counter.

## Change checklist

When changing any capacity value:

1. Update the constant and its rustdoc.
2. Update this document.
3. Update userspace validation/clamps if the public tuning range changed.
4. Update drop-counter decoding if the change adds, removes, or renumbers a
   `DROP_*` counter.
5. Run formatting, tests, clippy, and documentation rendering:

   ```bash
   RUSTUP_TOOLCHAIN=nightly cargo fmt --all
   RUSTUP_TOOLCHAIN=nightly cargo test --all
   RUSTUP_TOOLCHAIN=nightly cargo clippy --all-targets -- -D warnings
   RUSTUP_TOOLCHAIN=nightly cargo doc --workspace --no-deps
   ```
