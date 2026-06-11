# Supervisor handoff validation

Date: 2026-06-11
Validated handoff target: `fyp-report-final-v4` cleanup worktree
Validated base commit: `145e7c2e78348d78db3f1a7959b7bb1bb8e1152c`
Toolchain used: system cargo (`rustup` unavailable in this validation
environment)
Cargo version: `cargo 1.95.0 (f2d3ce0bd 2026-03-21) (gentoo)`

Validation commands verified for the cleanup worktree:

- `cargo fmt --all --check`
- `cargo test -p stutter watch_process_selection_treats_wine_backslashes_as_path_separators`
- `cargo run -p xtask -- ci`
- `cargo run -p xtask -- fixture-check`
- `cargo run -p xtask -- dependency-hygiene`
- `scripts/demo/supervisor_offline_demo.sh`
- `cd evidence-bundle && sha256sum -c MANIFEST.sha256`
- `cd release && sha256sum -c ASSETS_SHA256SUMS`
- `cd release && sha256sum -c SHA256SUMS`

Expected result summary:

- Full non-root validation passes.
- Targeted Wine-path regression test passes.
- Fixture-check passes.
- Dependency hygiene passes.
- Supervisor offline demo passes.
- Evidence bundle manifest verifies.
- Individual release asset checksums verify.
- Assembled release archive checksum verifies.

Notes:

- The cleanup edits were validated in the working tree on top of the base commit
  above. A final release tag should be checked with `git rev-parse HEAD` and
  `git rev-parse fyp-report-final-v4` after committing/tagging this cleanup.
- The release tag is the authoritative supervisor handoff target once updated.
- The supervisor demo script exports `RUSTUP_TOOLCHAIN=nightly-2026-06-06` by
  default, but this environment used system Cargo because `rustup` is not
  installed.
- Privileged eBPF loader smoke remains a manual/local check because it requires
  sudo/doas credentials and machine-specific kernel permissions.
