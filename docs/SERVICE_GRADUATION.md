# Bounded service qualification

The `system-bounded-autotune` installation mode passed targeted acceptance (below) and a
production boot check on one NixOS host, which promotes `always_on_autotune` for the packaged
systemd service and the two graduated operations only. OpenRC remains unqualified. The
eight-hour endurance contract has not passed and is not claimed; no endurance or in-campaign
reboot result is implied by the source tests or the older direct-controller soak.

The systemd controller runs as the monitored desktop account. Its root worker authenticates
the private socket's peer, uses root-owned policy, and limits requests to the existing
autonomous registry. It checks task UID ownership before preparation and after the commit
barrier. Cgroup placement uses the real cgroup mount and an administrator-configured
destination; it cannot request cpuset policy changes through this contract. Root-issued
restoration tokens are retained before commit and settled on worker restart or stop.

The optional root-worker `autotune.validation_workload_cgroup` setting withholds mutations
outside a disposable workload subtree. It grants no operation and bypasses no focus, health,
objective or registry gate. `autotune.profiles` supplies an administrator profile file; the
explicit CLI `--profiles` setting takes precedence.

`xtask service-campaign init` freezes clean source, binaries, kernel, owner and the
[service manifest](validation/service-graduation.toml). `continue`, `status`, `report`,
`verify` and `cancel` retain immutable attempts under `.llm-runs/service-validation`.
An interrupted attempt stays in the record. A source or binary change invalidates the
campaign. Software and validation failures cannot be retried into a passing qualification.

The runner installs the exact embedded templates under `/run/systemd/system`, keeps its
root-owned qualification configuration under `/run/stutter-bounded-service-validation`, and pins the
product binary under a protected root-owned directory, and refuses existing administrator
units or configuration. Later actions verify the unit/configuration ownership receipt and
reject drop-ins. Typed fixture actions can attach only the campaign user's pinned
`live-workload` process with its exact start time. Service actions accept no executable,
shell command, unit name or arbitrary systemctl argument.

Live discovery uses real executable identities and the production heuristic resolver. Its
positive fixture has an active game-labelled event-loop root and an owned Wine-like helper. A plain Steam-path-only
root is a separate low-confidence negative case. Browser, compositor, unknown, ambiguous
and rapidly replaced workloads are also negative cases under actual scheduler pressure.
The history records the service's qualified confidence and PID/start-time map. No resolver
snapshot or validation override is injected.

The existing soak driver has direct-child and packaged-service adapters. The service adapter
attests real UID, capabilities, binary digest, boot identity, socket ownership and root token
state. The validation runner uses SYS_PTRACE to read the fixed actors and owned fixtures through /proc; that capability is absent from the product controller. Workload exit must leave the same controller alive with no retained action. A focus
swap uses that same process. Service suspend uses a fixed typed RTC-wake action and checks
the kernel suspend success counter.

The recovery, reboot-resume, coverage-driven service soak and portable evidence stages are
still being completed. They explicitly refuse qualification until their live runners and
required evidence are present. The completion gate remains at least eight hours of verified
service dwell, six clean schedule cycles, repeated dangerous events and kept/reverted
outcomes for both supported operations, with no unsupported apply or unrecovered token.

## Targeted acceptance and extended endurance

Two contracts exist for the same packaged service and never stand in for each other.

* **Extended endurance** ([service-graduation.toml](validation/service-graduation.toml)): at least
  eight hours of verified live dwell, six complete cycles and two real reboots inside the
  campaign. Its floors are never lowered, and a shorter run never claims to have passed it.
* **Targeted acceptance** ([service-acceptance.toml](validation/service-acceptance.toml)): started
  with `STUTTER_SERVICE_ACCEPTANCE=1`, exported as the `live_bounded_service_acceptance` suite.
  It passes on observed, physically verified behavior: one complete 22-round cycle in which every
  dangerous and service event runs once, both CPU affinity and cgroup placement are kept and
  reverted, every root token settles and nothing stays applied. It has no dwell floor and no
  in-campaign reboot, and a global timeout bounds it. Reboot persistence of the production
  service is checked separately on that service. A kernel-refused suspend is repeated, never
  counted, and ambiguous outcomes fail.

