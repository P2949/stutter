# Validation corpus

## Purpose

The validation corpus is the committed set of `stutter` run artifacts used to
protect the stable automation interface exposed by:

```bash
cargo run -p stutter -- report --analysis-json <run-dir>
```

The corpus exists to catch regressions in:

* diagnosis selection,
* diagnosis candidate evidence,
* artifact loading,
* artifact count consistency,
* data-quality warnings and errors,
* foreground-window privacy handling,
* community-rules classification,
* serialized `analysis-json` shape.

The corpus is not a benchmark suite. It is a correctness and regression suite.

## Layout

Fixture directories live under:

```text
stutter/tests/fixtures/runs/<fixture-name>/
```

Every fixture directory must contain:

```text
fixture.toml
session.json
metadata.json
```

A fixture may also contain optional NDJSON artifact streams:

```text
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

Selected small public examples live under:

```text
docs/examples/artifacts/v22/
  clean_baseline/
  game_thread_scheduler_delay/
  low_quality_truncated/
  README.md
```

Do not duplicate every large regression fixture under `docs/examples/artifacts/v22/`.
The larger regression corpus belongs under `stutter/tests/fixtures/runs/`.

## Fixture tiers

The corpus has two sources of truth.

### Generated synthetic fixtures

Generated synthetic fixtures live under `stutter/tests/fixtures/runs/` and are
owned by:

```text
stutter/src/test_fixture_builder.rs
```

They are intentionally small and deterministic. They protect low-level contracts
such as loader behavior, artifact counts, schema warnings, data-quality
downgrades, diagnosis selection, foreground handling, reused TID handling, and
community-rules classification.

Current generated synthetic fixtures include:

| Fixture                          | Purpose                                                |
| -------------------------------- | ------------------------------------------------------ |
| `clean_run`                      | Quiet run with no false diagnosis.                     |
| `cpu_pressure`                   | CPU pressure diagnosis contract.                       |
| `block_io_stall`                 | Block I/O diagnosis contract.                          |
| `irq_heavy`                      | IRQ overlap diagnosis contract.                        |
| `gpu_bound_clean_cpu`            | GPU-bound candidate contract with clean CPU pressure.  |
| `truncated_drop_counters`        | Truncation/drop-counter quality downgrade contract.    |
| `reused_tid_no_contamination`    | Reused TID separation contract.                        |
| `old_schema_warning`             | Old schema warning contract.                           |
| `game_thread_scheduler_delay`    | Synthetic game-thread scheduler-delay edge case.       |
| `compositor_scheduler_delay`     | Synthetic compositor scheduler-delay edge case.        |
| `foreground_window`              | Synthetic foreground-window privacy/summary edge case. |
| `community_rules_classification` | Synthetic community-rules classification edge case.    |

### Real sanitized validation recordings

Real sanitized validation recordings also live under:

```text
stutter/tests/fixtures/runs/
```

They must be created from real runs with:

```text
scripts/sanitize-run-artifact.py
```

Do not create fake real recordings in `test_fixture_builder.rs`. The point of
this tier is catching real artifact weirdness that synthetic generation will
miss.

Real sanitized fixture names should normally start with `real_`, for example:

```text
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

## Fixture naming rules

Use clear, stable fixture names.

Rules:

1. Use lowercase snake case.
2. Use `real_` for sanitized recordings derived from real runs.
3. Do not use usernames, hostnames, game titles, project names, or private app
   names in fixture names.
4. Name the regression behavior, not the machine or user that produced it.
5. Prefer names that describe the expected result:

   * `real_game_thread_scheduler_delay`
   * `real_irq_overlap`
   * `real_block_io_overlap`
   * `real_truncated_low_quality`
6. Do not rename existing fixtures unless the old name is actively misleading.
   Fixture names appear in tests, docs, and committed paths.

## `fixture.toml` contract

Every fixture must include:

```text
stutter/tests/fixtures/runs/<fixture-name>/fixture.toml
```

Example:

