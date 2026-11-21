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

## Production-composed synthetic baseline campaign (thirteenth audit, priority 3)

`driver_attempts_tests/baseline_campaign_tests.rs` runs the real `FunnelDriver`, journals, child studies,
selection, per-study reports and `child_verify::verify_children` over a host that injects only the world:
each launch writes a real native result folder (agreeing summary and frame file, 64.25 s scene, sealed
hashes, matched runtime) under a per-arm results directory. All eight baseline studies (two profiles
across screen, pilot, confirmation, holdout) qualify, every child verifies from disk (reconstruction, raw
files, matched runtime, cohort, rule, replication), and flipping one byte of a frame file after sealing is
found by independent verification. This exposed a real defect: a child's tools followed the selector's
order while the verifier demands the campaign's order; `child_intent` now always uses the campaign's own
order. Feature and action studies are not in this intent (no factors or actions authorized).

## Gate corrections (thirteenth audit)

- **G6** is no longer physical-only: `fault_matrix::DESTRUCTIVE_CLASSES` (worker death with tokens
  outstanding, settings crash, GPU-health halt) are proven by fixtures bound to this binary and a physical
  case for one is refused (`DestructiveFaultInjectedPhysically`); the benign classes still need a physical
  pass. Fixtures never stand for a benign class.
- **G7 / overall** no longer require every study to be `Qualified`: a feature study with result `Blocked`
  and reason `BlockedNoEligibleSource` (no authorized build supports the factor) is a legitimate exclusion
  (`gate_verdict::legitimately_excluded`). Nothing else blocked, failed or undecided resolves, and a
  comparison or action study is never excluded.
- **G3**'s direct frame source is optional: with no receipts the gate accepts and carries the note
  `DirectFrameEvidenceNotCollected` (`GateVerdict.notes`); receipts that exist must be sound or the source
  quality blocks.

## Frozen tail metric (thirteenth audit, priority 3 continued)

`metric_plan::run_values` is the one function every selector, rule freezer and verifier uses to turn a
study's completed runs into comparison values: each run's 99th-percentile frame time as an FPS-equivalent
(1000 / p99 ms, so higher stays better) when every run has a known frame population, otherwise every
run's average FPS. A pilot's `FrozenConfirmationRule.tail_metric` records which (absent = average, and the
digest only changes for the tail, so earlier rules are unchanged). The confirmation, holdout and matched
studies then keep the frozen metric (`metric_plan::values(.., Some(frozen))`); evidence that cannot supply
it is an error, never a silent switch to another metric. The synthetic baseline campaign now runs on the
tail metric end to end and every child report names `primary_metric = native_p99_frame_ms`. The matched
CachyOS rule is derived from the selection values and carries its pilot's frozen metric (the dependency rule's flag), in both the funnel and the verifier.

## Raw native archive (thirteenth audit, priority 3)

`raw_archive::archive` copies every completed attempt's sealed native files from the prefix's results
directory into the study (`studies/<study>/raw/<arm>/<result folder>/...`, private and write-once), hashing
each against the seal as it is copied and bounding file and study size; an existing copy must be
byte-identical and a path that escapes its folder is refused. Replay (`verify_raw_evidence`, live in
`DriverStudyAttempts::outcome` and offline in `child_verify`) then reads the campaign's own copy, so
rotating, editing or deleting the prefix results afterwards cannot change or defeat verification, and
editing the archived copy is found. Studies without a `raw/` directory (older campaigns, such as the
preserved real one) still replay from the supplied results directory.

## Independent budget replay (thirteenth audit)

`campaign_budget::replay` runs inside `child_verify::verify_children`: every child must have a spend
record, its recorded evidence bytes must equal what its own sealed evidence costs (recomputed), and the
totals (active seconds, evidence bytes, and the attempts the ledger counted) must stay within what the
operator authorized. A problem is `ChildrenVerification.budget_problem` and `all_verified` requires none.
Active seconds are the child's own record and are bounded, not rebuilt.

## Prefix initialization as a child study (thirteenth audit, priority 2)

A cold arm is initialized by its own child study run by the existing funnel and `Driver`, never by a second
launch authority:

