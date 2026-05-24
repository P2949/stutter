# Install

`stutter` is currently packaged for technical local use, not as a distro-ready package.

## Requirements

- Linux with eBPF support
- Rust nightly with `rust-src`
- `bpf-linker`
- privileges when loading eBPF programs for `monitor`, `record`, and live tracing paths

```bash
rustup toolchain install nightly --component rust-src
cargo install bpf-linker
```

## Build

```bash
RUSTUP_TOOLCHAIN=nightly cargo build -p stutter
RUSTUP_TOOLCHAIN=nightly cargo build --release -p stutter
```

## Install Local

```bash
scripts/install-local.sh
```

By default this installs:

```text
$HOME/.local/bin/stutter
```

Override the prefix if needed:

```bash
PREFIX=/opt/stutter scripts/install-local.sh
```

The installer does not set setuid bits and does not install Linux capabilities.
`stutter` still needs privileges to load eBPF, so use `sudo` or `doas` for recording when required by your system policy.

## Uninstall

```bash
scripts/uninstall-local.sh
```

The uninstaller removes only the binary from `$PREFIX/bin/stutter`. It does not delete run data or audit logs.

Default state paths:

```text
~/.local/state/stutter/runs
~/.local/state/stutter/tune-*
~/.local/state/stutter/audit/actions.jsonl
```

## Agent and Autotune Services

The service planner shows the exact filesystem changes before you install
units:

```bash
stutter service doctor --mode system-observe --manager systemd-system
stutter service install --dry-run --mode system-observe --manager systemd-system
stutter service install --dry-run --mode system-low-risk --manager systemd-system
stutter service install --dry-run --mode system-observe --manager openrc
```

`service install` copies packaged unit files and creates the configured
`/etc/stutter`, `/var/lib/stutter`, and `/var/log/stutter` directories. It does
not enable services automatically; the output prints the follow-up
`systemctl`, `systemctl --user`, or `rc-service` command. Use
`stutter service uninstall --dry-run ...` to preview removal of an installed
unit file.

`packaging/systemd/stutter-agent.service` starts the local agent on the Unix
socket `/run/stutter/agent.sock`. The standalone `stutter agent` command also
defaults to a Unix socket under `XDG_RUNTIME_DIR` when available. Use
`stutter agent --bind 127.0.0.1:9899` only when an HTTP TCP listener is needed
for compatibility with an existing local client.

For apply-medium-risk autotune, run the privileged mutator separately:

```bash
stutter privileged-worker --socket /run/stutter/privileged-worker.sock
```

The socket is created with mode `0600`. Point the daemon at a non-default path
with `[autotune].privileged_worker_socket = "/run/stutter/privileged-worker.sock"`.
Socket startup and shutdown poll timing can be tuned with
`[autotune].privileged_worker_socket_ready_timeout_ms`,
`[autotune].privileged_worker_socket_ready_retry_ms`, and
`[autotune].privileged_worker_shutdown_poll_ms`.

Agent auth supports a legacy full-access token through `STUTTER_AGENT_TOKEN`
or `--bearer-token-file`, plus split tokens for safer clients. The packaged
systemd unit reads `/etc/stutter/agent.env` if present:

```text
STUTTER_AGENT_READ_TOKEN=...
STUTTER_AGENT_APPLY_TOKEN=...
stutter agent --read-token-file /etc/stutter/agent-read.token --apply-token-file /etc/stutter/agent-apply.token
```

Read tokens may call status, health, history, and artifact endpoints. Apply
tokens are required for state-changing control when split tokens are
configured. The agent also applies an explicit JSON body-size limit, a request
rate limit, per-request audit records, and no CORS headers by default.
The daemon API includes `/daemon/status`, `/daemon/health`, `/daemon/policy`,
and `/daemon/explain`; `/daemon/explain` includes machine-readable
`why_no_optimize` and `what_changed` lists alongside the canonical policy rule
explanation.

`packaging/systemd/stutter-autotune-observe.service` starts the live observe-only
autotune controller. By default it uses:

```text
stutter autotune --mode observe --auto-focus --focus-source hybrid --preset diagnosis
```

That mode observes the whole `/proc` process set through the focus resolver,
follows the selected focus group, writes decision/history/status output, and
does not apply affinity, nice, ionice, scheduler-class, cgroup, IRQ, GPU, or
system-wide changes.

The autotune controller history and status UX is intentionally explicit. The
JSONL history at `~/.local/state/stutter/autotune/history.jsonl` records these
lifecycle decisions:

```text
observed
suggested
candidate_started
candidate_applied
candidate_kept
candidate_reverted
cooldown_entered
faulted
restored
```

`stutter autotune-status --json` and `stutter autotune-status` report the
current controller phase, mode, focus group, target root, current score,
active profile, active candidate, last decision, rollback availability,
cooldown remaining, data quality, last fault, and manual restore command.
`stutter autotune restore --dry-run` previews the active rollback path, and
`stutter autotune restore` writes a normalized `restored` history event after
successful emergency restore.

Daemon-level status is available through:

```bash
stutter daemon status
stutter daemon status --json
stutter daemon explain
stutter daemon why-not-optimize
stutter daemon what-changed
stutter daemon policy explain
stutter daemon profiles list
stutter daemon profiles explain
stutter daemon profiles forget --workload-hash <hash> --dry-run
stutter daemon doctor
stutter daemon reset-state --dry-run
stutter daemon watch
```

