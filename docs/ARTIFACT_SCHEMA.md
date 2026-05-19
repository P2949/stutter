# stutter Artifact Schema

This document describes the on-disk run artifact contract for `stutter`.
The canonical stable machine-readable interface is:

```text
stutter report --analysis-json <run-dir>
```

Raw artifact files are documented for debugging, testing, benchmarking, and
offline automation, but consumers should prefer `report --analysis-json` unless
they specifically need raw event streams.

## Canonical Artifact Registry

The canonical artifact list is `stutter/src/artifacts.rs`.

Any patch that adds, renames, or removes a run artifact must update
`ArtifactKind`, `ArtifactSpec`, session writing, session loading, validation,
report data-quality behavior, and probe registry references in the same change.

`frame_events.json` is the canonical frame event stream.
`frame_correlation.json` remains a legacy alias for compatibility with older
recordings.

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
  runtime_slices.json
  focus_events.json
  foreground_events.json
  kms_flip_events.json
  drm_fence_events.json
  wayland_presentation_events.json
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
- `runtime_slices.json`
- `focus_events.json`
- `foreground_events.json`
- `kms_flip_events.json`
- `drm_fence_events.json`
- `wayland_presentation_events.json`

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
document version, that value is `21`.

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
- `runtime_slice_count`
- `runtime_slice_read_errors`
- `runtime_slice_skipped_tasks`
- `runtime_slice_source`
- `focus_event_count`
- `foreground_event_count`
- `kms_flip_event_count`
- `drm_fence_event_count`
- `wayland_presentation_event_count`
- `display_path`
- `config.kms_timing`
- `config.kms_card`
- `config.kms_connector`
- `config.kms_crtc`
- `config.drm_fence_latency`
- `config.drm_fence_render_card`
- `config.drm_fence_display_card`
- `config.drm_fence_driver`
- `config.wayland_presentation`
- `config.wayland_presentation_log`
- `config.wayland_presentation_source`
- `config.display_path_label`
- `config.display_render_gpu`
- `config.display_scanout_gpu`
- `config.display_connector`
- `foreground_source`
- `final_foreground_pid`
- `final_foreground_app_id`
- `final_foreground_class`

Consistency rules:

- `interval_record_count` should match the number of records in `interval.json`
  when that file is present.
- `spike_events_retained_count` should match `spike_events.json` when present.
- `gpu_sample_count` should match `gpu_samples.json` when present.
- `block_io_event_count` should match `io_events.json` when present.
- `runtime_slice_count` should match `runtime_slices.json` when present.
- `frame_event_count` should match `frame_correlation.json` or
  `frame_events.json` when present.
- `foreground_event_count` should match `foreground_events.json` when present.
- `kms_flip_event_count` should match `kms_flip_events.json` when present.
- `drm_fence_event_count` should match `drm_fence_events.json` when present.
- `wayland_presentation_event_count` should match
  `wayland_presentation_events.json` when present.
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
- `focus_event_count`
- `foreground_event_count`
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
- `mem_psi_some`
- `mem_psi_full`
- `io_psi_some`
- `io_psi_full`
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

Notes:

- `target_pending_wakeups` and `observed_runnable_depth` are monitored-target
  diagnostic approximations. They are not literal kernel runqueue depth and
  must not be interpreted as `rq->nr_running`.

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

### `runtime_slices.json`

Purpose:

- Per-thread CPU runtime/wait deltas sampled from procfs.
- Adds context for whether a target was runnable-but-waiting or actually
  consuming CPU time near a spike.
- Used as supporting diagnosis evidence only.

Format: NDJSON.

Status: Optional stream. Written when `--runtime-slices` is enabled, including
through the `diagnosis` preset unless disabled with `--no-runtime-slices`.

Version behavior:

- Missing file is tolerated and should lower confidence rather than prove
  anything.
- Invalid present file is an error.

Important fields:

- `elapsed_ms`
- `task`
- `process_pid`
- `class`
- `comm`
- `process_comm`
- `source`: `proc_schedstat` or `proc_stat_fallback`
- `interval_ms`
- `runtime_delta_ns`
- `runqueue_wait_delta_ns`
- `timeslices_delta`
- `user_runtime_delta_ns`
- `system_runtime_delta_ns`
- `runtime_ratio`
- `wait_ratio`
- `avg_runtime_per_slice_ns`
- `avg_wait_per_slice_ns`
- `valid`
- `unavailable_reason`

Notes:

- `/proc/<pid>/task/<tid>/schedstat` provides runtime, runqueue wait time, and
  timeslice count in nanoseconds/counts.
- If schedstat is unavailable, `/proc/<pid>/task/<tid>/stat` is used as a
  runtime-only fallback. Fallback records do not include runqueue wait data.
- First samples establish a baseline and are not emitted as fake zero-delta
  records.

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

- If frame timestamps are not anchored to an observed monotonic timestamp,
  frame alignment is approximate and data quality may be downgraded.

### `foreground_events.json`

Purpose:

