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
| Per-thread CPU runtime slices | Was the task ready but delayed, or running but consuming too much CPU time? | `--runtime-slices`; writes optional `runtime_slices.json`; diagnosis preset enables it; missing schedstat falls back to `/proc/<tid>/stat` runtime-only records. | Implemented; documented in `docs/ARTIFACT_SCHEMA.md`, validated as optional NDJSON, bounded by `--runtime-slices-max-tasks`, and exposed in `report --analysis-json` as supporting context. |

## Probe Candidates

| Candidate | Diagnostic question | Existing foundation | Admission status |
| --- | --- | --- | --- |
| DRM/KMS pageflip timing | Did KMS pageflip/vblank completion timing line up with frame pacing outliers? | `ProbeKey::KmsPageflipTiming`, `--kms-timing`, `kms_flip_events.json`, session counters, validation, report summary, `stutter doctor --kms-timing`, and DRM/i915/amdgpu pageflip or vblank eBPF handlers are implemented. | Implemented for compatible DRM, i915, or amdgpu tracepoint layouts; missing events are unavailable evidence rather than proof of healthy scanout. |
| GPU scheduler / DRM fence latency correlation | Was frame stutter caused by GPU queue/fence delay rather than CPU runnable delay? | `ProbeKey::DrmFenceLatency`, `--drm-fence-latency`, `drm_fence_events.json`, session counters, validation, report summary, compatibility matrix, discovery command, provider-tagged generic/amdgpu/i915 wait/signal eBPF handlers, and KMS/frame correlation are implemented. | Implemented when compatible wait/signal tracepoints expose stable context/seqno or timeline/seqno identity; missing fence events are unavailable evidence, not proof of no GPU/display wait, and cross-GPU evidence is reported as candidate attribution rather than exact copy latency. |
| Wayland presentation timing | Did commit-to-present delay, discarded frames, output identity, or zero-copy hints correlate with frame outliers? | `ProbeKey::WaylandPresentationTiming`, `--wayland-presentation`, `--wayland-presentation-log`, `wayland-probe` (`--features wayland-probe`), `docs/WAYLAND_PRESENTATION_LOG.md`, live external-log ingestion, `wayland_presentation_events.json`, session counters, validation, and report summary are implemented. | Implemented for cooperative NDJSON logs from Gamescope/compositor/client wrappers and for a self-test surface baseline; arbitrary Wayland clients still require cooperation, and missing presentation events are unavailable evidence rather than proof of healthy presentation timing. |
| Display path cost comparison | What changed between a dGPU-display run and a UHD630-display run? | `ProbeKey::DisplayPathCost` exists as a view-only registry entry; `--display-path-label`, display GPU/connector metadata, and `stutter compare display-path` are implemented. No raw live artifact is emitted. | View-only A/B estimate; not a live probe and not photon-latency wording. Confidence depends on run comparability and optional display timing evidence. |
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

## DRM Fence Latency Admission Gate

Do not implement or enable this probe until:

- diagnosis fixture corpus includes true/false positives for GPU-bound cases
- `report --analysis-json` exposes threshold docs
- rejected-primary explanations exist
- low-quality data caps confidence
- runtime slices are implemented or explicitly ruled out for CPU-vs-GPU separation
- MangoHud/frame alignment tests exist
- vendor tracepoint compatibility matrix is documented
- fallback behavior is documented

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
