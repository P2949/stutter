# Bounded service qualification

The `system-bounded-autotune` installation mode is under qualification. It does not yet
promote `always_on_autotune`. OpenRC remains unqualified. No packaged-service soak or real
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
