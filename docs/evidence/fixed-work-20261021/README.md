# Fixed-work command timing smoke

PASS on 2026-10-21 at source commit recorded in `source-commit.txt`.
Two actual package-clean builds of this repository's `stutter-core` library ran
in the pinned Nix development shell. Each command cleaned only this package in
an isolated temporary Cargo target directory; dependencies were primed first.
Raw manifests, marker lines, build output and the executed script are retained.
The script contains this host's checkout path and creates a fresh temporary
build directory when rerun. Use the recorded source commit to reproduce it.

Baseline elapsed: 392,871,489 ns. Candidate elapsed: 391,818,357 ns.
Both have positive exact monotonic durations, identical workload/source and
process identities, distinct execution IDs, successful completion and ordered
non-overlapping times. These labels identify sequential executions; no tuning
change was applied. No foreground monitor, eBPF capture or mutation ran.

This proves the real command-to-marker path, not a throughput improvement,
controlled performance experiment, objective live acceptance, overhead bound,
foreground protection or calibration. The workload descriptions are declared
harness policy; source cleanliness and tree identity were checked by the runner.
