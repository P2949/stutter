# Supervisor Offline Demo

This document describes a conservative offline demo/checklist for supervisors
that does not require root, running games, or live eBPF tracing. It exercises
the core validation and artifact checks used for supervisor handoff.

Run the included script:

```bash
scripts/demo/supervisor_offline_demo.sh
```

The script builds the demo binaries once and captures normal cargo/build-script
output in `target/supervisor-demo/build.log`. It also writes verbose fixture and
dependency command output to `target/supervisor-demo/fixture-check.log` and
`target/supervisor-demo/dependency-hygiene.log`. If a step fails, the relevant
log is printed. On success, the visible demo output focuses on fixture-check
summaries, dependency-hygiene summaries, CLI shape, and evidence-bundle
checksums.

This checks the curated evidence bundle, fixture hygiene, dependency policy, and
CLI/reporting surface without requiring a live Proton workload.

What it does:

- Runs `xtask fixture-check` to validate test fixtures.
- Runs `xtask dependency-hygiene` to check dependency hygiene.
- Shows CLI shape for key commands (`--version`, `profile-plan --help`, `tune --help`).
- Verifies `evidence-bundle/MANIFEST.sha256` checksums.

Expected result: all commands finish without error and the evidence-bundle
checksums validate.
