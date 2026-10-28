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

Each capability names two evidence lists. *Required live evidence* (`required_evidence`) is the
historical proof that justified graduating it and is what `capability-check` verifies against the
index. *Required release evidence* (`required_release_evidence`) is what a release candidate must
bring fresh, exact-commit bundles for (`release-verify --require-acceptance`); it never re-demands
a proof too long to repeat per candidate. The bounded autonomous controller
(`bounded_autotune_controller`) graduates on the 24 h `live_autotune_soak` and ships on the short
`live_autotune_smoke`; its limitations are exactly the entries of `AUTONOMOUS_CONTRACTS`. The
daemon/service integration (`always_on_autotune`) is a separate capability and stays Experimental:
the packaged service runs `apply-low-risk`, which the soak does not exercise.

Mutation providers do not inherit each other's maturity: a registry test requires every
entry of `MUTATION_PROVIDER_KINDS` to be governed by exactly one capability, and
providers other than the graduated `cpu_affinity_profile` and `cgroup_placement` cannot target `Supported` in the first
release (`nice`, `ioprio`, `uclamp`, IRQ affinity, CPU/GPU power and
VM settings stay Experimental). CPU affinity is the first operation to graduate
(`xtask mutation-campaign`, see [MUTATION_CAMPAIGN.md](MUTATION_CAMPAIGN.md)): its observation,
affinity, recovery, suspend/resume and matched-pair benefit evidence is indexed at the campaign
commit, and `GameRunnableLatency` is the `Supported` objective that decides its keep/revert.
Cgroup placement is the second (campaign `mut-cgroup-placement-a509104-1793001876163`, see [MUTATION_GRADUATION_STATUS.md](MUTATION_GRADUATION_STATUS.md)); it does not inherit affinity's grant.

## Autonomous support matrix

A tier does not by itself authorise autonomous apply. Normal autonomous apply requires the
objective to be `Supported`, the operation's capability to be `Supported`, and an exact contract in
`stutter/src/autotune/autonomous_support/` (operation, workload, pressure, objective, effect scope,
prerequisites). Today the registry holds two contracts:

| Operation | Workload | Pressure | Objective | Scope |
| --- | --- | --- | --- | --- |
| CPU-affinity profile | stable, explicitly identified game | scheduler | `GameRunnableLatency` | local process tree |
| cgroup placement | stable, explicitly identified game, into the administrator-configured game cgroup | scheduler | `GameRunnableLatency` | cgroup |

Everything outside the matrix is suggest-only, benchmark-only, manual or rejected: other
affinity contexts, `uclamp_experiment`, `nice_experiment`, `ioprio_experiment`, every system-wide operation capability and always-on autotune
stay Experimental.

Mutating capabilities require at least `live_recovery` and `live_suspend_resume`
(`MUTATING_REQUIRED_EVIDENCE`). Changing a tier is a code change that must ship together
with the evidence-index entries that justify it.

<!-- campaign-policy:start -->
## v0.1.0 tier policy

The validation campaign applies one fixed, conservative policy once live observation, game-runnable, kernel, overhead, corpus, calibration and score evidence all verify:

- **Supported:** `doctor`, `record`, `bench`, `report`, `audit`, `completions_and_man`, `cpu_affinity_experiment`, `cgroup_placement_experiment`
- **Experimental:** `diagnose`, `advisor`, `tune_evaluation`, `restore`, `uclamp_experiment`, `nice_experiment`, `ioprio_experiment`, `cpu_power_epp`, `cpu_power_governor`, `gpu_power_profile`, `gpu_force_performance`, `vm_swappiness`, `vm_dirty_background_ratio`, `vm_dirty_ratio`, `irq_affinity_known_device`, `irq_affinity_storage_or_unknown`, `bounded_autotune_controller`, `always_on_autotune`, `remote_agent_mutation`
- **Internal:** `privileged_worker`, `fake_action_provider`

Diagnosis stays Experimental even after calibration. CPU affinity and cgroup placement are Supported only for their graduated scheduler-pressure tuples (see the autonomous support matrix above); no other mutating capability is promoted.
<!-- campaign-policy:end -->
