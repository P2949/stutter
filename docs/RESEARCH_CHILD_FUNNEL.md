# Research child-study funnel: status and remaining integration

Status at this revision: the funnel is **connected to the resident controller, default off**, and
unit/controller-tested with fake hosts. The composed funnel has not been qualified on a live game.

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
| `research/arm_selection.rs` | viable screen arms; control and candidate from repeated sealed pilot runs |
| `research/child_verify.rs`, `raw_verify.rs`, `verdict.rs` | per-child reconstruction and raw replay; separate verification statuses |
| `runtime/research_funnel.rs` + `research_control_drive.rs` | `research_child_funnel` switch (default false, needs live launch); tick, cancel, shutdown, report |
| `runtime/research_recovery.rs` | restart: settle every child's interrupted attempt via the witnesses, then restore profiles |
| `research/dispatch_select.rs`, `steam_port.rs` | the dispatcher and port find a child's attempt under `studies/<digest>/dispatch` |

## Remaining action-child integration (CP-LONG)

The normal controller has a per-candidate research gate and retains the existing worker lifecycle.
An authorized long work unit, its execution and evidence lifecycle, runnable-delay objective, and
complete independent action/restoration verification still need integration. A real route and
physical action remain unqualified; no second applier or forced candidate is authorized.

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

Feature and action studies measure **one fixed Proton build** selected from pilot evidence with the factor or
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
- A treatment study measures the build the pilot's verified results select for its factor
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
unknown, never zero. Current G3 logic incorrectly requires an eligible frame receipt for every completed
attempt; the plan permits degraded optional frame-source evidence and requires broader resident
observation. The gate and independent raw replay still need correction.
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
- **G6** currently consumes a typed `fault-matrix-*.json` receipt that incorrectly requires physical
  passes for all fault classes. This remains to be corrected to the plan's fixture and benign physical
  acceptance policy; destructive real-game tests are prohibited.
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
- **Prefix initialization.** `research::prefix_init` is the separate stage a cold arm needs before any
  profile or comparison (the screen applies a profile before launching, and a cold prefix has no
  settings file). It is a bounded (15 minute), hash-chained journal per arm of immutable records
  (`prefix-init-<arm>-NNNN.json`): `Planned -> Launched -> Settled -> Verified`, or `Abandoned`.
  Verification needs a warm prefix with its settings file; a profile is never allowed while a launch is
  in flight or unverified; a forged, gapped, reordered or time-reversed journal is an error. The
  supervised launch itself is a live action and is not run; the module is staged until it is. The
  shader-cache state is not established by the prefix state.
- **CP-LONG producer interface.** `research::long_receipt` defines the lifecycle a producer follows
  (`LongSessionDriver`: start, poll, abort), the record a completed session leaves, and `verify`, which
  accepts it only for the authorized unit (same id and definition digest), played for at least the
  unit's duration and no longer than the authorized window, in an identified process, settled, with
  non-overlapping phases covering at least 90% of the session. This build's driver is
  `UnavailableDriver` (`BlockedNoLongSessionDriver`): nothing here can play a route in the game.
- **Cache cohort.** A child's prefix cohort (cold or warm) is observed before the study exists and
  sealed into its own intent (`prefix_cohort`), not left as an editable sidecar. A qualified
  comparison stage (pilot, confirmation, holdout, action, feature) is verified only over a warm
  cohort; a cold or unrecorded one is `ColdOrUnknownCohortNotComparable`, and only the exploratory
  screen may be cold. Prefix state is still not shader-cache state: a warm Wine prefix does not prove
  the Vulkan and game caches are warm, and per-attempt cache identity is not recorded.
- **Warmup and cache state.** A completed screen establishes feasibility and supplies a prior run for
  each viable build; three new whole-run pilot measurements per viable build select the pair. Neither
  prefix state nor the existence of a prior run proves actual shader-cache temperature. Equivalent
  cache preparation and cache identity still require independent evidence before physical acceptance.
