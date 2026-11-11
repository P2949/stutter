# Tools installed for the W26–W30 evidence phases, and first real captures (2026-11-11)

NixosConf `hosts/desktop/stutter-tools.nix` (commits `6cfda09`, `df5ff2a`-line follow-up; `validate affected` PASS) installs
`perf`, `fio`, `iperf3`, `wrk2`, `h2load` (nghttp2), `numactl`, `llvm-bolt`, `blktrace`, `bcc`, `hyperfine`. The tools are inert
on PATH. Generation `pmxw5ix9…` was activated (the only unit change was `polkit` and `dbus-broker`, which restart on any closure
change; no Stutter unit was touched, both still active, 0 tokens). The previous generation `cm4bbc51…` is preserved.

Scope decision (user, 2026-11-11): development targets this machine only. Items that need different hardware are
`NotPlannedForVerification` (new closeout disposition, requires the reason): multi-node NUMA (5.19) and RDT/resctrl (5.24).

## Real captures on this host (loopback, no privileged action, no Stutter mutation)

* `xtask benchmarks offered-load` with the real `wrk2`, 200 req/s for 8 s against a loopback HTTP server: run A 1602 completed,
  0 errors, p99 1.67 ms; run B 0 errors, p99 3.37 ms. `compare-load` classified B `CandidateSlower`: two identical runs
  differed 2x in p99, so a single pair is noise-limited, which the paired-block design (W23/W24) exists to handle. This is a
  tool-workflow proof only; it is not a benefit claim.
* `hyperfine --export-json` imported by `xtask benchmarks import-hyperfine` (`all_exited_zero: true`).
* `perf stat` and `fio` (JSON output) run on this host; their importers stay fixture-verified until a captured artifact is
  imported in a later slice.
