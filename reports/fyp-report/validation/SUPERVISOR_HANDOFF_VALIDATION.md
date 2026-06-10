# Supervisor handoff validation

Date: 2026-06-09
Validated handoff target: `fyp-report-final-v4`

Validation commands verified for `fyp-report-final-v4`:

- `RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo fmt --all -- --check`
- `RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- ci`
- `RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- fixture-check`
- `RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo run -p xtask -- dependency-hygiene`
- `cd evidence-bundle && sha256sum -c MANIFEST.sha256`
- `cd release && sha256sum -c ASSETS_SHA256SUMS`
- `cd release && sha256sum -c SHA256SUMS`

Expected result summary:

- Full non-root validation passes.
- Fixture-check passes.
- Dependency hygiene passes.
- Evidence bundle manifest verifies.
- Individual release asset checksums verify.
- Assembled release archive checksum verifies.

Notes:

- The exact commit SHA is recorded by the Git tag and GitHub release metadata.
- The release tag is the authoritative supervisor handoff target.
- Privileged eBPF loader smoke remains a manual/local check because it requires
  sudo/doas credentials and machine-specific kernel permissions.
