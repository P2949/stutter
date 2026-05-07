# stutter Artifact Schema

This document describes the on-disk run artifact contract for `stutter`.
The canonical stable machine-readable interface is:

```text
stutter report --analysis-json <run-dir>
```

Raw artifact files are documented for debugging, testing, benchmarking, and
offline automation, but consumers should prefer `report --analysis-json` unless
they specifically need raw event streams.

## Run Directory Layout

A typical run directory contains:

```text
<run-dir>/
  metadata.json
  session.json
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
```

Required:

- `session.json`

Strongly recommended:

- `metadata.json`

Optional streams:

- `interval.json`
- `spike_events.json`
- `tree_events.json`
- `irq_events.json`
- `gpu_samples.json`
- `frame_correlation.json`
- `frame_events.json`
- `migration_events.json`
- `cpu_freq_samples.json`
- `io_events.json`
- `scx_events.json`

## JSON vs NDJSON

`session.json` and `metadata.json` are JSON objects.

All event streams are newline-delimited JSON, also called NDJSON. Each non-empty
line is one complete JSON object. Empty files are valid for optional streams
when the stream is intentionally empty.

Missing optional files should produce validation warnings or data-quality notes,
not immediate parse failures. Invalid optional files that are present are errors.
For example, a malformed `interval.json` makes validation fail, but a missing
`interval.json` may only lower data quality.

## Schema Versioning

The schema version is stored in:

- `session.core.schema_version`, serialized as top-level `schema_version` in
  `session.json`
- `metadata.core.schema_version`, serialized as top-level `schema_version` in
  `metadata.json`

The current supported version is `recorder::SESSION_SCHEMA_VERSION`. For this
document version, that value is `20`.

Version behavior:

- Older schema versions must warn, not hard-fail, if the JSON is still
  structurally deserializable.
- Newer schema versions are incompatible. Validation should fail.
- Metadata and session schema versions should be compatible with each other.

`stutter report --analysis-json` includes these data-quality fields:

- `data_quality.schema_version`
- `data_quality.expected_schema_version`
- `data_quality.validation_warnings`
- `data_quality.validation_errors`

## File Contracts

### `session.json`

Purpose:

- Complete run summary and primary artifact manifest.
- Includes run metadata, config, task summaries, retained top spikes, and counts
  for external artifact streams.

Format: JSON object.

Status: Required.

Version behavior:

- Older deserializable versions warn.
- Newer versions are incompatible and fail validation.

Key fields:

- `schema_version`
- `run_name`
- `duration_ms`
- `active_expanded_tasks`
- `interval_record_count`
- `spike_events_retained_count`
- `spike_events_dropped_count`
- `spike_events_truncated`
- `event_stream_write_errors`
- `first_event_stream_write_error`
- `block_io_correlation_basis`
- `drop_counters`
- `cpu_perf_sample_count`
- `cpu_perf_open_errors`
- `cpu_perf_read_errors`
- `cpu_perf_skipped_tasks`
- `cpu_perf_last_error`

Consistency rules:

- `interval_record_count` should match the number of records in `interval.json`
  when that file is present.
- `spike_events_retained_count` should match `spike_events.json` when present.
- `gpu_sample_count` should match `gpu_samples.json` when present.
- `block_io_event_count` should match `io_events.json` when present.
- `frame_event_count` should match `frame_correlation.json` or
  `frame_events.json` when present.
- `event_stream_write_errors > 0` means stream artifacts may be incomplete.
- Nonzero `drop_counters` means kernel-side event loss occurred.

### `metadata.json`

Purpose:

- Duplicate, compact run metadata and artifact counts.
- Useful for quick scanning, indexing, and validation without reading the full
  task list.

Format: JSON object.

Status: Strongly recommended.

Version behavior:

- Older deserializable versions warn.
- Newer versions are incompatible and fail validation.

Key fields:

- `schema_version`
- `run_name`
- `started_at`
- `ended_at`
- `duration_ms`
- `active_expanded_tasks`
- artifact count fields copied from `session.json`

Consistency rules:

- Counts should match `session.json` where duplicated.
- The schema version should be compatible with the session schema.

### `interval.json`

Purpose:

- Per-interval and per-task summary records.
- Used for time-windowed correlation around spike clusters and frame spikes.

Format: NDJSON.

Status: Optional stream.

Version behavior:

- Missing file is tolerated.
- Invalid present file is an error.

Important fields:

- `elapsed_ms`
- `task`
- `active`
- `class`
- `comm`
- `process_pid`
- `process_comm`
- `samples`
- `p99_ns`
- `max_ns`
- `over_1ms`
- `over_2ms`
- `over_5ms`
- `cpu_psi_some`
- `major_faults`
- `minor_faults`
- `cpu_perf`
- `percentile_scope`
- `histogram`
- `stored_samples`
- `truncated_samples`

Consistency rules:

- The number of records should match `session.json` field
  `interval_record_count`.
- `percentile_scope` describes whether percentile values are exact,
  histogram-estimated, or based on capped samples.

### `spike_events.json`

Purpose:

- Retained scheduler spike events above the configured threshold.
- Preferred raw input for cluster analysis.

