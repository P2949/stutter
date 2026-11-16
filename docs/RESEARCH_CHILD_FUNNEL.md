# Research child-study funnel: controller integration

Status: the pieces below exist and are unit-tested but **not connected** to the resident
controller. This document lists what connecting them requires so that no existing safety gate is
weakened. Nothing here is validated against a live game.

## Existing pieces (all under `stutter/src/autotune/research/`)

| Piece | Role |
| --- | --- |
| `child_study.rs` | frozen funnel plan, dependency gate, budget fit |
| `child_ledger.rs` | hash-chained, restart-safe decisions and spent attempts |
| `child_runner.rs` | polled stage runner (`Pending` / `Ran` / `Halted` / `Done`) |
| `child_intent.rs` | per-study intent (`child_study_sha256`, own attempt budget) |
| `study_executor.rs` | apply profile, run attempts, always restore, map outcome |
| `profile_applier.rs`, `profile_hook.rs` | atomic, read-back, byte-exact restore of `UserSettings.json` |
| `driver_attempts.rs` | one child study through the journalled `Driver`, one tick per poll |
| `prefix_identity.rs` | cold/warm class and Proton-stamp comparability |
| `action_child.rs`, `action_bridge.rs` | scoped authorization and sequencing for the action stage |
| `verdict.rs` | separate verification statuses; final gate never inferred |

## What the controller needs before the funnel can replace the flat schedule

The coordinator (`runtime/research_control_drive.rs`) assumes exactly one `Driver`. Each of these
paths must be given an explicit child-funnel behaviour, with a test, before the mode is enabled:

1. **Selection.** A deliberate switch (daemon config, default off). The flat schedule stays the
   default; no intent or deployed campaign changes behaviour.
2. **Lease phases.** `drive_attempts` advances the lease from each `Step`. The child runner hides
   `Waiting`/`Running`/`Sealed`; it must surface them (or take a callback) so the same
   `Launching -> Sealing -> Analysing -> NextArm -> AwaitSafeWindow` path is walked per attempt.
   The launch fence passed to `DriverStudyAttempts` must be the coordinator's own
   `AwaitSafeWindow -> Launching` advance.
3. **Cancel.** `request_driver_cancel` reaches only the flat driver. A running child driver must be
   cancelled the same way, its attempt aborted and sealed, and the funnel ended without deciding
   the remaining studies.
4. **Shutdown.** `shutdown_driver` must settle an in-process child attempt, restore the applied
   profile (proven), and refuse to exit cleanly when either is unproven.
5. **Deadline and active budget.** `Step::Expired` and `ActiveBudgetExhausted` apply to the
   campaign, not a study: the funnel must stop at the campaign boundary and record what it left.
6. **Restart recovery.** `Driver::recover_interrupted` is wired for the flat journal only. A
   restart with a child journal that holds an unsealed attempt must go through the same
   independent witnesses (process tree, worker tokens, settlement) before anything is sealed, and
   the interrupted study must be recorded `Failed("InterruptedStudy")` and halt the funnel. The
   executor already refuses to start a study whose directory exists; recovery must run first.
7. **Profile restoration after a crash.** If the controller dies while a profile is applied, the
   next start must find the applied file (the snapshot is in memory today) and restore it. The
   snapshot therefore has to be persisted durably before the profile is applied.
8. **Report and verify.** `persist_report`/`load_and_verify`/`raw_replay` reconstruct from the
   flat journal. Each child directory (`studies/<digest>/`) needs the same reconstruction, and
   `verify` must fail closed when any child is unverifiable.
9. **Health.** The GPU guard's `blocked()` must be read by the runner before every poll (it is a
   parameter of `StageRunner::step`) and a halt must end the funnel campaign-wide.
10. **Evidence budget and disk reserve** are per campaign: the sum over children, not per child.

## Not connectable without a live run

* The real `ActionWorker` over the controller and root worker (physical restore proof).
* Observed list indices for the CPU/GPU graphics profiles (`BlockedProfileMapping`).
* Real-container Proton paths (`RuntimeBinding::Matched` has never been produced live).
* Whether the game accepts and honours an edited `UserSettings.json`.
