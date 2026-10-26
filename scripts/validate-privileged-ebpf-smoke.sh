#!/usr/bin/env bash
set -euo pipefail

: "${RUSTUP_TOOLCHAIN:=nightly-2026-06-06}"
export RUSTUP_TOOLCHAIN

if [ "$(id -u)" -eq 0 ]; then
  echo "error: run this wrapper as your normal user; only the test executable needs privilege" >&2
  exit 1
fi

# Pick a privilege command that works without a prompt: probe each candidate non-interactively
# (sudo before doas) so a doas that merely exists cannot shadow a working sudo, and
# never fall back to a password prompt.
privilege_command=""
if command -v sudo >/dev/null 2>&1 && sudo -n true </dev/null >/dev/null 2>&1; then
  privilege_command="sudo -n"
elif command -v doas >/dev/null 2>&1 && doas -n true </dev/null >/dev/null 2>&1; then
  privilege_command="doas -n"
fi
if [ -z "$privilege_command" ]; then
  echo "error: privileged eBPF validation needs non-interactive privilege (sudo -n or doas -n); neither works here and no password prompt is attempted" >&2
  exit 1
fi

cd "$(dirname "${BASH_SOURCE[0]}")/.."
cargo build -p stutter

# Cargo and all build scripts stay unprivileged. Cargo's target runner elevates
# only the finished test executable, with the loader suite explicitly enabled.
host_target="$(rustc -vV | sed -n 's/^host: //p')"
if [ -z "$host_target" ]; then
  echo "error: could not determine the native Rust target" >&2
  exit 1
fi
runner_variable="CARGO_TARGET_$(printf '%s' "$host_target" | tr '[:lower:]-' '[:upper:]_')_RUNNER"
export "$runner_variable=$privilege_command env STUTTER_RUN_PRIVILEGED_EBPF_TESTS=1"
exec cargo test -p stutter --target "$host_target" \
  ebpf_loader::privileged_loader_tests::privileged_ -- --nocapture
