# Safety

`stutter` is designed to keep observation, recommendation, and system changes separate. The daemon policy contract in [docs/DAEMON_CONTRACT.md](DAEMON_CONTRACT.md) is the source of truth for mode labels, default denials, rollback requirements, and developer rules.

System-changing daemon actions are enforced through `DaemonPolicy::check_action` and `ActionDescriptor`; they are not controlled only by comments, help text, or convention.

## eBPF Privilege Boundary

Recording and live tracing require privileges on most systems because `stutter` loads eBPF programs. Offline commands such as `report`, `recommend`, `advisor`, and `audit` read files and do not need root.

Use `sudo` or `doas` only for commands that actually need live tracing.

The daemon privilege model separates three roles:

- `privileged_worker`: the small process role that may load eBPF, attach probes, apply actions, rollback actions, and write protected state
- `control_plane`: the local agent/API role that may request allowlisted privileged work over local control transport
- `ui_client`: an unprivileged status/reporting client

Privileged worker operations are represented by a typed allowlist in
`daemon::privilege`. Local Unix sockets are the preferred control transport.
Loopback TCP requires apply/control authorization for state-changing requests.
Non-loopback TCP is not allowed to request privileged worker operations, even
when a bearer token is present.

Medium-risk autotune apply uses a separate privileged worker instead of an
in-process mutator. Start it with:

```bash
stutter privileged-worker --socket /run/stutter/privileged-worker.sock
```

The worker listens on a Unix domain socket with mode `0600`; that filesystem
permission is the authentication boundary for the local control plane. The
unsafe in-process mutator is reserved for tests and explicit development config
(`autotune.unsafe_in_process_privileged_worker = true` with
`experimental = true`).

Every privileged operation has a stable audit action id, for example
`privilege-start-recording`, `privilege-apply-action`, and
`privilege-rollback-action`.

## Lifecycle Boundaries

The daemon treats suspend/resume, target restart or exit, cgroup movement, GPU
reset, compositor restart, and CPU topology changes as measurement boundaries.
These events flow through `daemon::lifecycle`, which turns them into explicit
daemon actions such as pausing experiments, clearing measurement windows,
refreshing target identity, refreshing eBPF maps, waiting for stabilization, or
scheduling rollback of the active experiment.

When a boundary invalidates an active experiment and no trusted rollback is
available, the daemon drops back to observe-only mode and clears active
experiment state rather than continuing to apply stale tuning. Resume events are
also surfaced through system health as `suspend_resume_stabilizing`, which blocks
new apply work until the runtime has stabilized.

## Restore Files

Managed profile application records original task state before changing affinity, nice, or ionice values. The current managed profile restore file is:

```text
~/.local/state/stutter/last_profile_restore.json
```

Legacy CPU-affinity-only restore state may also use:

```text
~/.local/state/stutter/last_affinity_restore.json
```

Restore saved profile state with:

```bash
stutter restore
```

Inspect what would be restored:

```bash
stutter restore --dry-run
```

`apply-profile --force` can replace an existing restore file. Without `--force`, new restore records are merged while preserving the earliest known original mask or priority for each task identity.

## Action Audit Log

Actions that change system state append audit events to:

```text
~/.local/state/stutter/audit/actions.jsonl
```

Inspect recent events:

```bash
stutter audit
stutter audit --tail 50
stutter audit --json
```

Audit entries include command name, action id, safety class, dry-run status, success flag, affected task count, restore path, and a message.

## No Broad Auto-tuning Yet

`stutter` deliberately does not auto-apply broad system tuning. Recommendations are candidates and suggested experiments, not proof. A stable recommendation can justify a manual test; it should not be treated as a permanent machine policy without validation.

Before wider tunables are added, every new system-changing action must expose an `ActionDescriptor`, every apply path must call `DaemonPolicy::check_action`, and the action must have:

- explicit safety class
- preflight checks
- dry-run behavior
- apply and verify steps
- rollback path available before apply
- durable audit event

## Checking CPU Topology

Before editing affinity profiles with explicit masks, check your CPU layout to avoid splitting hyperthreads across different classes or using unstable E-cores for critical tasks:

```bash
lscpu -e=CPU,CORE,SOCKET,NODE,ONLINE,MAXMHZ
cat /sys/devices/system/cpu/online
```

Example 8-core/16-thread mapping logic:
- `0-3`: First 4 threads (often Core 0 and 1 with hyperthreading)
- `4-15`: Remaining 12 threads (Cores 2 through 7)

Do not blindly copy masks from other machines; a mask that works on an 8-core CPU may be invalid or suboptimal on a 6-core or hybrid CPU.

## Rollback Expectations

CPU-affinity changes are reversible when the target tasks still exist and the restore file is intact. Some tasks may exit before restore; `stutter restore` reports skipped/dead tasks instead of treating them as a reason to panic.

For watch mode, Ctrl-C restores original masks unless `--keep-applied` was used.

## Applying Recommendations

Use this conservative loop:

```bash
stutter bench --watch-process Game.exe --persistent --duration 180 --scenario route --role baseline
stutter tune --tree-pid <PID> --profiles examples/profiles/common-game-layouts.toml --runs 5 --baseline-profile baseline-online
stutter recommend --baseline <baseline-run-dir> --tune <tune-dir>
```

Apply only when the workload was comparable and the recommendation is stable enough to justify a manual experiment. Keep `stutter restore` available while testing.


### A/B tuning uncertainty

The HTML recommendation report exposes A/B uncertainty: distribution charts,
bootstrap CI bands, effect size, sample counts, noise ratios, and warnings when
the comparison is underpowered. Treat recommendations as directional when CI
bands cross zero, sample counts are low, or noise ratios are high.
