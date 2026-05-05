# Stutter Smoke Tests

This directory contains smoke tests for validating the `stutter` tool in both build-time and live environments.

## Scripts

- **`build.sh`**: Non-root script that performs a full validation suite: formatting check, build, clippy, and unit/integration tests.
- **`sleep_tree_churn.sh`**: Live smoke test that monitors a high-churn process tree using eBPF.
- **`record_minimal.sh`**: Live smoke test that records a short session and validates the structural integrity of the generated JSON/NDJSON artifacts.

## Live Test Requirements

- Live smoke tests (`sleep_tree_churn.sh`, `record_minimal.sh`) require **root** privileges or appropriate capabilities (e.g., `CAP_BPF`, `CAP_PERFMON`, `CAP_SYS_RESOURCE`) to load eBPF programs.
- If the required privileges are missing, these scripts will gracefully **skip** with a message and exit with code 0.

## Notes

- These are **smoke tests**, not benchmarks. They are intended to verify functional correctness and artifact structure, not to measure performance characteristics.
- Output from live tests is directed to `target/stutter-smoke/` by default.
