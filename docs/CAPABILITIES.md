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
remaining evidence is the suspend/resume suite and the observation bundle, plus
empirical benefit from calibration.

Mutating capabilities require at least `live_recovery` and `live_suspend_resume`
(`MUTATING_REQUIRED_EVIDENCE`). Changing a tier is a code change that must ship together
with the evidence-index entries that justify it.
