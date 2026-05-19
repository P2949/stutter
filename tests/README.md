# Test Directory Conventions

Use these names when adding or moving tests:

- Unit tests live next to the code they exercise and cover a single function or small module.
- Contract tests verify serialized schemas, CLI mappings, daemon policy decisions, and API behavior that other layers depend on.
- Fixture tests use committed inputs from `testdata/`, `stutter/assets/`, or `stutter/tests/snapshots/`.
- Simulation tests run fake daemon, fake action, or deterministic runtime loops without touching live host state.
- Smoke tests live under `scripts/smoke/` or CI jobs and prove the packaged binary can start and execute representative commands.
- Live-smoke tests are opt-in only and may touch host state, kernel probes, or real services.

Prefer the narrowest category that proves the behavior. When moving large inline modules out of production files, keep the test module under the owning source module first, then promote it to an integration test only when it needs public APIs.