- **Diagnosis oracle producer.** `research::oracle_cases` maps the controller's own `Diagnosis` to the
  oracle's causes (a cause that needs an attribution chain is reported only with an explicit one; the
  unknown and the causes the oracle has no name for are an abstention), commits ground truth by
  digest before any output is captured, and scores an `oracle-cases-*.json` file only if every capture
  is dated at or after the commitment and the cases match one-to-one. The verification response and
  `diagnosis-oracle.json` carry the scored evaluation, or `NoOracleCases`. Producing the cases (injecting
  a cause and capturing the live controller's output for it) needs the real game and controller.
- **Functional probes.** `research::functional_probe` reads a feature's state from the game's own
  sealed native context: frame generation (`frameGenerationType`) and the upscaler (`upscalingType`).
  A nonzero enum is game-reported selection, not proof of the technology, replacement DLL or effective
  backend. Feature qualification remains blocked until independently replayable functional evidence is
  connected.
- **Measurement window.** The observer buckets the frames it receives per unix second into each
  receipt (`frame_seconds`), and the verifier counts only the frames inside the attempt's native
  recorded completion window toward the 2 400-frame floor. The recorded start is currently first
  game-process sighting, not an independently established benchmark-scene start, so it can include
  menu or loading frames. A receipt with truncated or missing buckets counts none.
- **Capture identity.** `procfs_state` requires the same process (pid, start time, owner) before and
  after reading its threads; a recycled pid mid-capture leaves the state unknown.
- **G8 campaign binding.** A deployment receipt attests G8 only if the deployed binary is the build
  that froze the campaign: the campaign's `source_sha256` is a hash of the embedded Git revision and
  the binary's own hash, and the verifier recomputes it from its own revision and the pinned hash
  (`DeployedBinaryIsNotTheCampaignsBuild`, `ReceiptNotBoundToACampaign`). A campaign frozen by an
  older build therefore cannot be attested by a newer deployment.
- **G1 single owner.** The inspection reports whether a live process holds the lease's exclusive
  directory lock (`LeaseOwnerStillLive`; `LeaseOwnerUnknown` when the lock table cannot be read),
  read from `/proc/locks` without taking any lock. On filesystems with anonymous device numbers
  (btrfs) the table lists the filesystem's own device, so those are matched by inode. That the lease
  ends only through `Restoring` is already enforced by the transition check. Authenticated IPC is
  tested elsewhere and is still not attested by the inspection.
- **G1 admission.** The lease header records the submitting peer (pid and start time), the host facts
  the resident service collected, their digest and the admission time. The inspection re-checks that
  record: `LeaseAdmissionPeerUnidentified`, `LeaseFactsInconsistent` (digest or agreement with the
  intent) and `LeaseAdmittedOutsideIntentWindow`; a header with no admission record fails all three.
  This shows the admission was recorded for an identified peer with intact facts; it does not re-run
  the socket's credential check, which happened in the service at the time. The real 2026-11-15
  lease passes all three.
- **Oracle on the real engine.** `diagnosis/tests/oracle_bridge.rs` runs the diagnosis engine on its own
  fixtures and scores the output through `oracle_cases::from_controller`: a scheduler delay is
  runnable delay, a GPU-bound diagnosis is reported only if the engine attributed it, and a quiet run
  is a correct abstention. This is a fixture exercise and never stands for the physical cases.
- **Protected player data (G2: original data intact).** Before any launch the production host
  fingerprints the player's Cyberpunk save directory and each account's Steam Cloud copy
  (`protected_state`): a read-only content hash that follows no links, keeps no private paths, and treats
  an absent root as part of the state. A baseline that cannot be made means no host is built. A
  restarted campaign keeps its first baseline. Verification compares the data now with it, and G2 is
  blocked by `ProtectedStateChanged`, `ProtectedStateNotRecorded`, `ProtectedStateUnreadable` or
  `ProtectedStateTooLarge`. A campaign recorded before this existed has no baseline and so cannot
  attest G2. On the real library the saves are 192 entries and 409 MB (about 22 s in a debug build).
- **Host hardware (G0).** Before any launch the production host records the kernel release, CPU model,
  online CPUs, SMT state and every CPU's thread-sibling list, and each GPU's vendor, device and bound
  driver (`host_hardware`, read-only from `/proc` and `/sys`, no serials or paths). A host that cannot be
  read records nothing and builds no host. Verification compares and G0 names the first change
  (`HostKernelChanged`, `HostCpuTopologyChanged`, `HostGpuChanged`, `HostHardwareNotRecorded`). On this
  machine: an i5-10600K whose threads pair 0/6 ... 5/11 (SMT siblings, not twelve cores), and an AMD GPU
  (0x1002:0x7550) on `amdgpu`.
- **PLAN test crosswalk.** Every test named in PLAN's package test lists was searched for in the code.
  Gaps found and closed: protected saves/cloud state (G2), hardware identity (G0), and a test that a
  lease is never created over a conflicting mutator or validation override. The rest map to existing
  tests (for example replayed tickets: `each_steam_verb_runs_once_and_replays...`; spoofed peer:
  `kernel_peer_is_bound_and_wrong_uid_is_rejected`; owner appearing mid-window:
  `an_owner_appearing_leaves_an_experiment_alone...`).
- **G1 control socket.** The inspection also stats the resident controller's socket directory and socket
  and reads the lock table (no connection, no change): the directory is private and owned by the user,
  the socket is a real socket, owned by the user, mode 0600 with one link, and a live process holds the
  directory lock. Blockers: `ControlDirectoryNotPrivate`, `ControlSocketNotPrivate`,
  `NoControllerHoldsTheControlDirectory`, `ControlSocketNotInspected`. Observed on the live service
  (`g1-live-ipc-2026-11-17.json`): the owning user's `research status` reached the controller and was
  answered (`UnsafeToResume`, matching the interrupted 2026-11-15 lease); another user was refused by the
  kernel (`Permission denied`) at the private directory.
- **Why real diagnosis cases need the game.** The controller's diagnosis runs on spike clusters built
  from live eBPF scheduler events of a selected target (`session/monitor_session/exporters.rs`), so it
  has no input without a target generating them, and PLAN asks for the causes to be injected into the
  real game. The oracle producer and its bridge test cover everything up to that input.
- **Real-data check of the functional probe.** Run on the 9 preserved native results of the 2026-11-15
  campaign (parsed by the product's own verifier), every result reports the same state: an upscaler on
  (`upscalingType` 2), frame generation off, ray tracing on, on the RX 9070 XT. The probe correctly
  returns `Unproven` (no Off run) or `NotObserved` (state contradicts the label) for every labelling,
  so a feature can be shown working only by runs where the factor differs, which only new launches
  produce. `docs/validation` holds summaries and markers and `testdata` planner fixtures: no raw
  scheduler-event recording exists for the diagnosis engine; capturing one needs an eBPF load as root on
  the live desktop, which is a decision, not an offline task.
- **Raw diagnosis capture needs root (demonstrated).** `stutter doctor` reports `unprivileged_bpf_disabled=2`
  and `is_root=no`, and `stutter record` as this user fails with `failed to read tracepoint format
  /sys/kernel/tracing/events/sched/sched_wakeup/format: Permission denied`
  (`ebpf-capture-unprivileged-2026-11-17.json`). A raw scheduler-event recording therefore cannot be made
  from this account; making one is a root eBPF load on the live desktop, which needs authorization.
- **PLAN test crosswalk, audited.** `plan-test-crosswalk-2026-11-17.json` maps 41 items from PLAN's
  package test lists to their covering tests by file and function, with none uncovered. The last
  gap found, "no operation bypassing `autonomous_apply_gate`", is now an architecture test: the one
  production call of the apply must follow the gate in its own function (it fails if the gate call is
  removed).

## Persistence transactions (thirteenth audit, 2026-11-21)

- `report_store::write_new` now publishes atomically: bytes are written and fsynced under a hidden
  `.NAME.tmp`, renamed into place with `RENAME_NOREPLACE`, and the directory fsynced. A crash leaves at
  most a temporary file, never a partial canonical record; `read_evidence` ignores such leftovers and the
  retry discards them. An existing record is never replaced (`AlreadyExists`).
- A settled child whose accounting could not be written keeps its original terminal disposition in
  `pending_end`. Polling again (and `abort_outcome`) retries only the persistence; the driver is never
  ticked again. `StudyAttempts::persistence_pending` lets the executor keep the study running (bounded by
  three consecutive failures, after which the existing abort path records it as unresolved).
- Telemetry receipts are frozen under the observer's lock, written outside it, and acknowledged by count.
- Scene length: a benchmark whose frames/averageFps differs by more than 5% from the others marks the
  native context `Drifted`.

## Prefix initialization stage (thirteenth audit, priority 2)

- `prefix_init_stage::InitStageRunner` runs one arm's journalled stage (`Planned -> Launched -> Settled ->
  Verified | Abandoned`) tick by tick over an injected `InitHost` (launch, observe, terminate-and-prove).
  It never replays a launch: one found unfinished after a restart is only settled. A launch needs the
  whole 15-minute bound authorized (never shortened) and an overdue one is terminated and proven gone.
- `init_phase::InitPhase` walks the cold arms in turn (warm or already-verified arms are skipped, an
  indeterminate prefix is not touched), calls the controller's launch fence before each launch, charges
  the elapsed seconds against the authorized initialization time, and abandons an arm that does not
  initialize (it stays cold, so the profile hook blocks its studies). Only an unprovable settlement ends
  the campaign.
- `FunnelDriver::with_init_phase` runs the phase before any study: ticks show `Waiting` until it is
  finished; cancellation and shutdown cancel and settle a launch in flight.
- The production `InitHost` over the authenticated Steam launcher (non-measurement launch purpose) is
  the remaining step; the stage and phase are exercised against synthetic hosts only.
- `init_steam_host::SteamInitHost` is the production `InitHost` over the same authenticated `SteamPort` the
  measured attempts use (same safe-window, GPU-health, disk-reserve and dispatcher admission, same
  independent settlement witness). Initialization differs only in purpose: nothing is measured and the
  game ending is the expected outcome, settled through `port.stop()`. A voided launch (GPU reset,
  identity mismatch) is settled before it is reported. The composition supplies the per-arm attempt slot
  and dispatch context; this module builds no identity.

## Native tail metric (thirteenth audit, priority 3)

- `frame_stats::FrameStats` (p99 frame time by nearest rank, 1%-low FPS) is computed from the verified
  per-frame times and stored in `NativeContext.frame_stats` only when the run holds at least 1000 frames;
  a smaller or corrupt population has none (never a guess). Old results lack it: `matches_replay`
  compares them without it, and a recorded tail must match the files exactly.
- `metric_plan::values` chooses the primary metric once per campaign from the sealed evidence: the 99th
  percentile when every completed attempt has a known population, otherwise the average-FPS fallback for
  the whole campaign (never a mix). The report's design, rule and values use it; the report names
  `primary_metric` only for the tail, so reports recorded before this still rebuild byte-for-byte (the
  real campaign `fdf5ba08ff2acfac` still replays and bundles).
- Remaining for the metric: the pilot/confirmation rule and the screen/pilot selectors still read average
  FPS; they move to the same plan when frame statistics are recorded for live runs.
