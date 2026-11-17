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

## Deployment receipt (G8)

`stutter research deploy-receipt --study-root <private dir> --systemctl <store path> --unit <unit>... --expect-binary-sha256 <hex>`
reads (read-only, locally) what each Stutter unit actually runs: the `ExecStart` store path, that file's
SHA-256 and the unit's active state, and writes `deploy-receipt-<unix>.json` into the study directory.
G8 is accepted only when the newest receipt says every unit is active and runs the expected pin **and**
that binary is the one performing the verification (`/proc/self/exe`). A flake pin or a built store
object is not a running service; a receipt about some other binary attests nothing about this one.

## Lease inspection (G1)

`research::lease_inspect::inspect` re-reads a campaign's lease directory read-only (no lock, nothing
created or changed) and reports whether the sealed records hold together: the `intent.json` header's
digest is its intent's digest, `event-NNNN.json` records are gapless and each names the SHA-256 of the
record before it (the header for the first), every transition is one the state machine permits, and
the lease ended (completed, blocked or aborted) for this campaign's intent. G1 reports each failure as
its own blocker (`LeaseChainBroken`, `LeaseTransitionsInvalid`, `LeaseHeaderMismatch`,
`LeaseForAnotherCampaign`, `LeaseNotSettled`, or `LeaseNotInspected`). Authenticated IPC and
single-owner properties are tested elsewhere and are not claimed by this inspection. Run against the
real 2026-11-15 lease it reports a valid chain that stopped in `Launching`.

## Production-path contracts added after the fifth audit

- **Treatments.** A within-source study runs each factor's own catalog environment for both arms
  (`features::arm_env(source, factors, on)`): the NTSync control is `PROTON_NO_NTSYNC=1`, the Wayland
  control is `PROTON_ENABLE_WAYLAND=0`, never an empty environment that could equal the study arm.
- **Unresolved ownership.** A halt whose settlement or restoration is unproven
  (`child_runner::leaves_ownership_unresolved`: `SettlementUnproven`, `RecoveryUnproven`,
  `RestoreUnproven`, `ExecutorError`) keeps the campaign lease and the action slot and sets
  `UnsafeToResume`, as a driver error does; it is never settled as an ordinary blocked campaign.
- **Campaign budgets.** The active-time and evidence-byte budgets belong to the campaign. A finished
  child records `budget.json` beside its study; the next child's driver starts from the sum, and a
  child with no record is charged the whole active budget.
- **CP-LONG window.** `long_attempt_seconds` (20-60 minutes, within the active budget) is authorized
  separately from the 720 s benchmark limit and applies only to a long-profile child; the work unit
  must fit it (`BlockedNoLongAttemptAuthorization`, `BlockedLongWorkUnitExceedsAuthorizedWindow`).
  The child verifier binds each child's limit to its study. No producer for a long session exists.
- **Cold prefixes.** A settings file that does not exist (a never-initialized prefix) blocks the
  profile as `BlockedPrefixNotInitialized` instead of failing the study. Initializing a prefix needs a
  supervised launch and is not implemented here. Each child records `prefix-cohort.json`.
- **G8** recomputes everything from the units: both required units exactly once, an identified main
  process (MainPID, start ticks, argv[0], and the executable hash where readable), the same boot, a
  fresh receipt, this very binary. Stored summary flags are ignored. The user cannot read the service
  processes' `/proc/<pid>/exe`, so argv[0] naming the unit's own immutable store path is the
  unprivileged tier; a readable executable that differs always refuses.
- **G1** lists the lease directory and requires a gapless chain with no stray `event-` entry; an
  unreadable record is an error, not the end of the chain.
- **G3** joins each receipt to the sealed native completion's pid, start ticks and uid, and the
  observer requires the source bound, healthy and from one producer across the whole attempt.
- **G5** joins each action receipt to a completed attempt the schedule treated On and requires every
  treated attempt to carry one (`ActionNotAppliedOnEveryTreatedAttempt` otherwise): a policy-only
  comparison is not reported as an applied-action study.
- **G6** consumes a typed `fault-matrix-*.json` receipt: every required class must have a physical
  pass for this binary; fixtures never count. No physical matrix has been run.
- **G9** also requires each holdout to reproduce its confirmation (`replication`): a sound holdout
  that reaches another conclusion, or an inconclusive one, blocks with `HoldoutDoesNotReplicate`.
- **Bundle.** Child rows carry telemetry receipts and process identities, and a bundle that cannot be
  written turns the verification response into `BundleNotWritten`.
- **Profiles.** `cpu_profile` (1080p) and `gpu_profile` (native 3440x1440) are raster-only with ray
  tracing, frame generation and the upscaler off. List indices come from observed mappings
  (`profile_mappings`): each arm's own settings file is read, its current selections are remembered as
  immutable `profile-mappings-<hash>.json` files beside the study, a contradicting observation is
  refused, and a value never observed keeps the profile `BlockedProfileMapping`. The shipped settings
  show native resolution and frame generation off but FSR2 selected, so the "no upscaler" value is the
  one mapping still unobserved. Context conformance is stage-aware: a feature study may differ in its
  own factor only.
- **G0** also re-reads the frozen Proton identities, the Steam client and the account's launch
  record (`ProtonIdentityChanged`, `SteamClientChanged`, `EntitlementNotVerified`).
