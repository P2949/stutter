# Full System Watcher Architecture

`stutter` is structured as an always-on observer, planner, and guarded action
runner. The important safety invariant is simple: observation and planning never
mutate the machine. Providers return candidates; mutation only happens through a
`TuningAction`, `actions/runner.rs`, and `DaemonPolicy`.

## Control Loop

1. Collect monitor events from the session runtime.
2. Update rolling windows for scheduler, frame, GPU, IRQ, I/O, focus, and drop
   counter evidence.
3. Build an `AutotuneObservation` with focus, health, capabilities, workload
   identity, protected tasks, and the cheap active configuration snapshot.
4. Resolve focus and classify the situation with `SituationClassification`.
5. Evaluate data quality, health, focus stability, protected-task warnings, and
   target identity.
6. Ask registered candidate providers for proposals.
7. Filter proposals through `DaemonPolicy` and provider-specific requirements.
8. Rank eligible proposals by workload objective, confidence, conflict group,
   cooldown state, and historical memory.
9. In `observe`, record the decision only. In `suggest`, emit candidates without
   mutation. In apply modes, start at most one experiment.
10. Measure the candidate against a comparable baseline and workload-specific
    objective.
11. Keep only if the objective improves without required-signal regressions;
    otherwise roll back.
12. Record the outcome, enter cooldown, and keep rollback recovery available.

## Boundaries

Observation reads live state and records evidence. It may build snapshots and
warnings, but it does not decide that an action is safe.

Diagnosis turns evidence into causes and situation classifications. It is pure:
no filesystem writes, no policy checks, and no mutation.

Candidate planning asks providers for `CandidateAction` values, evaluates policy
and quality gates, records denials, and selects at most one candidate.

Action execution converts a selected candidate into a `TuningAction` and runs it
through the action runner. Rollback tokens are required before autonomous apply.

Verification compares baseline and candidate windows with an objective selected
for the workload, not a single global score for every case.

Rollback is owned by action implementations, controller journals, startup
recovery, shutdown handling, and emergency restore.

## Planner Rules

- Providers must suggest first. A new family can produce visible candidates
  before autonomous apply is enabled.
- Providers must not mutate directly.
- Providers must include evidence, safety class, required mode, rollback
  requirement, capability requirements, conflict group, cooldown key, and
  objective.
- Unsupported apply paths must return structured denial records.
- Protected audio, input, compositor, recorder, kernel, and service tasks are
  excluded unless explicit policy opts in.

## Mode Compatibility

| Family | Observe | Suggest | Apply Low Risk | Apply Medium Risk | Apply High Risk |
| --- | --- | --- | --- | --- | --- |
| CPU affinity profile | observe only | suggested | reversible process tree | reversible process tree | not needed |
| Nice | observe only | suggested | denied | reversible process/task only | not needed |
| I/O priority | observe only | suggested | denied | reversible process/task only | not needed |
| Uclamp | observe only | suggested | denied | reversible process/task or allowlisted cgroup | not needed |
| Cgroup placement | observe only | suggested | denied | allowlisted cgroup only | not needed |
| IRQ affinity | observe only | suggested/manual | denied | denied by default | explicit high-risk/manual |
| CPU power | observe only | suggested/manual | denied | denied | explicit high-risk/manual |
| GPU power | observe only | suggested/manual | denied | denied | explicit high-risk/manual |
| VM knobs | observe only | suggested/manual | denied | denied | explicit high-risk/manual |
| sched_ext/scx | observe only | suggested/manual | denied | denied | explicit high-risk/manual |

Docs and CLI output must not claim an apply mode supports a family until both
policy and runtime enforcement support it.
