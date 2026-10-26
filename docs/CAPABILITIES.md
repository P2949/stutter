# Capability tiers

`stutter capabilities [--json]` prints the registry in
`stutter/src/capability_tiers.rs`. Every capability has a **tier** justified by the
evidence that exists today and a **target tier** for the first supported release.

| Tier | Meaning |
| --- | --- |
| Supported | Semantic contract, regression coverage, real-system evidence, known degraded behavior, artifact compatibility, diagnostics and documented limitations. Mutating capabilities additionally need explicit policy, bounded scope, dry-run, durable original state, apply, physical verify, rollback, crash recovery, rollback-failure handling, audit, journal and real acceptance evidence. |
| Experimental | Usable, but not yet backed by the evidence its target tier requires. |
| Internal | Implementation detail or test support; not a user surface. |
| Disabled | Present in the tree but refused at runtime. |

A capability may be `Supported` only when every live-acceptance suite it names under
*required live evidence* has a PASS entry for a verified portable bundle in
[`docs/evidence/index.json`](evidence/index.json). `cargo run -p xtask --
capability-check` (run in CI) fails if the registry claims more than the index proves
and lists the remaining graduation gaps.

Mutation providers do not inherit each other's maturity: a registry test requires every
entry of `MUTATION_PROVIDER_KINDS` to be governed by exactly one capability, and
providers other than `cpu_affinity_profile` cannot target `Supported` in the first
release (`nice`, `ioprio`, `uclamp`, `cgroup placement`, IRQ affinity, CPU/GPU power and
VM settings stay Experimental). CPU affinity is the first operation to graduate
(`xtask mutation-campaign`, see [MUTATION_CAMPAIGN.md](MUTATION_CAMPAIGN.md)): its observation,
affinity, recovery, suspend/resume and matched-pair benefit evidence is indexed at the campaign
commit, and `GameRunnableLatency` is the `Supported` objective that decides its keep/revert.

## Autonomous support matrix

A tier does not by itself authorise autonomous apply. Normal autonomous apply requires the
objective to be `Supported`, the operation's capability to be `Supported`, and an exact contract in
`stutter/src/autotune/autonomous_support/` (operation, workload, pressure, objective, effect scope,
prerequisites). Today the registry holds one contract:

| Operation | Workload | Pressure | Objective | Scope |
| --- | --- | --- | --- | --- |
| CPU-affinity profile | stable, explicitly identified game | scheduler | `GameRunnableLatency` | local process tree |

Everything outside the matrix is suggest-only, benchmark-only, manual or rejected: other
affinity contexts, `uclamp_experiment`, `nice_experiment`, `ioprio_experiment`, `cgroup_placement_experiment`, system-wide tuning and always-on autotune
stay Experimental.

Mutating capabilities require at least `live_recovery` and `live_suspend_resume`
(`MUTATING_REQUIRED_EVIDENCE`). Changing a tier is a code change that must ship together
with the evidence-index entries that justify it.

<!-- campaign-policy:start -->
## v0.1.0 tier policy

The validation campaign applies one fixed, conservative policy once live observation, game-runnable, kernel, overhead, corpus, calibration and score evidence all verify:

- **Supported:** `doctor`, `record`, `bench`, `report`, `audit`, `completions_and_man`, `cpu_affinity_experiment`
- **Experimental:** `diagnose`, `advisor`, `tune_evaluation`, `restore`, `uclamp_experiment`, `nice_experiment`, `ioprio_experiment`, `cgroup_placement_experiment`, `system_wide_tuning`, `always_on_autotune`, `remote_agent_mutation`
- **Internal:** `privileged_worker`, `fake_action_provider`

Diagnosis stays Experimental even after calibration. CPU affinity is Supported only for its graduated scheduler-pressure tuple (see the autonomous support matrix above); no other mutating capability is promoted.
<!-- campaign-policy:end -->
