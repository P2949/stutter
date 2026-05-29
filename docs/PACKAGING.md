# Packaging Guide

> Packaging status: distro packaging is currently skeleton/experimental. See
> [INSTALL.md](INSTALL.md#packaging-status) and
> [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md) before describing any package path
> as production-ready.

## Default Build

Building stutter builds both the userspace binary and the eBPF object locally:

```bash
cargo build --release
```

This requires a Rust nightly toolchain with `rust-src` and the `bpf-linker` tool.

## Prebuilt eBPF Object

Packagers can provide a prebuilt eBPF object to avoid requiring the eBPF build
toolchain during userspace compilation. This is useful for distribution packages,
containers, or CI environments where the eBPF toolchain is not available.

### Build-Time Override

Set `STUTTER_USE_PREBUILT_BPF=1` and `STUTTER_BPF_OBJECT` to embed a prebuilt
object during the userspace build:

```bash
STUTTER_USE_PREBUILT_BPF=1 \
STUTTER_BPF_OBJECT=/usr/lib/stutter/stutter.bpf.o \
cargo build --release
```

When `STUTTER_USE_PREBUILT_BPF=1` is set:

- `STUTTER_BPF_OBJECT` must point to a valid, non-empty eBPF object file.
- The build script copies that object into `OUT_DIR` and skips local eBPF
  compilation entirely.
- No eBPF toolchain, `bpf-linker`, or `rust-src` is required.

If `STUTTER_USE_PREBUILT_BPF=1` is set without `STUTTER_BPF_OBJECT`, the build
will fail with a clear error.

### Runtime Override

Developers can also override the eBPF object at runtime without rebuilding:

```bash
STUTTER_BPF_OBJECT=/usr/lib/stutter/stutter.bpf.o stutter monitor --pid 1234
```

When `STUTTER_BPF_OBJECT` is set at runtime:

- The loader reads the specified file instead of using the embedded object.
- If the file is missing, unreadable, or empty, stutter fails with a clear
  error. It does **not** silently fall back to the embedded object.
- All runtime tracepoint validation remains active and equally strict.

## ABI and Kernel Compatibility

A prebuilt eBPF object must match the userspace ABI — specifically the ring
buffer event structures, map names, and program entry points expected by the
loader. Mismatched objects will fail at load time.

Runtime validation still decides whether the object is compatible with the
current kernel. The loader validates tracepoint formats and patches dynamic
offsets into BPF globals at load time, exactly as it does for locally-built
objects. A prebuilt object that passes validation on one kernel may fail on
another if the kernel's tracepoint layout differs.

> **This is an advanced packaging escape hatch.** For most users, the default
> local build produces the correct object automatically.

## Release Artifacts

When a tagged release workflow publishes artifacts, the release should include
the userspace binary and a matching `stutter.bpf.o` eBPF object. Packagers can
then use that object with the build-time or runtime override.

Until that artifact path is part of the release process, distro packaging should
be treated as skeleton/experimental and local builds should use the standard
nightly eBPF build path.

## Service Packaging

Package managers should install the binary, the selected service units, and
empty state/config/log directories:

```text
/usr/bin/stutter
/etc/stutter
/var/lib/stutter
/var/log/stutter
```

The built-in planner can be used as a packaging smoke check:

```bash
stutter service doctor --mode system-observe --manager systemd-system
stutter service doctor --mode system-low-risk --manager openrc
stutter service install --dry-run --mode system-observe --manager systemd-system
```

Templates live under:

```text
packaging/systemd/
packaging/openrc/
packaging/arch/PKGBUILD
packaging/gentoo/stutter-9999.ebuild
packaging/tarball/MANIFEST.txt
```

The service command installs or removes unit files but intentionally does not
enable services automatically. Low-risk apply services must remain opt-in and
must preserve the `stutter daemon emergency-restore` hook on stop.

The packaged agent service should prefer Unix sockets over TCP. The upstream
systemd and OpenRC units bind `/run/stutter/agent.sock` by default; packagers
should only expose `--bind 127.0.0.1:9899` as an explicit compatibility
override.

For deployed services, prefer separate token files instead of a shared
full-access token. `--read-token-file` is suitable for status clients, while
`--apply-token-file` is required for state-changing endpoints such as record
start/stop, autotune start/stop, daemon pause/resume, and restore. The agent
has built-in request body and request rate limits and does not install a CORS
policy by default.

## Release Readiness Gates

Use the release gate command before tagging:

```bash
stutter release check --channel experimental
stutter release check --channel observe-stable --enforce
stutter release check --channel low-risk-stable --soak-tests --real-machine-validation --enforce
```

The command above checks source/runtime readiness and service-unit/local-install
readiness. It does not claim production-ready distro packaging. Packaging
readiness is visible as advisory gates unless explicitly marked with:

```bash
stutter release check \
  --channel low-risk-stable \
  --soak-tests \
  --real-machine-validation \
  --production-distro-packaging \
  --reproducible-packaged-ebpf-object \
  --packaging-install-tests \
  --packaging-service-smoke-tests \
  --versioned-release-tarball
```

`observe-stable` fails if apply actions are enabled. `low-risk-stable` requires
the action runner, rollback, crash recovery, service-unit/local-install support,
docs, and soak test evidence. This is **not** the same as production-ready
distro packaging.

Production distro packaging is tracked separately by advisory release gates:

- `production_distro_packaging`
- `reproducible_packaged_ebpf_object`
- `packaging_install_tests`
- `packaging_service_smoke_tests`
- `versioned_release_tarball`

These gates default to `not claimed`. Do not describe Gentoo, Arch, tarball, or
other distro packaging as production-ready unless those packaging gates are
explicitly met and the package path has been tested.

For a no-root deterministic soak smoke test:

```bash
stutter daemon soak --duration-seconds 3600 --profile observe-only
stutter daemon soak --duration-seconds 3600 --profile apply-low-risk-fake
```

The fake soak checks bounded memory, file descriptor, disk/history growth,
event queue depth, task count, CPU overhead, wakeups, and event drops. It is not
a replacement for a real privileged multi-hour soak, but it gives CI a cheap
budget gate.

The CI `acceptance` test enumerates the final daemon acceptance scenario as a
fake no-root suite. It must stay green, but release candidates still need the
real-machine acceptance pass described in the daemon roadmap before claiming a
low-risk-stable channel.

You can run the same no-root acceptance checklist locally:

```bash
stutter daemon acceptance
stutter daemon acceptance --json
```

## Environment Variable Reference

| Variable | Phase | Description |
|---|---|---|
| `STUTTER_USE_PREBUILT_BPF` | Build | Set to `1` to skip local eBPF compilation and use `STUTTER_BPF_OBJECT` |
| `STUTTER_BPF_OBJECT` | Build / Runtime | Path to a prebuilt eBPF object file |
