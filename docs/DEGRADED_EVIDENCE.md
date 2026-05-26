# Degraded Evidence Guide

`stutter` reports combine direct kernel/runtime evidence with optional probes and
best-effort correlations. Missing or approximate evidence should reduce
confidence instead of making a report unusable.

## Missing Optional Artifacts

Some artifacts are optional because the probe was not requested, the kernel did
not expose the source, or the run was collected by an older version. Reports
should keep rendering and mark the related section as unavailable or degraded.

Common examples:

- `gpu_samples.json` missing: GPU pressure is unknown.
- `frame_events.json` missing: frame pacing can be summarized only from other
  latency signals.
- `display_topology.json` missing: display-path diagnosis cannot confirm render
  and scanout GPUs.
- `foreground_events.json` or `focus_events.json` missing: foreground/focus
  attribution has lower confidence.

## DRM/KMS Unavailable

KMS and DRM fence tracepoints vary by kernel and driver. If selected tracepoints
are missing or lack required fields, reports must say that display timing is
unavailable rather than treating the absence as a clean display path.

When DRM fence evidence is missing, cross-GPU or fence-wait conclusions should
be marked as missing evidence. When only partial fence identity is present, the
summary should use approximate/degraded language.

## MangoHud Alignment Uncertainty

MangoHud CSV rows may start mid-file, use unexpected headers, omit frame
timestamps, or include rows that cannot be aligned to the monitor clock. Reports
should distinguish direct frame timing from aligned or plausibility-filtered
samples.

Alignment uncertainty should reduce confidence in frame-specific diagnosis but
should not erase scheduler, CPU, or block-I/O evidence collected at the same
time.

## CPU Accounting Untracked

The eBPF runnable-depth accounting tracks CPU ids below
`BPF_MAX_TRACKED_CPUS`. If the machine exposes a possible CPU id outside that
range, latency events remain safe, but target-local runnable-depth and pending
wakeup counters are skipped for those CPU ids.

Skipped CPU accounting is counted with `DROP_CPU_ACCOUNTING_UNTRACKED`. Reports
should treat nonzero values as degraded scheduler context, not necessarily as
lost latency events.

## Tracepoint Format Mismatches

Kernel tracepoint layouts can differ by kernel version, architecture, or distro
patches. Preflight checks validate required fields and offsets before the probe
is used.

If a required tracepoint is missing or incompatible, the monitor should fail or
disable the dependent optional probe. Reports should include the warning so bug
reports can show exactly which kernel field was unavailable.

## Drop Counters

Nonzero eBPF drop counters indicate that some internal state or event delivery
was lost. The authoritative list of drop-counter labels lives in
`stutter_common::DROP_COUNTER_METADATA`.

Reports and validation should surface nonzero counters as data-quality reasons.
Some counters affect only optional context, while ring-buffer reserve failures
can mean event loss for the core stream.
