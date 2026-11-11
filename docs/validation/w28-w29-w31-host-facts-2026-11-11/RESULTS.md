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
