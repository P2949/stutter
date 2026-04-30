# stutter

`stutter` is a Linux scheduler runnable-latency profiler built with Rust + Aya eBPF.

It measures:

```text
sched_wakeup timestamp -> sched_switch timestamp = runnable latency
```

That answers:

```text
Was this task ready to run but delayed before getting CPU time?
```

## Requirements

* Linux with eBPF support
* Rust stable + nightly
* `rust-src` for nightly
* `bpf-linker`
* privileges to load eBPF programs

Install basics:

```bash
rustup toolchain install stable
rustup toolchain install nightly --component rust-src
cargo install bpf-linker
```

On this project, use the explicit toolchain environment if Gentoo/system Rust interferes:

```bash
RUSTUP_TOOLCHAIN=nightly cargo build
```

## Build

```bash
RUSTUP_TOOLCHAIN=nightly cargo fmt
RUSTUP_TOOLCHAIN=nightly cargo build
RUSTUP_TOOLCHAIN=nightly cargo clippy --all-targets -- -D warnings
```

## Monitor manual PIDs

```bash
RUST_LOG=info RUSTUP_TOOLCHAIN=nightly cargo run -- monitor \
  --pid "$(pgrep -n sway)" \
  --summary-ms 1000
```

Older legacy form still works:

```bash
RUST_LOG=info RUSTUP_TOOLCHAIN=nightly cargo run -- \
  --pid "$(pgrep -n sway)"
```

## Monitor a process tree

Use this for Proton/Wine/game trees:

```bash
RUST_LOG=info RUSTUP_TOOLCHAIN=nightly cargo run -- monitor \
  --tree-pid <root-pid>
```

The monitor periodically scans `/proc`, finds descendant processes, expands each process into `/proc/<pid>/task/<tid>`, and updates the eBPF `TARGET_PIDS` map dynamically.
Add `--exclude-tree-pid <pid>` to subtract a process and all of its descendants from the monitored tree, for example to drop the Steam overlay from an otherwise useful game root.

For launchers where the PID changes between runs, watch for the process name:

```bash
RUST_LOG=info RUSTUP_TOOLCHAIN=nightly cargo run -- monitor \
  --watch-process KingdomCome.exe \
  --persistent \
  --watch-poll-ms 2000 \
  --watch-timeout-seconds 120
```

`--watch-process` scans `/proc` only while waiting for the process to appear or relaunch. Once a process is found, `stutter` follows that root PID and its descendants. `--persistent` requires `--watch-process`; if the watched process exits, stale TIDs are removed and the monitor waits for the next matching launch. Note that `--duration` begins after the watched process is found, not while waiting for it. Missing manual `--pid` targets are dropped with a warning by default; add `--keep-missing-pid` if you want an unknown fallback task retained. `--keep-missing-pid` is intended for manual PID workflows where the PID may appear later; it is not a substitute for tree-root based monitoring and should not be relied on to keep arbitrary tree roots.

## Inspect a tree before tracing

```bash
RUSTUP_TOOLCHAIN=nightly cargo run -- inspect-tree \
  --tree-pid <root-pid>
```

Example expected shape:

```text
root 128848 gamescope [GameScope]
└─ 128879 reaper [Helper]
   └─ 128951 pv-adverb [SteamRuntime]
      ├─ 129004 wineserver [WineServer]
      └─ 129076 KingdomCome.exe [Game]
         ├─ 129096 JobSystem_Worke [Game]
         ├─ 129213 RenderThread [Game]
         └─ ...
```

## Record a run

```bash
RUST_LOG=info RUSTUP_TOOLCHAIN=nightly cargo run -- record \
  --tree-pid <root-pid> \
  --duration 300 \
  --run-name kcd-test
```

Default output:

```text
~/.local/state/stutter/runs/<timestamp>_<run-name>/
  metadata.json
  session.json
  interval.json
  tree_events.json
  spike_events.json
  scx_events.json
  irq_events.json
  gpu_samples.json
  frame_correlation.json
```

Optional correlation inputs:

```bash
RUST_LOG=info RUSTUP_TOOLCHAIN=nightly cargo run -- record \
  --tree-pid <root-pid> \
  --irq-latency --irq 137 \
  --hwmon \
  --mangohud-log /path/to/MangoHud.csv
```

Hardware monitoring notes:

- On multi-GPU systems, prefer an explicit selector such as `--hwmon-drm-card card1`,
  `--hwmon-render-node /dev/dri/renderD129`, or the direct `--hwmon-root /path/to/hwmon`
  override so automatic discovery does not pick the wrong device.
- Frequent hwmon sampling uses cached file descriptors internally; if you see warnings about
  `latency_samples_truncated`, that means stutter is storing a bounded number of exact samples
  and will fall back to histogram-based percentile estimates for p95/p99.

TUI notes:

- The `--tui` flag currently renders a plain-text status summary (non-interactive).
  It prints aggregated counts and a simple bar graph per task class. If you need an
  interactive terminal UI with live histograms, please open an issue or contribute
  to the `tui.rs` module.

