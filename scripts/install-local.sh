#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

prefix="${PREFIX:-$HOME/.local}"
bindir="$prefix/bin"
toolchain="${RUSTUP_TOOLCHAIN:-nightly}"

mkdir -p "$bindir"
RUSTUP_TOOLCHAIN="$toolchain" cargo build --release -p stutter
install -m 0755 target/release/stutter "$bindir/stutter"

echo "installed $bindir/stutter"
echo "ensure $bindir is in PATH"
echo "stutter still needs privileges to load eBPF. Use sudo/doas when recording."
