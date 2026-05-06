# Safety

`stutter` is designed to keep observation, recommendation, and system changes separate.

## eBPF Privilege Boundary

Recording and live tracing require privileges on most systems because `stutter` loads eBPF programs. Offline commands such as `report`, `recommend`, `advisor`, and `audit` read files and do not need root.

Use `sudo` or `doas` only for commands that actually need live tracing.

## CPU Affinity Restore File

CPU-affinity profile application records original task masks before changing them. The default restore file is:

```text
~/.local/state/stutter/affinity_restore.json
```

Restore saved affinity with:

```bash
stutter restore
```

Inspect what would be restored:

```bash
stutter restore --dry-run
```

`apply-profile --force` can replace an existing restore file. Without `--force`, new restore records are merged while preserving the earliest known original mask for each task identity.

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

Before wider tunables are added, they should have:

- explicit safety class
- preflight checks
- dry-run behavior
- apply and verify steps
- rollback path
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
