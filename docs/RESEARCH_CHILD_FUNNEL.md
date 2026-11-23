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

## Real apply and restore on a stand-in process; hardware-derived thermal limits

- `a_real_apply_and_restore_on_a_stand_in_process_is_proven_against_procfs` runs the runtime's own,
  unsimulated apply on a process the test spawns (`sleep`, not the game): the gate permits it, the decision
  starts the experiment, `sched_setaffinity` really changes the target (`Cpus_allowed_list` becomes `0`), the
  stop rollback really restores it, and the gate's real `/proc` state probe proves the restoration
  (`clean`, one settled action, `kernel_state_proven`). It touches no game, GPU or other process. What stays
  live-only is the same sequence on the game with a GPU negative control, which needs a launched game.
- Thermal ceilings are no longer fixed guesses: each attempt records its sensors' own critical temperatures
  (`temp1_crit`: 100 C for this CPU, 110 C for this GPU) and is thermally limited within 5 C of them; the
  95 C / 100 C constants remain only as fallbacks for a sensor that reports no limit.

## CP-LONG: the production route driver; only the input backend is missing

`route_driver::RouteDriver` is the production `LongSessionDriver`: it plays a route plan (phase names and
lengths that must sum to exactly the authorized unit's duration) phase by phase on the clock through a
`RouteInput`, begins only once the host has attested the game process (`LongSessionDriver::observe_process`),
requires that very process (pid and start time) for the whole session, closes late polls at the true phase
boundaries, and records what actually happened. It never claims settlement: `LongAttemptHost` now stops the
session and the process tree first and only then attests the record settled before `long_receipt::verify`.
Tests cover the schedule, a refused input, a lost or replaced process, an abort, and a Long action study
driven end to end by the real driver through the host (qualified, verified offline). The only thing absent is
the `RouteInput` that sends the route's inputs to the game; the build's input is `UnavailableInput`, so a long
attempt still fails with the typed `BlockedNoLongSessionDriver` until a physical backend exists.

`scripted_input::ScriptedInput` is the scripted `RouteInput`: per-phase key scripts with times relative to the
phase, encoded as exact Linux `struct input_event` records (each followed by a sync report) and written through
an `EventSink`; the route driver feeds it the phase's elapsed time, a phase never inherits held keys, and
stopping, a failing sink or the end of the route releases every key. Everything but the device sink is tested.
No device sink is in the repository on purpose: a virtual keyboard registered on the user's machine is picked
up by the running compositor and types into the focused window, so a `/dev/uinput` sink can only be exercised
in a session of its own. The build keeps `UnavailableInput`.

## GPU negative control, offline portion

`planner_tests::policy_denials::a_cpu_affinity_action_is_outside_the_support_registry_for_a_gpu_bound_game_only`
evaluates the same CPU-affinity action (same objective, same mode) for a game under scheduler pressure and for
a game that is GPU bound: only the GPU-bound game puts it outside the autonomous support registry
(`NotInSupportRegistry`), so the planner never proposes it where moving CPU threads cannot help. What this
cannot show is the measured control on the real game: the action applied to a GPU-bound run producing no
benefit. That needs a launched game and a controlled GPU load, which the GPU incident halt forbids.

## Fourteenth audit: G3 and G7 acceptance paths

- G3: `telemetry_summary` now counts only attempts that have a direct-frame receipt. A campaign with no receipts at all is the optional probe not collected (accepted with the note `DirectFrameEvidenceNotCollected`); a receipt that exists but is duplicated, conflicting, unbound or too thin still rejects with `FrameSourceNotEligible`. The mandatory game-scoped CPU/GPU/runnable/PSI/ownership observation is not produced offline and stays a physical item.
- G7: a feature study whose treatment reached the game but whose result carries no report for the factor (`FeatureNotObservable`) is an exclusion by name, never a qualification. Reported-off, unproven activation and missing function proof remain unresolved.

## Fourteenth audit: settings bound to each arm

- The profile hook no longer drops an arm whose file cannot be read back after the edit: the application fails and the edits are put back, so the record has one digest per arm in the child's tool order.
- `settings_binding` holds each attempt to the digest of its own arm (from the journal schedule), not to any digest in the list. A record of the wrong length, a missing record in a study that keeps a raw archive and applied a profile, and a scrambled one all fail conformance (`ProfileViolated` live, a verifier problem on replay). Older studies without a raw archive keep replaying as before.
- Still not verified: the effective resolution and render scale are covered only through the settings hash, not parsed.

## Fourteenth audit: archive, budget and G6 evidence

- `raw_archive` walks both the game's result folders and the study's archive by directory descriptor without following links; every archive folder must be private and owned by this user, and a link or loose folder is refused (tested).
- `campaign_budget::replay` now also measures, from disk and without following links, every file under each study besides its sealed records and journal (the raw archive and receipts) and counts them toward `max_evidence_bytes`. Active time is still only bounded, not recomputed.
- G6: `fault_matrix::artifacts_verified` opens each required case's `fault-evidence/<sha256>.json`, re-hashes it, and requires it to say the same class, mode, outcome and binary; missing, modified, shared, misclassified or linked records are refused. A receipt of digests alone no longer attests.

## Fourteenth audit: CP-LONG delivery evidence

- `RouteDriver` fails a session with `LongSessionInputGap` when the controller goes more than `MAX_POLL_GAP_SECONDS` (5) between looks at a running route, instead of crediting the phases the silence covered. The sealed `LongSessionRecord` carries `max_poll_gap_seconds`, and `long_receipt::verify` rejects a record without it (`LongSessionDeliveryUnproven`) or over the bound.
- A key whose release write was refused stays recorded as held (`ScriptedInput::released()` is false); the next phase will not begin on top of it and the driver ends or aborts with `LongInputReleaseUnverified` instead of completing clean.
- Still no physical input backend: production keeps `UnavailableInput`, so a real long attempt fails with `BlockedNoLongSessionDriver`. Delivery is bounded by polling, not observed at the device.

## Fourteenth audit: scale and thermal notes

- Practical-effect scale: the threshold is stated on the frame-time (ms) scale that confirmation analyses; the pilot uses 1000/p99 only to estimate variability, and a test shows the coefficient of variation agrees between the two scales within 8 % for any spread a pilot accepts.
- Thermal: the production host always seals a thermal context (readings may be unknown), so a new study without sensors is `Unproven`, never `NotRecorded`; only results sealed before the field existed read as not recorded, and sealed evidence cannot have the field removed without breaking its digest.
- Open (typed, not done offline): G5 still has no justified no-pressure no-op disposition (needs an owned-pressure control record); a feature cannot qualify without an independent effective DLL/backend proof; the mandatory resident game-scoped CPU/GPU/PSI/ownership observation is not produced offline; historical verification still reads some current-host facts at finalization.

## Fourteenth audit: G5 no-op and owned-pressure control

- `pressure_control` defines the receipt (`pressure-control.json`: campaign-owned load, at least 1000 scoped events, runnable p95 visibly worse under load, action applied, restoration independently proven, fresh, this binary) and an independent reader; the control-report path passes its status into the gate.
- G5 accepts an action study that ended `Unqualified` with reason `ActionNotIndicated` only beside a passing control (note `ActionAbstainedNoPressure`); without the control the blocker is the control's own reason. Any other refusal is never a justified no-op.
- Not done: nothing produces `ActionNotIndicated` or the receipt yet; both belong to a physical run, so G5 cannot pass offline.
- Verified this pass: `cargo test -p xtask` (620+1 passed), `release-check` PASS, NixosConf research remote is e2f5ff37 pinning Stutter 9ceaef38 / binary 8d0f57ce.

## Fourteenth audit: bundle crosswalk and gate scope

- The final bundle's `capability-matrix.json` now carries `crosswalk_34`, the 34 Observed/Tested/Excluded/Blocked closure rows built by `crosswalk::build_rows` from the same embedded ledger (an invalid ledger is reported as `CrosswalkInvalid`, never a shorter table).
- It also carries `gate_scope`: G2–G5, G7, G9 as historical (sealed evidence) and G0, G1, G6, G8 as current-host at verification time, with a note that a later host change never rewrites the historical result. The per-gate verdict itself is unchanged.

## Fourteenth audit: historical result apart from host gates

`Verdict.historical_evidence_qualified` is the campaign's sealed-evidence result (study-backed facts plus G2–G5, G7, G9). `final_gate_qualified` still also needs G0, G1, G6 and G8, which describe the host today; a later host change can move the second but never the first.

## Fifteenth audit P0: profiles over the child's own arms

- The production `FileProfileHook` was built over every parent arm and recorded one digest per parent arm, while the verifier expects one per child arm; a six-arm campaign's two-arm confirmation would have read `ProfileViolated`, and an unusable unselected arm blocked a valid pair.
- `StudyAttempts::selected_arms` resolves the child's arms from the recorded results before any profile is applied; `ProfileHook::apply_arms` carries them. `FileProfileHook::with_arm_map` names each file's and each unavailable path's arm (set by `runtime::research_funnel::profiles`); an arm-aware apply edits, loads mappings from and records only the selected arms, in the campaign's order, and only selected unavailable arms block. Recovery and restore still cover every file.
- Tests (real hook): two of six arms edited and recorded in campaign order and restored byte for byte; an unusable unselected arm does not block, a selected one does, and an apply without arms stays all-or-nothing.

## Fifteenth audit P1: qualified actions replay

- The shared judgement replayed every Action study with `stage_evidence = Some(false)`, so any live-qualified action read `QualifiedStudyJudgementContradicted: ActionEvidenceMissing` under `verify_children`. `child_judgement::action_replay` now rebuilds it from the sealed `action-receipts.json`: receipts valid (same process, transaction, identical restored placement), attributed to completed attempts, on every attempt the schedule treated On and none treated Off.
- `child_verify_rules::processes_of` now also attributes a long-session attempt's process (before, a long action study had no attributable attempts at all).
- Test: per-attempt processes; passes with a valid receipt on every treated attempt; fails for a missing, extra-on-untreated, differently-restored or absent receipt file. Not yet done: a full campaign test with an action study through `verify_children`, the no-op disposition producer, the CP-LONG latency metric.
- Hygiene: `url` updated to 2.5.9 (cargo-deny advisory); `arm_gate_for` moved to the persistence module for the file-size cap.

## Fifteenth audit P1: justified abstention and the long-session metric

- `ActionNotIndicated` now has a producer: when an action study's gate was armed, nothing was applied or left outstanding and no receipt exists, the driver writes `action-abstention.json` ({completed, applied: 0}) and the judgement returns `Unqualified / ActionNotIndicated`. `child_judgement` replays it (record names exactly the completed attempts, applied is 0, and `action-receipts.json` is truly absent, not merely unreadable).
- It is a terminal study result everywhere the verdict looks: `gate_verdict::resolved` accepts it, `actions_restored` accepts it only with a reconstructed child and no receipts, and G5 additionally requires the owned-pressure control (blocker = the control's own reason). Verdict-level test covers with and without control.
- Limit: the abstention proves the action was never applied, not why; the "why" (no eligible pressure) rests on the control plus the controller's own planner events, which this repository cannot produce without the game.
- A long session's sealed placeholder FPS (1.0) is never analysed: `treatment_result` returns `ActionMetricUnavailable` with no numeric verdict. `benefit_demonstrated` additionally requires the action study to be qualified and its child reconstructed without a problem.

## Fifteenth audit P2: no silent downgrades

- A child study with native results must have its raw archive: `verify_child` fails with `RawArchiveMissing` instead of replaying from the game's results directory, and the settings binding is strict for every verified study (no "legacy" fallback; no child study from an earlier build exists in any campaign, the preserved real campaign is flat).
- Telemetry receipts: absent is the optional probe not run; present but unreadable, not JSON or not a list is carried as `InvalidTelemetryReceipts` and counts every completed attempt of that child as collected but not eligible (`FrameSourceNotEligible`). Tested over the production-composed campaign.
- Admission (`campaign_budget::spent`) now counts the archive and receipts on disk, the same measurement the final replay uses.
- `pressure_control::status` and `fault_matrix::artifacts_verified` read through directory descriptors without following links and measure on the open descriptor.

## Fifteenth audit P3: historical result, scope and crosswalk

- `Verdict.historical_evidence_qualified` is now computed from gates recomputed with `health_blocked = false`, `protected_state = Ok` and an expired control receipt read as fresh, so a later GPU incident, different saves or an old receipt cannot move it (tested); `historical_notes` says the protected-state comparison is still not sealed at finalization. The current-host `final_gate_qualified` and `AbortedHealth` are unchanged.
- `Verdict.qualification_scope`: `QualifiedEndToEnd`, `QualifiedWithUnobservedFeatures` (FeatureNotObservable), `QualifiedWithExcludedOptionalFeatures` (no eligible source) or `QualifiedWithJustifiedAbstention`.
- Crosswalk rows now carry `planned_closure` (the role, not a result), `observed_disposition` (`NotObservedInThisCampaign` unless the campaign supplies a digest of its own evidence via `build_rows_observed`) and `observed_source_sha256`. The bundle supplies none yet, so no row claims an observation.
- Tests that sealed native results under tiny evidence caps now use realistic caps, because admission counts the archive.

## Fifteenth audit P4: CP-LONG definition and delivery timing

- `RouteDriver::definition_sha256` hashes the route (phase names and lengths in order) and the input's own key script (`RouteInput::script_sha256`, a digest of every step's time, key and direction for `ScriptedInput`). `start` refuses a unit whose `definition_sha256` is another one (`BlockedRouteDoesNotMatchTheUnit`), so the unit's frozen definition is derived from what is actually played, not copied.
- `MAX_POLL_GAP_SECONDS` is now 2 and `ScriptedInput` fails a step delivered more than `MAX_STEP_LATENESS_MS` (2000) after its time with `LongInputLate`, releasing every held key first. Sub-second timing is still bounded by polling, not observed at a device; there is still no physical backend and none is built while the only desktop is the live Hyprland session.

## Sixteenth audit P1: scientific gate semantics

- Confirm/Holdout/MatchedCachyOs studies are `Unqualified / ComparisonRefused` when any arm's paired analysis produced no numeric verdict (`AttemptsOutcome.comparison_valid`, computed live from the report and on replay from the child's report).
- An Action study needs a valid paired GameRunnableLatency record (`action_latency`: `action-latency.json`; per pair the schedule's own Off and On attempts of one block, the very processes they recorded, >=1000 scoped events each side, finite positive p95, source Eligible, guardrails held, >=3 pairs). Applied-and-restored without it is `Unqualified / ActionMetricUnavailable`; a null result (no improvement) with a valid record is still a measurement. Nothing produces the record offline, so G5 cannot pass until a physical run does.
- G3 has a mandatory input independent of the optional direct frames: the sealed `resident-observation.json` per measuring child (`resident_observation`: per completed attempt the exact process, renderer bound, cgroup, >=10 CPU/GPU/PSI samples, >=1000 runnable events, owners identified, source Eligible). Missing, corrupt or sparse records block G3 (`ResidentObservation*`), whether or not any frame receipt exists. Nothing produces the record offline.
- `campaign_budget::replay` now bounds each child's recorded active seconds below by the time its completed attempts witnessed (`BudgetActiveSecondsUnderstated`).

## Sixteenth audit P2: production funnel integration

- Cold-prefix initialization: an initialization attempt's native observer (`begin_initialization` through `ResultProbe`/`NativeObservationSource`, chosen by `launch_checked` from `intent.initialization`) tolerates the arm's missing `UserSettings.json` at first sight and records an empty digest, because nothing is measured and creating that file is the attempt's purpose; a measuring attempt still fails as an identity mismatch. Tested at the observer. Not tested: the full port with a real launch.
- One selection per study: `DriverStudyAttempts::frozen_inputs` freezes the sources, order and confirmation rule the first time they are asked for; profile application (`selected_arms`), the attempt count and the child intent reuse it, and `begin` re-reads the selector and fails the study (`SelectionChangedAfterItWasFrozen`) if it differs. `FileProfileHook::apply_selected` refuses a requested subset without an arm map (`ProfileArmMapMissing`) instead of widening to every arm.
- A pilot's reservation is now three runs of every authorized tool (worst case), so admission never assumes attempts the pilot will consume; `estimated_active_seconds`/`deferred_for_time` state which studies cannot fit `max_active_seconds` (eight >=20-minute long attempts per action are at least 160 minutes), with the runtime clock still the enforcement.
- Restoration after exited threads: the live rule allows a thread that exited; the receipt now records them (`exited`, original threads absent from the complete post-restore listing) and the verifier accepts a missing thread only when it is named, absent from the listing, and not the leader; an unnamed absence is unknown, a named exit still listed is a contradiction.

## Sixteenth audit P3: reads, oracle, historical seal

- `newest_record::newest` scans a private directory through a descriptor, no-follow and bounded, and reports a present record that is a link, oversized, unreadable or not valid as `Invalid` (never skipped for an older pass). The fault matrix (`FaultMatrixInvalid`) and the oracle cases (`OracleCasesInvalid`) use it.
- The oracle's G3 rule accepts any two distinct preregistered non-GPU causal controls (not a fixed RunnableDelay + IoPressure pair), still with a clean GPU-bound negative and no unsafe action; the report folds that gate result and the resident observation into G3's mandatory input (`OracleGateNotPassed`, `NoOracleCases`).
- `protected_state::seal_unchanged` writes, once, that the player's saves were unchanged when the campaign was first evaluated after preparation (and never seals a changed state); `sealed` is the historical input, so the historical verdict no longer substitutes a fake `Ok` (`historical_notes` still names a campaign that was never sealed).
- The duplicate `note` key in `action-transactions.json` is merged.
- Still open: crosswalk rows have no per-row campaign evidence yet (a provider receipt per dossier does not exist); G7's FeatureNotObservable exclusion is not narrowed (no per-factor optional list); decisions and worker transactions are not independently reconstructed from raw records.

## Sixteenth audit: G7 narrowed

`FeatureNotObservable` is no longer an exclusion (this supersedes the earlier G7 note and the `QualifiedWithUnobservedFeatures` scope). Only `BlockedNoEligibleSource` (no authorized build can switch the factor) resolves a feature study without qualification; a feature whose control exists and reached the game but whose function was not established stays unresolved, so a full G7 needs a real functional proof, which nothing produces offline.

## Seventeenth audit: incremental Steam settlement

`SteamPort::stop_step` (`Settled` / `Pending` / `Unsettled`) replaces the blocking loop in the polled attempt path. `ProductionSteamPort` starts its settlement window at the first step, runs one cleanup-and-witness pass per call and returns at once. `SteamAttemptHost` decides the attempt's outcome first, keeps it in `settling` while the tree is not yet proven gone (the attempt stays running, ownership is kept, the port's state is not read again) and delivers it only once settled; a window that closes unproven fails the attempt as `SettlementUnverified` (unresolved ownership). `stop()` (abort) still blocks. The inline tests of `steam_host.rs` moved to `steam_host_tests.rs` for the size cap.

