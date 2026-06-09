#!/usr/bin/env bash
set -euo pipefail

export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-nightly-2026-06-06}"

echo "== stutter supervisor offline demo =="
echo "Toolchain: ${RUSTUP_TOOLCHAIN}"
echo

echo "== 1. Fixture check =="
cargo run -p xtask -- fixture-check

echo
echo "== 2. Dependency hygiene =="
cargo run -p xtask -- dependency-hygiene

echo
echo "== 3. CLI shape =="
cargo run -p stutter -- --version
cargo run -p stutter -- profile-plan --help | sed -n '1,80p'
cargo run -p stutter -- tune --help | sed -n '1,80p'

echo
echo "== 4. Evidence bundle manifest =="
(
  cd evidence-bundle
  sha256sum -c MANIFEST.sha256
)

echo
echo "Offline demo completed successfully."
