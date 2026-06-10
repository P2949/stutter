# stutter

[![CI](https://github.com/P2949/stutter/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/P2949/stutter/actions/workflows/ci.yml?query=branch%3Amain)

`stutter` is a Linux scheduler runnable-latency profiler built with Rust + Aya eBPF.

For supervisor review, start with [SUPERVISOR_README.md](SUPERVISOR_README.md).
The proposed FYP scope is CPU-affinity/process-placement validation for
Linux/Proton game frame pacing; this root README is the technical
user/developer manual, not the first pitch. The full raw KCD1 archive is
published as a release asset, while the normal Git checkout keeps only the
source tree, polished reports, and curated evidence bundle.

## Contents

- [Requirements](#requirements)
- [Build](#build)
- [Install](#install)
- [Recommended workflow](#recommended-workflow)
- [Recording and benchmarking](#record-a-run)
- [Reports](#generate-a-report)
- [Tuning and recommendations](#tune-and-recommend-profiles)
- [Daemon and autotune modes](#daemon-and-autotune-modes)
- [Doctor / preflight](#doctor--preflight)
- [Applying and restoring affinity profiles](#apply-and-restore-affinity-profiles)
- [Interpretation notes](#important-interpretation-notes)
- [CLI flags](#cli-flags-quick-reference)
- [Generated JSON files](#generated-json-files-overview)
- [License](#license)

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
* Rust via `rustup`
* `bpf-linker`
* privileges to load eBPF programs

The repository pins its expected Rust toolchain in `rust-toolchain.toml`:

```toml
[toolchain]
channel = "nightly-2026-06-06"
components = ["rust-src", "rustfmt", "clippy"]
```

Install basics:

```bash
rustup show
cargo install bpf-linker
```

`rustup` will use the repository-pinned nightly toolchain when commands are run from this checkout. If a system Rust setup interferes, use an explicit override:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo build
```

## Build

```bash
cargo fmt
cargo build
cargo clippy --all-targets -- -D warnings
```

Check the build identity reported by the binary:

```bash
cargo run -- --version
```

The version output includes the Cargo package version and the Git revision when the repository metadata is available. Builds from source archives without `.git` report `git unknown`.

## Install

For a local technical-user install:

```bash
scripts/install-local.sh
```

This installs `stutter` under `~/.local/bin` by default. It does not install setuid bits or Linux capabilities. See [docs/INSTALL.md](docs/INSTALL.md) for uninstall and user-service notes.

### User config file

`stutter monitor` can read default options from:

```text
~/.config/stutter/config.toml
```

Override the path with:

```bash
STUTTER_CONFIG=/path/to/config.toml stutter monitor ...
```

Example:

```toml
summary_ms = 500
spike_us = 1000
hwmon = true
cpu_freq = true
include_comm = ["Game", "/Render.*/"]
exclude_comm = ["steamwebhelper"]
max_tasks = 256
retain_intervals = 120
```

Precedence is:

```text
built-in defaults < config file < CLI arguments
```

Target selection stays CLI-only; the config file does not set target PIDs or cgroups.

Foreground-window context can also be configured explicitly:

```toml
foreground_window = true
focus_source = "hybrid"
foreground_source = "auto"
foreground_poll_ms = 1000
foreground_max_stale_ms = 2500
foreground_include_title = false
live_diagnosis_cluster_window_ms = 5

[mangohud]
log_live = true
tail_idle_sleep_ms = 75
alignment_poll_ms = 500

[alerts]
desktop_timeout_ms = 10000
```

`foreground_include_title` defaults to `false` because browser tab titles and
terminal titles can leak private data.

`live_diagnosis_cluster_window_ms` controls how close live diagnosis evidence
items must be to be grouped into the same candidate window. The default is `5`.
Increase it when correlated events are slightly farther apart on a noisy system;
decrease it when separate short spikes are being grouped together.

The same setting is available as an explicit CLI override:

```bash
stutter monitor --preset diagnosis --pid 1234 --live-diagnosis-cluster-window-ms 10
```

The value must be greater than zero. Config-file values are overridden by
presets and explicit CLI flags according to the precedence rules below.

### Monitor presets

`stutter monitor` supports named presets for common collection settings:

```bash
stutter monitor --preset gaming --watch-process gamescope
stutter monitor --preset diagnosis --pid 1234
stutter monitor --preset lightweight --watch-process game
```

Available presets:

* `gaming`: enables hardware monitoring, CPU frequency, faults, and stat-wait collection.
* `recording`: enables a broader recording-oriented set including block I/O.
* `diagnosis`: enables heavier diagnostic collection. IRQ latency still requires explicit `--irq-latency --irq N`.
* `lightweight`: disables optional heavier collectors.

Presets do not choose targets. You still need to pass a target such as `--pid`, `--watch-process`, or `--cgroupv2`.

Precedence:

```text
built-in defaults < config file < preset < explicit CLI flags
```

Example:

```bash
stutter monitor --preset diagnosis --no-cpu-freq --pid 1234
```

Here `--no-cpu-freq` overrides the preset.

### Advanced eBPF map sizing

Most users should keep the automatic map sizing defaults. If a recording shows ring-buffer drops or wakeup-map pressure, advanced users can override selected map sizes:

```bash
stutter monitor --pid 1234 --ringbuf-size-kb 8192
stutter monitor --pid 1234 --wakeup-map-factor 4
```

* `--ringbuf-size-kb KB`: increases the event ring buffer. Larger values can reduce drops during bursts, but use more locked kernel memory.
* `--wakeup-map-factor N`: sizes the wakeup tracking map as roughly `max_tasks * N`, clamped to built-in safety limits. Larger values can reduce wakeup insert failures, but use more memory.

Valid ranges:

* `--ringbuf-size-kb`: `64..=16384`
* `--wakeup-map-factor`: `1..=64`

The shared capacity constants, BPF map multipliers, memory-budget assumptions,
and change checklist are documented in [docs/EBPF_CAPACITY.md](docs/EBPF_CAPACITY.md).

These flags are escape hatches. The automatic defaults are usually correct.

### Latency flamegraph SVG

`stutter report` can export a flamegraph-style SVG for spike latency attribution:

```bash
stutter report --flamegraph latency.svg run/
```

This is not a CPU stack flamegraph. Stutter does not collect stack traces for this view. The SVG is built from pseudo-stacks (task/thread/CPU) weighted by total spike latency. Use it to see which task/thread/CPU combinations account for the most observed scheduler latency.

## Recommended workflow

`stutter` recommendations are experiments, not proof of root cause.

1. Run `doctor`.
2. Record a baseline with `bench`.
3. Tune a small, explicit profile set.
4. Compare baseline and tune output with `recommend`.
5. Apply only if the recommendation is stable enough to trust.
6. Restore if needed.

Check your CPU topology (e.g. `lscpu -e`) before editing profiles to use explicit masks. See [docs/SAFETY.md](docs/SAFETY.md) for operational safety details and [docs/DAEMON_CONTRACT.md](docs/DAEMON_CONTRACT.md) for daemon modes, default denials, rollback expectations, and developer policy rules.

Example:

```bash
stutter doctor

stutter bench \
  --watch-process Game.exe \
  --persistent \
  --duration 180 \
  --scenario kcd \
  --role baseline

stutter tune \
  --tree-pid <PID> \
  --profiles examples/profiles/common-game-layouts.toml \
  --runs 5 \
  --baseline-profile baseline-online

stutter recommend \
  --baseline <baseline-run-dir> \
  --tune <tune-dir>
```

### Privileged runtime workflow

Loading eBPF programs usually requires root or suitable capabilities. Prefer building as your normal user and running the already-built binary with privileges:

```bash
cargo build

doas target/debug/stutter record \
  --pid "$(pgrep -n sway)" \
  --duration 10 \
  --run-name smoke-sway
```

Avoid `doas cargo run` unless root has the same Rust, cargo, rustup, and `bpf-linker` setup as your normal user. `doas cargo run` can rebuild the eBPF crate under root with a different PATH and fail even though the normal user build succeeds.

Root-owned recordings are written under root’s state directory by default, so report them with privileges:

```bash
doas target/debug/stutter report /root/.local/state/stutter/runs/<run>
```

Or pass an explicit output directory that is readable by your user:

```bash
mkdir -p "$HOME/.local/state/stutter/runs"

doas target/debug/stutter record \
  --pid "$(pgrep -n sway)" \
  --duration 10 \
  --run-name smoke-sway \
  --out-dir "$HOME/.local/state/stutter/runs/smoke-sway"
```

Note: depending on `doas`/`sudo` configuration, `$HOME` may refer to root’s home. Use an absolute path when in doubt.

## Monitor manual PIDs

For privileged live tracing, use the build-then-run workflow from the privileged runtime section rather than `doas cargo run`.

```bash
RUST_LOG=info RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- monitor \
  --pid "$(pgrep -n sway)" \
  --summary-ms 1000
```

Older legacy form still works:

```bash
RUST_LOG=info RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- \
  --pid "$(pgrep -n sway)"
```

## Monitor a process tree

Use this for Proton/Wine/game trees:

```bash
RUST_LOG=info RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- monitor \
  --tree-pid <root-pid>
```

The monitor periodically scans `/proc`, finds descendant processes, expands each process into `/proc/<pid>/task/<tid>`, and updates the eBPF `TARGET_PIDS` map dynamically.
Add `--exclude-tree-pid <pid>` to subtract a process and all of its descendants from the monitored tree, for example to drop the Steam overlay from an otherwise useful game root.

For launchers where the PID changes between runs, watch for the process name:

```bash
RUST_LOG=info RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- monitor \
  --watch-process KingdomCome.exe \
  --persistent \
  --watch-poll-ms 2000 \
  --watch-timeout-seconds 120
```

`--watch-process` scans `/proc` only while waiting for the process to appear or relaunch. Once a process is found, `stutter` follows that root PID and its descendants. `--persistent` requires `--watch-process`; if the watched process exits, stale TIDs are removed and the monitor waits for the next matching launch. Note that `--duration` begins after the watched process is found, not while waiting for it. Missing manual `--pid` targets are dropped with a warning by default; add `--keep-missing-pid` if you want an unknown fallback task retained. `--keep-missing-pid` is intended for manual PID workflows where the PID may appear later; it is not a substitute for tree-root based monitoring and should not be relied on to keep arbitrary tree roots.

## Foreground window context

Use `--foreground-window` to record the currently foreground application/window
as environmental context:

```bash
RUST_LOG=info RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- monitor \
  --tree-pid <root-pid> \
  --foreground-window
```

This writes `foreground_events.json` when recording is enabled. Foreground
events are not focus target-selection events; they answer which visible app was
active near scheduler or frame spikes.

Foreground-aware auto-focus is explicit:

```bash
RUST_LOG=info RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- monitor \
  --auto-focus \
  --focus-source hybrid \
  --foreground-source auto
```

Modes:

- `--focus-source heuristic`: existing heuristic behavior.
- `--focus-source foreground`: choose focus groups that match the foreground
  window when provider data is available and safe.
- `--focus-source hybrid`: prefer foreground-matching focus groups, then fall
  back to the existing heuristic when foreground data is unavailable or stale.

Provider selection:

- `--foreground-source auto`: choose a supported provider automatically.
- `--foreground-source sway`: use `swaymsg -t get_tree -r`.
- `--foreground-source x11`: use `xprop`.
- `--foreground-source hyprland`: accepted as a provider selector, but currently
  returns unsupported until a safe Hyprland-specific implementation is added.

Privacy:

- Window titles are redacted by default.
- `title` is recorded as `null` unless `--foreground-include-title` is passed.
- Do not enable title capture on shared recordings unless tab and terminal
  titles are safe to expose.

## Community rules

`stutter` can import Ananicy-compatible community rules as process
classification hints. The core package does not ship the full GPL Ananicy rules
database; users may import a local checkout into their own XDG data directory.

```bash
stutter rules import --source /path/to/ananicy-rules
stutter rules status
```

Imported rules are used as identity/classification hints only. They do not copy
Ananicy scheduling policy such as nice values, ionice values, scheduler classes,
CPU affinity, or systemd policy.

See [docs/COMMUNITY_RULES.md](docs/COMMUNITY_RULES.md) for the full licensing,
storage, packaging, and runtime-loading details.

## Inspect a tree before tracing

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- inspect-tree \
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

## Inspect IRQs

Use `inspect-irqs` to discover useful numeric IRQ IDs from `/proc/interrupts`:

```bash
stutter inspect-irqs
stutter inspect-irqs --filter amdgpu
stutter inspect-irqs --filter xhci --top 10
stutter inspect-irqs --json
```

Example output:

```text
IRQ        total        kind       name
146        12345678     PCI-MSI    524288-edge amdgpu
147        123456       PCI-MSI    524289-edge xhci_hcd

Suggestions:
  Use: stutter monitor --irq-latency --irq 146
```

Only numeric IRQ IDs can be passed to `--irq`. Non-numeric interrupt rows such as `NMI` or `LOC` may appear in the table but are not suggested as `--irq` values.

## Record a run

For real eBPF recording, prefer the privileged runtime workflow above: build once as your normal user, then run the already-built `target/debug/stutter` binary with `doas`/`sudo`. The `cargo run` examples below are mainly development shorthand and should not be combined with `doas` unless root has the same Rust and `bpf-linker` setup.

```bash
RUST_LOG=info RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- record \
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
RUST_LOG=info RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- record \
  --tree-pid <root-pid> \
  --irq-latency --irq 137 \
  --hwmon \
  --mangohud-log /path/to/MangoHud.csv
```

Hardware monitoring notes:

- On multi-GPU systems, prefer an explicit selector such as `--hwmon-drm-card card1`,
  `--hwmon-render-node /dev/dri/renderD129`, or the direct `--hwmon-root /path/to/hwmon`
  override so automatic discovery does not pick the wrong device.
- `--hwmon-root` is a trusted direct override. It is not restricted to `/sys/class/hwmon`,
  so development and test fixtures can use alternate roots, but the path must exist, be a
  directory, and contain at least one supported hwmon sensor file before sampling starts.
  Passing a regular file or a directory without supported sensor files is rejected as an
  invalid hwmon directory.
- Frequent hwmon sampling uses cached file descriptors internally; if you see warnings about
  `latency_samples_truncated`, that means stutter is storing a bounded number of exact samples
  and will fall back to histogram-based percentile estimates for p95/p99.

## Interval CSV Streaming

Stream per-interval summaries to a file or stdout:

```bash
# Stream to a file
stutter monitor --stream-csv output.csv

# Stream to stdout (suppresses human-readable output)
stutter monitor --stream-csv -
```

## Bench a repeatable route

`bench` is a guided recording wrapper for baseline/current comparisons. It uses stricter naming and records by default.

```bash
RUST_LOG=info stutter bench \
  --watch-process Game.exe \
  --persistent \
  --duration 180 \
  --scenario test-route \
  --role baseline
```

## Scenario workflow

Scenario files store route metadata and recording options. This reduces typing mistakes between baseline/current runs and makes benchmarks more comparable.

1. Create a scenario:
   ```bash
   stutter scenario create kcd-route
   ```
2. Edit the generated TOML file:
   ```bash
   $EDITOR ~/.config/stutter/scenarios/kcd-route.toml
   ```
3. Run the baseline and current benchmarks:
   ```bash
   stutter scenario run kcd-route --role baseline
   stutter scenario run kcd-route --role current
   ```
4. Compare the runs:
   ```bash
   stutter scenario compare kcd-route
   ```

Example TOML:

```toml
name = "kcd-route"
watch_process = "KingdomCome.exe"
duration = 180
preset = "diagnosis"
mangohud_log = "/path/to/MangoHud.csv"
expected_classes = ["Game", "GameScope", "Compositor"]
notes = "Ride from town gate to forest path"

persistent = true
summary_ms = 1000
spike_us = 1000
```

Use `--dry-run` to inspect effective settings before recording:
```bash
stutter scenario run kcd-route --role baseline --dry-run
```

Scenario outputs go under `~/.local/state/stutter/scenarios/<name>/`. `scenario compare` uses the latest baseline and current runs by default, but you can override them with `--baseline` and `--current` flags.

TUI notes:

- `--tui` opens an interactive ratatui alternate-screen UI.
- Controls:
  - `q`: quit
  - `p`: pause/resume interval collection/render updates
  - `s`: cycle sort field
  - `f`: cycle task-class filter
- The UI shows:
  - active/known task counts
  - eBPF drop-counter status
  - compact foreground-window line when foreground context is enabled
  - compact focus line with focus roots and switch count
  - sortable task latency table
  - global max-latency sparkline
  - per-CPU max-latency heat bars
  - recent live diagnosis candidates

## Generate a report

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- report \
  ~/.local/state/stutter/runs/<run-dir>
```

Show more rows or tighten spike cluster grouping:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- report \
  --top 25 \
  --cluster-ms 2 \
  ~/.local/state/stutter/runs/<run-dir>
```

JSON output:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- report \
  --json \
  ~/.local/state/stutter/runs/<run-dir>
```

Reports use `spike_events.json` for spike cluster detection when it is present. Older runs without that file still report clusters from retained per-task `top_spikes`.

When `foreground_events.json` is present, reports include a foreground-window
summary and annotate spike clusters with the nearest foreground event at or
before the cluster time. This helps distinguish a spike in a helper process
while a game is foreground from a spike in the foreground game process itself.

Reports include a `data quality` section. This is a run-level trust summary derived from schema validation, optional artifact presence, spike-event truncation, eBPF drop counters, event-stream write errors, percentile scope, block-I/O correlation basis, and MangoHud timestamp alignment.

The levels are:

- `High`: no known data-quality problems.
- `Medium`: the run is usable, but some evidence is approximate, missing, truncated, or degraded.
- `Low`: validation errors, incompatible schema, or recording write errors make the report unreliable.

For machine-readable consumers, `stutter report --analysis-json <run-dir>` is the stable machine-readable interface and includes the same data-quality summary. `stutter validate <run-dir>` validates artifact compatibility and data quality before automation consumes a run.

Live daemon/autotune status also carries machine-readable online quality reason
codes such as `insufficient_samples`, `target_missing`,
`focus_low_confidence`, `drop_counters_high`, `workload_changed`, and
`thermal_degraded` so policy rejections can be explained without parsing prose.

Diagnosis output uses candidate wording intentionally. A `High` confidence diagnosis means the evidence is strong for a profiler inference, not proof of root cause. `Medium` and `Low` confidence diagnoses should be treated as leads for further testing. Text reports show the top diagnosis candidates and evidence items so the score is auditable instead of sounding final.

Generate a self-contained HTML report:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- report \
  --html report.html \
  ~/.local/state/stutter/runs/<run-dir>
```

HTML reports include:

- PSI pressure timeline derived from interval data.
- Frame-pacing/outlier view derived from MangoHud frame events.
- Spike cluster explanations with chosen evidence, competing candidates, and missing evidence.
- Foreground-window context near clusters when foreground collection was enabled.

## Tune and recommend profiles

Tune benchmarks a TOML affinity profile set and writes both machine-readable and human-readable recommendation artifacts:

```text
tuning_summary.json
tuning_recommendation.json
tuning_recommendation.md
```

Example:

```bash
stutter tune \
  --tree-pid <PID> \
  --profiles examples/profiles/common-game-layouts.toml \
  --runs 5 \
  --baseline-profile baseline-online
```

Use `recommend` to compare a separately recorded baseline run against a tuning output directory:

```bash
stutter recommend \
  --baseline ~/.local/state/stutter/runs/<baseline-run-dir> \
  --tune ~/.local/state/stutter/tune-<timestamp>
```

If ranking confidence is `Unstable`, `tune --keep-best` will not apply anything. Treat `Low` confidence as a prompt to retest under a comparable workload.

## Offline advisor

`advisor` reads an existing run and suggests next experiments without changing system state:

```bash
stutter advisor --run ~/.local/state/stutter/runs/<run-dir>
stutter advisor --run ~/.local/state/stutter/runs/<run-dir> --json
```

It can also watch completed run directories and emit reports as they appear:

```bash
stutter advisor --watch-runs --once
stutter advisor --watch-runs
```

Advisor output is deliberately cautious: it reports candidates and suggested experiments, not confirmed root causes, and it does not auto-apply any tuning.

## Daemon and autotune modes

The daemon policy mode labels are:

```text
observe
suggest
apply-low-risk
apply-medium-risk
apply-high-risk
```

Their contract is documented in [docs/DAEMON_CONTRACT.md](docs/DAEMON_CONTRACT.md).

Live `stutter autotune --mode` currently supports `observe`, `suggest`, `apply-low-risk`, and `apply-medium-risk` when `--allow-medium-risk` is set. `apply-medium-risk` is limited to reversible process-local/cgroup candidates (`nice`, `ionice`, `uclamp`, `cgroup_placement`, and medium-risk CPU-affinity profiles), still requires an explicit target, and still passes through the strict planner and daemon policy gates. `apply-high-risk` remains a policy label and is not implemented for live apply.

`apply-low-risk` is the default apply ceiling and currently applies CPU-affinity candidates only for explicit target process trees. It is not a system-wide auto-tuner.

Remote autotune uses the same mode labels, but remote apply support is bounded by the agent's configured limits. High-risk remote support is never enabled by default.

`suggest` mode does not apply candidate changes. Candidate suggestion text always includes a dry-run command, `required_mode`, `required_safety_class`, and `rollback=stutter restore`. A manual apply command is shown only when the central CLI daemon policy would allow that candidate; high-risk candidates do not get direct apply commands. CPU-affinity-profile suggestions preserve the existing `stutter apply-profile ...` command. Generic candidate suggestions write stable plan files under `$HOME/.local/state/stutter/autotune/candidate_plans/<action_kind>-<candidate_name>.json` and use `stutter autotune apply-candidate --candidate-json <file> --dry-run`; reversible process-local plans may also show `stutter autotune apply-candidate --candidate-json <file>` when policy allows manual apply.

CPU power suggestions also require power and thermal headroom. They are suppressed while a battery is discharging unless `[autotune].allow_cpu_power_on_battery = true` is explicitly configured; battery apply-time guards remain in force for actual governor/EPP writes.

VM knob suggestions are policy-table driven and remain high-risk/manual-only. Current suggestions are limited to `vm.swappiness` for swap activity and dirty writeback ratio knobs when direct writeback evidence is present; ratio suggestions are skipped when the corresponding bytes knob is active.

Daemon status and watch commands are intended to answer "what is it doing?"
without reading logs:

```bash
stutter daemon status
stutter daemon status --json
stutter daemon status --explain-last 10
stutter daemon doctor
stutter daemon reset-state --dry-run
stutter daemon watch
stutter daemon watch --verbose --interval-ms 1000
```

`daemon status` includes the active workload, active action, rollback status,
health, watchdog state, current score when available, manual restore command,
and recent autotune decisions. `daemon doctor` checks the persisted daemon
state, health, watchdog, rollback state, and machine capabilities, and reports
when safe observe-only mode is required. `daemon reset-state --dry-run`
previews a safe state-store reset; without `--dry-run`, it backs up the current
snapshot before writing observe-only disabled state. `daemon watch` is quiet by default: it prints a
compact first line and then emits notifications only for action apply,
rollback, fault, or restore-needed transitions. Use `--iterations N` for a
bounded watch in scripts.

## Doctor / preflight

Run `stutter doctor` before recording to check whether tracing is likely to work and which optional telemetry may be missing or degraded. The doctor command does not attach eBPF programs or perf probes by default, so it is a preflight check rather than a guarantee that a future recording will succeed.

`doctor` reports whether the current process appears to have eBPF runtime privileges. A non-root `doctor` warning can be normal if you plan to run the built binary with `doas`/`sudo`.

Useful optional checks:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- doctor
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- doctor --json
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- doctor --hwmon --hwmon-drm-card card1
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- doctor --block-io
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- doctor --irq-latency --irq 137
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- doctor --faults
```

Use `--hwmon`, `--block-io`, `--irq-latency`, `--faults`, and `--mangohud-log <PATH>` to inspect the optional probes you plan to use.

## Apply and restore affinity profiles

Apply a TOML profile to the current process tree:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- apply-profile \
  --tree-pid <root-pid> \
  --profile profile.toml
```

Preview the planned changes without changing live affinity, nice, ionice, audit state, or restore state:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- apply-profile \
  --dry-run \
  --tree-pid <root-pid> \
  --profile profile.toml
```

`apply-profile --dry-run` is one-shot only. It cannot be combined with `--watch`.

By default real application is one-shot. Use `--watch` to keep applying the profile to new threads:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- apply-profile \
  --tree-pid <root-pid> \
  --profile profile.toml \
  --watch
```

Watch mode restores the original masks on Ctrl-C by default. Add `--keep-applied` to leave the profile active and restore later:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- restore
```

Audit a pending restore file without applying it:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -- restore --dry-run
```

Inspect recent action audit events:

```bash
stutter audit
stutter audit --tail 50
stutter audit --json
```

`apply-profile --force` discards an existing restore file. Without `--force`, new records are merged into the existing restore file while preserving the earliest original mask for each TID.

Profile `match_comm` entries use literal substring matching unless the whole pattern is wrapped in `/.../`. For example, `KingdomCome.exe` matches that literal dot, while `/KingdomCome[.]exe$/` is treated as a regex.

### Profile rule order

Profile rules are evaluated in file order. The first matching rule wins. Put specific rules before broad class rules.

For example, place a specific `RenderThread` rule before a broad `Game` fallback:

```toml
# Specific RenderThread first
[[profile.rules]]
match_comm = ["RenderThread"]
affinity = "2-5"

# Broad Game fallback second
[[profile.rules]]
match_class = ["Game"]
affinity = "0-7"
```

If a broad rule appears first, later specific rules may never apply.

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

`target_pending_wakeups` is a profiler-side counter of wakeups for monitored tasks, not Linux kernel runqueue depth. In reports, `target_pending_on_switch_cpu` means other monitored wakeup records still pending on the CPU that actually ran the task after this task was dequeued from its original wakeup target CPU. It is useful as a rough target-local pressure signal, but do not score it as the queue depth that delayed the task.

Block I/O overlap records the run-level `block_io_correlation_basis`. When request pointer offsets match between issue and completion tracepoints it uses `request-pointer`; otherwise it falls back to approximate `dev+sector` hashing, where concurrent same-sector requests can collide.

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
- `foreground_events.json`: foreground-window context when enabled; titles are
  `null` unless `--foreground-include-title` is set.

For programmatic consumption, inspect the example outputs in a sample run directory created under `~/.local/state/stutter/runs/`.

The formal artifact contract is documented in `docs/ARTIFACT_SCHEMA.md`, with versioned sanitized examples under `docs/examples/artifacts/`.

Note: the CSV exporter is intentionally compact and omits some newer fields. `interval.json` and the other JSON artifacts are full-fidelity raw outputs: they contain PSI samples, major/minor fault deltas, drop counters, and histogram/truncation details. Prefer `report --analysis-json` for stable programmatic analysis unless you specifically need raw streams. Recorded run directories are loaded through a shared `session_io` path; missing optional artifact files are gracefully tolerated for older recordings.

## Probe admission policy

New probes must answer a specific diagnostic question and include schema docs, fixture/replay tests, data-quality behavior, validation behavior, and cautious diagnosis/report integration before they are accepted. See `docs/PROBE_ADMISSION.md` for the full checklist.

Use `stutter probes` or `stutter probes --json` to list implemented, view-only, and planned telemetry. Foreground-window context appears as the `foreground_window` probe. The pressure timeline overlay in `report --analysis-json` is derived from existing interval PSI data; it does not add a new live probe.

## CLI flags (quick reference)

- `--pid <PID>`: add a manual TID/PID to monitor (can repeat).
- `--tree-pid <PID>`: monitor an entire process tree rooted at `PID` (expands into per-task TIDs).
- `--exclude-tree-pid <PID>`: exclude a process subtree from monitored tree roots (can repeat).
- `--watch-process <COMM>`: poll `/proc` for a process whose name/comm matches `<COMM>` and automatically follow its tree; combine with `--persistent` to keep waiting across restarts.
- `--persistent`: use with `--watch-process` to continue monitoring across relaunches.
- `--auto-focus`: automatically choose a process tree using the focus resolver when no explicit target is provided.
- `--focus-source <heuristic|foreground|hybrid>`: choose whether auto-focus uses the existing heuristic, foreground-window context, or foreground-first hybrid fallback.
- `--foreground-window`: record foreground-window context to `foreground_events.json` even when explicit targets are used.
- `--foreground-source <auto|sway|hyprland|x11>`: choose the foreground-window provider.
- `--foreground-poll-ms <MS>`: foreground-window polling interval; must be at least 100ms.
- `--foreground-max-stale-ms <MS>`: maximum stale foreground snapshot age before focus context is cleared.
- `--foreground-include-title`: record window titles; disabled by default because titles may expose private tab or terminal text.
- `--summary-ms <MS>`: interval for interval summaries written to `interval.json` and printed to the TUI.
- `--epoch <MS>`: explicit reset-and-report mode; prints interval stats with the `epoch` label every `<MS>` and skips the final cumulative session recap.
- `--spike-us <US>`: spike detection threshold in microseconds (e.g., `--spike-us 1000` for 1ms).
- `--alert-threshold-ms <MS>`: send an alert when a runnable-latency spike reaches this threshold. Uses `notify-send` by default.
- `--alert-webhook-url <URL>`: with `--alert-threshold-ms`, POST alert JSON to a webhook instead of using `notify-send`. You can also set `STUTTER_ALERT_WEBHOOK_URL`.
- `--include-comm <PATTERN>` / `--exclude-comm <PATTERN>`: case-insensitive substring filters against task `comm` and process `comm`; exclude wins.
- `--irq-latency`: enable IRQ latency tracing and record `irq_events.json`; at least one explicit `--irq <IRQ>` is required. Use `stutter inspect-irqs` to find the IRQ numbers for your devices.
- `--irq <IRQ>`: add an IRQ number to target for IRQ latency measurement (can repeat).
- `--hwmon`: enable GPU hwmon sampling; combine with `--hwmon-drm-card`, `--hwmon-render-node`, or `--hwmon-root` to avoid ambiguous multi-GPU discovery.
- `--hwmon-root <PATH>`: trusted direct hwmon discovery override. The path may be outside `/sys/class/hwmon`, but it must exist, be a directory, and contain supported hwmon sensor files.
- `--hwmon-drm-card <CARD>`: choose a DRM card such as `card0` or `card1` for hwmon discovery.
- `--hwmon-render-node <PATH>`: choose the DRM render node whose device hwmon should be sampled.
- `--mangohud-log <PATH>`: provide a MangoHud CSV to correlate frame times.
- `--tui`: open an interactive ratatui alternate-screen UI.
- `stutter bench --duration <SECONDS> --scenario <NAME>`: record a named baseline/current route with safer defaults.
- `stutter tune --tree-pid <PID> --profiles <FILE>`: apply each profile, keep refreshing it for new threads during the measurement epoch, score interval summaries, and restore after each candidate by default. Candidate run directories are kept next to the tuning summary for auditability. Add `--keep-best` to reapply the best profile at the end.
- `stutter recommend --baseline <RUN> --tune <DIR>`: compare a baseline recording against a tune output directory.
- `stutter advisor --run <RUN>`: read an existing run and suggest conservative next experiments without applying changes.
- `stutter audit`: show recent action audit events from `~/.local/state/stutter/audit/actions.jsonl`.

Tune counterbalances profile order across iterations to reduce order bias. `tuning_summary.json` includes the candidate order, per-profile median/IQR stats, and a ranking-confidence field. If the ranking is unstable, no best profile is selected and `--keep-best` will not apply a profile.

## What TUI shows

The `--tui` mode uses ratatui and crossterm alternate-screen rendering. It shows active and known task counts, eBPF drop-counter status, compact foreground-window and focus lines when enabled, a sortable per-task latency table, a global max-latency sparkline, per-CPU max-latency heat bars, and recent live diagnosis candidates. Press `q` to quit, `p` to pause or resume interval collection and render updates, `s` to cycle sort fields, and `f` to cycle the task-class filter.

## Generated JSON files (overview)

- `metadata.json` — run metadata: CLI flags, `run_name`, timestamps, and schema version.
- `session.json` — per-task summary for the full session. Contains per-task histograms, sample counts, `max`, `percentile_scope` (either `capped_prefix` or `histogram` depending on whether exact samples were retained), and task identity fields (tid, process_pid, comm, class).
- `interval.json` — periodic interval summaries matching `--summary-ms`, used by `report` for time-windowed analysis.
- `spike_events.json` — detected latency spike clusters used by the HTML report and cluster summaries.
- `irq_events.json` — IRQ enter/exit capture when `--irq-latency` is enabled.
- `gpu_samples.json` — periodic hwmon samples when `--hwmon` is enabled.
- `foreground_events.json` — foreground-window context when `--foreground-window`
  or foreground-aware focus is enabled; titles are `null` unless
  `--foreground-include-title` is set.

Note: when present, `cpu_frequency` tracepoint samples are emitted as system-wide telemetry and are not filtered to individual target tasks; treat them as global context rather than per-task signals.

If you need machine-readable schemas, open a recorded run under `~/.local/state/stutter/runs/<run-dir>/` and inspect the files; they are stable across releases but may add fields in minor versions.

### Remote agent mode (experimental)

Run the collector as a privileged local agent:

```bash
sudo stutter agent
```

By default, the agent listens on a Unix socket under `XDG_RUNTIME_DIR` when
available. For older local HTTP clients, start an explicit loopback listener:

```bash
sudo stutter agent --bind 127.0.0.1:9899
```

Control the loopback listener from a client:

```bash
stutter monitor --remote http://127.0.0.1:9899 --pid 1234 --duration 10
```

The agent provides a local HTTP JSON API:

* `GET /health`
* `GET /version`
* `GET /capabilities`
* `POST /record/start`
* `POST /record/stop`
* `GET /record/status`
* `GET /runs`
* `GET /runs/<id>/session.json`
* `GET /runs/<id>/artifact/<name>`

**Security & Hardening:**

* **Unix socket default:** The agent uses a local Unix socket by default when no bind address is provided.
* **Loopback TCP:** TCP listeners should be explicit, for example `--bind 127.0.0.1:9899`.
* **Unsafe bind:** Binding to a non-loopback address requires the `--allow-unsafe-bind` flag.
* **Authentication:** Legacy full-access bearer authentication can be enabled with `STUTTER_AGENT_TOKEN` or `--bearer-token-file`.
* **Split tokens:** Use `STUTTER_AGENT_READ_TOKEN` for read-only clients and `STUTTER_AGENT_APPLY_TOKEN` for state-changing control, or pass `--read-token-file` and `--apply-token-file`.
* **Client Auth:** The local `stutter monitor --remote` client will automatically send the token if `STUTTER_AGENT_TOKEN` is set in its environment.
* **Request Limits:** The agent enforces default limits on JSON request size, request rate, recording duration, and target count. Duration and target count can be configured via `--max-duration-seconds` and `--max-targets`.
* **Artifact Whitelist:** Artifact downloads are restricted to a known allowlist of stutter-generated files.
* **CORS:** The agent does not enable browser cross-origin access by default.

> [!WARNING]
> Do not expose the stutter agent to untrusted networks. It is a privileged control plane that can remotely start eBPF profiling sessions.

**Discovery:**
Use `GET /version` to see schema compatibility and `GET /capabilities` to see configured limits and available routes.

## TID reuse detection (what we do)

To reduce false positives when a TID/PID number is recycled by the kernel, `stutter` now combines multiple heuristics:

- Prefer `starttime` fields read from `/proc/[pid]/stat` when available (these are clock-ticks since boot).
- Compare the `/proc/[pid]/exe` file metadata (device + inode) between the previously-observed logical task and the newly-observed PID/TID. If the exe inode differs, we treat it as a different logical task.
- When available, we compare the previously-observed process starttime with the current one to further disambiguate.

These checks reduce collisions compared to relying on `starttime` alone, but in extremely constrained environments (shared containers or odd boot-time adjustments) collisions remain possible. For the most robust detection you can combine exe inode checks with cgroup membership or an explicit profile that targets executable paths.

## License

The userspace crates are dual-licensed MIT OR Apache-2.0.

The eBPF code is dual-licensed MIT OR GPL-2.0-only.