## Seventeenth audit: evidence contracts tightened

- Protected-state seal: written only by the durable preterminal finalization step (`research_control_finalize`), bound by digest into `finalization-prepared.json` (`protected_state_seal_sha256`) and re-checked at terminal verification; ordinary report verification is read-only (a guard test reads the sources). A campaign never sealed stays historically unsealed.
- `action_latency` (schema 2) carries sealed per-window runnable histograms, not stated percentiles: the 99th percentile is recomputed (nearest rank), each window's source is judged with the controller's own `judge_runnable` (lossless, same target, >=1000 events), windows must lie inside their attempt, the foreground p99 guardrail (5%) is recomputed, every planned pair (both attempts completed) is required and none else, and the frozen rule gives Keep / Revert / Null (median relative change at least 5%, most pairs improved for Keep). Tested with the real eight-attempt AB/BA schedule and a dropped fourth pair.
- `resident_observation` (schema 2): per attempt the window inside the attempt, renderer, cgroup, CPU/GPU/PSI samples (none lost), owners; the 1000-event runnable floor (lossless) applies only to an attempt with `runnable_claim`. A quiet benchmark passes.
- G3: an ineligible or corrupt optional frame source only rejects the frame-primary claim (`FramePrimaryClaimRejected` note); the mandatory resident observation and oracle controls are what block.
- `newest_record`: two different records at the newest instant fail closed; a strictly newer one settles it. `deploy_receipt::read_latest` is anchored and strict.
- Oracle: the truth commitment (`oracle-truth-*.json`) and the controller capture (`oracle-capture-*.json`) are separate files; the capture names the SHA-256 of the commitment's bytes and both name the campaign; edited truths, another campaign, a capture dated before the commitment or a corrupt file are each refused. The controller mapping cannot emit ColdShaderCache/OwnerConflict (tested), so those positives can be scored but never produced live.

