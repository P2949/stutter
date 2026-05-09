# Validation corpus

The validation corpus is the set of committed recording artifacts used to keep
`stutter report --analysis-json` honest across diagnosis and artifact-schema
changes.

Each fixture directory must contain a machine-readable metadata contract:

```text
stutter/tests/fixtures/runs/<fixture-name>/fixture.toml
```

Selected public examples are generated under:

```text
docs/examples/artifacts/v21/
```

Public examples use the same `fixture.toml` contract.

## Fixture tiers

The corpus has two sources of truth:

1. **Generated synthetic fixtures** in `stutter/tests/fixtures/runs/`.
These are intentionally small and deterministic. They protect low-level
loader, artifact-count, data-quality, schema-warning, diagnosis, foreground,
and classification contracts. `stutter/src/test_fixture_builder.rs` owns these
fixtures.
2. **Real sanitized validation recordings** in `stutter/tests/fixtures/runs/`.
These must be created with `scripts/sanitize-run-artifact.py` from real runs.
Do not create fake real recordings in `test_fixture_builder.rs`; the point of
this tier is catching real artifact weirdness that synthetic generation will
miss.

The public examples are safe to publish because they contain no real command
lines, window titles, user names, host names, file paths, hardware serial
numbers, or raw process identities from a user machine.

## `fixture.toml` contract

Every fixture metadata file uses this shape:

```toml
name = "real_world_game_scheduler_delay"
schema_version = 21
source = "sanitized-real-recording"
quality_expectation = "High"
description = "Game main/render thread had scheduler delay during a visible frame spike."

[expected]
primary_cause = "GameThreadSchedulerDelay"
accepted_confidence = ["Medium", "High"]
quality_reasons_contain = []
data_quality = "High"

[expected.artifacts]
spikes = 3
intervals = 3
irq_events = 0
gpu_samples = 1
frames = 4
block_io_events = 0

[expected.evidence]
contains = [
  "game thread",
  "delayed",
]

[privacy]
titles_redacted = true
paths_redacted = true
hostnames_redacted = true
usernames_redacted = true
```

The validation tests also understand minimum artifact counts. Use these only when
a fixture is expected to grow without changing the meaning of the case:

```toml
[expected.artifacts]
spikes_min = 1
intervals_min = 1
gpu_samples_min = 0
frames_min = 1
```

Do not set both the exact and `_min` form for the same artifact count in new
fixtures unless there is a specific reason.

For fixtures where the primary diagnosis may legitimately vary but a candidate
must be present, set:

```toml id="bl79bi"
[expected]
primary_cause = "Any"
required_candidate = "GpuBoundCandidate"
required_candidate_evidence = ["GPU busy"]
accepted_confidence = []
data_quality = "High"
```

Use this for GPU-bound-looking captures where scheduler evidence may be stronger
than GPU evidence, but the report must still surface the GPU-bound candidate.

For any fixture with `expected.data_quality = "Medium"` or
`expected.data_quality = "Low"`, `expected.quality_reasons_contain` must list
the user-facing quality explanation substrings that must appear in
`data_quality.reasons`, `data_quality.validation_warnings`, or
`data_quality.validation_errors`.

Examples:

```toml id="9gsc4w"
[expected]
primary_cause = "Unknown"
accepted_confidence = []
quality_reasons_contain = ["truncated", "drop"]
data_quality = "Medium"
```

```toml id="949zdp"
[expected]
primary_cause = "Unknown"
accepted_confidence = []
quality_reasons_contain = ["older than current"]
data_quality = "Medium"
```

Supported `expected.primary_cause` values are:

```text
CompositorSchedulerDelay
GameThreadSchedulerDelay
IrqDelayCandidate
GpuBoundCandidate
BlockIoCandidate
CpuPressureCandidate
Unknown
Any
```

Use `Unknown` when the fixture should produce no strong diagnosis.

Supported `expected.accepted_confidence` values are:

```text
Low
Medium
High
```

Supported `expected.data_quality` values are:

```text
High
Medium
Low
```

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

## Current generated synthetic fixtures

| Fixture                          | Expected primary cause     |
| -------------------------------- | -------------------------- |
| `clean_run`                      | `Unknown`                  |
| `cpu_pressure`                   | `CpuPressureCandidate`     |
| `block_io_stall`                 | `BlockIoCandidate`         |
| `irq_heavy`                      | `IrqDelayCandidate`        |
| `gpu_bound_clean_cpu`            | `GpuBoundCandidate`        |
| `truncated_drop_counters`        | `Unknown`                  |
| `reused_tid_no_contamination`    | `Unknown`                  |
| `old_schema_warning`             | `Unknown`                  |
| `game_thread_scheduler_delay`    | `GameThreadSchedulerDelay` |
| `compositor_scheduler_delay`     | `CompositorSchedulerDelay` |
| `foreground_window`              | `Unknown`                  |
| `community_rules_classification` | `Unknown`                  |

## Real sanitized validation recordings

Real sanitized recordings should also live under `stutter/tests/fixtures/runs/`,
but they must not be generated by `test_fixture_builder.rs`. Create them with
`scripts/sanitize-run-artifact.py`, add or update their `fixture.toml`, and add a
targeted validation test only when the metadata contract is not enough.

