use serde::Serialize;

use crate::{
    artifacts::ArtifactKind,
    probe_catalog::{ProbeOverhead, ProbeStatus},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeKey {
    SchedulerRunnableLatency,
    IrqLatency,
    GpuHwmon,
    FrameLog,
    ForegroundWindow,
    BlockIo,
    CpuFreq,
    Faults,
    CpuPerf,
    PsiTimeline,
    PressureStallTimelineOverlay,
    RuntimeSlices,
    KmsPageflipTiming,
    DrmFenceLatency,
    WaylandPresentationTiming,
    DisplayPathCost,
    PerfCounterPresets,
    CompositorFramePacingViews,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeCapability {
    Ebpf,
    Tracepoint,
    PerfEvent,
    Procfs,
    Hwmon,
    ExternalLog,
    WindowSystem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EbpfProgramSpec {
    pub name: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TracepointSpec {
    pub category: &'static str,
    pub name: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PerfEventSpec {
    pub name: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DataQualityRule {
    pub key: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ProbeSpec {
    pub key: ProbeKey,
    pub catalog_key: &'static str,
    pub title: &'static str,
    pub status: ProbeStatus,
    pub answers_question: &'static str,
    pub cli_flags: &'static [&'static str],
    pub artifacts: &'static [ArtifactKind],
    pub default_enabled: bool,
    pub overhead: ProbeOverhead,
    pub required_capabilities: &'static [ProbeCapability],
    pub ebpf_programs: &'static [EbpfProgramSpec],
    pub tracepoints: &'static [TracepointSpec],
    pub perf_events: &'static [PerfEventSpec],
    pub data_quality_rules: &'static [DataQualityRule],
    pub validation_contract: &'static str,
}

pub const PROBE_REGISTRY: &[ProbeSpec] = &[
    ProbeSpec {
        key: ProbeKey::SchedulerRunnableLatency,
        catalog_key: "scheduler_runnable_latency",
        title: "Scheduler runnable latency",
        status: ProbeStatus::Implemented,
        answers_question: "Was a monitored task ready to run but delayed before CPU time?",
        cli_flags: &["core"],
        artifacts: &[
            ArtifactKind::Session,
            ArtifactKind::SpikeEvents,
            ArtifactKind::Interval,
            ArtifactKind::TreeEvents,
            ArtifactKind::MigrationEvents,
        ],
        default_enabled: true,
        overhead: ProbeOverhead::Medium,
        required_capabilities: &[ProbeCapability::Ebpf, ProbeCapability::Tracepoint],
        ebpf_programs: &[
            EbpfProgramSpec {
                name: "sched_wakeup",
            },
            EbpfProgramSpec {
                name: "sched_wakeup_new",
            },
            EbpfProgramSpec {
                name: "sched_switch",
            },
            EbpfProgramSpec {
                name: "sched_process_exit",
            },
            EbpfProgramSpec {
                name: "sched_migrate_task",
            },
            EbpfProgramSpec {
                name: "sched_process_exec",
            },
        ],
        tracepoints: &[
            TracepointSpec {
                category: "sched",
                name: "sched_wakeup",
            },
            TracepointSpec {
                category: "sched",
                name: "sched_wakeup_new",
            },
            TracepointSpec {
                category: "sched",
                name: "sched_switch",
            },
            TracepointSpec {
                category: "sched",
                name: "sched_process_exit",
            },
            TracepointSpec {
                category: "sched",
                name: "sched_migrate_task",
            },
            TracepointSpec {
                category: "sched",
                name: "sched_process_exec",
            },
        ],
        perf_events: &[],
        data_quality_rules: &[DataQualityRule {
            key: "scheduler_artifacts",
            description: "session, spike, and interval records must remain schema-compatible",
        }],
        validation_contract: "session, spike, and interval records are validated by stutter validate and reported through analysis JSON.",
    },
    ProbeSpec {
        key: ProbeKey::IrqLatency,
        catalog_key: "irq_latency",
        title: "IRQ latency",
        status: ProbeStatus::Implemented,
        answers_question: "Did a selected IRQ handler overlap scheduler or frame spikes?",
        cli_flags: &["--irq-latency --irq <IRQ>"],
        artifacts: &[ArtifactKind::IrqEvents],
        default_enabled: false,
        overhead: ProbeOverhead::Medium,
        required_capabilities: &[ProbeCapability::Ebpf, ProbeCapability::Tracepoint],
        ebpf_programs: &[
            EbpfProgramSpec {
                name: "irq_handler_entry",
            },
            EbpfProgramSpec {
                name: "irq_handler_exit",
            },
        ],
        tracepoints: &[
            TracepointSpec {
                category: "irq",
                name: "irq_handler_entry",
            },
            TracepointSpec {
                category: "irq",
                name: "irq_handler_exit",
            },
        ],
        perf_events: &[],
        data_quality_rules: &[DataQualityRule {
            key: "irq_optional_stream",
            description: "missing IRQ stream degrades optional IRQ evidence only",
        }],
        validation_contract: "irq_events.json is optional NDJSON; missing files degrade gracefully and present files validate as IrqEventRecord.",
    },
    ProbeSpec {
        key: ProbeKey::GpuHwmon,
        catalog_key: "gpu_hwmon",
        title: "GPU hwmon",
        status: ProbeStatus::Implemented,
        answers_question: "Was the GPU busy, thermally limited, or clock-limited near spikes?",
        cli_flags: &["--hwmon"],
        artifacts: &[ArtifactKind::GpuSamples],
        default_enabled: false,
        overhead: ProbeOverhead::Low,
        required_capabilities: &[ProbeCapability::Hwmon],
        ebpf_programs: &[],
        tracepoints: &[],
        perf_events: &[],
        data_quality_rules: &[DataQualityRule {
            key: "hwmon_optional",
            description: "missing hwmon data is a degraded evidence condition, not proof of no GPU pressure",
        }],
        validation_contract: "gpu_samples.json is optional NDJSON; hwmon absence is a doctor/preflight condition and reports tolerate missing samples.",
    },
    ProbeSpec {
        key: ProbeKey::FrameLog,
        catalog_key: "frame_log",
        title: "MangoHud frame log",
        status: ProbeStatus::Implemented,
        answers_question: "Did frame-time outliers line up with scheduler or system evidence?",
        cli_flags: &["--mangohud-log <PATH>"],
        artifacts: &[ArtifactKind::FrameEvents],
        default_enabled: false,
        overhead: ProbeOverhead::Low,
        required_capabilities: &[ProbeCapability::ExternalLog],
        ebpf_programs: &[],
        tracepoints: &[],
        perf_events: &[],
        data_quality_rules: &[DataQualityRule {
            key: "frame_timestamp_alignment",
            description: "frame data quality depends on timestamp alignment metadata",
        }],
        validation_contract: "frame_events.json is the canonical optional NDJSON frame contract; frame_correlation.json is a legacy alias.",
    },
    ProbeSpec {
        key: ProbeKey::ForegroundWindow,
        catalog_key: "foreground_window",
        title: "Foreground window context",
        status: ProbeStatus::Implemented,
        answers_question: "Which application/window was foreground near scheduler or frame spikes?",
        cli_flags: &["--foreground-window / --focus-source foreground"],
        artifacts: &[ArtifactKind::ForegroundEvents, ArtifactKind::FocusEvents],
        default_enabled: false,
        overhead: ProbeOverhead::Low,
        required_capabilities: &[ProbeCapability::WindowSystem],
        ebpf_programs: &[],
        tracepoints: &[],
        perf_events: &[],
        data_quality_rules: &[DataQualityRule {
            key: "foreground_optional",
            description: "missing foreground data is tolerated unless foreground collection was requested",
        }],
        validation_contract: "foreground_events.json is optional NDJSON; missing file is tolerated unless foreground collection was requested; window titles are redacted by default.",
    },
    ProbeSpec {
        key: ProbeKey::BlockIo,
        catalog_key: "block_io",
        title: "Block I/O",
        status: ProbeStatus::Implemented,
        answers_question: "Did block I/O completion latency overlap spikes?",
        cli_flags: &["--block-io"],
        artifacts: &[ArtifactKind::BlockIoEvents],
        default_enabled: false,
        overhead: ProbeOverhead::Medium,
        required_capabilities: &[ProbeCapability::Ebpf, ProbeCapability::Tracepoint],
        ebpf_programs: &[
            EbpfProgramSpec {
                name: "block_rq_issue",
            },
            EbpfProgramSpec {
                name: "block_rq_complete",
            },
        ],
        tracepoints: &[
            TracepointSpec {
                category: "block",
                name: "block_rq_issue",
            },
            TracepointSpec {
                category: "block",
                name: "block_rq_complete",
            },
        ],
        perf_events: &[],
        data_quality_rules: &[DataQualityRule {
            key: "block_io_correlation_basis",
            description: "correlation basis is recorded and approximate matching downgrades quality",
        }],
        validation_contract: "io_events.json is optional NDJSON; correlation basis is documented and approximate matching downgrades data quality.",
    },
    ProbeSpec {
        key: ProbeKey::CpuFreq,
        catalog_key: "cpu_freq",
        title: "CPU frequency",
        status: ProbeStatus::Implemented,
        answers_question: "Were system CPU frequency samples low or changing near spikes?",
        cli_flags: &["--cpu-freq / --no-cpu-freq"],
        artifacts: &[ArtifactKind::CpuFreqSamples],
        default_enabled: true,
        overhead: ProbeOverhead::Low,
        required_capabilities: &[ProbeCapability::Ebpf, ProbeCapability::Tracepoint],
        ebpf_programs: &[EbpfProgramSpec {
            name: "cpu_frequency",
        }],
        tracepoints: &[TracepointSpec {
            category: "power",
            name: "cpu_frequency",
        }],
        perf_events: &[],
        data_quality_rules: &[DataQualityRule {
            key: "cpu_freq_optional",
            description: "missing CPU frequency samples are tolerated as optional system context",
        }],
        validation_contract: "cpu_freq_samples.json is optional NDJSON; samples are system-wide context and reports tolerate missing tracepoint support.",
    },
    ProbeSpec {
        key: ProbeKey::Faults,
        catalog_key: "faults",
        title: "Page faults and stat wait",
        status: ProbeStatus::Implemented,
        answers_question: "Did page-fault deltas or procfs stat wait rise around scheduler spikes?",
        cli_flags: &["--faults"],
        artifacts: &[ArtifactKind::Interval],
        default_enabled: false,
        overhead: ProbeOverhead::Low,
        required_capabilities: &[
            ProbeCapability::Procfs,
            ProbeCapability::PerfEvent,
            ProbeCapability::Tracepoint,
        ],
        ebpf_programs: &[
            EbpfProgramSpec {
                name: "major_fault",
            },
            EbpfProgramSpec {
                name: "minor_fault",
            },
            EbpfProgramSpec {
                name: "sched_stat_wait",
            },
        ],
        tracepoints: &[TracepointSpec {
            category: "sched",
            name: "sched_stat_wait",
        }],
        perf_events: &[
            PerfEventSpec {
                name: "major_fault",
            },
            PerfEventSpec {
                name: "minor_fault",
            },
        ],
        data_quality_rules: &[DataQualityRule {
            key: "fault_stat_wait_defaults",
            description: "fault and stat-wait deltas default to zero when unavailable",
        }],
        validation_contract: "fault and stat-wait deltas live in interval.json and default to zero when unavailable.",
    },
    ProbeSpec {
        key: ProbeKey::CpuPerf,
        catalog_key: "cpu_perf",
        title: "CPU perf counters",
        status: ProbeStatus::Implemented,
        answers_question: "Was the workload low IPC or cache-miss bound during sampled task intervals?",
        cli_flags: &["--cpu-perf"],
        artifacts: &[ArtifactKind::Interval],
        default_enabled: false,
        overhead: ProbeOverhead::High,
        required_capabilities: &[ProbeCapability::PerfEvent],
        ebpf_programs: &[],
        tracepoints: &[],
        perf_events: &[
            PerfEventSpec { name: "cycles" },
            PerfEventSpec {
                name: "instructions",
            },
            PerfEventSpec {
                name: "cache-references",
            },
            PerfEventSpec {
                name: "cache-misses",
            },
        ],
        data_quality_rules: &[DataQualityRule {
            key: "cpu_perf_availability",
            description: "open/read/skipped status is reflected in data quality",
        }],
        validation_contract: "CPU perf data lives in interval.json and session task summaries; open/read/skipped status is reflected in data quality.",
    },
    ProbeSpec {
        key: ProbeKey::PsiTimeline,
        catalog_key: "psi_timeline",
        title: "PSI interval samples",
        status: ProbeStatus::Implemented,
        answers_question: "Was CPU/memory/I/O pressure present in interval summaries?",
        cli_flags: &["interval sampling"],
        artifacts: &[ArtifactKind::Interval],
        default_enabled: true,
        overhead: ProbeOverhead::Low,
        required_capabilities: &[ProbeCapability::Procfs],
        ebpf_programs: &[],
        tracepoints: &[],
        perf_events: &[],
        data_quality_rules: &[DataQualityRule {
            key: "psi_defaults",
            description: "PSI fields use serde defaults so older recordings remain readable",
        }],
        validation_contract: "PSI fields live in interval.json with serde defaults so older recordings remain readable.",
    },
    ProbeSpec {
        key: ProbeKey::PressureStallTimelineOverlay,
        catalog_key: "pressure_stall_timeline_overlay",
        title: "Pressure-stall timeline overlay",
        status: ProbeStatus::ViewOnly,
        answers_question: "Did CPU/memory/I/O pressure line up with spikes?",
        cli_flags: &["report --analysis-json"],
        artifacts: &[ArtifactKind::Interval],
        default_enabled: true,
        overhead: ProbeOverhead::Low,
        required_capabilities: &[],
        ebpf_programs: &[],
        tracepoints: &[],
        perf_events: &[],
        data_quality_rules: &[],
        validation_contract: "Derived report view from existing interval.json PSI fields; empty when interval data is unavailable.",
    },
    ProbeSpec {
        key: ProbeKey::RuntimeSlices,
        catalog_key: "per_thread_runtime_slices",
        title: "Per-thread CPU runtime slices",
        status: ProbeStatus::Implemented,
        answers_question: "Was the task ready but delayed, or running but consuming too much CPU time?",
        cli_flags: &["--runtime-slices"],
        artifacts: &[ArtifactKind::RuntimeSlices],
        default_enabled: false,
        overhead: ProbeOverhead::Medium,
        required_capabilities: &[ProbeCapability::Procfs],
        ebpf_programs: &[],
        tracepoints: &[],
        perf_events: &[],
        data_quality_rules: &[DataQualityRule {
            key: "runtime_slice_optional",
            description: "missing schedstat falls back to proc stat or reports unavailable",
        }],
        validation_contract: "runtime_slices.json is optional NDJSON; missing schedstat falls back to proc stat or reports unavailable; analysis never treats missing runtime data as proof.",
    },
    ProbeSpec {
        key: ProbeKey::KmsPageflipTiming,
        catalog_key: "kms_pageflip_timing",
        title: "KMS pageflip timing",
        status: ProbeStatus::Implemented,
        answers_question: "Did KMS pageflip/vblank completion timing line up with frame pacing outliers?",
        cli_flags: &["--kms-timing"],
        artifacts: &[ArtifactKind::KmsFlipEvents],
        default_enabled: false,
        overhead: ProbeOverhead::High,
        required_capabilities: &[ProbeCapability::Ebpf, ProbeCapability::Tracepoint],
        ebpf_programs: &[
            EbpfProgramSpec {
                name: "drm_flip_request",
            },
            EbpfProgramSpec {
                name: "drm_flip_done",
            },
            EbpfProgramSpec {
                name: "drm_vblank_event",
            },
            EbpfProgramSpec {
                name: "i915_flip_request",
            },
            EbpfProgramSpec {
                name: "i915_flip_done",
            },
            EbpfProgramSpec {
                name: "amdgpu_flip_request",
            },
            EbpfProgramSpec {
                name: "amdgpu_flip_done",
            },
            EbpfProgramSpec {
                name: "amdgpu_vblank_event",
            },
        ],
        tracepoints: &[
            TracepointSpec {
                category: "drm",
                name: "drm_vblank_event",
            },
            TracepointSpec {
                category: "i915",
                name: "i915_flip_request",
            },
            TracepointSpec {
                category: "i915",
                name: "i915_flip_complete",
            },
            TracepointSpec {
                category: "amdgpu",
                name: "amdgpu_flip_complete",
            },
        ],
        perf_events: &[],
        data_quality_rules: &[DataQualityRule {
            key: "kms_timing_optional",
            description: "KMS timing requires compatible DRM/i915/amdgpu pageflip or vblank tracepoints; missing events are unavailable evidence, not proof of healthy scanout",
        }],
        validation_contract: "kms_flip_events.json is optional NDJSON populated from compatible DRM/i915/amdgpu pageflip or vblank tracepoints; missing KMS events means unavailable evidence, not proof that scanout was healthy.",
    },
    ProbeSpec {
        key: ProbeKey::DrmFenceLatency,
        catalog_key: "drm_fence_latency",
        title: "DRM fence latency",
        status: ProbeStatus::Implemented,
        answers_question: "Was frame stutter caused by GPU queue/fence delay rather than CPU runnable delay?",
        cli_flags: &["--drm-fence-latency"],
        artifacts: &[ArtifactKind::DrmFenceEvents],
        default_enabled: false,
        overhead: ProbeOverhead::High,
        required_capabilities: &[ProbeCapability::Ebpf, ProbeCapability::Tracepoint],
        ebpf_programs: &[
            EbpfProgramSpec {
                name: "drm_fence_wait_start",
            },
            EbpfProgramSpec {
                name: "drm_fence_wait_done",
            },
            EbpfProgramSpec {
                name: "drm_fence_signal",
            },
        ],
        tracepoints: &[
            TracepointSpec {
                category: "dma_fence",
                name: "dma_fence_wait_start",
            },
            TracepointSpec {
                category: "dma_fence",
                name: "dma_fence_wait_end",
            },
            TracepointSpec {
                category: "dma_fence",
                name: "dma_fence_signaled",
            },
        ],
        perf_events: &[],
        data_quality_rules: &[DataQualityRule {
            key: "drm_fence_optional",
            description: "DRM fence latency requires compatible wait/signal tracepoints with stable context/seqno or timeline/seqno identity",
        }],
        validation_contract: "drm_fence_events.json is optional NDJSON populated from compatible fence wait/signal tracepoints; missing fence events are unavailable evidence, not proof of no GPU/display wait.",
    },
    ProbeSpec {
        key: ProbeKey::WaylandPresentationTiming,
        catalog_key: "wayland_presentation_timing",
        title: "Wayland presentation timing",
        status: ProbeStatus::Implemented,
        answers_question: "Did Wayland commit-to-present timing, discarded frames, or direct-scanout hints correlate with frame outliers?",
        cli_flags: &["--wayland-presentation"],
        artifacts: &[ArtifactKind::WaylandPresentationEvents],
        default_enabled: false,
        overhead: ProbeOverhead::Low,
        required_capabilities: &[ProbeCapability::ExternalLog],
        ebpf_programs: &[],
        tracepoints: &[],
        perf_events: &[],
        data_quality_rules: &[DataQualityRule {
            key: "wayland_presentation_cooperative_source",
            description: "Wayland presentation timing requires a cooperative client, Gamescope/compositor log, or self-test source; missing events are unavailable evidence",
        }],
        validation_contract: "wayland_presentation_events.json is optional NDJSON populated from cooperative presentation logs; arbitrary Wayland clients cannot be observed without cooperation.",
    },
    ProbeSpec {
        key: ProbeKey::DisplayPathCost,
        catalog_key: "display_path_cost",
        title: "Display path cost",
        status: ProbeStatus::ViewOnly,
        answers_question: "How did frame/presentation metrics differ between two controlled display-path runs?",
        cli_flags: &[],
        artifacts: &[],
        default_enabled: false,
        overhead: ProbeOverhead::Low,
        required_capabilities: &[],
        ebpf_programs: &[],
        tracepoints: &[],
        perf_events: &[],
        data_quality_rules: &[],
        validation_contract: "view-only comparison derived from two runs; no raw display_path_cost artifact is emitted.",
    },
    ProbeSpec {
        key: ProbeKey::PerfCounterPresets,
        catalog_key: "perf_counter_presets",
        title: "Perf counter presets",
        status: ProbeStatus::Planned,
        answers_question: "Which low-overhead preset should be used for IPC/cache diagnosis?",
        cli_flags: &[],
        artifacts: &[],
        default_enabled: false,
        overhead: ProbeOverhead::High,
        required_capabilities: &[ProbeCapability::PerfEvent],
        ebpf_programs: &[],
        tracepoints: &[],
        perf_events: &[],
        data_quality_rules: &[],
        validation_contract: "not implemented; must add schema/docs/fixtures before enabling",
    },
    ProbeSpec {
        key: ProbeKey::CompositorFramePacingViews,
        catalog_key: "compositor_frame_pacing_views",
        title: "Compositor/frame-pacing views",
        status: ProbeStatus::Planned,
        answers_question: "Did frame pacing problems correlate with compositor/gamescope scheduler delay?",
        cli_flags: &[],
        artifacts: &[],
        default_enabled: false,
        overhead: ProbeOverhead::Low,
        required_capabilities: &[],
        ebpf_programs: &[],
        tracepoints: &[],
        perf_events: &[],
        data_quality_rules: &[],
        validation_contract: "not implemented; must add schema/docs/fixtures before enabling",
    },
];

pub fn probe_spec(key: ProbeKey) -> &'static ProbeSpec {
    PROBE_REGISTRY
        .iter()
        .find(|spec| spec.key == key)
        .expect("ProbeKey must have a ProbeSpec")
}

pub fn implemented_probe_specs() -> impl Iterator<Item = &'static ProbeSpec> {
    PROBE_REGISTRY
        .iter()
        .filter(|spec| spec.status == ProbeStatus::Implemented)
}
