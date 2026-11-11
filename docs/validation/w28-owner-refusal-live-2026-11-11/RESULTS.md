# W28: live owner-conflict refusal, production checks on live host state (2026-11-11)

New observation-only subcommand `xtask owner-refusal-probe`. It does not touch the deployed service (which never proposes
the refused knobs). It reads the live kernel owner state with the production `integrations::probe` and asks the production
`knob_is_honored` and `owner_denial` (the functions `planning/evaluate.rs` uses to raise `SchedulerSemanticsUnverified` and
`ExternallyOwnedResource`) what a Nice or Uclamp candidate would get. No game was running; nothing was mutated.

| Live state | sched_ext | nice / uclamp / cgroup cpu.weight honored | planner refuses Nice/Uclamp |
|---|---|---|---|
| baseline (`baseline-disabled.json`) | Disabled | yes / yes / yes | no |
| `scx_bpfland` loaded 12 s (`scx-bpfland-enabled.json`) | Enabled `bpfland_1.1.4_x86_64_unknown_linux_gnu` | no / no / no | **yes (`SchedulerSemanticsUnverified`)** |

Second live refusal, both states: `owner_denial(Nice)` returns "GameMode adjusts task priority and power", because the
`gamemoded` process is running on this host (correcting the earlier note that gamemode was inactive: the unit was
inactive, the daemon process is not). This is the `ExternallyOwnedResource` path.

After the run `/sys/kernel/sched_ext/state` returned to `disabled`; `stutter-autotune-bounded@p2949` and
`stutter-privileged-worker@p2949` stayed `active`.

Limit: this exercises the production decision functions on real live owner state, not the full candidate-evaluation
pipeline inside the running service, which cannot emit these candidates under its two-contract limit.
