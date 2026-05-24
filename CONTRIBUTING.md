# Contributing

This repository uses `xtask` commands as the local development gate. Run them with the nightly toolchain from the repository root.

## Normal local flow

1. Format the workspace:

   ```sh
   RUSTUP_TOOLCHAIN=nightly cargo fmt --all
   ```

2. Run the non-root CI gate before committing:

   ```sh
   RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- ci
   ```

3. For changes that touch eBPF, validation fixtures, safety-sensitive action logic, daemon/autotune behavior, or release-readiness checks, run the fuller validation gate:

   ```sh
   RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- validate
   ```

4. Use the preflight check when setting up a new machine or when a validation command fails before the Rust build starts:

   ```sh
   RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- preflight
   ```

## What the gates run

`cargo run -p xtask -- ci` runs:

```text
cargo fmt --check
cargo run -p xtask -- no-allow-attrs
cargo build
cargo clippy --all-targets -- -D warnings
cargo test
bash scripts/smoke/offline_recommendation.sh
bash scripts/smoke/advisor_offline.sh
cargo test -p stutter architecture_tests
```

`cargo run -p xtask -- validate` runs the same checks and additionally builds the eBPF crate explicitly with:

```text
rustup run nightly cargo build -p stutter-ebpf -Z build-std=core --target bpfel-unknown-none
```

## CI mapping

The GitHub workflow in `.github/workflows/ci.yml` mirrors the same policy in separate jobs:

- dependency hygiene and non-root build/smoke checks;
- daemon, action-runner, watchdog, soak, service, release, acceptance, and architecture safety gates;
- artifact-contract and validation-corpus fixture gates;
- optional live smoke tests on manual workflow dispatch.

Keep `xtask`, smoke scripts, and the workflow in sync when adding a required validation step.
