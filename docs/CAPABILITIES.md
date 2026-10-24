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
VM settings stay Experimental). CPU affinity is the first graduation candidate; its
recovery, affinity and suspend/resume evidence is indexed; the remaining gaps are the
observation bundle and an empirical-benefit decision. It is not promoted yet.

Mutating capabilities require at least `live_recovery` and `live_suspend_resume`
(`MUTATING_REQUIRED_EVIDENCE`). Changing a tier is a code change that must ship together
with the evidence-index entries that justify it.

<!-- campaign-policy:start -->
## v0.1.0 tier policy

The validation campaign applies one fixed, conservative policy once live observation, game-runnable, kernel, overhead, corpus, calibration and score evidence all verify:

- **Supported:** `doctor`, `record`, `bench`, `report`, `audit`, `completions_and_man`
- **Experimental:** `diagnose`, `advisor`, `tune_evaluation`, `restore`, `cpu_affinity_experiment`, `process_priority_and_placement`, `system_wide_tuning`, `always_on_autotune`, `remote_agent_mutation`
- **Internal:** `privileged_worker`, `fake_action_provider`

Diagnosis stays Experimental even after calibration, and no mutating capability is promoted because CPU-affinity safety testing is strong.
<!-- campaign-policy:end -->
