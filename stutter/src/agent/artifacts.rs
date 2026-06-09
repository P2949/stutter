//! Agent artifact allowlist and validation boundary.

pub(crate) const AGENT_ARTIFACT_ALLOWLIST: &[&str] = &[
    "metadata.json",
    "session.json",
    "interval.json",
    "spike_events.json",
    "tree_events.json",
    "irq_events.json",
    "gpu_samples.json",
    "frame_correlation.json",
    "frame_events.json",
    "migration_events.json",
    "cpu_freq_samples.json",
    "io_events.json",
    "scx_events.json",
    "runtime_slices.json",
    "foreground_events.json",
];
