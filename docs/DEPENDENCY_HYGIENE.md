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

## Aya git dependency

Aya is intentionally pinned to a specific upstream git revision rather than a
floating branch. The project uses Aya's userspace and eBPF-side crates together,
and the pinned revision keeps those APIs synchronized with `Cargo.lock` for
reproducible source checkouts. This is a prototype dependency-management choice,
not an unpinned latest-source dependency.

Supervisor note: Aya is consumed from a pinned git revision because the project
uses Aya's userspace and eBPF-side APIs together, and the pinned revision keeps
the source checkout reproducible.

## Cargo deny bans policy

`deny.toml` currently allows multiple versions and wildcard dependency
requirements because `stutter` is still a research prototype with a large
transitive dependency graph. The deny gate is still useful: it denies yanked
crates, unknown registries, unknown git sources, and non-allowlisted licenses.
Duplicate versions are highlighted for review rather than treated as release
blockers.

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