- `ResearchIntentV1.prefix_init_arms` (omitted when empty, so existing intents and digests are unchanged)
  freezes the arms whose prefix was cold when the campaign was issued; it must name distinct authorized
  tools. It is operator-authored with the rest of the intent; a cold arm left out is still safe, because
  the profile hook blocks its studies (`BlockedPrefixNotInitialized`).
- `child_study::plan` opens with `Stage::Init` only when that list is non-empty (one attempt per listed
  arm) and every screen depends on it; an unqualified initialization therefore blocks every study that
  rests on it (`PredecessorNotQualified`) instead of comparing cold prefixes.
- The Init child's intent has `initialization = true` and exactly the listed arms. Its schedule is one
  attempt per arm (`init_schedule`; no balanced blocks, one arm is enough). The executor applies no
  profile for it.
- `SteamAttemptHost` (`steam_host_init.rs`) runs an initialization attempt through the same authenticated
  port as every attempt (same admission, tickets, settlement witness): it stops the game, proven gone
  through the port, once the arm's registry and `UserSettings.json` have held for 15 s, or lets it end;
  the placeholder evidence has no native completion, so nothing is measured.
- After the attempts the host observes the prefixes once (`check_prefixes`); `init-observation.json` is
  sealed beside the study and the live judgement and every verifier read the same record
  (`init_receipt`). The study qualifies only when every listed arm was observed warm.
- `child_verify` replays an Init child from its journal and that receipt; it needs no raw native files or
  runtime match, and its cold cohort is exempt from the warm-cohort rule. The synthetic baseline campaign
  covers a cold arm (nine studies, all verified) and an initialization that leaves the prefix cold (every
  dependent study blocked).
- `prefix_init` (the journalled stage contract) remains as a staged module; the earlier standalone stage
  runner, phase and Steam init host were removed in favour of this composition.
- Authoring the cold-arm list: `stutter research freeze-cold-arms --intent <private intent.json>
  --study-root <private study dir>` prints the intent with `prefix_init_arms` filled from what each
  authorized tool's `compatdata/<arm>` prefix is now (cold arms listed in the intent's tool order; a warm
  prefix is left out; a partly created one is refused). Read-only and local. Run against the real preserved
  campaign intent and study directory it lists nothing (both arms are warm), so that intent and its digest
  are unchanged.

## Gated action through the normal controller path (thirteenth audit, priority 5)

`runtime/tests/planning.rs` drives a CPU-affinity candidate judged by `GameRunnableLatency` (a Supported
objective) through the real `AutotuneRuntime`: the action gate permits it, `decide_autotune_transition`
starts an experiment, the runtime applies it (simulated effects), the gate sees the outstanding action, a
stop rolls it back, and the gate settles and records the restored action (`evidence.clean`, one settled
action; the kernel readback is honestly not claimed without a state probe). The same candidate judged by
`StutterScore` (Experimental) is refused by the product maturity gate (`objective_not_supported`) and
starts nothing. What this cannot show offline is the real causal effect: a real apply and restore on the
running game, with a GPU negative control.

Also: three `commands::daemon::status` tests read the live daemon's state snapshot from the default path
and failed whenever the machine's controller was in another mode; they now read a path that does not exist.

## CP-LONG: the long attempt is a sealed session record (thirteenth audit, priority 5)

- `ResearchIntentV1.long_work_unit` (omitted when absent) freezes the one authorized unit with the
  separately authorized `long_attempt_seconds`; it must be valid and fit that window. The funnel's
  `long_unit_blocker` reads it, so a long study with no unit stays `BlockedNoComparableLongWorkUnit`.
- `AttemptEvidenceV1.long_session` (omitted when absent) carries a `LongSessionRecord` in place of a native
  benchmark result, never both. `raw_verify::verify_raw_evidence_for` verifies such an attempt with
  `long_receipt::verify` against the intent's unit and window (and the same user); the live outcome and
  `child_verify` treat it as verified with no runtime to match and conformance `NotSpecified`, and
  `evidence_context_consistent` requires every session to be the same unit.
