# W28, W29 and W31 on this host (2026-11-11)

## W28: sched_ext, GameMode, TuneD, LACT and power-controller coexistence
Read-only: `sched_ext` state `disabled`; `scx`, `lact`, `gamemoded`, `tuned`, `tlp`, `irqbalance`, `ananicy-cpp`, `thermald` and
`power-profiles-daemon` are installed (or present) but all `inactive`, so no competing owner of CPU, scheduler or power policy runs.

Live, no game running, bounded 25 s, fully reversed: `sudo scx_bpfland` was loaded (`/sys/kernel/sched_ext/state` became
`enabled`, ops `bpfland_1.1.4_x86_64_unknown_linux_gnu`) while the installed Stutter controller and worker were running. Both
units stayed `active`; the controller kept emitting its per-second status (46 records, phase `Cooldown`, no target) and
performed no mutation; the scheduler then unregistered cleanly and the state returned to `disabled`. So a foreign sched_ext
scheduler appearing under the installed service neither crashes nor wedges it. **Limit:** the controller's status lines carry no
sched_ext-owner field, and with no target there was no candidate to refuse, so the owner-change *refusal* path is covered by
the existing fixtures, not by this live run. The coexistence claim is "no interference observed", not "owner conflict handled
live".

## W29: PM QoS, DAMON/DAMOS (single node, root-only devices)
`/dev/cpu_dma_latency` is `root:root 0600`; the PM QoS lease is a HighRisk operation and `HIGH_RISK_APPLY_IMPLEMENTED=false`, so
no live lease is taken (standing constraint, not a hardware gap). One `kdamond` already runs, owned by an in-kernel module.
Multi-node NUMA and RDT are `NotPlannedForVerification` (ledger 5.19, 5.24, 5.8).

## W31: portability and boot
This host is NixOS/systemd 6.18.57; OpenRC is `NotPlannedForVerification` (ledger 5.21). Alternate-kernel (RT, Zen, Liquorix,
CachyOS) campaigns need reboots, which this session may not perform: `BootUnavailable`, ledger `extra.kernels`. No blank
cross-init Supported entry exists.

## W28 follow-up: a lower-risk live attempt at the owner-conflict refusal (2026-11-11)

Tried `sudo stutter autotune --mode suggest` (observation only, decision log and recording in a scratch directory and root's
state directory; no mutation) against a fixture workload, once without and once with `scx_bpfland` loaded
(`/sys/kernel/sched_ext/state` `enabled` during the run, `disabled` afterwards; no game running). Both runs ended `rc=0`. Neither
decision log contains any sched_ext or owner field, and the fixture never leaves `Observing` (`LowFocusConfidence`,
`candidate_count` 0), so the planner has no candidate to refuse and the owner-conflict denial cannot appear. Result: this path
cannot demonstrate the refusal. Showing it live needs a new smoke case (a foreign scheduler loaded while the controller
holds an exact fenced fixture and a candidate is eligible), which is a harness change, a new exact-build receipt and another
guarded activation. The refusal itself is covered by the planner fixtures (`planning/denial.rs`: a sched_ext scheduler in
charge that is not verified to honor the knob).

### Why a live owner-conflict refusal is not reachable with the deployed service (read from the code, 2026-11-11)

`planning/evaluate.rs` raises `SchedulerSemanticsUnverified` only for `Nice` and `Uclamp` candidates while a sched_ext scheduler
is enabled and `knob_is_honored` is false (`integrations/sched_semantics.rs`: no scheduler is on the verified list). The deployed
bounded service proposes only its two `AUTONOMOUS_CONTRACTS` operations (CPU-affinity profile and cgroup placement), which are
not those knobs, so under a foreign sched_ext scheduler it has no candidate of the refused kind to refuse. nice and uclamp are
Experimental and never autonomous. A live refusal would therefore require enabling an operation the service is not allowed to
run, which is outside the standing constraints. What can be shown live is what was shown: with a foreign scheduler loaded the
service neither crashes nor mutates, and its Supported operations are unaffected. The refusal stays covered by the planner
fixtures. This item is closed as not applicable to the deployed contract rather than left open.