Format: NDJSON.

Status: Optional stream.

Version behavior:

- Missing file is tolerated; reports can fall back to `session.top_spikes`.
- Invalid present file is an error.

Important fields:

- `elapsed_ms`
- `task`
- `class`
- `comm`
- `process_pid`
- `latency_ns`
- `wakeup_ns`
- `switch_ns`
- `wakeup_target_cpu`
- `target_pending_wakeups`
- `observed_runnable_depth`
- `waker_tid`
- `waker_comm`
- `cause_tags`
- `primary_cause`

Consistency rules:

- The number of records should match `session.json` field
  `spike_events_retained_count`.
- If `spike_events_truncated` is true, consumers should treat absence of some
  spikes as expected.

### `gpu_samples.json`

Purpose:

- GPU telemetry samples from supported local sources.

Format: NDJSON.

Status: Optional stream.

Version behavior:

- Missing file is tolerated.
- Invalid present file is an error.

Important fields:

- `elapsed_ms`
- `gpu_busy_percent`
- `vram_used_bytes`
- `vram_total_bytes`
- `vram_used_percent`
- `gpu_clock_mhz`
- `mem_clock_mhz`
- `temp_millidegrees`
- `power_microwatts`

Consistency rules:

- The number of records should match `session.json` field `gpu_sample_count`.

### `io_events.json`

Purpose:

- Block-I/O correlation events.

Format: NDJSON.

Status: Optional stream.

Version behavior:

- Missing file is tolerated.
- Invalid present file is an error.

Important fields:

- `elapsed_ms`
- `tid`
- `correlation_basis`
- `dev`
- `sector`
- `nr_sector`
- `duration_ns`
- `timestamp_ns`
- `rwbs`

Consistency rules:

- `session.core.block_io_correlation_basis` describes how these events were
  correlated.
- If the correlation basis is weak or fallback, consumers should treat diagnosis
  confidence cautiously.
- The number of records should match `session.json` field
  `block_io_event_count`.

### `frame_events.json` and `frame_correlation.json`

Purpose:

- Frame timing artifacts for frame-spike diagnosis.

Format: NDJSON.

Status: Optional streams.

Version behavior:

- Missing files are tolerated.
- Invalid present files are errors.

Important fields:

- `elapsed_ms`
- `frametime_ms`

Compatibility notes:

- `frame_correlation.json` is the historical/report file.
- `frame_events.json` is accepted as a stream fallback.
- If both files are present, validation checks both.
- Consumers should use `report --analysis-json` for stable frame diagnosis
  output.

Consistency rules:

- The number of frame records used for reporting should match
  `session.json` field `frame_event_count`.
- If frame timestamps are not anchored to an observed monotonic timestamp,
  frame alignment is approximate and data quality may be downgraded.

## Data-Quality Levels

`DataQualityLevel` has three values:

- `High`: no known data-quality problems.
- `Medium`: the run is usable, but some evidence is approximate, missing,
  truncated, or degraded.
- `Low`: validation errors, incompatible schema, or recording write errors make
  the run unreliable for automation.

Likely downgrade reasons include:

- missing optional metadata or artifact files
- invalid optional artifacts
- schema warnings or errors
- spike event truncation
- event stream write errors
- nonzero eBPF drop counters
- percentile truncation or approximation
- block-I/O correlation basis limitations
- frame timestamp alignment issues
- CPU perf open, read, or skipped-task status

## Canonical Interface: `report --analysis-json`

`stutter report --analysis-json <run-dir>` is the preferred stable machine
interface. It includes:

- `session`
- `cluster_analysis`
- `frame_diagnoses`
- `artifacts_summary`
- `data_quality`

External automation should prefer this over parsing raw report text. Text
reports and HTML reports are user-facing and less stable. Raw artifacts are
lower-level debugging and forensics inputs.

## Validation Command

Expected command forms:

```text
stutter validate <run-dir>
stutter validate --json <run-dir>
stutter validate --strict <run-dir>
```

`<run-dir>` may also be a direct path to `session.json`; the run directory is
then inferred from its parent.

Output modes:

- Default output is concise, grep-friendly human text.
- `--json` emits only structured JSON.

Default exit policy:

- Exits `0` for compatible `High` quality runs.
- Exits `0` for compatible `Medium` quality runs when no hard validation errors
  are present.
- Exits nonzero when validation errors are present.
- Exits nonzero when report analysis cannot be built.
- Exits nonzero when `data_quality.level == Low`.
- Exits nonzero when `data_quality.validation_errors` is non-empty.
- Exits nonzero when the schema is newer than supported.

Strict exit policy:

- Includes all default failures.
- Exits nonzero for validation warnings.
- Exits nonzero for missing optional files.
- Exits nonzero when `data_quality.level != High`.
- Exits nonzero for event stream write errors.
- Exits nonzero for truncated spike events.
- Exits nonzero for nonzero eBPF drop counters.

## Versioned Examples

Versioned examples live under:

```text
docs/examples/artifacts/v20/
```

The version number matches `recorder::SESSION_SCHEMA_VERSION`. These examples
are sanitized, minimal, and covered by tests so they remain executable artifact
contracts.
