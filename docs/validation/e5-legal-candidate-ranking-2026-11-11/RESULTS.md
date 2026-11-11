# E2-E5 closure: owned legal-candidate studies and offline ranking (2026-11-11)

(a) Code: `xtask owned-study` (`xtask/src/owned_study/{mod,manifest,run,analyze,tests}.rs`), first committed in `4548f6c1`, context digest added in
`b666fbab` (the code SHA the clean-clone receipt covers). It starts only processes it owns under the existing supervisor, uses no
privileged worker and no daemon registry entry, and has no field for a path, PID, UID or root target.

(b) Tests (16, all passing; `cargo test -p xtask owned_study`, workspace clippy clean): malformed or out-of-effective-cpuset masks, no-op
candidates, mixed families, zero or huge weights, duplicate or path-like names, bounds on blocks/threads/candidates/timeouts, unknown
fields and placement kinds do not deserialize; counterbalanced deterministic schedule with unique slots; resume repeats no recorded
slot; strict start/completion markers (failed, incomplete, duplicated or malformed builds are refused); readback must match the
request (and one role's line is not another's); launcher commands use only `taskset` / `systemd-run --user --scope`; a competing game
is found while a shell merely mentioning one is not, and an unreadable /proc is not a safe host; a corrupted or foreign-manifest
results file authorizes nothing; mixed or resumed contexts are refused; records for illegal operations (nice-19, uclamp-max,
pm-qos-0, gpu-power-profile, scx-switch) with the best score never reach the ranker, and fewer than three legal candidates report
`InsufficientLegalCandidateEvidence`.

(c) Results (digests are the first 16 hex of the analysis.json sha256):
- E2 harness A/A: `../e2-owned-study-aa-2026-11-11/` and, with the context digest, `../e5-context-check-2026-11-11/`: 4 valid slots each, readbacks
  matched, resume repeated 0 slots, a tampered context refused the resume, one context reported.
- E3 CPU affinity: `../e3-affinity-study-2026-11-11/` (`ceae578855ea040e`): confining the owned generator to CPUs 6-11 +42.9% (95% CI 40.1..45.9), partition +43.5%
  (40.8..46.4), compile on 0-5 +9.0% (3.9..15.7), two-CPU control Inconclusive -2.2% (-9.2..3.4). Held-out ranking Spearman 1.0, regret 0.
- E4 transient-scope weight: `../e4-scope-weight-study-2026-11-11/` (`b587a99e0eacdd45`): compile 1000/100 +7.1% (5.3..8.9), hogs 10 +5.9% (2.3..10.4), hogs 50 Inconclusive,
  starved compile (1/100) Harmful -323%. Held-out ranking Spearman 1.0, regret 0.
- E5: both rankings are over legal manifest candidates only (`ConfigSpace` + `import_measurements` + `held_out_ranking`), top-1 agrees, regret 0 on
  untouched holdout blocks, with the no-change baseline at 0. The original W25 nice-hog data is not legal-action training data and is not used.

(d) Host state: no game ran in any slot (checked before every slot); no stress-ng, fixed-work, owned-study or `stutter-study` scope process
remained after any study; both Stutter units stayed active and unchanged; `/sys/kernel/sched_ext/state` was `disabled`; zero reboots.
The E3/E4/E2 records were produced before the context digest existed, so they carry no context field (`records_without_context` counts
them); the digest is recorded on the later context-check run and on every future run.

(e) Remains experimental: every effect above is offline, owned-scope evidence for ONE fixed cold compile under ONE synthetic pressure shape
on ONE host, 6 blocks. None is a Supported tuple, a default-selection change, worker authority or an autonomous cgroup/affinity
promotion; `AUTONOMOUS_CONTRACTS` is unchanged and nothing was deployed. Game or GameRunnableLatency benefit is not shown by this work.
