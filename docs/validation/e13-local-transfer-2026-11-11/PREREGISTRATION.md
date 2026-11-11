# E13 preregistration: one genuinely different owned workload on this host (written before any measurement)

Question (PLAN.md E13): do the E3 owned-affinity effects generalise to a materially different owned compile workload on the SAME
i5-10600K host (not another machine)? E3/E4 measured one two-thread debug `stutter-config` build; this study changes the workload,
not the seed.

Workload (immutable): git tree `de98052df9b18dac7d5ba33ea28a9c8d4354883d` (the clean checkout at `/tmp/claude-1000/nixfull/w23/src`, tree and
cleanliness verified by the harness before every slot), toolchain rustc 1.98.0-nightly (8954863c8 2026-06-05) from the project dev shell,
command `cargo build --offline --locked --release -j 12 -p stutter-validation-runner`, i.e. a RELEASE profile build (optimised,
parallel codegen) of a different crate with 12 jobs instead of a 2-job debug build of `stutter-config`. Cold every slot: the study-owned
`workspace/target` is removed and `CARGO_TARGET_DIR` points at it; the source tree is only read. Success marker: `WORKLOAD_COMPLETE` with
exit 0. Completed-output check: the sha256 of `release/libstutter_validation_runner.rlib` must be IDENTICAL in every valid slot (a
pilot showed three cold builds at one fixed target path give the same hash); a slot whose output differs is Invalid and kept as
evidence. Unloaded cold builds of this workload took about 9 s in a pilot (not part of the data).

Pressure context: the same 12 owned `stress-ng --cpu` hogs for the whole compile. Candidates (owned child affinity only, SMT siblings are
N and N+6 so CPUs 0-5 and 6-11 are NOT physical-core isolation): `hogs-sibling-threads` (hogs on 6-11) and `compile-primary-threads`
(compile on 0-5), each against `baseline` (nothing restricted). No transient-scope candidate (a separate family, not repeated here).

Design (frozen, no extension after seeing data): 4 counterbalanced blocks, 12 slots, arm order rotated by block and reversed on even
blocks. Blocks 1-2 training, 3-4 holdout (with two candidates the ranker reports InsufficientLegalCandidateEvidence by design). Primary KPI
`elapsed_ns` of the cold build, lower is better; improvement = (t_baseline - t_candidate) / t_baseline. Seeded 95% bootstrap over
the 4 blocks, practical threshold 2 points: Beneficial if the lower bound is above +2, Harmful if the upper bound is below -2,
Equivalent if inside (-2, +2), else Inconclusive. Fewer than 3 valid paired blocks for a candidate is declared
InsufficientIndependentBlocks. Hard guardrails: any game or launcher, or an inspection that cannot be completed, stops the study before a
slot and invalidates a slot during it; every role's CPU mask is read back inside its target. Every outcome, including Inconclusive or
Harmful, is reported and no slot is repeated. Manifest digest `ae6b74296931460fbd7cf69c608ba0b29769d0d2a39140550b6268382a57bad5`.

Claims allowed: whether the E3 hog-confinement and compile-pinning effects did or did not appear for this second owned workload on this
host. Not allowed: a game, frame or renderer effect, a production tuple, a default change, worker authority or any cross-hardware claim.
