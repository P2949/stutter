# DRM Fence Compatibility Matrix

This matrix records which kernel DRM/fence tracepoints can support
`--drm-fence-latency`. Missing events are unavailable evidence, not proof that a
GPU/display fence wait did not happen.

Initial target:

- render GPU: amdgpu / RX 9070 XT
- display GPU: i915 / UHD630
- mode: Gamescope DRM session where possible
- monitor: single output

Use `stutter inspect-drm-tracepoints` to collect the local tracepoint inventory
without starting a monitored game.

| Kernel version | Driver | Tracepoint category | Tracepoint name | Fields needed | Fields present | Confidence | Notes |
|---|---|---|---|---|---|---|---|
| local | generic | `dma_fence` | discovered at runtime | `context`, `seqno`, wait/signal timestamp from event time | run `stutter inspect-drm-tracepoints` | preferred when present | Generic dma-fence tracepoints are the best basis for cross-driver correlation. |
| local | generic | `drm_sched` | discovered at runtime | job/fence identity, queue/start/done events | run `stutter inspect-drm-tracepoints` | medium | Useful for render queue delay, weaker for display-import attribution. |
| local | amdgpu | `amdgpu` | discovered at runtime | job or fence identity, queue/start/finish events | run `stutter inspect-drm-tracepoints` | medium | Supports render GPU queue/fence delay wording, not exact copy latency. |
| local | i915 | `i915` | discovered at runtime | wait begin/end, context/seqno or equivalent identity | run `stutter inspect-drm-tracepoints` | medium/high when identity is stable | Display-side imported/shared-buffer waits are the target signal for UHD630 scanout. |
| local | dma-buf/sync-file | `dma_buf`, `sync_file` | discovered at runtime | fence export/import or wait identity | run `stutter inspect-drm-tracepoints` | low/medium | Useful supporting evidence when driver-specific identity is incomplete. |

Admission notes:

- i915 display wait: supported when i915 wait begin/end tracepoints expose a
  stable context/seqno, timeline, or equivalent key plus duration or paired
  timestamps.
- amdgpu scheduler: supported when amdgpu/drm_sched tracepoints expose a stable
  job/fence identity and queue/start/done or signal timing.
- generic dma_fence: preferred when available because it can avoid vendor-only
  naming and improve cross-driver correlation.
- Runtime provider tagging maps amdgpu tracepoints to `gpu_role=render`, i915
  tracepoints to `gpu_role=display`, and drm_sched tracepoints to render-side
  supporting evidence.
- When a display-side wait interval observes a previously signaled fence with the
  same key, the interval records the wait provider as `importer_driver` and the
  signal provider as `exporter_driver`.
- Reports preserve `signal_ns` for matched fences and derive
  `cross_gpu_fence` candidates when display-side waits, KMS/pageflip timing, and
  frame outliers line up. This is stronger attribution than a raw wait duration,
  but still not a direct measurement of copy latency or photon latency.
- If no stable key exists, only low-confidence signal or overlap evidence should
  be emitted.
- Reports must say `render GPU queue/fence delay`, `display-side fence wait`, or
  `cross_gpu_display_wait_candidate`; they must not claim exact copy latency.