## Seventeenth audit: admission, crosswalk observations, action campaign

- Active-time admission: the funnel tick decides (as `Blocked / DeferredActiveTime`, never started) any study whose least possible time (`min_attempts` x 60 s for a short attempt, x the work unit's length for a long one) already exceeds the active seconds left (authorization less recorded child spend). The attempt-limit ceiling (`estimated_active_seconds`) is reporting only and never refuses work.
- Crosswalk: `build_rows_observed` is fed from the campaign's own `crosswalk-observations.json` (the report response carries it; a malformed record, unknown id, bad digest or repeated id makes the whole record unusable). Nothing writes that file yet, so all 34 rows still read `NotObservedInThisCampaign`.
- New production-composed synthetic campaign with an authorized action (`baseline_campaign_action_tests.rs`): screens, pilots, confirmations and holdouts over real native files, then the action study over the long-session host with a real `ActionGate`. The controller never applies the action, so the study is `Unqualified / ActionNotIndicated`; `verify_children` replays every child including the abstention, `actions_restored` accepts it, and removing the abstention record fails replay. The long child's recorded active seconds are set to the witnessed twenty minutes per attempt because the synthetic clock ticks one second per read.
- Not covered: a qualified action (its `action-latency.json` and `resident-observation.json` must exist before the study ends, which only the physical producers can arrange) and a feature study through `verify_children` in this campaign.

## Seventeenth audit: six-arm campaign with the real hook

`baseline_campaign_hook_tests.rs` runs the production-composed synthetic campaign over six parent arms with the real `FileProfileHook` editing six real `UserSettings.json` files (the synthetic native host seals the digest of the file its arm actually has). The screens and pilots run over all six arms, confirmations, holdouts and the matched CachyOS study over two; each study records one digest per arm it ran; every file is restored byte for byte after each study; the independent verification binds each result to its own arm's digest, and scrambling one digest is found. The profile spec is the index-free ray-tracing profile; the production GPU profile still needs observed list mappings, so it is not exercised here.

## Seventeenth audit: worker-transaction timeline in the action receipt

The gate now records, per proven restoration, when the action was permitted, when the manager first showed it held, when its restoration was accepted, and the most root-worker tokens seen outstanding while it was applied (`permitted_unix`, `applied_unix`, `settled_unix`, `peak_tokens`; the clock is injectable). `receipts_valid` requires them (present, in order, `peak_tokens >= 1`: the worker actually committed something) and `action_replay` requires every receipt's whole timeline to lie inside the very attempt it is attributed to. These are still the live gate's observations carried in a receipt; they are not independently rebuilt from the root worker's own log, which nothing here can read.

## Seventeenth audit: producer collectors

`evidence_producers` holds the producer half of the sealed records: `ResidentCollector` (counts of CPU/GPU/PSI samples, lost samples, runnable events, cgroup, renderer, owners; `finish` yields the attempt's entry) and `LatencyWindowBuilder` (raw runnable-delay events binned into the histogram, lost events, target change; `finish` yields the window), plus the document builders. The tests feed them and check the output with `resident_observation::verify` and `action_latency::verify`, so producer and verifier cannot drift apart. They hold only what they are given; feeding them from the live game through the runtime's event path is the part that needs the game and is not wired.

## G5 objective unification (eighteenth audit)

The action study's measurement no longer has a decision rule of its own. `autotune/objective/runnable_replay.rs`
builds, from a sealed window (the controller's canonical `LatencyHistogram` buckets, its maximum, the foreground
frame tail and over-5-ms count, thermal and power flags), the very `ObjectiveSignals` the live controller builds
(`set_runnable_latency_from_histogram` shares its validation and percentile code with the interval-record path) and
decides each off/on pair with `compare_for_objective` for `GameRunnableLatency`: 12.5% improvement, 7.5%
regression, 1000-sample floor, worsening >5 ms tail rejection, frame (+1 ms), foreground (+2 ms and over-5-ms share),
thermal and power guards. The only addition is resolution honesty: an improvement is reported only when the
candidate's bucket bound already clears the frozen improvement from the bottom of the baseline's bucket; otherwise the
pair is inconclusive ("resolution cannot show the claimed improvement"). A differential test checks the offline
signal field for field against the controller's interval-record signal on identical populations.

`research/action_latency.rs` (schema 3) combines the per-pair production decisions over every planned pair: Keep when
most pairs improved and none regressed; Revert when most regressed; otherwise Null. A broken guard is a regressed
pair, not a refused record. `LatencyWindowBuilder` counts into the canonical histogram from nanosecond events (no
float, so no NaN or negative delay is representable) and carries the guard facts.

Also: the live action qualification now uses the replay's own coverage function (a receipt on every attempt treated
On, each attributed to its own process), so a partly applied action can no longer be live-qualified and then
contradicted at verification; a qualified long action study with a collector-built latency record is proven through
`verify_qualification`; the daemon restore dry-run test uses its own state instead of the live controller's journal.

## Crosswalk observations are reopened evidence (eighteenth audit)

`research/crosswalk_evidence.rs`: an observation names a dossier and the SHA-256 of an immutable artifact beside the
study (`crosswalk-evidence-<sha>.json`). The report reopens each one (no-follow, private, bounded), requires it to
hash to the digest, to be bound to this campaign and this dossier, to name a provider and a direct or derived source
quality, and to carry a real interval. A malformed, repeated, unknown, missing, altered or foreign record is the
distinct integrity failure `CrosswalkObservationsInvalid` in the bundle, never an empty observation map.

Finalization: the protected-state seal result is kept, an existing seal is accepted only when well formed, a qualified
campaign without a valid seal is blocked rather than completed, and terminal verification requires a valid seal for
Completed.

## Time admission, budget clock and production profiles (eighteenth audit)

- `time_facts()` returns the active seconds left plus the least and the most one short and one long attempt can take,
  or `BudgetEvidenceUnreadable` (the funnel halts on it) instead of planning against a guess of zero spend. A child
  whose own spend record is unreadable is still charged the whole authorization.
- `child_study::time_feasibility` answers `Impossible` (even the least attempts cannot fit), `Guaranteed` (even the
  slowest fit) or `Uncertain`; only `Impossible` defers a study (`DeferredActiveTime`). A pilot's least is one
  surviving arm (three attempts), not the six that two arms need.
- The synthetic action campaign runs on one clock shared with the route, so the production accounting records the
  twenty-minute attempts itself; budget tampering is a separate negative test.
- The real `cpu_profile`/`gpu_profile` run through the file hook over realistic per-arm settings: resolution, upscaler
  and frame generation resolved against observed indices, unobserved values and partial failure refuse the whole
  study with every arm's bytes untouched, restoration is byte for byte. Effective rendered resolution stays unproven.

## Justified abstention rests on the planner's observation (eighteenth audit)

`research/planner_observation.rs`: `ActionNotIndicated` is a claim about the planner, so it needs
`planner-observation.json`: for every completed attempt, the resident planner's own evaluations of that game process
over a window inside the attempt (at least ten, direct source, nothing lost) and no candidate for the authorized
action. No receipts alone is no longer an abstention (a controller that never ran leaves none either): live outcome
and replay both require the observation, and the synthetic campaign now seals it from the route's own history.

The production-composed campaign also runs a qualified action (applied on every treated attempt, collector-sealed
latency measurement) through `verify_children`, a worse action (valid study, Revert decision), and tampers (measurement
removed, restoration altered, receipt dropped) that verification refuses.

## Controller journal corroborates every receipt (eighteenth audit)

`research/controller_provenance.rs`: the gate's receipts are the gate's own account. The campaign also keeps an archive
of the controller's own journal records (`controller-journal.json`, the production `ControllerJournalRecord` type, one
final record per transaction), and the replay joins every receipt to its record: the same transaction, the production
action id (`cpu-affinity-profile:` or `cgroup:`), the very pid and start time, state `reverted` with
`rollback_verified`, a rollback token, no fault, and timestamps inside the attempt the receipt is attributed to. A
receipt without a corroborating record, or a record no receipt accounts for, refuses the study (live and replay use
the one function). The archive is produced by the resident runtime from the controller's journal during a physical run.

