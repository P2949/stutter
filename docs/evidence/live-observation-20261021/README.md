# Controlled real scheduler observation

PASS on Linux 6.18.55 at clean committed source `1e0760d`, with the stutter
binary reporting that same revision. The manifest retains the exact binary
digest, source commit, toolchain and Cargo.lock digest. The receipt records
9,946 scheduler samples, 42 target interval records and zero drop counters.

The canonical harness ran for ten seconds under noninteractive sudo after
building stutter and xtask in the pinned Nix shell. A disposable helper's periodic
wakeup and CPU worker threads shared one CPU selected from the helper's allowed
set. Target process identity survived recording. Artifact validation, JSON
analysis/diagnosis, HTML rendering and JSON advisor output completed. Optional
correlation probes were deliberately disabled; Medium report quality and advisor
CollectMoreData are valid outcomes for this core scheduler capture.

The receipt's maximum interval p99 is a positive-data invariant, not a merged
window percentile. The helper was classified Service. This does not prove live
game-objective behavior, mutation, recovery, overhead, calibration, or another
kernel environment. The Core Validation milestone remains open.

The original full local bundle is retained at
`/tmp/stutter-live-clean-source-20261021`. Only selected summaries are published
here; builder user/host identities in /proc/version are redacted. Full sanitized
artifact export and integrity manifests remain follow-up work. Reproduce from
the recorded source using `docs/LIVE_ACCEPTANCE.md`, a clean tree, and a new
output directory; host timings and PIDs will differ.
