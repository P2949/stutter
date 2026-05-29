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
