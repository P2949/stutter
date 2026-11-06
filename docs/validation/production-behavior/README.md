# Production service behaviour: findings, fixes and the milestone definitions (2026-11-06)

This records what the installed bounded service actually did, as opposed to whether its units were healthy,
and the milestone wording that separates the operational service from machine-wide mutation.

## What was observed on the production service (build `d50ef5d4`)

The controller runs `stutter autotune --config /etc/stutter/autotune-bounded-<user>.toml --mode apply-medium-risk
--allow-medium-risk --auto-focus --focus-source heuristic --preset lightweight`. The administrator file only sets
worker-only mutation and the worker socket; there is no game or compile cgroup destination and no profile file.
During idle and during unrelated builds the controller only observed (decisions `observed`; reasons "low data
quality: no scored samples" and "no candidate action available"), which is correct: there was no eligible game.

Three defects were found by measuring the idle service, none of them visible in unit status:

| Defect | Evidence | Fix |
|---|---|---|
| Idle CPU burn | 14.01 percent of a core over 60 s (11.23 percent in the smoke); `strace -c` over about 7 s: 634,363 `write` syscalls (about 90,000 a second), 7 `fsync` and 7 `rename` | `candidate-memory.json` (706 KB at its 512 + 512 record caps) was serialized to an unbuffered file and rewritten on every tick. It is now serialized once, written once and rewritten only when it changed (`6ef64546`). |
| Leaked temporary files | 67 `candidate-memory.tmp-*` files, 24 MB, oldest 2026-11-02, in the production state directory | A failed save removes its temporary file; loading removes temporaries left by dead processes (`6ef64546`). |
| Unproven counter semantics | the history totals checkpoint was written separately from the append and fell back to zero totals | position-based, durably committed checkpoint with idempotent replay and an explicit gap report (`b54b4ef5`, `7b24817c`). |

A fourth observation is a property, not a defect: after keeping an action the controller will not repeat the same
action (keyed on the action, not the workload) for 300 seconds. In the smoke history: `candidate_kept` at 11:01:55,
then "same action 'cpu-affinity-profile:game-isolate-render' is still cooling down" until about 11:06:55. The smoke
therefore runs its cases that never reach a kept outcome first and tolerates a cooldown left by an earlier run.
During a cooldown the controller appends one `cooldown_entered` record a second; the history is bounded by
rotation, but that volume is worth a later look.

## Smoke results (raw cases: `smoke-before.json`, `smoke-after-551150dd.json`)

| Check | Old build `d50ef5d4` | First fixed build `551150dd` |
|---|---|---|
| idle controller CPU (limit 5 percent) | FAIL 11.23 percent | PASS 4.83 percent |
| idle controller memory | PASS | PASS 85,476 kB |
| an idle controller changes nothing | PASS | PASS (0 non-observation decisions in 30 records) |
| no stale temporary files | FAIL 67 | PASS 0 |
| history stays bounded | PASS | PASS 74,328,025 bytes |
| fixture discovered, mutated through the worker, physically applied | PASS | PASS |
| experiment resolved | PASS | PASS (`candidate_kept`, runnable p99 improved, confidence High) |
| tokens settled after the workload | PASS | PASS (0 outstanding of 513) |
| controller killed while applied: token settled, placement restored | PASS | PASS |
| workload exits while applied | FAIL (test design: shared identity hit the cooldown) | FAIL (same cause) |

The later commits reorder the smoke (exit and kill first, kept case last, a cooldown-tolerant timeout) and drop the
fsync and checkpoint rewrite for routine observations (`7b24817c`). They have not yet run live: the second upgrade
attempt was interrupted before activation, so the idle figure of `7b24817c` and the exit case are still unmeasured.

## Decisions

- No cgroup destination or profile file is configured. The existing topology-aware CPU-affinity operation is the
  Supported path and was applied, kept and recovered; nothing shows that cgroup placement would do better, and the
  plan allows a destination only on a clear advantage with an owned, reversible lifecycle.
- Compile optimisation stays suggestion-only. `CompileThroughputWithForegroundProtection` is Supported for completed
  fixed work with explicit `WORKLOAD_START`/`WORKLOAD_COMPLETE` markers; an ordinary build produces none, so `Compile`
  is not added to the autonomous contracts.
- The cgroup `cpu.weight`/`cpu.idle` pilots (about 9 to 10 percent, below their preregistered floor) are not built into
  an operation, and no real-workload study is started: nothing in the production behaviour shows a latency problem the
  Supported operations leave unsolved.

## Milestone definitions

- **Operational system-level autotuner:** PASS when evidenced on the installed exact build: persistent service,
  correct discovery and policy enforcement, at least one Supported optimisation physically applied and recovered,
  user/root separation, crash/stop/restart recovery with no outstanding tokens, acceptable idle overhead, bounded
  persistent storage, and the exact-build smoke.
- **Broad workload autonomy:** a separate capability; each additional workload/operation/objective tuple needs direct
  evidence, and missing objective data never becomes a keep.
- **Machine-wide mutation:** `DeferredEvidence`. No system-wide operation has a measurable benefit on this host
  (`vm.swappiness`, IRQ steering, CPU EPP, the GPU profile). The mixed-scope acceptance (M14) is `NotApplicable`
  while no machine-wide tuple exists; it is not a PASS.
