# Dependency hygiene

This repository keeps dependency hygiene checks explicit so dependency,
license, advisory, and duplicate-version drift is visible during local
development and CI.

## Required CI/local gate

Install the required external cargo subcommand once:

```sh
cargo install cargo-deny --locked
```

Run the repository gate:

```sh
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- dependency-hygiene
```

The gate validates:

* `deny.toml`
* `Cargo.toml`
* `Cargo.lock`
* crate manifests such as `stutter/Cargo.toml`, `stutter-common/Cargo.toml`,
  `stutter-config/Cargo.toml`, `stutter-core/Cargo.toml`,
  `stutter-ebpf/Cargo.toml`, `stutter-report/Cargo.toml`, and
  `xtask/Cargo.toml`

The gate currently runs:

```sh
cargo deny check
cargo tree -d
dependency feature audit
```

`cargo deny check` enforces the repository advisory, license, source, and
dependency policy in `deny.toml`.

`cargo tree -d` reports duplicate dependency families. The xtask gate allows
the current baseline duplicate package names in `APPROVED_DUPLICATE_PACKAGES`
inside `xtask/src/main.rs` and fails when a new unapproved duplicate package
family appears. Prefer removing the duplicate by unifying dependency versions.
Only update the allowlist when the duplicate is intentional.

The dependency feature audit reports dependency shapes that need deliberate
review when they change:

* dependencies that still use default features
* optional dependencies that are not wired to an explicit feature
* resolved packages that expand the network/TLS surface

These sections are informational today, while `cargo deny check` and the
unapproved duplicate-version check remain the hard failures.

## Manual unused-dependency scan

`cargo machete` is useful for finding unused manifest dependencies, but it is
not a required CI gate yet because the workspace has low-level and generated
build paths that may need deliberate review before enforcing it.

Install and run it manually with:

```sh
cargo install cargo-machete --locked
cargo machete
```

If `cargo machete` reports an unused dependency, verify the relevant feature,
build script, generated code path, or target-specific path before removing it.

## Optional udeps scan

`cargo udeps` is optional and nightly-only. It is not wired into CI.

Install and run it manually with:

```sh
cargo install cargo-udeps --locked
RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo udeps --workspace --all-targets
```

Treat `cargo udeps` output as advisory until a separate patch intentionally
turns it into a gate.
