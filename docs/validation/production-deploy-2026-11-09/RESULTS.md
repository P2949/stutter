# W19 production deployment record (2026-11-09)

Stutter source `4f3baa07aface5cdf355095e2de82ece4ebc4c21` (clean CI, workspace Clippy and
`release-check` all PASS, see `../source-ci-2026-11-09-4f3baa07.json`). Release binary SHA-256
`bf79d9c4a4fd3da18a079f1ccddf8c772f9a7c1b12da49e7d698a8c20c20d8a6`, added to the Nix store as
`/nix/store/rvd6k3ww69z9v3r32r6cvwnh1xd68ic1-stutter`.

NixosConf branch `host/stutter-w19-328b3f2b`, commit `250e3cf` (source pin and binary hash moved
together; the earlier commit `4686fb5` pinned `328b3f2b`). `./scripts/validate affected`: PASS
with an exact-commit receipt. The desktop toplevel built; the only closure change was the Stutter
binary and the two unit files. `dry-activate` listed only the two Stutter units; no display
manager, login, kernel or GPU policy change.

| Step | Result |
|---|---|
| First attempt, build of `328b3f2b` | guarded `test` activation succeeded, but `production-smoke` FAILED one of 35 checks: `idle_controller_cpu_is_bounded` 5.63% of one core over 30 s (limit 5%). Evidence: `.llm-runs/production-smoke-20261109-328b3f2b`. Not switched. |
| Diagnosis | eleven cheap sensors read thousands of sysfs/procfs files every tick. Same-flags idle measurement, observe mode: the previous production binary 4.85% of a core, the `328b3f2b` build 5.28%, the fixed build 3.23%. |
| Fix | commit `4f3baa07`: counter and inventory sensors are sampled every 2 to 30 seconds; guards stay at the tick cadence |
| Second attempt, build of `4f3baa07` | guarded `test` activation: running controller (user p2949) and root worker both execute the binary with SHA-256 `bf79d9c4...`, 0 outstanding tokens, display manager untouched. `production-smoke`: **PASS (35 checks)**. Evidence: `.llm-runs/production-smoke-20261109-4f3baa07`. |
| Switch | after the pass only: system profile set and `switch-to-configuration switch`; previous generations retained; services active; 0 outstanding tokens |

Honest limits: the passing idle-CPU check measured **4.97% of one core against the 5% limit**, a thin
margin. The same measurement of the previous production binary under the same host load was
4.85%, and the 2026-11-07 smoke measured 1.87% on a quieter host, so the check is sensitive to host
load rather than showing a regression of this build. The exact-build smoke was run once for the
deployed binary after the failed first attempt, as the plan allows. No reboot, no GPU or LACT
change, and no public GitHub issue, PR, comment or release was created (the NixosConf branch was
pushed without a PR).
