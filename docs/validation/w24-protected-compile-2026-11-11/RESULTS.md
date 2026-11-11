# W24 result: protected compile under background contention (2026-11-11)

Run exactly as preregistered in `PREREGISTRATION.md` (committed before any measurement, sha256 of that file
`32d8d28b…`; thresholds and rule unchanged). 32 executions, all exit 0, source unchanged, no game running (checked before every
execution). Raw evidence: `results.tsv`, `markers-all.ndjson`; harness: `study.sh`.

| study | candidate action | blocks | mean paired change in compile time | 95 % interval | classification |
|---|---|---|---|---|---|
| 1 contention, nice 0 vs hogs at nice 19 | reduce background priority (unprivileged, on the hogs only) | 8 | -29.49 % (about 8.8 s to 6.2 s) | [-32.96 %, -26.02 %] | **Beneficial** (upper bound < -7.5 %) |
| 2 harmful control, no contention | compile restricted to one CPU | 8 | +72.25 % | [+70.98 %, +73.52 %] | **Harmful** (lower bound > +7.5 %) |

Both outcomes match the expectation written beforehand. Study 2 shows the method detects harm; the A/A cohort in W23 (CV 1.62 %)
is the no-effect control.

**Scope.** This shows that, for a completed fixed-work compile under CPU contention on this machine, lowering the contending
background tasks' priority helps the protected compile; the guardrail (successful completion, unchanged source) held in all 32
executions. It is a benefit classification of a synthetic-contention scenario, not a game result: the real-game direct-frame
benefit study stays `BlockedResource` (no W22 reference, no unattended game benchmark). It grants no autonomy and creates no
Supported tuple; a nice change on a foreign process is not an action Stutter's worker performs.
