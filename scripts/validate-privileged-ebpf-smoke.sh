#!/usr/bin/env bash
set -euo pipefail

: "${RUSTUP_TOOLCHAIN:=nightly}"

exec doas env \
  HOME="$HOME" \
  CARGO_HOME="$HOME/.cargo" \
  RUSTUP_HOME="$HOME/.rustup" \
  PATH="$PATH" \
  RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN}" \
  cargo run -p xtask -- privileged-ebpf-smoke