## The resident observer is fed by the runtime's own events (eighteenth audit)

`research/resident_observer.rs` is a shared observer in the telemetry observer's pattern: the funnel arms it on the
attested game process of each attempt (`DriverStudyAttempts::arm_resident`), and the existing runtime event path feeds
it (`AutotuneRuntime::on_event`: interval records and the events their sources dropped; GPU samples). It counts only
what concerns that exact pid, merges the target game classes' histograms in the controller's canonical layout (an
interval that is not a valid canonical population is counted as unusable, never merged; another game process in the
target's intervals marks the window not one population), and at study end `record_resident` keeps one raw window per
attempt beside the study as `resident-windows.json` (write-once, acknowledged only after it is durable). No second
sensor path, optimizer or mutation channel is created.

Typed gaps still needed before these windows become the sealed `resident-observation.json` and `action-latency.json`:
the renderer binding from the frame-source receipt, the target's cgroup, owner attribution, and the foreground
frame-tail, over-5-ms and thermal/power guard facts of each window; `resident_entry` and
`LatencyWindowBuilder::from_population` derive the records once those are fed.

## Per-factor backend proof (eighteenth audit, G7)

`research/feature_proof.rs`: activation (the treatment reached the game's environment) and the game's own reported
selection do not say the backend that implements a factor was in use. For the factors whose backend is modelled the
runtime now reads the kernel's own view of the game process during each feature attempt (the files its address space
maps, the devices it holds open, its ancestor processes; merged over the attempt, read every few seconds, because a
backend loads after the process appears) and keeps one witness per attempt as `feature-proof.json`. The proof holds
only when the backend shows on every attempt treated On and on none treated Off, from witnesses of the very processes
the attempts recorded: `WineWayland` (`winewayland.drv` mapped), `NtSync` (`/dev/ntsync` held open), `Fsr4`
(`amdxcffx64.dll` mapped), `Gamescope` (a `gamescope` ancestor). Every other factor stays `FeatureBackendNotModelled`
and unproven: no backend is guessed. A feature's stage evidence is now activation AND this proof, by the one function
the live outcome and the replay share. The names were checked against the builds installed on the host (see `feature_proof::backend`); the others are deliberately
must be confirmed against a real run before a feature result is trusted; a wrong name fails closed.

## The sealed resident observation is derived (eighteenth audit)

At study end `seal_resident_observation` derives `resident-observation.json` from the raw windows and the facts of each
attempt: the cgroup the kernel showed for the process when its window opened (`/proc/<pid>/cgroup`), the renderer
binding the attempt's own telemetry receipt shows (bound to this pid and start time, healthy, at least one snapshot),
owners attributed by the game's own interval records, and the lost counts. The interval is clamped to the part of the
window inside the attempt. It is written only when the real `resident_observation::verify` accepts it for every
completed attempt; an unbound renderer, a missing or sparse window leaves no record (and a campaign without it has no
G3). Still not derived: `action-latency.json`, which needs each window's foreground frame-tail, over-5-ms and
thermal/power guard facts from the controller's window score.

## The latency record is derived from the runtime's own events (eighteenth audit, G5)

The resident observer now holds, per attempt, the controller's own `RollingWindow` fed the same events the controller's
window gets (interval records, frame events, GPU samples, CPU frequency events). At the end of the attempt the guard
facts (foreground frame tail and frame count, over-5-ms count, thermal and power flags) are read from the production
`compute_objective_signals`, not redefined, and kept with the raw window (`None` when any signal the guards need was
never observed). At study end `seal_action_latency` builds each planned pair's off and on population in the
canonical layout (the part inside the attempt, with dropped events and any target change), and writes
`action-latency.json` only when the real `action_latency::verify` accepts every planned pair; the production
`compare_for_objective` then decides each pair. A missing guard fact (for example no CPU frequency events) leaves no
record and the action has no measurement. A whole-second window that opened and closed inside one second still spans
that second.

## Non-blocking attempt stop (eighteenth audit)

A timeout, active-budget stop, campaign deadline, failure or cancellation of a running attempt no longer calls the
host's blocking `abort` (for Steam up to the 120 s settlement window) from inside the tick. `AttemptHost::abort_step`
asks the host one bounded step (`Settled`, `Pending`, `Unsettled`; the default is the blocking abort for hosts with
nothing to wait for); `SteamAttemptHost` answers with the port's own `stop_step` and `LongAttemptHost` with its inner
host's. While settlement is pending the driver keeps the attempt active (its time still accrues), reports it running,
starts no further launch, and seals the outcome it began with only once the witness proves the tree gone; an unproven
settlement is an error, the stop stays owed, and nothing is ever sealed over a possibly running game. An explicit
shutdown or cancellation (`tick_cancelled_to_settlement`) ticks through the window (bounded, 150 s) so it still returns
proven or unproven. Still blocking by design: the launch-refusal abort (nothing started), the coordinator's error-path
`abort_host`, and the flat driver's own shutdown.

## Effective rendered resolution (eighteenth audit)

The native summary does report the resolution the game rendered at (`renderWidth`, `renderHeight`) and whether dynamic
resolution scaling was on (`DRSEnabled`); an earlier note that it did not was wrong. `NativeContext` now reads them (bounded;
older sealed contexts replay without them). `target_resolution(profile)` reads the resolution a study profile asks for from
the very edit that sets it (CPU 1920x1080, GPU 3440x1440). `effective_resolution(profile)` is `Confirmed` (the reported
resolution is the profile's and dynamic scaling was off), `Contradicted` (another resolution, or dynamic scaling on, which
changes the internal resolution whatever the output says), `Unproven` (the result does not report it) or `NotSpecified`
(the long profile). A contradiction is a profile violation (`ProfileConformance::Violates`); a silent result is typed
`Unproven` and is never assumed to match.

Which factors have no modelled backend, and why (checked against the installed Proton builds): `AudioBackend` and
`InputBackend` act in Wine's services processes (`winepulse`/`winealsa`/`winebus`), not the game's; `Vkd3dLowLatency` is a
Vulkan implicit layer (GE ships `libVkLayer_KORTHOS_LowLatency.so`, the CachyOS tool directory here is only a stub);
`AntiLag2`, `Hdr` and `FrameGeneration` have no environment control or file witness to prove from outside the process.
A real run decides whether the modelled names are mapped; a wrong name fails closed.

## Effective graphics in the bundle, and the isolated-seat gate (eighteenth audit)

`research/effective_graphics.rs`: every verified child carries `effective_graphics` (completed attempts whose results
confirmed the profile's rendered resolution with dynamic scaling off, contradicted it, did not report it, or are of a
profile that asks for none), and `capability-matrix.json` carries the campaign totals with
`all_resolutions_confirmed`, which is false while any attempt is unproven, so the bundle never implies a check that was
not made.

`research/isolated_seat.rs`: the gate any future route-input device sink must pass. `attest` turns the game process's
own environment and the user's live session into an `IsolatedSeat` only when the game runs on a Wayland display, X
display and session of its own and not on the live Hyprland instance (absent or shared is refused); `IsolatedSink` can
only be built from an attested seat and a sink that names the very display it is bound to, so a sink bound to the live
display is refused before a single event is written. No physical sink exists in this build (it keeps
`UnavailableInput`); the gate and its release-every-key behaviour are exercised with fixtures that touch no device.

## Two more backend models, and the evidence for the four that stay unmodelled (eighteenth audit)

Attempted for every remaining factor against the tools installed on the research host:

- `AudioBackend` (CachyOS, `PROTON_USE_PIPEWIRE=0`): the tool's `proton` script drops the default `pipewire` configuration
  and sets `WINE_AUDIO_DRIVER=pulse,alsa`; the tool ships both `winepulse` and `winepipewire`. Modelled: the pulse driver
  (`winepulse.so` or `.drv`) is mapped on every On attempt and on no Off attempt.
- `Vkd3dLowLatency` (CachyOS): the tool ships two different `d3d12core.dll` builds (`vkd3d-low-latency` 78e63ad2…,
  `vkd3d-proton` b73d1c10…) and copies one into the prefix, under one file name. Modelled by content: the witness hashes
  the mapped `d3d12core.dll`, the host hashes the tool directory's two builds (`AttemptHost::tool_file_sha256`, relative
  paths only), and the proof needs the On attempts to map the low-latency build and the Off attempts the standard one, the
  two builds known, distinct and the same on every attempt. The tool hashes are the host's own reading of the tool
  directory at attempt time; they are not recomputable at replay without the directory.
