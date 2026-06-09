# Supervisor release notes

Version: supervisor-hand-off (experimental)
Date: 2026-06-09T12:31:36Z
Commit: 8745c722b3e0939d7e3f06da1de0b17db65097b9

Contents:

- `FYP_SUPERVISOR_PITCH.pdf` — supervisor pitch PDF
- `KCD1_EXPERIMENT_REPORT.pdf` — KCD1 case-study report
- `evidence-bundle.tar.gz` — curated evidence bundle (small)
- `stutter-supervisor-release-8745c722b3e0.tar.gz` — assembled archive
- `SHA256SUMS` — checksums for the assembled archive

Notes:

- Full validation (`xtask ci`) completed and passed on the above commit. All tests and architecture checks passed locally.
- eBPF build noise suppressed by the `scripts/wrappers/bpf-linker` wrapper; set `STUTTER_EBPF_VERBOSE=1` to re-enable verbose linker output during builds.
