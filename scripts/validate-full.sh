#!/usr/bin/env bash
set -euo pipefail

: "${RUSTUP_TOOLCHAIN:=nightly}"

run() {
  echo
  echo "+ $*"
  "$@"
}

run env RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN}" cargo fmt --all
run env RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN}" cargo test --all
run env RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN}" cargo clippy --all-targets -- -D warnings
run cargo run -p xtask -- ebpf-smoke

cat <<'EOF_MSG'

Non-privileged full validation completed.

Privileged validation is intentionally not run automatically because it may
prompt for doas/sudo credentials and touches privileged eBPF paths.

Run this manually when finalizing the branch:

doas env \
  HOME="$HOME" \
  CARGO_HOME="$HOME/.cargo" \
  RUSTUP_HOME="$HOME/.rustup" \
  PATH="$PATH" \
  RUSTUP_TOOLCHAIN=nightly \
  cargo run -p xtask -- privileged-ebpf-smoke
EOF_MSG