- `InputBackend` (EM, `PROTON_USE_SDL=1`): the script only sets `PROTON_DISABLE_HIDRAW` and `PROTON_NO_STEAMINPUT`, read by
  `winebus` in a services process, and both input buses coexist, so the baseline cannot be shown free of SDL: a game
  environment echo would be activation, not backend. Not modelled.
- `FrameGeneration` (EM, `MLFG_UPGRADE=1`): the string appears nowhere in the EM tool: no artifact to witness. The game's
  own reported `frameGenerationType` stays the only evidence (`functional_probe`). Not modelled.
- `AntiLag2` and `Hdr`: no source can switch them (`factor_env` reports no environment control, or that no qualified
  output exists), so they are legitimately excluded rather than unproven.

## CP-LONG input: the offline portion is built (eighteenth audit)

A key-injecting device must live inside the isolated seat's own session, so this build never opens one; everything on
this side of that device is now real and tested without a device:

- `scripted_input` (existing): the route script, timing, encoding and the guarantee no key is left held.
- `isolated_seat`: the attestation that the game runs on a display and session of its own, and `IsolatedSink`, which refuses
  any sink not bound to that display.
- `seat_socket_sink`: the transport to a helper inside the isolated session. It connects only to a Unix socket that is a
  real socket (not a link), owned by the expected user, writable by no one else, whose kernel-recorded peer (`SO_PEERCRED`)
  is that user; it names the display the helper's device is bound to; it writes whole events with a bounded timeout and
  reports a failed write (so the route fails instead of passing as clean). `decode` is the helper's side of the stream:
  whole records, key presses and releases of the keys a route may use, each followed by its synchronization report, and
  nothing else, so a helper that decodes with it can inject nothing a route could not have sent.

