# Always-on soak (M9)

`xtask soak` runs the real graduated operations for a wall-clock budget and checks, from what the
controller itself wrote, that nothing the plan forbids happened. It is the real counterpart of the
[deterministic fault simulation](AUTOTUNE_FAULT_SIMULATION.md).

## What runs

A resumable orchestrator starts, for each round of a fixed schedule, a disposable game-like
workload on a contended CPU, the production `stutter autotune --mode apply-medium-risk` controller
on it, and the production `stutter privileged-worker` as a separate process (same user, ambient
capabilities only, never root and never a password prompt). The administrator's game cgroup is a
cpuset on the quiet CPU inside a delegated cgroup subtree. The controller's state lives in an
isolated `HOME`, so the soak never touches the user's own autotune state. No validation override is
involved: every apply goes through the support registry exactly as in production.

Each round is a scenario (the target's CPU is contended so an experiment is kept, reverted for no
benefit, or reverted for harm), an operation set (CPU affinity, cgroup placement, or both) and one
event:

| Event | What is injected |
| --- | --- |
| plain | one experiment, then the workload exits |
| controller crash | `kill -9` of the controller mid-experiment; the next controller must restore it and must not retry the action at once |
| controller stop | SIGINT mid-experiment; the workload must be left restored |
| worker kill | `kill -9` of the privileged worker mid-experiment, restart |
| suspend | the machine suspends (`rtcwake`) mid-experiment |
| workload kill | `kill -9` of the workload mid-experiment |
| focus swap | one workload leaves, another becomes the focus |
| long | one workload for minutes: several experiments, cooldowns, conflicts, candidate memory |

The same operation is never started again inside the controller's own candidate-memory spacing
(five minutes); the soak waits instead of fighting the pacing.

## Acceptance

The full profile is a **coverage-qualified** real-system soak, not a duration. `xtask soak verify`
requires all of:

- at least 8 h of wall-clock time (the minimum dwell);
- at least 96 clean rounds and at least 6 complete clean 16-round schedule cycles (a cycle counts
  only when all 16 of its rounds are present and clean; a failed or interrupted round breaks it);
- at least 6 clean executions of each dangerous round kind: `controller_crash`, `controller_stop`,
  `worker_kill`, `suspend`, `workload_kill`, `focus_swap`, `long`;
- for each graduated operation (`cpu_affinity_profile`, `cgroup_placement`): at least 12 resolved
  experiments, 3 kept and 3 reverted;
- the earlier sanity minima: 20 resolved experiments, 10 workload/focus transitions, 2 controller
  restarts, 2 privileged-worker restarts, 2 suspend/resume cycles;
- the zero-violation list: no unrecovered token or stale kept action at rest, nothing applied
  outside the support matrix, no conflicting kept group, no cooldown or same-action spacing
  violation, and every injected fault followed by its audit and history evidence.

The configured duration is the **minimum dwell**. The run finishes when the dwell is over and the
coverage floor is met; if it is not met at the dwell it keeps running rounds, up to a cap of the
dwell plus two hours (10 h for the 480 minute default), and then finishes and `verify` fails. A
round that violates the list stops a full run early, once it is durably recorded: the
qualification is already impossible.

## Running it

```text
cargo build -p stutter -p xtask
target/debug/xtask soak init --dir .llm-runs/soak/<id> --minutes 480           # binds the binaries and commit
target/debug/xtask soak run --dir .llm-runs/soak/<id>                          # resumable; Ctrl-C pauses
target/debug/xtask soak verify --dir .llm-runs/soak/<id>
```

`--profile smoke --minutes 60 --allow-dirty` runs the same events on short windows to check the soak
itself. A soak is bound to the binaries it was initialised with; rebuilding invalidates it.

## Profiles and evidence

Verification is profile-aware. The **full** profile requires everything listed under Acceptance. The **smoke**
profile (a short, exact-source production-controller acceptance) requires both graduated
operations, a keep and a revert, at least one controller restart, one privileged-worker restart,
one suspend/resume cycle and one workload/focus transition. Both profiles apply exactly the same
zero-violation checks (`rounds_clean`, `final_state`, `support_matrix`, `conflicts_and_cooldowns`,
`faults_audited`); a short run never claims the full qualification.

`xtask soak controller-stop-stress --iterations 12` is a separate regression gate that `ci-live`
runs: it interrupts the production controller with SIGINT in the middle of an applied
cgroup-placement experiment, each time from fresh isolated state, and requires a normal exit, the
placement restored, a clean journal and an empty kept-action ledger.

A PASSED, finished soak started from a clean tree writes `<soak>/evidence-raw` (`environment.json`,
`result.json`, `verify.json`, `report.txt` and compact round, controller-history and audit/fault
summaries - never the isolated `HOME`). It is published with the canonical exporter:

```text
xtask evidence-export --raw <soak>/evidence-raw --out <bundle>
xtask evidence-verify <bundle>
xtask evidence-index <bundle>
```

The suites are `live_autotune_smoke` and `live_autotune_soak`; both cover exactly
`cpu_affinity_profile` and `cgroup_placement`. A soak started with `--allow-dirty`, or not yet
finished, produces no evidence, and verifying with a different xtask than `soak init` bound is
refused. The provenance (source commit, binary and harness digests, kernel, CPU) is the shared
`evidence_metadata` record that live acceptance also uses.
