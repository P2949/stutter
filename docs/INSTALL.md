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

## Packaging Status

There is no distro package yet. The local scripts are intentionally small and conservative so you can inspect exactly what they do.