```toml
name = "real_game_thread_scheduler_delay"
schema_version = 22
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
foreground_events = 0

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

The validation tests also support minimum artifact counts when the exact count is
not important:

```toml
[expected.artifacts]
spikes_min = 1
intervals_min = 1
gpu_samples_min = 0
frames_min = 1
```

Do not set both exact and `_min` forms for the same artifact count unless there
is a specific reason.

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

Use `Any` when the primary diagnosis may legitimately vary but a candidate must
be present. Example:

```toml
[expected]
primary_cause = "Any"
required_candidate = "GpuBoundCandidate"
required_candidate_evidence = ["GPU busy"]
accepted_confidence = []
quality_reasons_contain = []
data_quality = "High"
```

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

For any fixture with `expected.data_quality = "Medium"` or
`expected.data_quality = "Low"`, `expected.quality_reasons_contain` must list
user-facing explanation text that must appear in `data_quality.reasons`,
`data_quality.validation_warnings`, or `data_quality.validation_errors`.

Examples:

```toml
[expected]
primary_cause = "Unknown"
accepted_confidence = []
quality_reasons_contain = ["truncated", "drop"]
data_quality = "Medium"
```

```toml
[expected]
primary_cause = "Unknown"
accepted_confidence = []
quality_reasons_contain = ["older than current"]
data_quality = "Medium"
```

## How to sanitize a real recording

Do not hand-edit large real recording JSON files. Use the sanitizer.

Example:

```bash
scripts/sanitize-run-artifact.py \
  --input /path/to/real/run \
  --output stutter/tests/fixtures/runs/real_game_thread_scheduler_delay \
  --name real_game_thread_scheduler_delay
```

If the output directory already exists:

```bash
scripts/sanitize-run-artifact.py \
  --input /path/to/real/run \
  --output stutter/tests/fixtures/runs/real_game_thread_scheduler_delay \
  --name real_game_thread_scheduler_delay \
  --force
