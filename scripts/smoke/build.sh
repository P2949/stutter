#!/usr/bin/env bash
set -euo pipefail

# Navigate to repo root
cd "$(dirname "$0")/../.."

TOOLCHAIN="${RUSTUP_TOOLCHAIN:-nightly}"

echo "--- STAGE: cargo fmt ---"
RUSTUP_TOOLCHAIN="$TOOLCHAIN" cargo fmt --check

echo "--- STAGE: cargo build ---"
RUSTUP_TOOLCHAIN="$TOOLCHAIN" cargo build

echo "--- STAGE: cargo clippy ---"
RUSTUP_TOOLCHAIN="$TOOLCHAIN" cargo clippy --all-targets -- -D warnings

echo "--- STAGE: cargo test ---"
RUSTUP_TOOLCHAIN="$TOOLCHAIN" cargo test

echo "--- STAGE: offline workflow smoke scripts are separate CI steps ---"
echo "scripts/smoke/offline_recommendation.sh"
echo "scripts/smoke/advisor_offline.sh"

echo "PASS"
