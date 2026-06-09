#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

prefix="${PREFIX:-$HOME/.local}"
while [ "$#" -gt 0 ]; do
	case "$1" in
		--prefix)
			prefix="${2:?--prefix requires a path}"
			shift 2
			;;
		--prefix=*)
			prefix="${1#--prefix=}"
			shift
			;;
		*)
			echo "unknown argument: $1" >&2
			exit 64
			;;
	esac
done
bindir="$prefix/bin"
toolchain="${RUSTUP_TOOLCHAIN:-nightly-2026-06-06}"

mkdir -p "$bindir"
RUSTUP_TOOLCHAIN="$toolchain" cargo build --release -p stutter
install -m 0755 target/release/stutter "$bindir/stutter"

echo "installed $bindir/stutter"
echo "ensure $bindir is in PATH"
echo "stutter still needs privileges to load eBPF. Use sudo/doas when recording."
