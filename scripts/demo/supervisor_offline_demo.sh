#!/usr/bin/env bash
set -euo pipefail

export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-nightly-2026-06-06}"
export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-never}"

echo "== stutter supervisor offline demo =="
echo "Toolchain: ${RUSTUP_TOOLCHAIN}"
echo

log_dir="target/supervisor-demo"
mkdir -p "${log_dir}"
build_log="${log_dir}/build.log"
fixture_log="${log_dir}/fixture-check.log"
dependency_log="${log_dir}/dependency-hygiene.log"

run_logged() {
  local label="$1"
  local log="$2"
  shift 2

  echo "Running ${label}; verbose output: ${log}"
  if ! "$@" >"${log}" 2>&1; then
    echo "${label} failed. Full output:"
    cat "${log}"
    exit 1
  fi
}

echo "== 0. Build demo binaries =="
echo "Capturing cargo/build-script output in ${build_log}"
if ! cargo build -p xtask -p stutter >"${build_log}" 2>&1; then
  echo "Build failed. Full cargo output:"
  cat "${build_log}"
  exit 1
fi
echo "Build completed."

xtask_bin="target/debug/xtask"
stutter_bin="target/debug/stutter"

echo
echo "== 1. Fixture check =="
run_logged "fixture check" "${fixture_log}" "${xtask_bin}" fixture-check
grep -E "^(fixture coverage|real fixtures:|synthetic fixtures:|vendors:|compositors:|kernels:|data quality:|known false positives:|known false negatives:|missing:|maturity warnings:|privacy warnings:|test result: ok)" "${fixture_log}" || true
echo "Fixture check passed."

echo
echo "== 2. Dependency hygiene =="
run_logged "dependency hygiene" "${dependency_log}" "${xtask_bin}" dependency-hygiene
grep -E "^(advisories ok|duplicate versions:|network/TLS dependency surface:|unused optional feature mappings:)" "${dependency_log}" || true
echo "Dependency hygiene passed."

echo
echo "== 3. CLI shape =="
"${stutter_bin}" --version
"${stutter_bin}" profile-plan --help | sed -n '1,80p'
"${stutter_bin}" tune --help | sed -n '1,80p'

echo
echo "== 4. Evidence bundle manifest =="
(
  cd evidence-bundle
  sha256sum -c MANIFEST.sha256
)

echo
echo "Offline demo completed successfully."
echo "Captured logs:"
echo "  build: ${build_log}"
echo "  fixture check: ${fixture_log}"
echo "  dependency hygiene: ${dependency_log}"
