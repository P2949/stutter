# Cyberpunk × Proton research: implementation state and physical blockers (2026-11-12)

Source head when written: `c28013c6` on `feat/system-wide-autotune`. Nothing below was run
against a real Steam or Cyberpunk process; every test listed is a unit or fixture test.

## Implemented and tested (fixtures only)

| Area | Module | Evidence |
| --- | --- | --- |
| Counterbalanced schedule | `research/scheduler.rs`, `driver::build_schedule` | position/adjacency balance tests |
| Fixed-N paired statistics | `research/analysis.rs` | verdict, multiplicity, refusal tests |
| Steam launch dispatch | `dispatcher.rs`, `dispatch_context.rs`, `launch_ticket.rs`, `stutter research dispatch` | one-use per-verb tickets, replay/expiry/wrong-user, tool re-attestation |
| Resident attempt driver | `driver.rs`, `runtime/research_control_drive.rs` | single-shot attempts, cancel/timeout/deadline, restart refuses unsealed attempt |
| Evidence, report, verify | `report.rs`, `report_store.rs` | `Report`/`Verify` requests rebuild byte-for-byte from journal + raw evidence |
| Feature catalog | `features.rs` | documented fork env only; typed blocks; dispatcher controlled-key check |
| Steam host logic | `steam_host.rs` | fake-port tests |

## Not built

P4 telemetry join, P6 diagnosis oracle, P7 worker action study, feature performance runs, P10
exact-build deployment, P11 final campaign, the production `SteamPort`.

## Physical blockers (precise)

1. **Steam is not running** and the user's graphical session (`tty1`) is occupied. The plan's
   occupied-session guard forbids launching Steam or the game there; no automatic safe-window
   qualification has been exercised.
2. **Per-launch compatibility-tool selection is unresolved.** The dispatcher compat tool must be
   selected for AppID 1091500 without silent edits of Steam's `config.vdf`. The plan allows only a
   one-time agent-controlled reversible UI selection with readback; Wayland input automation access
   is not established on this host.
3. **Pressure-vessel visibility is unverified.** The dispatcher `proton` wrapper (NixosConf commit
   `1f3698f`, unpushed) runs inside Steam's runtime container, which may not expose the Nix store
   or the private dispatch directory. This needs one real Steam launch.
4. **`SteamAppId` / `LD_PRELOAD` behaviour** assumed by `plan_dispatch` is unqualified against real
   Steam invocations.
5. **Deployment:** the running service is the older build without research paths. A NixosConf
   switch restarts the controller and worker and needs a verified-free host; it is premature
   until the production `SteamPort` exists.
6. **Native `-benchmark` behaviour** is a candidate, not a documented guarantee; CP-LONG has no
   qualified unattended route.

No acceptance gate (G0–G9) that depends on a real game launch has been claimed.

## Occupied-session evidence (2026-11-12, measured, not assumed)

`loginctl show-session 11`: `Type=wayland Active=yes State=active IdleHint=no IdleSinceHint=0`
(Hyprland compositor, the user's terminal session). No Steam, Wine or Cyberpunk process exists.
Every physical gate that launches Steam or the game, selects the compat tool through the UI, or
switches the running Stutter service requires an objectively idle desktop. While the user is
present in the active session that condition is false, and the guard (`BlockedOccupiedSession`)
must not be bypassed. This is the irreducible external blocker for the physical gates; it clears
only when the desktop is genuinely idle, which Stutter's own safe-window detector must observe.

### Stutter's own detector agrees (live, read-only, 2026-11-12)

`cargo test -p stutter --features wayland-probe --lib observes_current_session -- --ignored`
(the existing live test, no input injection, 600 s idle threshold, output printed locally) observed
`Monitoring { inhibitor_idle: false, input_idle: false }` for the active Hyprland session: the
product's own safe-window signal is false, so `AwaitSafeWindow` correctly never advances to
`Launching`. This exercised the detector against the real compositor.
