# Contributing

This repository uses `xtask` commands as the local development gate. Run them with the nightly toolchain from the repository root.

## Nix / NixOS development environment

On systems with Nix and flakes enabled, the preferred development environment is the repository-local flake:

```sh
nix develop
```

The shell provides the pinned `nightly-2026-06-06` Rust toolchain, `rust-src`,
`rustfmt`, `clippy`, `bpf-linker`, `cargo-deny`, `rust-analyzer`, Git, and Python
for the offline smoke scripts.
The flake takes bpf-linker from a separate locked nixpkgs input and builds
it against LLVM 22 to match the pinned compiler's bitcode. The stable
nixpkgs linker predates LLVM 22 support.

The Nix development environment deliberately does not install or use rustup.
The Rust compiler, Cargo, Rust components, and eBPF linker should resolve from
the Nix store. Do not install `bpf-linker` with `cargo install` merely to make
the Nix shell work.

The repository still uses `RUSTUP_TOOLCHAIN=nightly-2026-06-06` as the canonical
toolchain identity because the build and validation code also supports
non-Nix/rustup environments. When rustup is absent, Stutter and Aya use the
compiler already supplied on `PATH`.

Before diagnosing a Nix-shell build failure, verify:

```sh
command -v rustc
command -v cargo
command -v bpf-linker
command -v cargo-deny
command -v rustup || true
rustc --version
cargo --version
```

The Nix-owned commands should resolve beneath `/nix/store`, and `rustup` should
not be visible inside the Stutter Nix shell.

## Normal local flow

1. Format the workspace:

   ```sh
   RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo fmt --all
   ```

2. Run the non-root CI gate before pushing (`xtask setup-local-ci` installs it as a hook):

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

The normal `ci` and `validate` gates are non-root. For changes that affect eBPF loading, eBPF map names or types, tracepoint attach logic, or BPF object build/load behavior, also run the gated privileged loader smoke suite on a Linux machine with tracefs mounted and eBPF privileges:

```sh
scripts/validate-privileged-ebpf-smoke.sh
```

The wrapper selects `doas` when available and otherwise uses `sudo`, while
keeping Cargo and compilation unprivileged. Only the finished loader test
executable runs under privilege through Cargo's native target runner. Run the
wrapper as your normal user; it rejects root invocation.

Run this before merging loader-sensitive changes. It intentionally stays outside the default validation flow because it requires kernel facilities and privileges that regular CI and local non-root checks should not assume.

## What the gates run

Verification is repository-owned and local-only; see [docs/LOCAL_VERIFICATION.md](docs/LOCAL_VERIFICATION.md).
Hosted CI and GitHub-runner-dependent verification must not be reintroduced.

- `cargo run -p xtask -- verify-fast` (the pre-commit hook): policy, fmt, lint contracts, diff check.
- `cargo run -p xtask -- ci` (the pre-push hook): the full deterministic non-root CI in named stages;
  `--stage NAME` reruns one.
- `cargo run -p xtask -- ci-live`: the live smokes with non-interactive privilege.
- `cargo run -p xtask -- release-check`: local release packaging and verification.
- `cargo run -p xtask -- ci-all`: all of the above.
- `cargo run -p xtask -- setup-local-ci`: install the hooks for this checkout.

`cargo run -p xtask -- validate` runs the same checks and explicitly builds
Stutter again. Its Aya build script compiles the eBPF crate for
`bpfel-unknown-none` using `-Z build-std=core`.

When rustup is unavailable, the Aya build path uses the current compatible
nightly compiler supplied by the environment.

`cargo run -p xtask -- privileged-ebpf-smoke` runs preflight and invokes the
same wrapper. The wrapper builds Stutter and runs the gated
`ebpf_loader::privileged_loader_tests::privileged_` suite with
`STUTTER_RUN_PRIVILEGED_EBPF_TESTS=1` on the test executable. It uses an
explicit native target so the privilege runner cannot affect build scripts.

## CI mapping

There is no hosted CI. `xtask ci` is the single list of required checks (`xtask/src/local_ci.rs`);
`xtask local-ci-policy` rejects hosted-CI configuration. Keep `xtask`, the smoke scripts and
[docs/LOCAL_VERIFICATION.md](docs/LOCAL_VERIFICATION.md) in sync when adding a required step.
