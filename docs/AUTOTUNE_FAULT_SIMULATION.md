# Deterministic fault simulation (M9)

`stutter/src/autotune/fault_simulation_tests/` drives the real autotune pieces against a simulated host
with virtual time. Nothing sleeps and nothing touches the machine, so every run is reproducible
from its seed.

What is real: `decide_autotune_transition`, the live-experiment manager (start, keep, revert, fault,
kept-action release), the controller journal, the kept-action ledger, startup recovery, candidate
memory persistence and the autonomous support gate. What is simulated: the host (one resource per
conflict group, rollback tokens that name a restore file) and the privileged worker boundary.

## Events

Workload start and stop, focus change, candidate offered by the planner (graduated tuples and one
that is not graduated), external writer, scripted faults (apply refused, worker dies after the
write, rollback fails, worker down), worker restart, controller death and restart, clean shutdown,
suspend/resume and reboot. Every scenario and the seeded random walk call the same `check` after
every event.

## Invariants

- No unrecovered token: every effect live on the host has a durable token (the journal or the
  kept-action ledger), at every point, so a crash at any instant is recoverable.
- No two live or kept actions in conflicting groups.
- Nothing is applied outside an exact `AUTONOMOUS_CONTRACTS` tuple.
- Every host mutation is accounted for by an audited transition.
- No experiment starts inside a cooldown or inside the same action's minimum spacing, including
  across a controller restart.
- A faulted startup recovery keeps the controller down; a cooldown never clears a fault.

## Running it

```text
cargo test -p stutter --lib fault_simulation_tests
STUTTER_FAULT_SIM_SEEDS=2000 cargo test -p stutter --lib fault_simulation_tests::scenarios::random
```

The wide sweep is for hunting counterexamples; a failing seed is a complete bug report because the
assertion prints the audited event trail.

## What it found

The first run of the simulation exposed three defects that were fixed with it: the controller
journal holds one record, so a second kept action overwrote the first one's durable token (now the
kept-action ledger); a stop restored only the active experiment and left kept actions applied (now
released on stop and on workload exit); and a cooldown decision during a fault cleared the fault
(now it does not). Candidate memory was lost on restart (now persisted).
