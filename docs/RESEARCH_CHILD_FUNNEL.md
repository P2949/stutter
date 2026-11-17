# Research child-study funnel: status and remaining integration

Status at this revision: the funnel is **connected to the resident controller, default off**, and
unit/controller-tested with fake hosts. Nothing here has run against a live game.

## Built and connected (all under `stutter/src/autotune/`)

| Piece | Role |
| --- | --- |
| `research/child_study.rs` | frozen funnel: screen, pilot, confirmation, action, feature; dependency gate, budget fit; per-stage arm requirement (all tools, or a pair) |
| `research/child_ledger.rs` | hash-chained, restart-safe decisions, spent attempts, frozen pilot rules |
| `research/child_runner.rs` | polled stage runner; health stop aborts (settle then restore) before halting |
| `research/child_intent.rs` | per-study intent: arm subset, budget, study binding, frozen pilot rule (confirmation only) |
| `research/pilot_rule.rs` | rule in basis points, digest-bound, thresholds must follow from the pilot's spread |
| `research/study_executor.rs` | recover, settle, apply, run, settle, restore, judge; stage evidence required for feature/action |
| `research/profile_applier.rs`, `profile_hook.rs` | atomic edit, readback, durable snapshot, crash recovery, all-or-nothing per arm |
| `research/driver_attempts.rs` | one child through the journalled `Driver`, own journal/dispatch/evidence/intent/report |
| `research/funnel_driver.rs` | presents the funnel as a campaign driver: ticks map to the `Step`s the lease already uses |
| `research/arm_selection.rs` | control + best candidate from the screen's sealed FPS |
| `research/child_verify.rs`, `raw_verify.rs`, `verdict.rs` | per-child reconstruction and raw replay; separate verification statuses |
| `runtime/research_funnel.rs` + `research_control_drive.rs` | `research_child_funnel` switch (default false, needs live launch); tick, cancel, shutdown, report |
| `runtime/research_recovery.rs` | restart: settle every child's interrupted attempt via the witnesses, then restore profiles |
| `research/dispatch_select.rs`, `steam_port.rs` | the dispatcher and port find a child's attempt under `studies/<digest>/dispatch` |

## Not connectable without a design decision: the Stutter action child (CP-LONG)

`runtime.research_campaign_active` (set in `runtime/session.rs`) blanket-suppresses the autonomous
loop while research owns the action slot. `research/action_child.rs` and `action_bridge.rs` model the
authorization and the apply/readback/decide/restore sequence, but the *normal* controller applies
actions through `LiveExperimentManager` and `RuntimeLiveExperimentActionExecutor`
(`live_experiment/executor.rs`), with its own journalled, two-phase, token-bearing lifecycle. A
second applier would create a parallel optimizer, which the plan forbids. The real bridge is a
**per-candidate gate** in place of the boolean:

1. `session.rs` holds an `ActionChildScope` (built from the campaign intent's `actions`, the frozen
   game process identity and the window) while a CP-LONG child is running.
2. Before `RuntimeLiveExperimentActionExecutor::apply_candidate`, the gate is asked for a grant for
   (candidate kind, target pid, start ticks, uid, now). A refusal blocks the apply exactly as the
   boolean does today.
3. The experiment manager reports `applied(readback_ok)`, `kept`, `reverted` and `restored` events
   back to the scope (the experiment already journals each of these phases).
4. `restored(true)` requires the worker's own kernel readback **and** zero outstanding tokens
   (`UnixSocketPrivilegedActionService::outstanding_tokens`), as `action_bridge` already enforces.
5. The study's `stage_evidence` becomes `Some(true)` only when the scope settled the action with a
   proven restoration; benchmark FPS alone never qualifies it.

This touches the normal autotuner's lifecycle and needs a review of its journal phases and of the
diagnosis that selects the candidate. It cannot be proven without a live CP-LONG run.

## Still requires a live run or a human decision

* Real restore proof of an applied action (worker readback plus token count).
* Observed list indices for the CPU/GPU graphics profiles (`BlockedProfileMapping`).
* Real-container Proton paths (`RuntimeBinding::Matched` has never been produced live).
* Whether the game accepts and honours an edited `UserSettings.json`.
* The GPU-stability incident acknowledgement (a human decision; see `research health-check`).

## Action gate wiring (offline, fake-worker verified)

The funnel builder creates one `ActionGate` (unarmed) and shares it with both the
`DriverStudyAttempts` (an Action study arms it on the sighted game process) and the
resident runtime (`set_research_action_gate`, installed each loop from the coordinator).
`select_candidate_for_normal_control` consults it instead of the blanket research
suppression: only the two supported actions the child's scope authorizes, only against the
sighted process. Restoration evidence reads the existing worker's outstanding-token count;
unknown counts as unproven. No new root IPC. Not yet exercised against the real worker.

## Treatment designs

Feature and action studies measure **one fixed Proton build** (the screen's control) with the factor or
action off versus on (`StudyArms::Single`, `treatment`, `Treatment::{Off,On}`): the schedule balances the
two treatments in complete seeded blocks, the host applies the factor only to attempts treated On, the
action gate permits an action only during On attempts, and the report is one paired result carrying the
treatment. See `docs/RESEARCH_TREATMENT_DESIGN.md`. Whether the real Steam host honours the factor and
whether an action helps are still unproven without a live run.

## Held-out validation

Each profile's confirmation is followed by a `Holdout` study: the same selected pair and the same
pilot-frozen rule, re-measured on fresh runs under a different study digest (so a different seeded
schedule). It runs only after its confirmation qualified, and a campaign's `child_studies_qualified`
needs every planned study (holdouts included) qualified, so no winner is claimed from a single
confirmation. The verifier checks its rule and arm selection exactly as it does a confirmation's.

## Long sessions and selection

- Studies on the long profile need a `LongWorkUnit` (`research/long_session.rs`: an id, a 20-60
  minute duration and a definition digest). None is installed, so they are recorded as
  `BlockedNoComparableLongWorkUnit`; the native benchmark's FPS is never reported as a long session.
- A treatment study measures the build the screen's verified results select for its factor
  (`arm_selection::select_treatment_source`): a fork-local feature is never measured on Valve
  Stable, and a factor no measured build can run is `BlockedNoEligibleSource`.
- Before any study creates anything, its arms' prefixes must be all cold, or all warm and stamped by
  their own build (`prefix_identity::check`).

## Resident telemetry receipts (G3)

While a research attempt runs, the resident runtime reports into a shared `TelemetryObserver` (no second
monitor): each frame event it receives, and its own frame source's status (the producer pid and start
time it is bound to, drop and gap counters, health, competing targets, clock alignment). When the
attempt ends, a receipt per attempt is written to the study's `telemetry-receipts.json` stating whether
the source was bound to the attested game process and what `quality::judge_frame_source` says of it.
Competing candidate games stand in for an ambiguous swapchain; a counter the runtime cannot read stays
unknown, never zero. G3 is accepted only when every completed attempt of the verified children has an
`Eligible`, game-bound receipt; otherwise it reports `NoTelemetryReceipts` or `FrameSourceNotEligible`.
Nothing here has run against a real game: the real frame sources' behaviour is unproven.
