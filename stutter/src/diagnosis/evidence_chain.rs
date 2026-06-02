//! Structured evidence chain builders for diagnosis output.

use std::collections::BTreeMap;

use super::{
    Diagnosis, EvidenceChain, EvidenceChainKind, EvidenceChainNode, EvidenceChainNodeKind,
    StutterCause,
};
use crate::{
    irq_inspect::{IrqDeviceClass, IrqLine, classify_irq_device},
    metrics::format_latency,
    recorder::{BlockIoRecord, DrmFenceEventRecord, FrameEvent, GpuSample, IrqEventRecord},
    spike::SpikeCluster,
};

pub(super) struct EvidenceChainInputs<'a> {
    pub(super) frame: Option<&'a FrameEvent>,
    pub(super) irq_event: Option<&'a IrqEventRecord>,
    pub(super) irq_line: Option<&'a IrqLine>,
    pub(super) gpu_sample: Option<(&'a GpuSample, u32)>,
    pub(super) drm_fence_event: Option<&'a DrmFenceEventRecord>,
    pub(super) block_io_event: Option<&'a BlockIoRecord>,
}

fn cluster_elapsed_midpoint(cluster: &SpikeCluster) -> Option<u64> {
    let mut values = cluster.points.iter().filter_map(|point| point.elapsed_ms);
    let first = values.next()?;
    let mut min = first;
    let mut max = first;

    for value in values {
        min = min.min(value);
        max = max.max(value);
    }

    Some(min + (max.saturating_sub(min) / 2))
}

