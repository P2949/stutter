# E4 preregistration: owned transient-scope CPU weight on the fixed cold compile (written before any measurement)

Manifest `manifest.json` (frozen digest `192c1d731c2c9ee324c8b14744c7b598c08b066eb5996e9f7eb7927eb392912a`). Same harness, workload identity, KPI, block design, analysis and decision
rule as E3 (see `../e3-affinity-study-2026-11-11/PREREGISTRATION.md`); only the candidate family differs.

Mechanism: OwnedUserScopeCpuWeight. The compile and the 12-hog pressure generator each run in their OWN transient user scope created
with `systemd-run --user --scope -p CPUWeight=N` (unique unit names, siblings under the user manager's `app.slice`). The
user manager delegates `cpu`, so no root, no worker and no file write outside systemd's own API are involved. The kernel's
`cpu.weight` of each target's cgroup is read back from inside the target before it execs; a mismatch makes the slot Invalid.
Scopes end when their processes do. Nothing in the login session, compositor, Stutter units or any slice other than these two
scopes is touched. `io` and `cpuset` are not delegated and are not used.

Why this is a new hypothesis (PLAN.md E4): earlier cpu.weight/cpu.idle pilots changed weights for live-session workloads and are
retained unchanged as negative evidence. This study instead tests RELATIVE weight between two owned sibling scopes holding a fixed
completed-work compile and an owned pressure generator, with a no-change baseline (both scopes at the default 100).

Candidates: `hogs-scope-weight-50`, `hogs-scope-weight-10` (the generator's scope deprioritised; a dose-response pair),
`compile-scope-weight-1000` (compile prioritised), and `compile-scope-weight-1-control` (compile starved to weight 1 against 100: a
harmful control, expected Harmful; if it is not, that is reported). Baseline: both scopes weight 100.
Design, thresholds and verdict rule as E3: 6 counterbalanced blocks, blocks 1-3 training, 4-6 untouched holdout, practical threshold
2 points, seeded bootstrap, invalid slots reported not repeated, no extra blocks. Offline owned-scope evidence only: no Supported
tuple, no default change, no worker authority, no autonomous cgroup weight.
