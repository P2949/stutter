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

## Update 2026-11-14 (source head after this section: see git log)

Added since the section above, all fixture or unit evidence only:

| Area | Module | Evidence |
| --- | --- | --- |
| 34-row integration crosswalk | `research/crosswalk.rs` | exactly 34 rows, 33/unknown ids rejected, only 5.21 names a supported action |
| Frame/runnable source quality | `research/quality.rs` | typed verdicts; unknown is never zero; 2,400/2 ms/1,000 gates unchanged |
| Source-by-factor feature matrix | `research/features.rs` | 60 cells, HDR never a planned environment cell |
| G5 action outcome classifier | `research/action_study.rs` | external actors, zero-TID, token and gate bypass refused |
| Launch-request handshake | `dispatch_context.rs`, `driver.rs` | a delayed earlier dispatch cannot consume a newer attempt's tickets |
| Claim settlement witnesses | `research/settlement.rs` | procfs process witness; worker witness fails closed when absent |
| Report context drift | `research/report.rs` | derived from sealed native completions; unknown is not stable |
| Idempotent ticket retirement | `launch_ticket.rs` | repeat and torn `revoked-v1` markers; empty marker still refused |
| Epoch-bound telemetry join | `research/join.rs` | process, epoch, window and timestamp checks with counted rejections |

NixosConf research worktree (`feat/stutter-cyberpunk-research`, local only): `validate fast` and
`validate affected` pass (receipt for 1cc7de70). `validate full` is reserved for the exact merge.

Still blocked, unchanged: the user's graphical session is active and Stutter's own safe-window
detector reports it is not idle, so no Steam or Cyberpunk launch, compat-tool selection,
deployment or campaign was performed. Open design decision: the production worker-token witness
needs a read-only outstanding-token query; the only existing call (`settle_issued_tokens`)
mutates, and the worker IPC is not widened without approval. No G0-G9 gate that depends on a real
launch is claimed.

## Review follow-up (2026-11-14, after the external review of `491d725c`)

Fixed (unit evidence only):

- `action_study::classify` no longer relabels a kept action whose guardrails failed (now
  `NotQualified`); a verified revert reports `kept: false`; a no-op needs source-qualified evidence.
- Delayed dispatch: the launch-request marker now records the boot-relative tick of the request and
  `authorize_dispatch` requires the dispatcher's own `/proc` start tick to be at or after it, so an
  invocation Steam started for an earlier request cannot consume a later attempt's tickets. This
  closes the "before the new request" case in code. Residual, not provable offline: an invocation
  that Steam itself starts *after* the new request for an earlier launch cannot be told apart by
  time; the driver's one-attempt-at-a-time settlement is the remaining guard.

Open, accepted as not yet resolved (needs a real Steam launch to design against):

- Process settlement still rests on dispatcher anchors plus the conservative name prefilter. A
  durable per-launch unit/cgroup ownership witness requires the real user-manager launch
  transaction (`user_manager.rs` is read-only today) and Steam's actual scope layout.
- Post-launch recovery (`RecoveryJob::run`) still refuses any lease with a started launch.
- `outstanding_tokens` is a worker-wide ledger count, not campaign-specific physical restoration,
  and its live-worker assertions have not been executed.
- `context_state` compares all native completions as one cohort; it must be made study/block
  specific before CP-CPU and CP-GPU studies are reported together.

## Update 2026-11-14 (second review round)

Added, unit evidence only, none wired into the resident runtime yet:

| Module | Purpose | Not yet |
| --- | --- | --- |
| `research/recovery_plan.rs` | pure post-launch recovery decision: seal only provably settled attempts, never replay, `UnsafeToResume` otherwise | gathering its facts in `RecoveryJob::run` |
| `research/child_study.rs` | frozen, distinct child studies per profile/stage/factor/action; `DeferredBudget` funnel | used by `build_schedule` and the report |
| `research/launch_txn.rs` | persist-before-send launch over a user-manager port; lost ack reconciled by observation, never resent | composed with `VerifiedUserManager` and a real D-Bus port |
| `action_study::classify` | blocker denial needs source-qualified evidence; a worker transaction without a controller decision is refused | real controller/worker evidence |

Still open: per-launch dispatch binding beyond start ticks (needs the real Steam lifecycle),
`ProductionSteamPort::stop` cannot settle a detached game tree, the real `NativeObservationSource`,
wiring the telemetry join, NixosConf exact-build packaging, and every physical gate. The desktop is
occupied by the user's own Steam game; no G0-G9 gate is claimed.
