# W23: a reproducible real fixed-work compilation cohort with a direct KPI (2026-11-11)

**Workload.** A real, completed compilation: `cargo build --offline --locked -j 2 -p stutter-config` (which compiles
`stutter-core` and `stutter-config`) in a fresh clone of Stutter at `80a49a3c` (git tree `de98052d…`), with the target
directory removed before every execution (cold build, no incremental cache, `CARGO_INCREMENTAL=0`). `identity.json` is the
`FixedWorkIdentity` (kind `compile`, immutable git source tree, job count 2). `run.sh` is the exact loop.

**Direct KPI.** `xtask fixed-work` writes a validated `WORKLOAD_START`/`WORKLOAD_COMPLETE` pair per execution; the KPI is the
completion marker's monotonic `elapsed_ns` (lower is better). It counts only a build that exited successfully with the source
tree unchanged. `markers.ndjson` is the raw evidence (24 markers, 12 executions, 0 failed).

**Reliability (A/A, no treatment, no other load by design, no game running).**
n = 12: min 3.674 s, median 3.731 s, max 3.905 s, mean 3.747 s, sd 60.7 ms, **CV 1.62 %**. That is well under the 7.5 %
symmetric elapsed-time threshold frozen by the objective campaign, so a real effect of that size would be detectable by this
cohort; the observed spread is the noise floor a future paired A/B (W24) must beat.

**Not claimed.** This is a repeatable KPI and a short finite campaign, not a benefit result. No real-game benchmark is available
unattended on this host, so real-game benefit stays `BlockedResource`; the earlier local HTTP cohort (`wrk2`, p99 1.67 vs 3.37 ms
on two identical runs) is too noisy at a single pair to be a KPI without paired blocks. A background-contamination guard is
the existing game/host-busy refusal used by the smoke.
