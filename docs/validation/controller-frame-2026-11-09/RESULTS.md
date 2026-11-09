# Default-preset controller run on a clean committed build (2026-11-09)

`controlled_real` engineering evidence, not game benefit or G2/G3 qualification. A transient
bounded `observe`-mode controller (profiling capabilities only, no worker, no operation) ran
against one vsync-capped 640x480 vkcube launched through `game-run` with MangoHud. The binary was
built from the clean commit `1962369e` (empty diff); production units were untouched.

| Check | Result |
|---|---|
| Default diagnosis preset starts | Yes. The 2026-11-08 run exited before its first tick on `sched_stat_wait.pid` offset 12; with an incompatible optional tracepoint now degrading, the controller ran to its duration and exited with `Result=success`. |
| Frame source | One producer resolved (pid and start ticks), `ValidatedDirect`, zero competing targets, zero dropped frames, source age 22-42 ms. |
| Direct-frame primary eligibility | Denied (`AlignmentUncertain`), alignment uncertainty 454-496 ms against the unchanged 2 ms gate. |
| Cleanup | Both units inactive, MainPID 0; no kernel fault lines; zero outstanding Stutter restore tokens. |

The 2 ms requirement is still not met; see the EvidenceDeferred note in
`docs/REAL_WORKLOAD_TELEMETRY.md`. Raw artifacts stay under `.llm-runs/controller-frame-20261109`;
their hashes are in `summary.json`.