What remains is not offline work: the helper process that owns the virtual keyboard inside a real isolated session
(a nested compositor or equivalent) and the scene/save identity of that session. Both need a session of their own on the
physical machine, which the GPU incident halt forbids starting.

## The worker's own audit corroborates every receipt; a feature qualifies through the campaign (eighteenth audit)

`research/worker_provenance.rs`: the privileged worker writes its own boundary decisions to its audit log (`daemon_privilege`
events: stage, verdict, operation, action id, timestamp from its own clock). The campaign keeps an archive of those events
for the study (the production `AuditEvent` type, `worker-audit.json`), and the replay joins it to every receipt: inside
that receipt's window the worker allowed exactly one successful, non-dry-run apply of the authorized action's id that
touched at least one task, and exactly one rollback after it; and no apply of that action was allowed during the study's
attempts that no receipt accounts for. With the gate's receipts and the controller's journal this makes three
independent accounts of one action. The worker's own token ledger cannot serve (it compacts settled tokens), so the audit
log is the durable record. The archive is copied by the resident runtime during a physical run.

The production-composed campaign now also runs a Feature child (ntsync, backend witnessed by the kernel on every attempt
treated On) to a qualified study through `verify_children`, and a removed proof contradicts the claimed qualification.

## PLAN.md re-check items closed (eighteenth audit)

