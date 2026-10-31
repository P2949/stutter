# Release Checklist

This checklist separates source/runtime readiness from production distro
packaging readiness.

## Source/runtime readiness

A source/runtime release may be suitable for technical local users when:

- required release gates for the selected channel pass;
- observe/report paths are validated;
- apply modes remain gated by daemon policy;
- rollback and emergency restore behavior are tested for enabled apply families;
- docs describe the supported install path honestly.

## Service-unit/local-install readiness

The current supported install path is:

```bash
scripts/install-local.sh
```

Service templates under `packaging/systemd/` and `packaging/openrc/` document the
intended daemon service shape. Passing service-unit release gates means these
templates and local install/service commands are coherent. It does **not** mean
that distro packages are production-ready.

## Production distro packaging readiness

Do not claim production-ready distro packaging (or the `production_distro_packaging` gate) until all of these are true:

* reproducible packaged eBPF object build or release artifact path exists;
* Gentoo/Arch/tarball packaging has install/layout tests;
* packaged service units have start/stop smoke tests;
* versioned release tarballs/artifacts are published;
* package docs no longer need skeleton-only warnings;
* the ebuild/PKGBUILD path works without local developer-only adjustments.

The release command tracks this separately through advisory gates:

```bash
stutter release check \
  --channel low-risk-stable \
  --production-distro-packaging \
  --reproducible-packaged-ebpf-object \
  --packaging-install-tests \
  --packaging-service-smoke-tests \
  --versioned-release-tarball
```

These flags should only be passed when the packaging evidence exists.

## Release contract (v0.1.0 and later)

`cargo run -p xtask -- release-package --binary BIN --bpf-object OBJ --out DIR` builds,
from a clean committed tree, the artifacts named below; `release-verify DIR`
recomputes and checks them independently. `cargo run -p xtask -- release-check [--out DIR]
[--expected-tag vX.Y.Z]` is the local release gate: it builds `stutter --release`, locates the
required eBPF object, runs both commands and checks the tag against the crate version. It runs
`release-verify` without `--require-acceptance`, so it never declares live acceptance; that is the
exact-RC path below. There is no hosted release workflow.

- `stutter-<version>-linux-x86_64.tar.gz`: the layout in `packaging/tarball/MANIFEST.txt`,
  reproducible (sorted names, fixed owner, commit-time mtime).
- `stutter-<version>-source.tar.gz`: `git archive` of the exact commit.
- `sbom.cdx.json`: CycloneDX component list derived from `Cargo.lock`.
- `SUPPORT.md`: support statement generated from the capability registry.
- `release-manifest.json`: version, full git revision, `Cargo.lock` / binary / eBPF object
  digests, artifact-compatibility schema versions, supported capabilities, packaged file
  digests.
- `SHA256SUMS`: digests of every published file.

**eBPF object:** the supported package ships `lib/stutter/stutter.bpf.o` and the binary
embeds an identical object (`STUTTER_BPF_OBJECT` is only a runtime override). A missing,
empty or non-ELF object is a release failure, never a warning.

`release-verify` checks digests and `SHA256SUMS` coverage, unpacks both archives, matches
the packaged layout and file digests, requires the `Cargo.lock` digest to equal the source
archive's, requires the packaged binary to report the manifest version and git revision,
requires the support statement and `supported_capabilities` to equal the capability
registry (a release can never claim more than the registry), and checks the SBOM covers
every locked package. On tags, `--require-acceptance` additionally requires, via repeatable
`--acceptance-bundle BUNDLE`, exported evidence bundles that are verified on the spot (manifest,
content and privacy), are `PASS`, and whose `source_commit` equals the release commit, covering
every release suite (`required_release_evidence`) of every Supported capability. It refuses a release with no
Supported capability, any bundle that fails verification (in any order), a bundle for another
commit, a missing required suite and a capture supplied twice. The committed
`docs/evidence/index.json` is the historical catalogue and never satisfies this gate: evidence
for commit X cannot be committed into X, so the bundles stay outside the release commit.

Tag the intended release version only on the exact commit that passed the required
exact-release acceptance; do not validate one commit and tag another.

## Publication (authoritative sequence)

```text
exact-RC live acceptance -> release-package -> release-verify --require-acceptance
  -> install smoke -> tag -> release-publish
```

`cargo run -p xtask -- release-publish DIR --acceptance-bundle BUNDLE... --notes NOTES.md`
re-runs the independent verification with the external bundles, requires the local and origin
tag to resolve to the manifest's commit, and only then creates the GitHub Release (non-draft,
attached to the existing tag) with the six assets. It is idempotent: an identical release is a
no-op, missing assets are uploaded, and an asset whose remote digest differs or a release on
another tag is a hard failure; nothing is ever overwritten. A historical package must be
verified and published with the xtask built from its own tag, because verification compares the
package with the capability registry compiled into the verifier.

`release-publish` is the one optional GitHub adapter: it is publication, not verification, and
needs `gh` (`nix develop .#publish`). Nothing in release validity, exact-RC acceptance or tagging
consults GitHub, and pushing a valid local tag is likewise outside verification.

## Autonomous campaign path

`cargo run -p xtask -- validation-campaign continue` performs the sequence above (freeze, exact-RC
offline and live acceptance, packaging, verification, installation smoke, tag) without
interactive confirmation; see `docs/VALIDATION_CAMPAIGN.md`. The manual commands in this file
remain the reference for what each step runs.

## Installed-package smoke

`cargo run -p xtask -- release-install-smoke --package DIR --recording RECORDING [--expect-commit SHA]`
unpacks the packaged tarball and exercises the installed binary on the exact-RC observation
recording. The expected version, git revision, Supported capability list and eBPF digest come from
the package's own `release-manifest.json`, not from a validation-campaign manifest. The Core
Validation campaign's install-smoke task runs the same implementation.
