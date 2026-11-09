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
