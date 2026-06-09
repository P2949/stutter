# Kernel Compatibility

Stutter decodes Linux tracepoints from eBPF. The eBPF object reads fields at
compiled offsets, so the userspace loader validates tracepoint format files from
the running kernel before attaching probes.

## Runtime preflight is the source of truth

Stutter intentionally does not validate tracepoint layouts against the build host kernel at compile time. Build hosts, package builders, and target machines
can run different kernels. The runtime preflight checks the target machine's
tracefs format files under:

```text
/sys/kernel/tracing/events
```

If a required scheduler tracepoint is missing or has different offsets, stutter
refuses to attach the affected eBPF program rather than decoding wrong bytes.

## Required scheduler tracepoints

The runnable-latency path requires compatible formats for:

* `sched/sched_wakeup`
* `sched/sched_switch`

Optional probes use additional tracepoints and may be disabled or degraded when
their formats are unavailable:

* `sched/sched_wakeup_new`
* `sched/sched_migrate_task`
* `power/cpu_frequency`
* `sched/sched_stat_wait`
* `irq/irq_handler_entry`
* `irq/irq_handler_exit`
* `block/block_rq_issue`
* `block/block_rq_complete`
* KMS/DRM tracepoints selected by driver/provider discovery

## Fixture coverage

Known-compatible tracepoint format fixtures live under:

```text
stutter/tests/fixtures/tracepoints/
```

These fixtures validate stutter's parser and expected offsets against kernel
tracefs format files. They do not replace runtime preflight.

When adding support evidence for a new kernel or distro flavor:

1. run `stutter doctor tracepoints --dump --json`;
2. record the kernel release and distro/kernel flavor;
3. add the captured tracepoint formats under a new fixture directory;
4. add the fixture directory to the tracepoint fixture test list;
5. run `cargo test -p stutter known_kernel_tracepoint`.

## Bug reports

For tracepoint compatibility issues, include:

```bash
stutter doctor tracepoints --dump --json
uname -a
cat /proc/version
```

Also include whether the kernel is distro, vanilla, CachyOS, Zen, Liquorix,
custom Gentoo, or another patched flavor.

## Packaging note

Distro packages should not treat build-host tracepoint validation as sufficient.
Tracepoint compatibility must be checked on the target runtime kernel.