- D7 / CP-GAMESCOPE "no silent llvmpipe": a result whose reported renderer is a software rasteriser (`llvmpipe`, `lavapipe`,
  `softpipe`, `swrast`) violates every profile, whatever else it shows.
- CP-FSR-GE "requested DLL actually loaded (hash/version)": a library backend (`amdxcffx64.dll`) is hashed in the kernel's
  witness of each attempt, so the build actually loaded is on record beside the proof of its presence.

## The resident runtime now produces the provenance and observation records (eighteenth audit)

Built, so no verifier waits on a hand-written file:

- `controller-journal.json`: a read-only tap on the controller journal's durable writes (`set_journal_tap`; it cannot change
  or veto a write) keeps the last record per transaction as it is written (`evidence_archive::JournalArchive`), because the
  journal itself holds only its latest record. At study end the records for the study's game processes are archived beside
  the action study.
- `worker-audit.json`: the worker's append-only audit file is read line by line (a torn line is skipped, never repaired) and
  only its own `daemon_privilege` events for the authorized action's id inside the study's attempts are archived. The audit
  path is `audit::default_audit_log_path()`; if the root worker is configured to write elsewhere, that path is what a
  deployment must hand `with_provenance`.
- `planner-observation.json`: the observer counts, from the decision stream the runtime already emits, how often the planner
  evaluated the attested process and which authorized actions it offered; the abstention record is derived from it and
  written only when the real verifier accepts it.