```

By default, the sanitizer verifies the result with:

```bash
cargo run -p stutter -- validate stutter/tests/fixtures/runs/<fixture>
cargo run -p stutter -- report --analysis-json stutter/tests/fixtures/runs/<fixture>
```

Use `--no-verify` only when sanitizing on a machine that cannot build or run the
Rust project. A committed fixture must still pass validation and report
generation before merge.

The sanitizer should preserve diagnosis-relevant fields such as:

```text
TaskClass
timing fields
latency_ns
wakeup_ns
switch_ns
elapsed_ms
cpu_psi_*
IRQ timestamps and durations
block I/O timestamps and durations
GPU busy percentage
frame times
foreground PID/app/class
```

The sanitizer must redact private fields such as:

```text
usernames
home paths
hostnames
command-line paths
window titles
cgroup usernames
Steam library paths
executable paths
browser tab titles
URLs with private query strings
```

## How to add a fixture

### Add a generated synthetic fixture

1. Add a fixture constructor in:

   ```text
   stutter/src/test_fixture_builder.rs
   ```

2. Add it to `write_validation_corpus`.

3. Add a `fixture_metadata_for` entry.

4. Regenerate fixtures:

   ```bash
   cargo test -p stutter regenerate_validation_corpus -- --ignored
   ```

5. Add a `validation_corpus_*` test only when the fixture needs extra
   case-specific assertions beyond the metadata-driven checks.

6. Run:

   ```bash
   cargo test -p stutter validation_corpus
   ```

### Add a real sanitized fixture

1. Capture a real run.

2. Sanitize it:

   ```bash
   scripts/sanitize-run-artifact.py \
     --input /path/to/real/run \
     --output stutter/tests/fixtures/runs/real_example_name \
     --name real_example_name
   ```

3. Add or update:

   ```text
   stutter/tests/fixtures/runs/real_example_name/fixture.toml
   ```

4. Run:

   ```bash
   cargo run -p stutter -- validate stutter/tests/fixtures/runs/real_example_name
   cargo run -p stutter -- report --analysis-json stutter/tests/fixtures/runs/real_example_name
   ```

5. Add a `validation_corpus_*` test only when the fixture needs assertions that
   cannot be expressed in `fixture.toml`.

6. Run:

   ```bash
   cargo test -p stutter validation_corpus
   ```

Do not add a new diagnosis enum variant just to make a fixture pass.

## Required checks per fixture

Every fixture must be covered by metadata-driven checks from `fixture.toml`.

Required checks:

1. Fixture name matches its directory name.
2. Fixture schema version matches `SESSION_SCHEMA_VERSION`.
3. Source is non-empty.
4. Description is non-empty.
5. Expected data quality matches `analysis.data_quality.level`.
6. High-quality fixtures have no validation errors or warnings.
7. Medium/Low-quality fixtures include useful reason, warning, or error text.
8. Expected primary diagnosis is present, absent, `Unknown`, or `Any` according
   to the fixture contract.
9. Expected evidence substrings are present.
10. Required candidate diagnosis is present when declared.
11. Artifact counts or minimum counts match.
12. `report --analysis-json` top-level keys are stable.
13. `data_quality` JSON keys are stable.
14. Privacy expectations are enforced when present.

Additional fixture-specific checks should be added for:

* anchor task class,
* frame correlation,
* IRQ correlation window behavior,
* block I/O correlation basis,
* foreground-window annotation,
* reused TID separation,
* community-rules classification,
* old schema warnings,
* truncation/drop-counter quality behavior.

## Privacy checklist

This checklist is intentionally strict.

No real usernames.

No home paths.

No hostnames.

No real window titles.

No browser tab titles.

No absolute game library paths.

No access tokens.

No URLs with query strings.

No command arguments containing private paths.

No real project names from private directories.

No private executable paths.

No Steam library paths that include local machine layout.

No cgroup names that include real usernames.

No stable personally identifying process, workspace, or window names.

No raw command lines copied from a user machine unless every private component is
redacted.

Foreground titles must be `null` or `"redacted"` unless the fixture intentionally
uses a generic title and the test explicitly documents why.

## How to regenerate synthetic fixtures

Regenerate the committed synthetic validation fixtures with:

```bash
cargo test -p stutter regenerate_validation_corpus -- --ignored
```

Regenerate the selected public v22 examples with:

```bash
cargo test -p stutter regenerate_public_examples_v22 -- --ignored
```

After regeneration, run:

```bash
cargo test -p stutter validation_corpus
```

Commit regenerated JSON, NDJSON, and `fixture.toml` files together with the Rust
changes that changed their shape or expected diagnosis.

Do not run ignored regeneration tests in CI. Regeneration rewrites committed
fixture artifacts and is a maintainer operation.

## How to run corpus tests

Normal validation-corpus test pass:

```bash
cargo test -p stutter validation_corpus
```

CI should run:

```bash
cargo test -p stutter validation_corpus
```

CI should not run:

```bash
cargo test -p stutter regenerate_validation_corpus -- --ignored
cargo test -p stutter regenerate_public_examples_v22 -- --ignored
```

Smoke-check one fixture through the public CLI:

```bash
cargo run -p stutter -- validate stutter/tests/fixtures/runs/real_clean_baseline
cargo run -p stutter -- report --analysis-json stutter/tests/fixtures/runs/real_clean_baseline
```

A typical local corpus maintenance pass is:

```bash
cargo fmt --all
cargo test -p stutter regenerate_validation_corpus -- --ignored
cargo test -p stutter regenerate_public_examples_v22 -- --ignored
cargo test -p stutter validation_corpus
cargo run -p stutter -- validate stutter/tests/fixtures/runs/real_clean_baseline
cargo run -p stutter -- report --analysis-json stutter/tests/fixtures/runs/real_clean_baseline >/dev/null
```

## Why generated fixture metadata matters

The artifact schema is broad. A run may include `session.json`, `metadata.json`,
`spike_events.json`, `interval.json`, `irq_events.json`, `gpu_samples.json`,
`frame_correlation.json`, `io_events.json`, foreground streams, focus streams,
SCX streams, and other event streams.

Generated synthetic fixtures should be created through `test_fixture_builder` so
the JSON shape stays aligned with Rust types.

Real sanitized fixtures should be created through `scripts/sanitize-run-artifact.py`
so privacy redaction is repeatable and the corpus stays trustworthy.