## Sanitizing real recordings

Do not hand-edit large real recording JSON files when creating new validation
fixtures. Use the sanitizer script first:

```bash id="epkvab"
scripts/sanitize-run-artifact.py \
--input /path/to/real/run \
--output stutter/tests/fixtures/runs/game_thread_scheduler_delay \
--name game_thread_scheduler_delay
```

The sanitizer copies the recognized `stutter` artifact files:

```text id="iro2w1"
session.json
metadata.json
interval.json
spike_events.json
tree_events.json
irq_events.json
gpu_samples.json
frame_correlation.json
frame_events.json
migration_events.json
cpu_freq_samples.json
io_events.json
scx_events.json
focus_events.json
foreground_events.json
```

It redacts private strings such as user names, home paths, host names, command
line paths, executable paths, Steam library paths, cgroup user slices, window
titles, and X11/Wayland window identifiers. It preserves the semantic fields
needed by diagnosis: `TaskClass`, timing fields, latency fields, elapsed times,
CPU PSI values, IRQ timestamps/durations, block I/O timestamps/durations, GPU
busy percentages, frame times, and foreground PID/app/class.

By default, the sanitizer verifies the output with:

```bash id="eerng8"
cargo run -p stutter -- validate stutter/tests/fixtures/runs/<fixture>
cargo run -p stutter -- report --analysis-json stutter/tests/fixtures/runs/<fixture>
```

Use `--no-verify` only when sanitizing on a machine that cannot build or run the
Rust project. A fixture submitted to the repository must still pass both commands
before it is committed.

If the output directory already exists, pass `--force` to replace it:

```bash id="uwv6oj"
scripts/sanitize-run-artifact.py \
  --input /path/to/real/run \
  --output stutter/tests/fixtures/runs/game_thread_scheduler_delay \
  --name game_thread_scheduler_delay \
  --force
```

Real sanitized fixture directories should normally use names that make the run
origin clear, for example:

```text id="za8pva"
real_clean_baseline/
real_game_thread_scheduler_delay/
real_compositor_scheduler_delay/
real_irq_overlap/
real_gpu_bound_looking/
real_block_io_overlap/
real_truncated_low_quality/
real_foreground_window/
real_community_rules_classification/
```

Those directories are not regenerated by `cargo test -p stutter
regenerate_validation_corpus -- --ignored`. If a real fixture is committed, the
sanitized artifact files are the source of truth.

After sanitizing a new fixture, add or regenerate its `fixture.toml` metadata
contract and add a validation test only when the fixture needs extra
case-specific assertions beyond the metadata-driven checks.

## Corpus runner commands

Use this command for the normal validation-corpus test pass:

```bash
cargo test -p stutter validation_corpus
```

Use this command in CI:

```bash
cargo test -p stutter validation_corpus
```

Do not run ignored regeneration tests in CI. Regeneration rewrites committed
fixture artifacts and is a maintainer operation, not a validation step.

To regenerate the committed validation corpus locally, run:

```bash
cargo test -p stutter regenerate_validation_corpus -- --ignored
```

To regenerate the selected public v21 examples locally, run:

```bash
cargo test -p stutter regenerate_public_examples_v21 -- --ignored
```

To smoke-check a single fixture through the public CLI, run:

```bash
cargo run -p stutter -- validate stutter/tests/fixtures/runs/real_clean_baseline
cargo run -p stutter -- report --analysis-json stutter/tests/fixtures/runs/real_clean_baseline
```

A typical local corpus maintenance pass is:

```bash
cargo fmt --all
cargo test -p stutter regenerate_validation_corpus -- --ignored
cargo test -p stutter regenerate_public_examples_v21 -- --ignored
cargo test -p stutter validation_corpus
cargo run -p stutter -- validate stutter/tests/fixtures/runs/real_clean_baseline
cargo run -p stutter -- report --analysis-json stutter/tests/fixtures/runs/real_clean_baseline >/dev/null
```

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
  clean_baseline/
  game_thread_scheduler_delay/
  low_quality_truncated/
  README.md
```

Only small representative examples belong under `docs/examples/artifacts/v21/`.
The larger regression corpus belongs under `stutter/tests/fixtures/runs/`.

After regeneration, run:

```bash
cargo test -p stutter validation_corpus_
```

Commit regenerated JSON, NDJSON, and `fixture.toml` files together with the Rust
changes that changed their shape or expected diagnosis.

## Adding a new corpus case

 A new corpus case must follow these rules:

1. Prefer adding a fixture constructor in `stutter/src/test_fixture_builder.rs`.
2. Add the fixture to `write_validation_corpus`.
3. Add a `fixture_metadata_for` entry for the fixture.
4. Do not hand-edit generated fixture JSON or `fixture.toml` unless the generator
   cannot represent the case.
5. Do not add a new diagnosis enum variant just to make a fixture pass.
6. Add a `validation_corpus_*` test in `stutter/src/validation_corpus_tests.rs`
   only if the fixture has extra case-specific invariants beyond the generic
   metadata checks.
7. Keep public examples sanitized:

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
consistent with the Rust types and avoids stale hand-written JSON. Generating
`fixture.toml` from the same fixture constructor keeps documentation,
expectations, privacy assertions, and committed artifacts in sync.
