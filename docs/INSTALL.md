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

`packaging/systemd/stutter-agent.service` starts the local HTTP agent on
`127.0.0.1:9899`.

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

`packaging/systemd/stutter-autotune-low-risk.service` is intentionally a
disabled template that exits with an explanation. Continuous daemon-side
`apply-low-risk` remains blocked until the observe/suggest runtime has stable
audit history, status, rollback recovery, and replay behavior.

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

There is no distro package yet. The local scripts are intentionally small and conservative so you can inspect exactly what they do.
