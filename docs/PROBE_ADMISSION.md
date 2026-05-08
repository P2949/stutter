# Probe Admission Policy

## Core Rule

New probes must answer a specific diagnostic question. Do not add telemetry just because it is available.

## Required Checklist

Before a probe is accepted, it must have:

- Clear question answered
- User-facing CLI flag or preset behavior defined
- Default-off unless overhead is negligible and failure mode is harmless
- Artifact schema documented in docs/ARTIFACT_SCHEMA.md
- Versioned example artifact added under docs/examples/artifacts/
- DataQualitySummary behavior defined
- `stutter validate` behavior defined
- `report --analysis-json` output contract updated
- Diagnosis wording remains cautious
- Offline fixture/replay test added
- Doctor/preflight behavior added if permissions/kernel support may fail
- Overhead and cardinality bounded
- Works without root in tests
- Missing/unavailable probe degrades gracefully

## Admitted Probes

| Probe | Diagnostic question | CLI / artifact contract | Admission status |
| --- | --- | --- | --- |
| Foreground window context | Which application/window was foreground near scheduler or frame spikes? | `--foreground-window` and `--focus-source foreground`/`hybrid`; writes optional `foreground_events.json`; titles are redacted unless `--foreground-include-title` is set. | Implemented; documented in `docs/ARTIFACT_SCHEMA.md`, reported through `foreground_summary`, validated as optional NDJSON, and exposed in `stutter probes`. |

## Probe Candidates

| Candidate | Diagnostic question | Existing foundation | Admission status |
| --- | --- | --- | --- |
| GPU scheduler / DRM fence latency correlation | Was frame stutter caused by GPU queue/fence delay rather than CPU runnable delay? | MangoHud frames + GPU hwmon exist, but DRM fence telemetry not yet present. | Later; needs kernel/driver-specific design and fixtures. |
| Per-thread CPU runtime slices | Was the task ready but delayed, or running but consuming too much CPU time? | Scheduler runnable latency exists; interval task summaries exist. | Good first targeted probe if implemented through low-risk procfs/task CPU deltas or sched runtime tracepoint. |
| Pressure-stall timeline overlay | Did CPU/memory/I/O pressure line up with spikes? | `psi.rs` already reads PSI; interval records already carry some PSI. | Prefer report/view improvement before new probe. |
| Perf counter presets | Was the workload low IPC/cache-miss bound? | `perf_counters.rs` already exists and is optional. | Prefer preset UX/docs before new counters. |
| Compositor/frame-pacing views | Did gamescope/sway/KDE frame pacing correlate with spikes? | MangoHud parsing and task classes already exist. | Prefer report/view improvement before new probes. |

## DRM Fence Latency Design Questions

- Which tracepoints are stable across AMD/Intel/NVIDIA?
- Can events be tied to a process/thread or only GPU queue?
- What is the artifact schema?
- How does it align with MangoHud frames?
- What is the fallback when DRM tracepoints are unavailable?
- What are false-positive tests?

## Perf Counter Presets

CPU perf counters already exist behind `--cpu-perf` and collect cycles, instructions, cache misses, and optionally cache references. Future presets must remain optional and bounded.

Potential preset designs, documentation only:

- `ipc-basic`: cycles + instructions
- `cache-mpki`: cycles + instructions + cache misses
- `cache-rate`: cycles + instructions + cache misses + cache references when available

## Compositor And Frame-Pacing Views

Compositor/frame-pacing work should be admitted as a report/view feature before adding compositor-specific probes. Potential later outputs include:

- frame outliers near compositor spikes
- gamescope/sway/KDE task-class grouping
- frame-time histogram around scheduler clusters

## Explicit Non-Goals

- Do not add generic tracepoints without a diagnosis path.
- Do not add unbounded event streams.
- Do not make optional probes required for basic reports.
- Do not make live kernel/GPU availability required for tests.
- Do not expose private foreground-window titles unless the user explicitly opts in.
- Do not silently turn on high-overhead probes in presets.