Earlier endurance campaigns remain historical supporting evidence with their own commits and
limitations. They do not satisfy a check of a later exact commit.

## Qualification record

* **Targeted acceptance:** campaign `service-adef09a-1793911466788`, source `adef09a8`, all seven
  stages passed and strict verification reported `qualified`. 22 of 22 rounds were clean in one
  cycle of 1.53 h (11 CPU-affinity and 5 cgroup-placement experiments resolved, every dangerous and
  service event once, no repeated attempt, every interrupted root token settled by name). The
  exported bundle is indexed as `live_bounded_service_acceptance-adef09a8-1793919895854`.
* **Production boot check:** the NixosConf module (opt-in `services.stutterBounded`) renders the
  units of `packaging/systemd` with an immutable store binary pinned by the SHA-256 of the file.
  After a real reboot both units were active without the validation runner (controller uid 1000
  with only `CAP_BPF`, `CAP_PERFMON`, `CAP_SYS_RESOURCE` and `CAP_DAC_READ_SEARCH`, `NoNewPrivileges`;
  worker root with only `CAP_SYS_NICE`, `CAP_SYS_PTRACE`, `CAP_CHOWN` and `CAP_DAC_OVERRIDE`), both
  state directories were their own mounts with preserved contents, the root token ledger was intact
  with nothing outstanding, and a game-like fixture received focus and an experiment applied through
  the worker and settled.
* **Earlier endurance attempts** (`service-6079f71`, `service-b791597`, `service-7f687a8`,
  `service-7ffc4c6`) ended on harness or hardware issues, one of them a kernel-refused suspend, and
  are retained as historical supporting evidence only.

## Per-release smoke of the installed service

A release candidate brings a fresh `live_packaged_service_smoke` bundle for its exact build, so a
new binary cannot inherit confidence from an older service binary. It runs against the units the
host already has and never installs or replaces one:

```sh
cargo run -q -p xtask -- production-smoke --binary-sha256 <sha256 of the build under test> \
  --out <new raw evidence directory>
cargo run -q -p xtask -- evidence-export --raw <raw directory> --out <bundle>
```

It checks that both units are running, declared by the host and executing the pinned immutable
binary (root-owned and protected, hash equal to the build under test) as the intended accounts
with exactly the intended capabilities; that a workload fixture is discovered and a prepared
mutation is applied through the root worker, physically readable, resolved and settled; and that
a controller restart, a worker stop (which takes the controller with it) and a worker start leave
a clean service with no root token outstanding. It needs passwordless `sudo` to read the root
ledger. It does not exercise every fault (the acceptance suite does), endurance, or boot.

The validation runner installs transient units with the same names, so campaigns that install
their own units cannot run while the production units are enabled; run this smoke instead.

### What the smoke checks now (since `d34ab086`)

Besides the unit, account, capability and immutable-binary checks above, the smoke measures the service as it
behaves, not only as it is configured:

- **Idle:** the controller's CPU averaged over a window (limit 5 percent of one core), its resident memory (limit
  256 MB), that it changes nothing (policy activity such as `cooldown_entered` is not a change; only applies,
  keeps, reverts and restores count), that no stale candidate-memory temporary files exist, and that the history stays
  bounded.
- **A workload that exits while a mutation is applied:** the placement was read back as changed before it exited
  (a resolution alone is not evidence of a physical change), the root ledger settles, and nothing is started on the dead
  process id.
- **A controller killed while a mutation is applied:** the placement was read back as changed before the kill, the
  controller restarts as a *new process* (a different main pid, the old one reaped), the ledger settles and the original
  placement returns.
- **The normal workload:** discovered, applied, the placement read back as changed while applied, resolved, tokens
  settled.

The controller will not apply the same action again for 300 seconds after applying it, whichever workload it was
applied to, so every applied case starts that cooldown. The smoke computes the remaining cooldown from the history and
waits it out once per gap, using the first gap for the lifecycle checks; it never clears candidate memory or overrides
the policy. A full smoke therefore takes about twelve minutes. The result for the installed build is recorded in
[`validation/production-behavior/`](validation/production-behavior/README.md).
