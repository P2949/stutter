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

`research/save_identity.rs`: `save_identity_sha256(dir)` hashes a game save directory (file names, sizes and contents in
name order; refuses symlinks, nested directories, an empty or oversized save) and `session_definition_sha256` binds the
route, its script and that save into one definition digest, so two long runs are comparable only from the same save.
Offline-tested on temporary directories. The operator still has to choose the save and the in-game conditions and
authorize the resulting `LongWorkUnit`; the module is staged (dead in the production build) until then.
