# W32 capability and evidence review (prose), 2026-11-11

Source of truth: `stutter capabilities` (the registry; 46 capabilities) read against `docs/integration-coverage.json`.

**Supported (12):** `doctor`, `record`, `bench`, `report`, `audit`, `completions_and_man` (observation and reporting, backed by
indexed real-system observation, kernel and overhead evidence), and the autonomous path: `cpu_affinity_experiment`,
`cgroup_placement_experiment`, `bounded_autotune_controller`, `always_on_autotune`. The autonomous path is Supported **only** as
the packaged bounded systemd service with exactly the two `AUTONOMOUS_CONTRACTS` entries (CPU-affinity profile and cgroup
placement for a stable game under scheduler pressure), qualified by targeted acceptance and, for the current build, an exact-build
installed smoke (37 checks, 2026-11-11). Nothing done on 2026-11-11 widened this.

**Experimental (30):** every other mutating mechanism (uclamp, nice, ionice, EPP/governor, GPU profile/DPM, vm sysctls, IRQ
affinity, PM QoS, resctrl, DAMOS, all cgroup writes other than placement, block scheduler, sched_ext strategy, NUMA migration,
TCP/net knobs) plus diagnose, advisor, tune evaluation, restore and the remote agent. They stay manual or offline: the worker has no
HighRisk path (`HIGH_RISK_APPLY_IMPLEMENTED=false`). **Internal (2):** the privileged worker and the synthetic action provider.

**Why these stay below Supported, per class** (the "why not tuned" chain a user can read from the evidence):
* no direct outcome measurement for the exact tuple: the W24 compile study shows a real benefit/harm classification method, but
  for a synthetic contention scenario and for an action (renicing foreign tasks) that the worker does not perform, so it does
  not graduate any tuple;
* no independent frame-stamp reference (W22 hardware), so direct-frame keep is denied for every game objective;
* an owner or hardware precondition absent on this host (single NUMA node, no RDT, kernel-owned kdamond, `io`/`cpuset` not
  delegated to the user manager, no disposable block device), each stated in its ledger `unblock`;
* earlier negative host pilots (cpu weight/idle, EPP) are preserved and not repeated.

**Evidence honesty.** 34 dossiers covered; dispositions are CodeComplete 34, FixtureVerified 34, RealObserved 21,
MechanismValidated 4, NotPlannedForVerification 11, EvidenceDeferred 31, AutonomySupported 0; the closeout report fails
if the ledger claims more than its evidence. Limits of the 2026-11-11 runs are stated beside each result: the W21 layer handles
one instance/device per process; W24/W25 are one scenario on one host; the W28 live run shows no interference but not a live
owner-conflict refusal; W26 is five 60 s intervals with no per-sensor attribution.