## Generate a report

```bash
RUSTUP_TOOLCHAIN=nightly cargo run -- report \
  ~/.local/state/stutter/runs/<run-dir>
```

Show more rows or tighten spike cluster grouping:

```bash
RUSTUP_TOOLCHAIN=nightly cargo run -- report \
  --top 25 \
  --cluster-ms 2 \
  ~/.local/state/stutter/runs/<run-dir>
```

JSON output:

```bash
RUSTUP_TOOLCHAIN=nightly cargo run -- report \
  --json \
  ~/.local/state/stutter/runs/<run-dir>
```

Reports use `spike_events.json` for spike cluster detection when it is present. Older runs without that file still report clusters from retained per-task `top_spikes`.

Generate a self-contained HTML report:

```bash
RUSTUP_TOOLCHAIN=nightly cargo run -- report \
  --html report.html \
  ~/.local/state/stutter/runs/<run-dir>
```

## Apply and restore affinity profiles

Apply a TOML profile to the current process tree:

```bash
RUSTUP_TOOLCHAIN=nightly cargo run -- apply-profile \
  --tree-pid <root-pid> \
  --profile profile.toml
```

By default this is one-shot. Use `--watch` to keep applying the profile to new threads:

```bash
RUSTUP_TOOLCHAIN=nightly cargo run -- apply-profile \
  --tree-pid <root-pid> \
  --profile profile.toml \
  --watch
```

Watch mode restores the original masks on Ctrl-C by default. Add `--keep-applied` to leave the profile active and restore later:

```bash
RUSTUP_TOOLCHAIN=nightly cargo run -- restore
```

Audit a pending restore file without applying it:

```bash
RUSTUP_TOOLCHAIN=nightly cargo run -- restore --dry-run
```

`apply-profile --force` discards an existing restore file. Without `--force`, new records are merged into the existing restore file while preserving the earliest original mask for each TID.

Profile `match_comm` entries use literal substring matching unless the whole pattern is wrapped in `/.../`. For example, `KingdomCome.exe` matches that literal dot, while `/KingdomCome[.]exe$/` is treated as a regex.

## Important interpretation notes

For real stutter diagnosis, prioritize:

```text
max
over_1ms
over_2ms
over_5ms
session_spike
```

If `truncated_samples > 0`, newer recordings estimate `p95` and `p99` from fixed histogram buckets across the full session. The report marks this as:

```text
percentile_scope=histogram
```

Histogram percentiles are approximate bucket upper bounds. `max` and threshold counters remain exact. Older recordings may still show `percentile_scope=capped_prefix`; for those, trust `max` and threshold counters more than p95/p99.

`target_pending_wakeups` is a profiler-side counter of wakeups for monitored tasks, not Linux kernel runqueue depth. It is useful as a rough target-local pressure signal, but do not interpret it as scheduler `rq` depth.

Block I/O overlap is approximate: `io_events.json` correlates request issue/complete by `dev+sector`, not exact request pointer identity, so concurrent same-sector requests can collide.

## Current task classes

```text
Game
GameHelper
Launcher
WineServer
GameScope
Compositor
SteamRuntime
Helper
Unknown
```

Unknown `.exe` processes are no longer automatically treated as critical game tasks. Known launchers/helpers are separated from likely game binaries.

## Performance guidance

- Prefer `--summary-ms` / `--epoch` values that match the latency scale you care about. Shorter summary windows (100-500ms) increase sample frequency and reporter work; longer windows reduce overhead.
- At startup, `stutter` sizes the eBPF wakeup timestamp map and event ring buffer from the effective `RLIMIT_MEMLOCK` and `/proc/meminfo` `MemAvailable`, with conservative caps to avoid hoarding memory on large systems.
- Use `--hwmon` and `--irq-latency` only when you need those signals; sensor reads are cached but still impose blocking syscalls and should be sampled via the monitor's blocking worker.
- If you see significant overhead, lower the monitor sampling rate, drop unneeded `--tree-pid` roots, or narrow task filters to reduce map churn.

## Recording JSON schema (short)

- `metadata.json`: run metadata and CLI flags used.
- `session.json`: per-task summary, histogram buckets, and `percentile_scope` telling whether exact samples were used or histogram-based percentiles.
- `interval.json`: periodic interval summaries used by the `report` command.
- `spike_events.json`: detected spike clusters used for HTML report generation.

For programmatic consumption, inspect the example outputs in a sample run directory created under `~/.local/state/stutter/runs/`.

Note: the CSV exporter is intentionally compact and omits some newer fields. `interval.json` and the other JSON artifacts are the canonical, full-fidelity outputs — they contain PSI samples, major/minor fault deltas, drop counters, and histogram/truncation details. Prefer JSON for programmatic analysis.

## CLI flags (quick reference)

