# E13: does the E3 effect transfer to a different owned workload on this host? (2026-11-11)

Preregistration `PREREGISTRATION.md` (sha256 prefix `293cb8a1`, committed in `a79ea992` before any slot), manifest digest in `manifest.digest`. Workload: cold
RELEASE build `cargo build --offline --locked --release -j 12 -p stutter-validation-runner` of git tree `de98052d...` (E3/E4 were a 2-job debug
build of `stutter-config`), 12 owned stress-ng hogs, harness `xtask owned-study` at `a79ea992` (journal, owned workspace, fail-closed host preflight).
4 counterbalanced blocks, 12 slots, all valid, 0 invalid, no game, no leftovers. Every slot's CPU mask was read back inside its target; the build output
`release/libstutter_validation_runner.rlib` had the SAME sha256 in all 12 slots (prefix `1f1ae214`), so each arm did identical work. The source tree was
verified clean and unchanged before and after every slot. Context digest prefix `a7008790`. Raw: `slots/`, `*.markers.ndjson`, `run.log`, `analysis.json`.

| Arm | Mean elapsed | Mean improvement vs baseline | Seeded 95% bootstrap interval | Verdict |
|---|---|---|---|---|
| baseline | 19.34 s | n/a | n/a | n/a |
| `hogs-sibling-threads` (hogs on CPUs 6-11) | 13.96 s | +27.70% | 23.66..32.75 | Beneficial |
| `compile-primary-threads` (compile on CPUs 0-5) | 22.99 s | -19.31% | -31.77..-7.91 | Harmful |

Result: **partial transfer.**
- Confining the owned pressure generator still shortens this different build under the same contention, by +27.7%, a smaller effect than E3's +42.9% on the old workload.
- Pinning the compile to CPUs 0-5 did NOT transfer: it helped the 2-job debug build modestly in E3 (+9.0%) but is Harmful here (-19.3%). A plausible reason is that
  this build asks for 12 jobs and cannot use them on 6 logical CPUs, but that was not tested and is only a hypothesis.
- The ranker reports `InsufficientLegalCandidateEvidence` by design (two candidates).

Caveats: logical CPUs 0-5 and 6-11 are SMT siblings of the same six physical cores (hwloc/lscpu: siblings are N and N+6), so this is not physical-core isolation, and E3's
full partition did not add a proven increment over hog confinement alone. One host, one second workload, 4 blocks, offline owned-scope evidence only: no Supported tuple, no
default change, no worker authority, no game or frame claim, no cross-hardware claim.