fn details(values: impl IntoIterator<Item = (&'static str, String)>) -> BTreeMap<String, String> {
    values
        .into_iter()
        .filter(|(_, value)| !value.trim().is_empty())
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn chain_node(
    kind: EvidenceChainNodeKind,
    label: impl Into<String>,
    timestamp_ms: Option<u64>,
    start_ns: Option<u64>,
    end_ns: Option<u64>,
    previous_timestamp_ms: Option<u64>,
    details: BTreeMap<String, String>,
) -> EvidenceChainNode {
    EvidenceChainNode {
        kind,
        label: label.into(),
        timestamp_ms,
        start_ns,
        end_ns,
        delta_from_previous_ms: timestamp_ms
            .zip(previous_timestamp_ms)
            .map(|(current, previous)| current as i64 - previous as i64),
        details,
    }
}

fn cluster_node(
    cluster: &SpikeCluster,
    previous_timestamp_ms: Option<u64>,
) -> (EvidenceChainNode, Option<u64>) {
    let timestamp_ms = cluster_elapsed_midpoint(cluster);
    let node = chain_node(
        EvidenceChainNodeKind::Cluster,
        "scheduler spike cluster",
        timestamp_ms,
        Some(cluster.min_switch_ns),
        Some(cluster.max_switch_ns),
        previous_timestamp_ms,
        details([
            ("max_latency", format_latency(cluster.max_latency_ns)),
            ("points", cluster.points.len().to_string()),
            ("distinct_tasks", cluster.distinct_tasks.to_string()),
        ]),
    );
    (node, timestamp_ms)
}

fn frame_chain_node(frame: Option<&FrameEvent>) -> Option<(EvidenceChainNode, Option<u64>)> {
    frame.map(|frame| {
        let timestamp_ms = Some(frame.elapsed_ms);
        (
            chain_node(
                EvidenceChainNodeKind::Frame,
                "visible frame spike",
                timestamp_ms,
                None,
                None,
                None,
                details([("frametime_ms", format!("{:.3}", frame.frametime_ms))]),
            ),
            timestamp_ms,
        )
    })
}

fn recommendation_node(
    label: impl Into<String>,
    previous_timestamp_ms: Option<u64>,
) -> EvidenceChainNode {
    chain_node(
        EvidenceChainNodeKind::Recommendation,
        label,
        None,
        None,
        None,
        previous_timestamp_ms,
        BTreeMap::new(),
    )
}

fn candidate_present(diagnosis: &Diagnosis, cause: StutterCause) -> bool {
    diagnosis
        .primary
        .as_ref()
        .is_some_and(|primary| primary.cause == cause)
        || diagnosis
            .candidates
            .iter()
            .any(|candidate| candidate.cause == cause)
}

fn build_irq_evidence_chain(
    cluster: &SpikeCluster,
    frame: Option<&FrameEvent>,
    event: &IrqEventRecord,
    line: Option<&IrqLine>,
) -> EvidenceChain {
    let mut nodes = Vec::new();
    let mut previous = None;
    if let Some((node, ts)) = frame_chain_node(frame) {
        previous = ts;
        nodes.push(node);
    }
    let (node, ts) = cluster_node(cluster, previous);
    previous = ts;
    nodes.push(node);

    let event_ts = event.elapsed_ms;
    nodes.push(chain_node(
        EvidenceChainNodeKind::Event,
        format!("IRQ {} handler", event.irq),
        event_ts,
        Some(event.enter_ns),
        Some(event.exit_ns),
        previous,
        details([
            ("cpu", event.cpu.to_string()),
            ("duration", format_latency(event.duration_ns)),
        ]),
    ));
    previous = event_ts.or(previous);

    let device = line
        .map(|line| format!("IRQ {} {}", line.irq, line.name))
        .unwrap_or_else(|| format!("IRQ {} unknown device", event.irq));
    nodes.push(chain_node(
        EvidenceChainNodeKind::Device,
        device,
        event_ts,
        Some(event.enter_ns),
        Some(event.exit_ns),
        previous,
        details([
            (
                "class",
                format!(
                    "{:?}",
                    line.map(classify_irq_device)
                        .unwrap_or(IrqDeviceClass::Unknown)
                ),
            ),
            ("cpu", event.cpu.to_string()),
        ]),
    ));
    previous = event_ts.or(previous);

    nodes.push(recommendation_node(
        "inspect IRQ affinity before changing CPU tuning",
        previous,
    ));

    EvidenceChain {
        kind: EvidenceChainKind::Irq,
        explicit: true,
        summary: format!(
            "explicit IRQ chain: cluster window overlaps IRQ {} on CPU {} for {}; recommendation is inspection, not automatic tuning",
            event.irq,
            event.cpu,
            format_latency(event.duration_ns)
        ),
        nodes,
    }
}

fn build_gpu_evidence_chain(
    cluster: &SpikeCluster,
    frame: Option<&FrameEvent>,
    sample: &GpuSample,
    busy_percent: u32,
) -> EvidenceChain {
    let mut nodes = Vec::new();
    let mut previous = None;
    if let Some((node, ts)) = frame_chain_node(frame) {
        previous = ts;
        nodes.push(node);
    }
    let (node, ts) = cluster_node(cluster, previous);
    previous = ts;
    nodes.push(node);

    let event_ts = Some(sample.elapsed_ms);
    nodes.push(chain_node(
        EvidenceChainNodeKind::Event,
        "GPU busy sample",
        event_ts,
        None,
        None,
        previous,
        details([
            ("gpu_busy_percent", busy_percent.to_string()),
            (
                "power_limit_reason",
                sample.power_limit_reason.clone().unwrap_or_default(),
            ),
        ]),
    ));
    previous = event_ts;

    nodes.push(chain_node(
        EvidenceChainNodeKind::Device,
        "GPU device sample",
        event_ts,
        None,
        None,
        previous,
        details([
            (
                "drm_card",
                sample.drm_card.clone().unwrap_or_else(|| "-".to_owned()),
            ),
            (
                "render_node",
                sample.render_node.clone().unwrap_or_else(|| "-".to_owned()),
            ),
        ]),
    ));
    previous = event_ts;

    nodes.push(recommendation_node(
        "investigate GPU/display path before CPU-affinity tuning",
        previous,
    ));

    EvidenceChain {
        kind: EvidenceChainKind::Gpu,
        explicit: true,
        summary: format!(
            "explicit GPU chain: cluster is near a GPU busy sample at {busy_percent}%; recommendation is display/GPU investigation, not proof of CPU root cause"
        ),
        nodes,
    }
}

fn build_drm_fence_evidence_chain(
    cluster: &SpikeCluster,
    frame: Option<&FrameEvent>,
    event: &DrmFenceEventRecord,
) -> EvidenceChain {
    let mut nodes = Vec::new();
    let mut previous = None;
    if let Some((node, ts)) = frame_chain_node(frame) {
        previous = ts;
        nodes.push(node);
    }
    let (node, ts) = cluster_node(cluster, previous);
    previous = ts;
    nodes.push(node);

    let event_ts = Some(event.elapsed_ms);
    nodes.push(chain_node(
        EvidenceChainNodeKind::Event,
        "DRM fence wait",
        event_ts,
        event.wait_start_ns,
        event.wait_done_ns.or(Some(event.timestamp_ns)),
        previous,
        details([
            (
                "duration",
                event
                    .duration_ns
                    .map(format_latency)
                    .unwrap_or_else(|| "-".to_owned()),
            ),
            ("comm", event.comm.clone().unwrap_or_else(|| "-".to_owned())),
            (
                "role",
                event.gpu_role.clone().unwrap_or_else(|| "-".to_owned()),
            ),
        ]),
    ));
    previous = event_ts;

    nodes.push(chain_node(
        EvidenceChainNodeKind::Device,
        "DRM fence device",
        event_ts,
        event.wait_start_ns,
        event.wait_done_ns.or(Some(event.timestamp_ns)),
        previous,
        details([
            (
                "driver",
                event.driver.clone().unwrap_or_else(|| "-".to_owned()),
            ),
            ("card", event.card.clone().unwrap_or_else(|| "-".to_owned())),
            ("confidence", event.confidence.clone()),
        ]),
    ));
    previous = event_ts;

    nodes.push(recommendation_node(
        "inspect DRM fence/display path timing before CPU tuning",
        previous,
    ));

    EvidenceChain {
        kind: EvidenceChainKind::DrmFence,
        explicit: true,
        summary: format!(
            "explicit DRM fence chain: cluster window overlaps a fence wait of {}; recommendation is display-path investigation",
            event
                .duration_ns
                .map(format_latency)
                .unwrap_or_else(|| "unknown duration".to_owned())
        ),
        nodes,
    }
}

fn build_block_io_evidence_chain(
    cluster: &SpikeCluster,
    frame: Option<&FrameEvent>,
    event: &BlockIoRecord,
) -> EvidenceChain {
    let mut nodes = Vec::new();
    let mut previous = None;
    if let Some((node, ts)) = frame_chain_node(frame) {
        previous = ts;
        nodes.push(node);
    }
    let (node, ts) = cluster_node(cluster, previous);
    previous = ts;
    nodes.push(node);

    let event_ts = Some(event.elapsed_ms);
    nodes.push(chain_node(
        EvidenceChainNodeKind::Event,
        "block I/O request",
        event_ts,
        Some(event.timestamp_ns.saturating_sub(event.duration_ns)),
        Some(event.timestamp_ns),
        previous,
        details([
            ("duration", format_latency(event.duration_ns)),
            ("rwbs", event.rwbs.clone()),
            ("tid", event.tid.to_string()),
        ]),
    ));
    previous = event_ts;

    nodes.push(chain_node(
        EvidenceChainNodeKind::Device,
        format!("block device {} sector {}", event.dev, event.sector),
        event_ts,
        Some(event.timestamp_ns.saturating_sub(event.duration_ns)),
        Some(event.timestamp_ns),
        previous,
        details([
            ("dev", event.dev.to_string()),
            ("sector", event.sector.to_string()),
            ("nr_sector", event.nr_sector.to_string()),
            ("correlation_basis", event.correlation_basis.to_string()),
        ]),
    ));
    previous = event_ts;

    nodes.push(recommendation_node(
        "confirm storage activity before CPU-affinity tuning",
        previous,
    ));

    EvidenceChain {
        kind: EvidenceChainKind::BlockIo,
        explicit: true,
        summary: format!(
            "explicit block I/O chain: cluster window overlaps block I/O lasting {}; recommendation is storage investigation",
            format_latency(event.duration_ns)
        ),
        nodes,
    }
}

pub(super) fn attach_evidence_chains(
    diagnosis: &mut Diagnosis,
    cluster: &SpikeCluster,
    inputs: EvidenceChainInputs<'_>,
) {
    if candidate_present(diagnosis, StutterCause::IrqDelayCandidate)
        && let Some(event) = inputs.irq_event
    {
        diagnosis.evidence_chains.push(build_irq_evidence_chain(
            cluster,
            inputs.frame,
            event,
            inputs.irq_line,
        ));
    }

    if candidate_present(diagnosis, StutterCause::GpuBoundCandidate) {
        if let Some((sample, busy_percent)) = inputs.gpu_sample {
            diagnosis.evidence_chains.push(build_gpu_evidence_chain(
                cluster,
                inputs.frame,
                sample,
                busy_percent,
            ));
        }
        if let Some(event) = inputs.drm_fence_event {
            diagnosis
                .evidence_chains
                .push(build_drm_fence_evidence_chain(cluster, inputs.frame, event));
        }
    }

    if candidate_present(diagnosis, StutterCause::BlockIoCandidate)
        && let Some(event) = inputs.block_io_event
    {
        diagnosis
            .evidence_chains
            .push(build_block_io_evidence_chain(cluster, inputs.frame, event));
    }
}
