# stutter

`stutter` is a Linux scheduler runnable-latency profiler built with Rust + Aya eBPF.

It measures:

```text
sched_wakeup timestamp -> sched_switch timestamp = runnable latency
````

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
```

## Generate a report

```bash
RUSTUP_TOOLCHAIN=nightly cargo run -- report \
  ~/.local/state/stutter/runs/<run-dir>
```

JSON output:

```bash
RUSTUP_TOOLCHAIN=nightly cargo run -- report \
  --json \
  ~/.local/state/stutter/runs/<run-dir>
```

## Important interpretation notes

For real stutter diagnosis, prioritize:

```text
max
over_1ms
over_2ms
over_5ms
session_spike
```

If `truncated_samples > 0`, then `p95` and `p99` are based only on the stored exact sample window. The report marks this as:

```text
percentile_scope=capped_prefix
```

When capped, trust `max` and threshold counters more than p95/p99.

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

## License

The userspace crates are dual-licensed MIT OR Apache-2.0.

The eBPF code is dual-licensed MIT OR GPL-2.0.
