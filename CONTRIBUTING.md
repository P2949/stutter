# Contributing

This repository uses `xtask` commands as the local development gate. Run them with the nightly toolchain from the repository root.

## Normal local flow

1. Format the workspace:

   ```sh
   RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo fmt --all
   ```

2. Run the non-root CI gate before committing:

   ```sh
   RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- ci
   ```

3. For changes that touch eBPF, validation fixtures, safety-sensitive action logic, daemon/autotune behavior, or release-readiness checks, run the fuller validation gate:

   ```sh
   RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- validate
   ```

   For real validation-corpus fixtures, also follow `docs/VALIDATION_CORPUS.md`:
   complete `fixture.toml`, preserve only bucketed platform details, set
   expected behavior, use a unique `sanitized_capture_id`, and run
   `RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- fixture-check`.

4. Use the preflight check when setting up a new machine or when a validation command fails before the Rust build starts:

   ```sh
   RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- preflight
   ```

## Action boundary and string ID migration validation

When touching action boundary errors or raw `action_id` / `experiment_id`
fields, run the focused migration guard before the broader validation gates:

```sh
scripts/validate-action-boundary-string-id-migration.sh
```

This verifies that action boundary modules use typed errors instead of
string-coded `anyhow::bail!`, rollback framework paths are excluded
precisely, `ActionBoundaryError` round-trips through `ActionError` serde,
and remaining raw string ID migration debt is tracked with reasons, exit
criteria, and counts.

## Privileged eBPF smoke

The normal `ci` and `validate` gates are non-root. For changes that affect eBPF loading, eBPF map names or types, tracepoint attach logic, or BPF object build/load behavior, also run the ignored privileged loader smoke test on a Linux machine with tracefs mounted and eBPF privileges:

```sh
sudo -E env PATH="$PATH" RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- privileged-ebpf-smoke
```

Run this before merging loader-sensitive changes. It intentionally stays outside the default validation flow because it requires kernel facilities and privileges that regular CI and local non-root checks should not assume.

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
rustup run nightly-2026-06-06 cargo build -p stutter-ebpf -Z build-std=core --target bpfel-unknown-none
```

`cargo run -p xtask -- privileged-ebpf-smoke` first runs preflight, then builds the eBPF crate and runs:

```text
cargo test -p stutter ebpf_loader::tests::load_and_attach_real_bpf_object_smoke_test -- --ignored --exact --nocapture
```

## CI mapping

The GitHub workflow in `.github/workflows/ci.yml` mirrors the same policy in separate jobs:

- dependency hygiene and non-root build/smoke checks;
- daemon, action-runner, watchdog, soak, service, release, acceptance, and architecture safety gates;
- artifact-contract and validation-corpus fixture gates;
- optional live smoke tests on manual workflow dispatch.

Keep `xtask`, smoke scripts, and the workflow in sync when adding a required validation step.
