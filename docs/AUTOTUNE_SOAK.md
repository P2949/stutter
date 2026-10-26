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

`xtask soak verify` checks the minimums (24 h of wall-clock time, 20 completed keep/revert
experiments, 10 workload/focus transitions, 2 controller restarts, 2 privileged-worker restarts,
2 suspend/resume cycles, both operations exercised, kept and reverted outcomes) and the
zero-violation list: no unrecovered token or stale kept action at rest, nothing applied outside the
support matrix, no conflicting kept group, no cooldown or same-action spacing violation, and every
injected fault followed by its audit and history evidence.

## Running it

```text
cargo build -p stutter -p xtask
target/debug/xtask soak init --dir .llm-runs/soak/<id> --minutes 1440          # binds the binaries and commit
target/debug/xtask soak run --dir .llm-runs/soak/<id>                          # resumable; Ctrl-C pauses
target/debug/xtask soak verify --dir .llm-runs/soak/<id>
```

`--profile smoke --minutes 60 --allow-dirty` runs the same events on short windows to check the soak
itself. A soak is bound to the binaries it was initialised with; rebuilding invalidates it.
