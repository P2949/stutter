# Supervisor Offline Demo

This document describes a conservative offline demo/checklist for supervisors
that does not require root, running games, or live eBPF tracing. It exercises
the core validation and artifact checks used for supervisor handoff.

Run the included script:

```bash
scripts/demo/supervisor_offline_demo.sh
```

What it does:

- Runs `xtask fixture-check` to validate test fixtures.
- Runs `xtask dependency-hygiene` to check dependency hygiene.
- Shows CLI shape for key commands (`--version`, `profile-plan --help`, `tune --help`).
- Verifies `evidence-bundle/MANIFEST.sha256` checksums.

Expected result: all commands finish without error and the evidence-bundle
checksums validate.