- Foreground-window context from compositor/X11 providers.
- Used to correlate scheduler/frame spikes with the user-visible active app.

Format: NDJSON.

Status: Optional stream.

Version behavior:

- Missing file is tolerated unless foreground collection was requested.
- Invalid present file is an error.

Important fields:

- `elapsed_ms`
- `source`
- `status`
- `pid`
- `app_id`
- `class`
- `title`
- `window_id`
- `workspace`
- `confidence`
- `reason`

Privacy:

- `title` is `null` unless `--foreground-include-title` is set.
- Browser tab titles and terminal titles can leak private data; consumers must
  not assume titles are present.

Consistency rules:

- The number of records should match `session.json` field
  `foreground_event_count`.
- The final foreground identity fields in `session.json` are derived from the
  last recorded foreground event.

### `kms_flip_events.json`

Purpose:

- Optional DRM/KMS pageflip, vblank, and flip-duration evidence.
- Helps correlate frame outliers with scanout/pageflip completion timing.

Format: NDJSON.

Status: Optional stream.

Version behavior:

- Missing file is tolerated and means the probe was unavailable or disabled.
- Invalid present file is an error.
- Missing KMS events are not proof that scanout timing was healthy.
- Live collection currently emits from compatible DRM, i915, or amdgpu pageflip/vblank tracepoints.

Important fields:

- `elapsed_ms`
- `timestamp_ns`
- `source`
- `card`
- `driver`
- `crtc_id`
- `connector`
- `event_kind`
- `sequence`
- `request_ns`
- `done_ns`
- `duration_ns`
- `flags`
- `confidence`

Consistency rules:

- The number of records should match `session.json` field
  `kms_flip_event_count` when present.
- Report analysis exposes a derived `kms_timing.scanout_window_estimate` summary
  from consecutive `done_ns` timestamps. It estimates top-of-screen visibility at
  `pageflip_done_ns` and bottom-of-screen visibility at
  `pageflip_done_ns + refresh_period_ns`; this is not photon latency and excludes
  monitor processing and pixel response.

### `drm_fence_events.json`

Purpose:

- Optional DRM/dma-fence wait, signal, and interval evidence.
- Helps identify GPU queue/fence delay near frame or KMS timing outliers.

Format: NDJSON.

Status: Optional stream.

Version behavior:

- Missing file is tolerated and means the probe was unavailable or disabled.
- Invalid present file is an error.
- Missing fence events are not proof that no GPU/display wait occurred.
- Live collection can tag generic dma-fence/drm-sched, amdgpu render-side, and
  i915 display-side providers when compatible tracepoints expose stable identity
  fields.

Important fields:

- `elapsed_ms`
- `timestamp_ns`
- `source`
- `event_kind`
- `driver`
- `card`
- `gpu_role`
- `pid`
- `tid`
- `comm`
- `context`
- `seqno`
- `timeline_hash`
- `wait_start_ns`
- `wait_done_ns`
- `duration_ns`
- `exporter_driver`
- `importer_driver`
- `correlation_basis`
- `confidence`

Consistency rules:

- The number of records should match `session.json` field
  `drm_fence_event_count` when present.
- `importer_driver` describes the wait side of an interval. `exporter_driver`
  describes a matched signal side when the same fence key was observed. When
  both are present, reports may emit `cross_gpu_display_wait_candidate` style
  evidence, but must not treat it as exact copy latency.
- `correlation_basis=context_seqno` is the strongest supported key.
  `timeline_seqno` and `driver_time_overlap` are weaker supporting evidence.

### `wayland_presentation_events.json`

Purpose:

- Optional Wayland presentation feedback or cooperative compositor log events.
- Helps correlate commit-to-present delay, discarded frames, output identity,
  and zero-copy/direct-scanout hints with frame outliers.

Format: NDJSON.

Status: Optional stream.

Version behavior:

- Missing file is tolerated and means no cooperative presentation source was
  available or enabled.
- Invalid present file is an error.
- Missing Wayland presentation events are not proof that presentation timing was
  healthy.
- Cooperative log producers should follow `docs/WAYLAND_PRESENTATION_LOG.md`.
- The `wayland-probe` self-test command writes the same stream for stutter's own
  test surface when the binary is built with `--features wayland-probe`.

Important fields:

- `elapsed_ms`
- `source`
- `app_id`
- `surface_role`
- `commit_ns`
- `presented_ns`
- `commit_to_present_ns`
- `output_name`
- `refresh_ns`
- `sequence`
- `zero_copy`
- `discarded`
- `flags`
- `confidence`

Consistency rules:

- The number of records should match `session.json` field
  `wayland_presentation_event_count` when present.

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
- malformed foreground-window event artifacts
- CPU perf open, read, or skipped-task status
- degraded DRM fence evidence, including missing tracepoint streams,
  signal-only events, unstable fence keys, unknown driver/role mapping, missing
  render/display card identity, or kernel event drops

## Canonical Interface: `report --analysis-json`

`stutter report --analysis-json <run-dir>` is the preferred stable machine
interface. It includes:

- `session`
- `cluster_analysis`
- `frame_diagnoses`
- `frame_pacing`
- `pressure_timeline`
- `artifacts_summary`
- `data_quality`
- `focus_summary`
- `foreground_summary`
- `kms_timing`
- `drm_fence_timing`
- `wayland_presentation`

External automation should prefer this over parsing raw report text. Text
reports and HTML reports are user-facing and less stable. Raw artifacts are
lower-level debugging and forensics inputs.

### `pressure_timeline`

`pressure_timeline` is derived from `interval.json`. It is not a raw artifact
file. Reports populate it from the bounded interval records loaded around spike
clusters and frame-spike correlation windows. If interval data is unavailable or
no relevant windows are loaded, the summary is empty and reports still succeed.

Fields:

- `sample_count`: number of interval records considered.
- `max_cpu_some`: maximum `cpu_psi_some` value in the considered intervals.
- `max_mem_some`: maximum `mem_psi_some` value, or `null` when no memory PSI
  fields are available.
- `max_mem_full`: maximum `mem_psi_full` value, or `null` when unavailable.
- `max_io_some`: maximum `io_psi_some` value, or `null` when unavailable.
- `max_io_full`: maximum `io_psi_full` value, or `null` when unavailable.
- `windows[]`: interval records sorted by `elapsed_ms`, with CPU, memory, and
  I/O pressure values.
- `windows[].near_spike`: true when the interval lies within the configured
  cluster window around a scheduler spike cluster.
- `peak_windows[]`: highest pressure windows across CPU, memory, and I/O PSI
  streams for display.
- `pressure_notes[]`: conservative display-only notes about high pressure or
  missing PSI coverage.
- `coverage`: interval/PSI coverage flags, including whether any loaded
  pressure window was near a spike cluster.

### `frame_pacing`

`frame_pacing` is derived from `frame_events.json` / `frame_correlation.json`,
spike clusters, foreground events, and task classes. It is not a raw artifact
and does not require a new probe.

Fields:

- `frame_count`: number of frame events loaded.
- `median_frametime_ms`: median frame time, or `null` when unavailable.
- `p95_frametime_ms`: p95 frame time, or `null` when unavailable.
- `p99_frametime_ms`: p99 frame time, or `null` when unavailable.
- `max_frametime_ms`: maximum frame time, or `null` when unavailable.
- `outlier_count`: number of frame events above the display outlier threshold.
- `outliers[]`: frame outliers linked to the nearest scheduler cluster and
  foreground context when available.
- `compositor_cluster_count`: scheduler clusters anchored on compositor or
  gamescope tasks.
- `game_cluster_count`: scheduler clusters anchored on game-related tasks.
- `notes[]`: display-only notes for missing frame events or notable cluster
  context.

### Display Timing Summaries

`kms_timing`, `drm_fence_timing`, and `wayland_presentation` are derived from
optional display-timing streams. Missing streams are tolerated and reported as
missing or low-confidence evidence, not as proof that scanout, fence waits, or
presentation timing were healthy.

Important `drm_fence_timing` fields:

- `event_count`
- `wait_interval_count`
- `median_wait_ms`
- `p95_wait_ms`
- `p99_wait_ms`
- `max_wait_ms`
- `render_gpu_wait_count`
- `display_gpu_wait_count`
- `cross_gpu_candidate_count`
- `waits_near_frame_outliers`
- `waits_near_kms_delays`
- `top_waits[]`
- `notes[]`
- `confidence`

DRM fence data quality downgrades to Medium when requested evidence is missing,
only signal/marker events are present, stable fence keys are absent, provider or
GPU-role mapping is incomplete, render/display cards are not both identified, or
kernel drop counters indicate loss. Missing fence events must never be reported
as proof that no GPU wait occurred.

Important `wayland_presentation` fields:

- `event_count`
- `presented_count`
- `discarded_count`
- `zero_copy_count`
- `zero_copy_ratio`
- `source_counts`
- `surface_role_counts`
- `median_commit_to_present_ms`
- `p95_commit_to_present_ms`
- `p99_commit_to_present_ms`
- `max_commit_to_present_ms`
- `delays_near_frame_outliers`
- `delays_near_kms_delays`
- `compositor_queue_candidate_count`
- `outputs_seen`
- `notes[]`

### Display Path Comparison

`stutter compare display-path --baseline <run> --test <run>` compares two
controlled runs, such as a dGPU-display baseline and a UHD630/i915 scanout test.
Runs can carry display metadata from:

```text
--display-path-label
--display-render-gpu
--display-scanout-gpu
--display-connector
```

The command reports frame-pacing, KMS, DRM-fence, Wayland-presentation, and
scheduler-control deltas. It refuses to frame one run as a high-confidence
estimate, downgrades confidence for non-comparable runs, and uses cautious
wording: this is an A/B estimate, not direct photon latency.

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
docs/examples/artifacts/v21/
```

The version number matches `recorder::SESSION_SCHEMA_VERSION`. These examples
are sanitized, minimal, and covered by tests so they remain executable artifact
contracts.