`stutter daemon status --explain-last 10` includes recent autotune decisions.
`stutter daemon explain` focuses on local state explainability: why the daemon
is not optimizing now, what changed recently, and the canonical policy rule
outcomes.
`stutter daemon why-not-optimize` and `stutter daemon what-changed` print the
two focused halves of that explanation for scripts or quick terminal checks.
`stutter daemon policy explain` shows the effective policy decision for
canonical observe, low-risk, medium-risk, high-risk, and missing-rollback
action shapes; add `--json` for machine-readable rule outcomes.
`stutter daemon profiles list` shows remembered kept candidates by workload
identity hash. `profiles explain` compares those records to the current kernel,
CPU topology, and scheduler state so stale learned profiles are visible before
reuse. `profiles forget` removes stale memory, and requires either
`--workload-hash` or `--all` so accidental broad deletion is explicit.
`stutter daemon doctor` checks state-store load, health, watchdog, rollback,
and capability status. If daemon state is corrupt or uncertain, `stutter daemon
reset-state --dry-run` previews the safe observe-only reset; running it without
`--dry-run` writes a new disabled observe state after backing up the old
snapshot when one exists.
`stutter daemon watch` is quiet by default and emits compact notifications for
action apply, rollback, fault, and restore-needed transitions. Add `--verbose`
to print the full status block on each tick.

Daemon user config supports conservative guardrail overrides in
`~/.config/stutter/config.toml`: `daemon_preset`,
`daemon_enabled_action_families`, `daemon_denied_action_families`,
`daemon_min_confidence`, `daemon_max_cpu_temp_celsius`,
`daemon_max_gpu_temp_celsius`, `daemon_min_disk_available_bytes`, and
`daemon_max_memory_pressure_some_avg10_percent`. System-wide and high-risk
daemon fields still require `experimental = true`.

Optional environment overrides for `/etc/stutter/autotune-observe.env`:

```text
STUTTER_AUTOTUNE_WATCH_PROCESS=Game.exe
STUTTER_AUTOTUNE_TREE_PID=1234
STUTTER_AUTOTUNE_PRESET=diagnosis
STUTTER_AUTOTUNE_SUMMARY_MS=1000
STUTTER_AUTOTUNE_FOCUS_SOURCE=hybrid
```

Set at most one of `STUTTER_AUTOTUNE_WATCH_PROCESS` and
`STUTTER_AUTOTUNE_TREE_PID`. If neither is set, the service uses focus-aware
whole-system observation.

`packaging/systemd/stutter-autotune-low-risk.service` is an opt-in system
service for continuous low-risk CPU-affinity experiments. It uses the same
focused target policy as observe mode, only permits `ReversibleLowRisk`
CPU-affinity profile candidates, writes controller history/audit/journal
state, and runs `stutter daemon emergency-restore` on service stop so both
controller rollback state and managed profile restore files are considered.

OpenRC equivalents are available under `packaging/openrc/` for the agent,
observe, and low-risk services. The OpenRC agent defaults to
`/run/stutter/agent.sock`; set `stutter_bind="127.0.0.1:9899"` in the service
environment if a loopback TCP listener is required instead.

Optional environment overrides for
`/etc/stutter/autotune-low-risk.env`:

```text
STUTTER_AUTOTUNE_WATCH_PROCESS=Game.exe
STUTTER_AUTOTUNE_TREE_PID=1234
STUTTER_AUTOTUNE_PROFILES=/etc/stutter/profiles.toml
STUTTER_AUTOTUNE_PRESET=diagnosis
STUTTER_AUTOTUNE_SUMMARY_MS=1000
STUTTER_AUTOTUNE_FOCUS_SOURCE=hybrid
STUTTER_AUTOTUNE_CANDIDATE_SECONDS=30
```

## Advisor Service

The provided systemd unit is a user service for offline advisor watch mode only. It does not load eBPF and does not tune the system.

Install it for the current user:

```bash
mkdir -p ~/.config/systemd/user
cp packaging/systemd/stutter-advisor.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now stutter-advisor.service
```

Inspect logs:

```bash
journalctl --user -u stutter-advisor.service
```

Disable it:

```bash
systemctl --user disable --now stutter-advisor.service
```

## Permissions

`advisor`, `recommend`, `report`, `audit`, and offline file inspection commands do not require root.
Commands that attach eBPF programs or trace live scheduler state generally require elevated privileges.

## CPU Topology

When using affinity profiles, it is important to check your CPU topology to ensure masks align with your cores and hyperthreads. Use `lscpu` or `/sys` to inspect your layout before applying explicit masks:

```bash
lscpu -e=CPU,CORE,SOCKET,NODE,ONLINE,MAXMHZ
cat /sys/devices/system/cpu/online
```

## Packaging Status

There is no production-ready distro package yet. The local install scripts are
the supported install path for now.

The Gentoo ebuild/overlay files, when present, should be treated as a packaging
skeleton only. They are useful for documenting the intended Portage shape, USE
flag direction, service-file layout, and dependency model, but they are not yet
expected to provide a fully automated production build.

In particular, the eBPF build path currently depends on Rust nightly,
`rust-src`, `bpfel-unknown-none`, and `-Z build-std=core`. That interacts poorly
with offline Cargo vendoring in distro package managers. Until the core project
is closer to production-ready, Gentoo packaging may require a manually prebuilt
eBPF object or local developer adjustments.

For now, prefer:

```bash
scripts/install-local.sh
```

A proper ebuild should be revisited once the runtime interface, eBPF object
layout, release process, and service model are stable.
