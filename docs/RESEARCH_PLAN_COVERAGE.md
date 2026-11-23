# PLAN.md coverage of the research campaign (state of the eighteenth-audit round)

What is built and tested without the game, per PLAN.md package, and the exact reason anything is not. "Physical" means it
needs the real Cyberpunk process, the real isolated session or the real root worker, none of which may be started while the
GPU incident halt is in force (`stutter research status` reports `UnsafeToResume`). Nothing here is evidence that a Cyberpunk
result exists. Module paths are under `stutter/src/autotune/research/`.

| Package | Built and tested offline | Physical or human remainder |
| --- | --- | --- |
| P0 host qualification | `host_hardware`, `host_baseline`, `protected_state` (baseline, preterminal seal), `entitlement`, `game_facts`, G0 inventory | attestation of the live host at campaign start |
| P1 contract, lease, IPC | `lease`, `lease_inspect`, `journal`, `control_socket`, `protocol`, `deploy_receipt` (strict newest-record reads), runtime `research_control*` | the deployed controller is still e5ccd5b6 (deploy is the operator's) |
| P2 native benchmark, CP-LONG | `cyberpunk`, `native_context` (resolution, dynamic scaling, software renderer), `native_probe`, `long_session`, `long_receipt`, `route_driver`, `scripted_input`, `isolated_seat`, `seat_socket_sink`, `seat_helper`, `uinput_injector` | the isolated session itself, the virtual keyboard registered in it, and its scene/save identity |
| P3 six Proton builds, Steam | `steam_host`, `steam_port`, `steam_port_launch`, `steam_inventory`, `dispatcher`, `profile_hook`, `profile_applier` (production CPU/GPU profiles), non-blocking `abort_step` | installed-tool qualification and real runs |
| P4 resident observations | `resident_observer` (fed by the runtime's interval, GPU, frame, CPU-frequency and decision events), `telemetry_observer`, `resident_observation`, `evidence_producers` | a running controller feeding them (the live service is older) |
| P5 scheduler, statistics | `scheduler`, `child_study`, `child_runner`, `pilot_rule`, `analysis`, `metric_plan`, `campaign_budget`, time admission (`time_feasibility`) | calibration of provisional thresholds on real data |
| P6 diagnosis oracle | `oracle`, `oracle_cases` (commitment and capture bound by digest) | real causal captures, a CPU-contention positive and a GPU negative |
| P7 supported action | `action_gate`, `action_child`, `action_study`, `action_latency` (production objective decides), `controller_provenance`, `worker_provenance`, `planner_observation`, `evidence_archive` | a real apply and restore; the worker's audit file is root-private (below) |
| P8 features | `features`, `functional_probe`, `feature_proof` (WineWayland, NtSync, Fsr4, Gamescope, AudioBackend, Vkd3dLowLatency); AntiLag2 and Hdr unswitchable; InputBackend and FrameGeneration have no witness | whether the game maps the named modules at run time |
| P9 reporting | `report`, `final_bundle`, `crosswalk`, `crosswalk_evidence`, `crosswalk_producer`, `effective_graphics`, `verdict`, `gate_verdict`, `child_verify` | none beyond a real campaign |
| P10 NixOS | pinned and validated on the research branch every milestone | deploy/switch and `research.childFunnel` are the operator's |
| P11 campaign | the production-composed synthetic campaign (screen, pilot, confirm, holdout, init, action qualified/worse/abstained/tampered, feature qualified/tampered) through `verify_children` | the real unattended Cyberpunk campaign |

Audit-18 findings: 1 (G5 uses the production objective), 2 (canonical histogram, resolution honesty), 3 (producers, qualified
action and feature through `verify_children`, worker and controller joins), 4 (seal required for Completed), 5 (crosswalk
evidence reopened and produced), 6 (resolution proof, software renderer), 7 (feature backends), 8 (time admission),
9 (shared clock), 10 (non-blocking abort), 11 (production profiles), 12 (controller journal, worker audit, planner
observation) are implemented and tested. The remainders are the physical items above.

Two facts that decide what can still be done without the operator:

* The root worker's audit log is under its own home, `/var/lib/stutter/worker-<user>/.local/state/stutter/audit/actions.jsonl`
  (mode 0700, root). The controller user cannot read it, so a real action will not qualify until the operator makes that
  file (or a root-owned copy) controller-readable. No worker IPC was widened.
* `/dev/uinput` is accessible to this user, and a uinput device is kernel-wide: creating one here would register a
  keyboard on the live seat. `uinput_injector` therefore writes nothing unless the device's own udev record puts it on an
  isolated seat other than `seat0`; its one kernel-touching function is never called by this build.

## Production wiring check (2026-11-23)

`runtime/research_funnel_production_tests.rs` calls the resident session's real `build` (not a test builder) with a
fake game host and proves the controller-side wiring a live run depends on: the shared resident observer and
telemetry exist, and the controller-journal tap installed by `build` delivers a durably written journal record to the
archive each action study is sealed with. What stays physical: a running controller process built from this commit
producing these records during a real game (the deployed controller predates them).

### Event-driven window production and ioctl cross-check (2026-11-23)

- `real_monitor_events_through_on_event_build_the_attempts_resident_window` (runtime/tests/planning.rs) drives
  `MonitorEvent::Interval`, `GpuSample`, `Frame` and `CpuFreqSample` through `AutotuneRuntime::on_event` into an armed
  resident observer and checks the resulting window (150 game events, 3 CPU/GPU/frequency samples, nothing invalid).
- The five uinput request numbers and `sizeof(struct uinput_setup)` (92) in `syscall.rs` were compared with a C program
  compiled against the kernel's own `linux/uinput.h` (6.18.7 headers): all equal (0x5501, 0x5502, 0x405c5503,
  0x40045564, 0x40045565, 92). No device was opened. What is still unexercised is only the live call sequence.

### Scene/save identity (2026-11-23)

`research/save_identity.rs` (child of `route_driver`): `save_identity_sha256(dir)` hashes a game save directory (file names, sizes and contents in
name order; refuses symlinks, nested directories, an empty or oversized save) and `session_definition_sha256` binds the
route, its script and that save into one definition digest, so two long runs are comparable only from the same save.
Offline-tested on temporary directories. The operator still has to choose the save and the in-game conditions and
authorize the resulting `LongWorkUnit`; the module is staged (dead in the production build) until then.

## Eighteenth audit, finding by finding (2026-11-23)

Each row names a test that was run by its exact name and passed. `dattempts` is `autotune::research::driver_attempts::tests`.

| # | Finding | Test |
|---|---|---|
| 1 | G5 uses the production objective | `objective::runnable_replay_tests` (`replay_pair` over the production comparison) and `action_latency` tests |
| 2 | Canonical histogram, bucket-resolution uncertainty | `action_latency` and `resident_observer` tests; `replay_pair` returns Inconclusive when the bucket resolution cannot show the improvement |
| 3 | Offline plumbing before another live run | `runtime::tests::planning::real_monitor_events_through_on_event_build_the_attempts_resident_window`; the production-funnel wiring test |
| 4 | Seal required and validated | `protected_state_tests::a_seal_that_is_corrupt_or_unwritable_never_stands_in_for_the_unchanged_state`, `finalization_neither_discards_the_seal_result_nor_completes_without_a_valid_seal` |
| 5 | Crosswalk evidence reopens | `crosswalk_evidence::an_observation_stands_only_when_its_evidence_reopens_and_matches_the_campaign_and_dossier`, `a_malformed_or_repeated_record_is_an_integrity_failure_not_zero_observations` |
| 6 | Effective resolution | `native_context_resolution_tests::the_rendered_resolution_and_dynamic_scaling_are_read_and_judged_against_the_profile` |
| 7 | A feature can qualify on backend proof | `dattempts::feature::a_feature_study_qualifies_only_on_the_kernels_witness_of_its_backend`, `feature_proof::the_backend_must_be_present_on_every_on_attempt_and_absent_on_every_off_attempt`, `dattempts::baseline_action::qualified::a_feature_whose_backend_the_kernel_showed_on_every_treated_attempt_qualifies_through_the_whole_campaign` |
| 8 | Time admission | The three outcomes are `TimeFeasibility::{Impossible, Uncertain, Guaranteed}` (`child_study.rs`), exercised in `driver_attempts_frozen_tests.rs`; `dattempts::frozen::the_admission_facts_are_the_active_seconds_left_and_the_least_one_attempt_can_take`  Behaviour asserted by `driver_attempts::tests::frozen::unreadable_spend_is_not_zero_and_a_study_is_refused_only_when_it_is_certain_not_to_fit`: refused only when the least attempts cannot fit, Guaranteed when the slowest fit, Uncertain between, and unreadable spend halts |
| 9 | No post-hoc budget repair | `dattempts::baseline_action::an_action_the_controller_never_applied_is_a_justified_abstention_through_the_whole_campaign` (shared clock) and `a_budget_record_that_understates_the_witnessed_sessions_is_refused_at_verification` (negative) |
| 10 | Incremental stop | `driver_refusal_tests::a_refused_launch_is_sealed_only_once_its_stop_settles_across_ticks` and `long_host_tests::a_session_that_completes_is_finished_only_after_its_stop_settles_across_polls`. Still blocking by design: the launch-time stop in `LongAttemptHost::launch` and the stops in `driver_attempts.rs` (lines 542 and 553). Traced: they are the branches with no running attempt driver and run from the controller's drive tick (`research_control_drive.rs`); a launched game tree always has a driver and is stopped stepwise through `tick_cancelled_to_settlement`, and `ProductionSteamPort::stop` only keeps looping while its witness says something is unsettled, so with nothing launched it returns after one pass (`steam_port_tests::a_port_that_launched_nothing_stops_at_once_even_with_a_long_settle_window`) |
| 11 | Production graphics profiles | `profile_production_tests::the_production_profiles_set_each_arms_resolution_and_turn_the_upscaler_and_frame_generation_off` |
| 12 | Independent controller and worker provenance | `evidence_archive::the_tap_keeps_the_last_record_the_journal_durably_wrote_for_each_transaction_and_nothing_else`, `the_worker_audit_is_read_line_by_line_and_only_its_own_events_for_the_action_inside_the_study_are_kept`, `dattempts::long::qualified::a_justified_abstention_stands_on_the_planner_observation_the_runtime_derived_from_its_decisions` |

Flake explained and fixed: `a_real_apply_and_restore_on_a_stand_in_process_is_proven_against_procfs` failed 12 of 150 runs under 12 busy loops with "prepared affinity transaction has no effective changes": the runtime's /proc scan has a 50 ms budget (`DEFAULT_MAX_PROC_SCAN_MS`) and on a loaded host it ends before reaching the stand-in, so the apply correctly refuses. The test now retries a refused round (nothing was changed on it); 0 of 150 failed under the same load afterwards. Production behaviour is unchanged.
