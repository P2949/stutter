# E7: read-only explanation of the installed service (2026-11-11)

New observation-only subcommand `xtask service-explain --instance <name> --out <new file>` (`xtask/src/service_explain.rs`, 6 unit tests).
It adds no permission engine and sends nothing to any worker: it reads `systemctl show`, the service's own state files through the
production `load_autotune_status`, the production `probe` owner report and the production `AUTONOMOUS_CONTRACTS` /
`HIGH_RISK_APPLY_IMPLEMENTED` constants. The shipped crates and the deployed service are unchanged. Live output: `explanation.json`
(sha256 prefix `1acb46a8`), `mutations_issued: 0`.

## What the plan's E7 list gets, on the live service

| PLAN.md E7 question | Answer (live, 2026-11-11) |
|---|---|
| Selected process | none: `target: null`, no stable game focused (idle desktop) |
| Active external owner, suspected vs confirmed | process hints GameMode, Lact, ScxLoader, SystemdOomd; all `ExternalSuspected` (process-name only, no exact receipt) |
| Objective source, stage, quality | Unknown (no target means no measurement window); stated as Unknown, not omitted |
| Allowed candidate | only `CpuAffinityProfile` and `CgroupPlacement`; `high_risk_apply_implemented = false` |
| Why not tuned | derived list (no target, phase cooldown in mode ApplyLowRisk, nothing kept, no rollback pending, sched_ext disabled) |
| Recovery state | no rollback pending; last decision "restored: startup crash recovery rollback succeeded; affected_tasks=0" |
| Running binary | configured ExecStart `/nix/store/4jm60ap...-stutter`, sha256 `6922c7c6d09f4fad2e1a02902da6440fa28823b423f54b78f2e8853c020942bc` for BOTH units: equal to the NixosConf pin. The running image itself is UNVERIFIED: `/proc/<pid>/exe` is not readable for either unit (sandboxed), recorded as `running_executable_readable: false` |

## Findings

1. **Existing status path works, but only with the service's HOME.** `stutter autotune-status --json` reads the invoking user's
   `~/.local/state/stutter/autotune/`. Run as the desktop user it reports `Disabled` / "no autotune history found" while the
   service is active; run with `HOME=/var/lib/stutter/autotune-p2949` (the unit's HOME, readable by that user) it reports the
   service's real state (`cooldown`, `ApplyLowRisk`, no target, no fault). Not changed: it is the shipped crate's behavior. `service-explain` uses the
   service's state home explicitly.
2. **Owner-hint false positive (diagnostic only).** The `ScxLoader` hint is raised by the comm pattern `scx_*`, which also matches the
   permanent kernel thread `scx_enable_helper`; no `scx_loader` process exists here. It fails closed, and no operation maps to the
   Scheduler domain in `operation_domain`, so it changes no decision today. Recorded as a caveat in the output, shipped code unchanged.
3. **Correction to earlier notes.** `docs/validation/w28-w29-w31-host-facts-2026-11-11/RESULTS.md` said LACT was inactive. `lactd` is
   active and `lact daemon` is running on this host. The standing LACT/GPU quarantine is unaffected (Stutter writes nothing to it).
   `systemd-oomd` is active; `gamemoded` runs while its unit is inactive.

## Not run, with reasons
- `stutter wayland-probe`: it opens a client surface on the active session (default 30 s, optional fullscreen); not run.
- perf, fio, trace-cmd, wrk2, hwloc: PLAN.md E7 says use them only when a measurement question justifies it; none was named, and the W30
  captures already exist. No observer-cost re-profile: no new sensor or regression (the 2.60% one-core snapshot stands).