- `crosswalk-observations.json` and `crosswalk-evidence-<sha>.json`: at finalization, from the children's sealed raw windows,
  the dossiers whose signal the runtime itself feeds (5.22 PSI, 5.13 GPU samples, 5.17 CPU frequency events) and every attempt
  of every child really received at or above the sample floor, each with an immutable content-named artifact the report
  reopens. No dossier is upgraded from a plan.
- `resident-windows.json` (raw), `resident-observation.json`, `action-latency.json`, `feature-proof.json`: as before.

Still outside any offline build: that these are fed by a running controller on the real host (the tap is installed by the
funnel builder; the audit path of a root worker is configured by the deployment).

## Worker audit path found, and the isolated-session helper's logic (eighteenth audit)

- The root worker unit runs with `HOME=/var/lib/stutter/worker-<user>` (state directory mode 0700, root), so its audit log is
  `/var/lib/stutter/worker-<user>/.local/state/stutter/audit/actions.jsonl`, not under the controller user's home, and the
  controller user cannot read it (`evidence_archive::worker_audit_path` derives it from the worker socket directory name
  `/run/stutter-worker-<user>/`). When it cannot be read the study records `worker-audit-unavailable.json` with the reason
  and the action has no independent corroboration, so it does not qualify: a stated absence, never a silent one. Making that
  one file readable to the controller (or another root-owned, controller-readable copy) is a deployment and trust decision
  for the operator; no new request was added to the root worker's IPC.
- `scripted_input::seat_helper`: the logic of the process inside the isolated session. Built only from an attested
  `IsolatedSeat`; reads the stream record by record and injects only what `decode` accepts through an `Injector` trait; tracks
  every key it holds and, on a clean end, a torn record, a refused event or a failed injection, releases every one and reports
  whether every release was accepted. The `Injector` that registers a virtual keyboard is the one part not in this build.
