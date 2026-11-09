# Live mechanism checks of the new typed operations (2026-11-09)

`MechanismValidated` only: physical write, kernel readback and restore. **No benefit is claimed** and
no workload outcome was measured. Run as root on the development host (kernel 6.18.57) through the
`#[ignore]` live tests in `stutter/src/actions`, with each scoped to something disposable. Raw
output: [raw.txt](raw.txt). Nothing was left behind (no cgroup, no loop device, host congestion
control unchanged at `cubic`).

| Operation | Scope | Observed |
|---|---|---|
| `PmQosCpuLatency` | the real `/dev/cpu_dma_latency`, 1 ms lease | aggregate target 2000000000 us before, **1000 while held**, 2000000000 after rollback |
| `CgroupCpuWeight`, `CgroupCpuIdle`, `CgroupCpuMax`, `CgroupMemoryHigh`, `CgroupMemoryLow`, `CgroupMemoryMin` | an empty disposable cgroup under `/sys/fs/cgroup` | each written, read back from the kernel, restored to its baseline. `cpuset.*` and `io.weight` were not exercised: the root does not enable those controllers here |
| `NetTcpCongestionControl`, `NetTcpBuffers` (rmem and wmem) | a private network namespace (`unshare -n`) | each written, read back, restored. `net.core.busy_poll` does not exist outside the initial namespace, and the action correctly refused it as `Unavailable` rather than reaching another namespace |
| `BlockIoScheduler` | a disposable loop device | `[none] mq-deadline kyber` to `none [mq-deadline] kyber` and back |

Not covered by this evidence: resctrl (not mounted), DAMOS (root-only interface, no isolated
workload), NUMA migration (single node), the sched_ext strategy (no `scx_loader`), `busy_poll` in the
initial namespace (would touch the host), and any mechanism through the privileged worker.

## DAMOS: BlockedResource on this host (live refusal, cleanly rolled back)

A `[kdamond.0]` kernel thread has been running since boot, so DAMON is already owned by an in-kernel
module here (its module parameters are root-only readable through the sysfs permission set used by
the test). With `nr_kdamonds` at 0, the live test created one kdamond and configured a monitoring-only
`stat` scheme for its own process. By hand as root every configuration write (operations, one target,
one scheme, action, quotas, size bounds) succeeded, but starting it (`state` = `on`) was refused by the
kernel: `EINVAL` once and `EBUSY` on later attempts. The transaction reported `not in effect`,
tore its configuration down, and `nr_kdamonds` was back at 0 afterwards. The mechanism is therefore
**not validated** (`BlockedResource`), while the refusal path, the cleanup and the "never touch
another owner's kdamond" rule were exercised on the real kernel.

## sched_ext: real kernel switch and restore observed (no `scx_loader`)

`/sys/kernel/sched_ext/state` was `disabled`. `scx_simple` (nixpkgs `scx_cscheds`, run as root for
8 s with SIGINT) took the kernel to `state=enabled`, `root/ops=simple`, `enable_seq=1`; after the
loader exited ("unregistered from user space") the state was `disabled` again with no scheduler
left behind. `stutter-autotune-bounded@p2949` and `stutter-privileged-worker@p2949` stayed active
throughout. This exercises the kernel enable/disable path the `sched_ext` sensor reads. It does
**not** validate the `scx_loader` D-Bus switch (5.7): no `scx_loader` exists on this host, so that
entry stays `HardwareUnavailable`/`EvidenceDeferred`. NUMA remains impossible with one node.

`scx_loader` availability (checked 2026-11-09): nixpkgs `scx.full`/`scx.rustscheds` 1.1.4 and
`scx.cscheds` were fetched and contain no `scx_loader` binary or D-Bus service file, and there is
no standalone nixpkgs package for it. The 5.7 D-Bus switch therefore stays unvalidated here; it
needs a distribution or build that ships `scx_loader` (e.g. the upstream `scx-loader` crate).

## Delegated validation subtree: owner death (live, real kernel)

`delegation::contain_end` (normal exits) and the new `DelegatedReaper` (owner death) were run
against a real cgroup v2 delegation with the ignored live tests in
`stutter-validation-runner/src/delegation.rs`:

- `live_containment_end_kills_a_setsid_grandchild_and_spares_an_unrelated_process`: a `setsid`,
  TERM-ignoring grandchild inside the subtree was killed by the ordinary cleanup; an unrelated
  sentinel survived; the subtree was removed.
- `live_owner_sigkill_leaves_no_descendant_in_the_delegated_subtree`: the heartbeat was held by a
  separate owner process which was then `SIGKILL`ed with the grandchild running; within 5 s the
  independent reaper had emptied the subtree, the sentinel survived, and a later ordinary cleanup
  found it empty and removed it.

## scx_loader D-Bus switch and restore: real loader on a private bus (2026-11-09)

The pinned nixpkgs ships no `scx_loader`, so the upstream crate `scx_loader` 1.1.3 was built from
crates.io (`cargo install --locked`, binary sha256
`12703406550c556a369872b538ea06b810bf6dbdb79945680746acffff1df699`) and run as root on a **private** root-run
`dbus-daemon` socket under `/tmp`; the system bus and the host's services were never touched.
`scx_bpfland` 1.1.4 (nixpkgs `scx.rustscheds`) was the scheduler. The ignored live test
`live_real_scx_loader_switch_and_restore_over_a_private_bus` then ran as the unprivileged user:

- authorization was minted by the **production** factory from a receipt a root process wrote under
  a root-owned path (scope `scheduler_switch`, this user, this boot) — no fixture constructor;
- the adapter's introspection, planning and `StartScheduler` call matched the real loader's method
  signatures; the loader reported `scx_bpfland` (mode 0) and the kernel's
  `/sys/kernel/sched_ext/state` read `enabled`; restore returned the loader to no scheduler and the
  kernel to `disabled`.

Two defects in the adapter were found only by this run and fixed: (1) the loader answers the call
as soon as it has spawned the scheduler and the kernel attaches it a moment later, so reading once,
immediately, reported a working switch as not in effect (it now polls for up to 5 s); (2) the kernel
names the scheduler `bpfland_1.1.4_x86_64_unknown_linux_gnu`, with a version and target suffix,
which the exact-name check rejected (it now accepts the bare name or the bare name followed by
`_<digit>`, and still rejects lookalikes such as `bpfland_other`). Both are covered by unit tests.

**What this is and is not.** The real loader's polkit check ("Checking auth") needs a polkit
authority, which a private bus does not have; a minimal stub authority that authorizes every request
was run on the private bus only. So the loader's own authorization layer is *not* exercised, the
loader was not the packaged system service, and no matched-workload benefit was measured. This is a
`RealObserved` mechanism result for the adapter against the real loader, not `MechanismValidated`
for a production `scx_loader` deployment. A first run of this test left `scx_bpfland` running when
the loader was killed (the loader does not stop its child); it was stopped by pid and the cleanup now
stops the scheduler by exact name and checks `sched_ext` is `disabled`.