- `--pid <PID>`: add a manual TID/PID to monitor (can repeat).
- `--tree-pid <PID>`: monitor an entire process tree rooted at `PID` (expands into per-task TIDs).
- `--exclude-tree-pid <PID>`: exclude a process subtree from monitored tree roots (can repeat).
- `--watch-process <COMM>`: poll `/proc` for a process whose name/comm matches `<COMM>` and automatically follow its tree; combine with `--persistent` to keep waiting across restarts.
- `--persistent`: use with `--watch-process` to continue monitoring across relaunches.
- `--summary-ms <MS>`: interval for interval summaries written to `interval.json` and printed to the TUI.
- `--epoch <MS>`: explicit reset-and-report mode; prints interval stats with the `epoch` label every `<MS>` and skips the final cumulative session recap.
- `--spike-us <US>`: spike detection threshold in microseconds (e.g., `--spike-us 1000` for 1ms).
- `--alert-threshold-ms <MS>`: send an alert when a runnable-latency spike reaches this threshold. Uses `notify-send` by default.
- `--alert-webhook-url <URL>`: with `--alert-threshold-ms`, POST alert JSON to a webhook instead of using `notify-send`. You can also set `STUTTER_ALERT_WEBHOOK_URL`.
- `--include-comm <PATTERN>` / `--exclude-comm <PATTERN>`: case-insensitive substring filters against task `comm` and process `comm`; exclude wins.
- `--irq-latency`: enable IRQ latency tracing and record `irq_events.json`; at least one explicit `--irq <IRQ>` is required. Inspect `/proc/interrupts` to find the IRQ for your GPU/device.
- `--irq <IRQ>`: add an IRQ number to target for IRQ latency measurement (can repeat).
- `--hwmon`: enable GPU hwmon sampling; combine with `--hwmon-drm-card`, `--hwmon-render-node`, or `--hwmon-root` to avoid ambiguous multi-GPU discovery.
- `--hwmon-root <PATH>`: override hwmon discovery path when automatic detection fails.
- `--hwmon-drm-card <CARD>`: choose a DRM card such as `card0` or `card1` for hwmon discovery.
- `--hwmon-render-node <PATH>`: choose the DRM render node whose device hwmon should be sampled.
- `--mangohud-log <PATH>`: provide a MangoHud CSV to correlate frame times.
- `--tui`: print a plain-text TUI status line periodically (non-interactive).
- `stutter tune --tree-pid <PID> --profiles <FILE>`: apply each profile, keep refreshing it for new threads during the measurement epoch, score interval summaries, and restore after each candidate by default. Candidate run directories are kept next to the tuning summary for auditability. Add `--keep-best` to reapply the best profile at the end.

## What TUI prints (example)

The current `--tui` mode prints a compact status block like:

```
stutter live active_tasks=3 tracked_stats=4 tui_mode=plain_text
class=Game        samples=123     max=12.345ms ################################
class=Helper      samples=42      max=2.100ms  ########
```

The header includes `active_tasks` (number of monitored TIDs) and `tracked_stats` (per-task stats stored). Each `class=...` line shows aggregated `samples` and the observed `max` latency with a small ASCII bar.

## Generated JSON files (overview)

- `metadata.json` — run metadata: CLI flags, `run_name`, timestamps, and schema version.
- `session.json` — per-task summary for the full session. Contains per-task histograms, sample counts, `max`, `percentile_scope` (either `capped_prefix` or `histogram` depending on whether exact samples were retained), and task identity fields (tid, process_pid, comm, class).
- `interval.json` — periodic interval summaries matching `--summary-ms`, used by `report` for time-windowed analysis.
- `spike_events.json` — detected latency spike clusters used by the HTML report and cluster summaries.
- `irq_events.json` — IRQ enter/exit capture when `--irq-latency` is enabled.
- `gpu_samples.json` — periodic hwmon samples when `--hwmon` is enabled.

Note: when present, `cpu_frequency` tracepoint samples are emitted as system-wide telemetry and are not filtered to individual target tasks; treat them as global context rather than per-task signals.

If you need machine-readable schemas, open a recorded run under `~/.local/state/stutter/runs/<run-dir>/` and inspect the files; they are stable across releases but may add fields in minor versions.

## TID reuse detection (what we do)

To reduce false positives when a TID/PID number is recycled by the kernel, `stutter` now combines multiple heuristics:

- Prefer `starttime` fields read from `/proc/[pid]/stat` when available (these are clock-ticks since boot).
- Compare the `/proc/[pid]/exe` file metadata (device + inode) between the previously-observed logical task and the newly-observed PID/TID. If the exe inode differs, we treat it as a different logical task.
- When available, we compare the previously-observed process starttime with the current one to further disambiguate.

These checks reduce collisions compared to relying on `starttime` alone, but in extremely constrained environments (shared containers or odd boot-time adjustments) collisions remain possible. For the most robust detection you can combine exe inode checks with cgroup membership or an explicit profile that targets executable paths.

## License

The userspace crates are dual-licensed MIT OR Apache-2.0.

The eBPF code is dual-licensed MIT OR GPL-2.0.
