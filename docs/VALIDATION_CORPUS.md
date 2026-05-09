# Validation corpus

The validation corpus is the set of committed recording artifacts used to keep
`stutter report --analysis-json` honest across diagnosis and artifact-schema
changes.

The corpus has two tiers:

1. **Synthetic contract fixtures** in `stutter/tests/fixtures/runs/`.
These are intentionally small and deterministic. They protect low-level
loader, artifact-count, data-quality, schema-warning, and diagnosis contracts.
2. **Sanitized real-world-shaped fixtures** in `stutter/tests/fixtures/runs/`.
These are still deterministic and privacy-safe, but they model full recording
situations more closely: a game main thread delay, a compositor delay, an IRQ
overlap, a block I/O stall, and a GPU-bound frame spike with clean CPU pressure.

Selected public examples are generated under:

```text
docs/examples/artifacts/v21/
```

Those examples are safe to publish because they contain no real command lines,
window titles, user names, host names, file paths, hardware serial numbers, or
raw process identities from a user machine.

## Current synthetic contract fixtures

| Fixture                       | Purpose                                                                     |
| ----------------------------- | --------------------------------------------------------------------------- |
| `cpu_pressure`                | Ensures high CPU PSI produces `CpuPressureCandidate`.                       |
| `block_io_stall`              | Ensures overlapping block I/O produces `BlockIoCandidate`.                  |
| `irq_heavy`                   | Ensures overlapping IRQ time produces `IrqDelayCandidate`.                  |
| `gpu_bound_clean_cpu`         | Ensures high GPU busy with clean CPU pressure produces `GpuBoundCandidate`. |
| `clean_run`                   | Ensures a quiet run remains high quality and produces no false diagnosis.   |
| `truncated_drop_counters`     | Ensures truncated or dropped event streams lower data quality.              |
| `reused_tid_no_contamination` | Ensures reused TIDs remain separate logical tasks.                          |
| `old_schema_warning`          | Ensures old schema versions warn instead of hard failing.                   |

## Current sanitized real-world-shaped fixtures

| Fixture                                 | Expected primary cause     |
| --------------------------------------- | -------------------------- |
| `real_world_game_scheduler_delay`       | `GameThreadSchedulerDelay` |
| `real_world_compositor_scheduler_delay` | `CompositorSchedulerDelay` |
| `real_world_irq_overlap`                | `IrqDelayCandidate`        |
| `real_world_block_io_stall`             | `BlockIoCandidate`         |
| `real_world_gpu_bound_clean_cpu`        | `GpuBoundCandidate`        |

## Regenerating committed fixtures

From the repository root, run:

```bash
cargo test -p stutter regenerate_validation_corpus -- --ignored
```

This regenerates the test corpus under:

```text
stutter/tests/fixtures/runs/
```

Then run:

```bash
cargo test -p stutter regenerate_public_examples_v21 -- --ignored
```

This regenerates selected public examples under:

```text
docs/examples/artifacts/v21/
```

After regeneration, run:

```bash
cargo test -p stutter validation_corpus_
```

Commit the regenerated JSON and NDJSON artifacts together with the Rust changes
that changed their shape or expected diagnosis.

## Adding a new corpus case

A new corpus case must follow these rules:

1. Prefer adding a fixture constructor in `stutter/src/test_fixture_builder.rs`.
2. Do not hand-edit generated fixture JSON unless the generator cannot represent
   the case.
3. Do not add a new diagnosis enum variant just to make a fixture pass.
4. Add a `validation_corpus_*` test in `stutter/src/validation_corpus_tests.rs`.
5. Assert the expected `StutterCause`, accepted confidence band, data-quality
   level, evidence substring, and artifact counts.
6. Keep public examples sanitized:

   * no real user names,
   * no host names,
   * no real window titles,
   * no private file paths,
   * no raw command lines from a real machine,
   * no hardware serial numbers,
   * no stable personally identifying process or workspace names.

## Why the corpus is generated

The artifact schema is broad: each run may include `session.json`,
`metadata.json`, `spike_events.json`, `interval.json`, `irq_events.json`,
`gpu_samples.json`, `frame_correlation.json`, `io_events.json`, and other event
streams. Generating fixtures through `test_fixture_builder` keeps the schema
consistent with the Rust types and avoids stale hand-written JSON.