- `long_host::LongAttemptHost` launches a long attempt through the ordinary host (same admission, tickets and
  settlement), plays the unit through a `LongSessionDriver`, verifies the record, settles both the session
  and the process tree, and only then finishes the attempt. A failure of the game wins; every other attempt
  is delegated untouched. The production host is wrapped with `UnavailableDriver`, so a long attempt fails
  with the typed `BlockedNoLongSessionDriver`; nothing is ever run on the benchmark.
- Tests: `long_host_tests` (verified, wrong unit, unsettled, no driver, no unit, game failure) and
  `driver_attempts_long_tests` (a Long action study qualifies on its session records plus the gate's proven
  restoration and verifies offline through `verify_child`; a record for another unit does not qualify).
- What remains needs the live game: a `LongSessionDriver` that plays a route inside Cyberpunk, and its work
  unit's save, route and in-game conditions.

## Feature observation replay (thirteenth audit, priority 6)

A feature study records `feature-observation.json` beside itself: per completed attempt, the treatment it ran
and whether that treatment was seen in the sighted game process's own environment. `child_judgement` now also
replays a feature study that was *not* qualified: the receipt must name exactly the completed slots, each with
the treatment the intent's own schedule (`treatments_of`) gives that slot; activation is rebuilt from it and
the functional report from the sealed native contexts, and the replayed reason must equal the recorded one
(`UnqualifiedFeatureJudgementContradicted` otherwise). A missing or inconsistent receipt replays as unproven
and disagrees with any stronger recorded reason. Feature studies themselves stay unqualifiable offline (the
game's enum cannot identify the exact backend); this makes every reason they carry independently replayable.

## Real-history diagnosis scoring: checked, not possible offline

The archived controller history (`history-archive-20261105.jsonl.zst`, 150,110 real observations,
2026-10-31 14:23 to 2026-11-06 01:00 UTC) cannot be scored against ground truth: every labelled execution on
this machine (the 80 marker files under `docs/validation`, cold cargo builds with start and end markers)
begins on 2026-11-11, at least five days after the last archived observation, and the archive's `Game*`
situations were never run under an injected cause. Zero seconds overlap (evidence:
`StutterProtonStudies/real-history-ground-truth-2026-11-21.json`). A real diagnosis score needs a live
capture with a known injected cause.

## Why the remaining items cannot be started now (observed 2026-11-21)

`stutter research status` against the resident controller answers `{"code":"UnsafeToResume","status":"blocked"}`:
the GPU incident file on disk has no acknowledgement (`gpu-stability-acks` does not exist; acknowledging is
human-only), the game is installed but not running, and the audit forbids any game launch, dispatcher
selection or GPU stress while that halt stands. A long-session driver, and a causal action apply/restore with
a GPU negative control, both require launching and driving the game, so neither can begin. Evidence:
`StutterProtonStudies/live-items-blocked-2026-11-21.json`.

## Settings and thermal context per attempt (thirteenth audit, priority 3)

- **Settings actually run with:** after a profile is applied the hook reports the SHA-256 of every settings
  file as the profile left it (`ProfileHook::applied_digests`); the executor hands them to the attempts before
  the first launch and `applied-settings.json` is sealed beside the study. A result whose sealed
  `settings_sha256` is not one of those digests did not run with the frozen profile (resolution, render scale
  and every other setting included): it is `ProfileViolated`, live and in the verifier's replay. Real
  campaign data shows the game does not rewrite the file after a run (the sealed hash equals the file's
  current hash), so a mismatch means a changed file. A study with no recorded digests (no profile, or older)
  is not held to this.
- **Thermal context:** `thermal.rs` reads the CPU package (`coretemp`/`k10temp`) and GPU (`amdgpu`) hwmon
  temperatures at launch and at the result (`SteamAttemptHost`, `AttemptEvidenceV1.thermal`; both sensors
  exist on this host). Attempts of one study must start within 10 C of each other and none may end at or
  above 95 C (CPU) / 100 C (GPU); a hot start or throttled end is `Drifted`, a partial or blind record is
  `Unproven`, and results from before this existed (none recorded) are not held to it.
