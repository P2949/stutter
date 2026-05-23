//! Monitor session runtime orchestration.
//!
//! Owns:
//! - monitor runtime construction, target/probe/output/UI scheduling, foreground and focus ticks,
//!   live diagnosis timing, recorder lifecycle integration, and high-level `run_monitor` flow.
//!
//! Does not own:
//! - CLI parsing, remote API serving, low-level action application, report rendering, or daemon
//!   policy authorization.
//!
//! Allowed dependencies:
//! - config models, eBPF loading, focus/foreground resolution, hwmon/MangoHud/system probes,
//!   metrics summaries, process-tree targeting, recorder artifacts, runtime slices, event buses,
//!   watch-process helpers, and session output sinks.
//!
//! Main entry points:
//! - `MonitorSession`, `run_monitor`, `configure_target_irqs`, and the `session/*` runtime,
//!   targeting, probe, sink, output, telemetry, and UI submodules declared from this file.
//!
//! Safety, mutation, and persistence invariants:
//! - target changes must flow through `TargetController`/`TargetPolicy` and retain start-time
//!   checks for stale process trees;
//! - recorder setup/finalize must bracket probe collection and preserve warning output;
//! - live diagnosis must use bounded tick windows and must not persist report-only conclusions;
//! - optional foreground/focus providers must degrade to safe behavior when unavailable or stale.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use crossterm::event::{Event, KeyCode};
use futures_util::{FutureExt, future::Fuse};
use log::{info, warn};
use tokio::{
    task,
    time::{MissedTickBehavior, interval},
};

use crate::{
    artifacts::ArtifactKind,
    config::{CsvStreamTarget, model::MonitorConfig},
    diagnosis::{LiveDiagnosisEntry, diagnose_cluster},
    dmabuf_log::DmaBufLogReader,
    ebpf_loader,
    focus::{FocusResolver, ResolvedFocus},
    gpu_engine::EngineSampler,
    hwmon, mangohud,
    metrics::{collect_interval_summaries_labeled, log_drop_counters, print_session_summaries},
    recorder::{self, FinalizeRecordingInput, SpikeEvent},
    session::{
        alerts::AlertRuntime,
        display_timing::{
            drm_fence_event_kind_name, drm_fence_provider_name, drm_gpu_role_name,
            elapsed_ms_from_event_timestamp, kms_flip_event_kind_name, kms_flip_flag_names,
            kms_flip_provider_name,
        },
        event_bus::MonitorEventBus,
        exporter::ExporterRuntime,
        hwmon_stage::HwmonRuntime,
        outputs::OutputRuntime,
        probes::ProbeRuntime,
        recording::RecordingRuntime,
        runtime::MonitorRuntime,
        sampler::SamplerRuntime,
        sinks::MonitorOutputSinks,
        targeting::{
            SessionTargetPlan, TargetController, TargetPolicy, needs_tree_tick_from_parts,
        },
        ticks::{focus::FocusTickContext, foreground::ForegroundTickContext},
        ui::{TuiRenderSnapshot, UiRuntimeStage},
    },
    session_events::MonitorEvent,
    watch::{WatchProcessState, find_process_by_pattern_at_with_cache, tree_root_is_stale},
    wayland_presentation::WaylandPresentationLogReader,
};

#[path = "session/alerts.rs"]
pub(crate) mod alerts;
#[path = "session/display_timing.rs"]
pub(crate) mod display_timing;
#[path = "session/event_bus.rs"]
pub(crate) mod event_bus;
#[path = "session/exporter.rs"]
pub(crate) mod exporter;
#[path = "session/hwmon.rs"]
pub(crate) mod hwmon_stage;
#[path = "session/live_telemetry.rs"]
pub(crate) mod live_telemetry;
#[path = "session/mangohud_frames.rs"]
pub(crate) mod mangohud_frames;
#[path = "session/monitor_session.rs"]
pub(crate) mod monitor_session;
#[path = "session/outputs.rs"]
pub(crate) mod outputs;
#[path = "session/probes.rs"]
pub(crate) mod probes;
#[path = "session/recording.rs"]
pub(crate) mod recording;
#[path = "session/runtime.rs"]
pub(crate) mod runtime;
#[path = "session/sampler.rs"]
pub(crate) mod sampler;
#[path = "session/sinks.rs"]
pub(crate) mod sinks;
#[path = "session/targeting.rs"]
pub(crate) mod targeting;
#[path = "session/ticks/mod.rs"]
pub(crate) mod ticks;

#[path = "session/ui.rs"]
pub(crate) mod ui;

pub use monitor_session::MonitorSession;
#[cfg(test)]
pub(crate) use ticks::foreground::foreground_identity_changed;

async fn optional_tick(tick: Option<&mut tokio::time::Interval>) {
    if let Some(tick) = tick {
        tick.tick().await;
    } else {
        futures_util::future::pending::<()>().await;
    }
}

pub(crate) struct SessionProbePlan {
    loaded: ebpf_loader::LoadedEbpf,
    block_io_correlation_basis: String,
    block_io_correlation_confidence: String,
    native_cgroup_filter: ebpf_loader::NativeCgroupFilterStatus,
}

impl SessionProbePlan {
    fn load(config: &MonitorConfig, target_policy: &TargetPolicy) -> anyhow::Result<Self> {
        let mut loaded = ebpf_loader::load_and_attach(config, target_policy)?;
        configure_target_irqs(&mut loaded, config)?;
        let block_io_correlation_basis = loaded.block_io_correlation_basis.as_str().to_owned();
        let block_io_correlation_confidence =
            loaded.block_io_correlation_basis.confidence().to_owned();
        let native_cgroup_filter = loaded.native_cgroup_filter.clone();

        Ok(Self {
            loaded,
            block_io_correlation_basis,
            block_io_correlation_confidence,
            native_cgroup_filter,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct TargetTickContext {
    event: TargetTickEvent,
}

#[derive(Debug, Clone, Copy)]
enum TargetTickEvent {
    Tree,
    Watch,
}

#[derive(Debug, Clone, Copy)]
struct SummaryTickContext;

#[derive(Debug, Clone, Copy)]
struct ProbeDrainContext;

#[derive(Debug)]
struct FrameTickContext {
    frame: recorder::FrameEvent,
}

#[derive(Debug)]
struct WaylandPresentationTickContext {
    event: recorder::WaylandPresentationEventRecord,
}

#[derive(Debug)]
struct DmaBufTickContext {
    event: recorder::DmaBufEventRecord,
}

#[derive(Debug, Clone, Copy)]
struct TelemetryTickContext {
    event: TelemetryTickEvent,
}

#[derive(Debug, Clone, Copy)]
enum TelemetryTickEvent {
    MangoHudAlignment { raw_ms: u64, monotonic_ns: u64 },
    Scx,
    Hwmon,
}

#[derive(Debug)]
struct UiTickContext {
    event: Event,
}

#[cfg(test)]
#[path = "session/foreground_session_tests.rs"]
mod foreground_session_tests;

pub async fn run_monitor(
    config: Arc<MonitorConfig>,
    shared_hwmon: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
    event_tx: Option<tokio::sync::mpsc::Sender<MonitorEvent>>,
    stop_rx: Option<tokio::sync::oneshot::Receiver<()>>,
) -> anyhow::Result<String> {
    let mut session = MonitorSession::new((*config).clone(), shared_hwmon, event_tx).await?;
    let stop_reason = session.run(stop_rx).await?;
    session
        .dispatch_monitor_event(MonitorEvent::Finished {
            reason: stop_reason.clone(),
        })
        .await?;
    session.finalize(stop_reason)
}

pub fn configure_target_irqs(
    loaded: &mut ebpf_loader::LoadedEbpf,
    config: &MonitorConfig,
) -> anyhow::Result<()> {
    if !config.probes.irq_latency {
        return Ok(());
    }

    let Some(target_irq_map) = loaded.target_irq_map.as_mut() else {
        warn!("irq_latency_requested_but_map_missing");
        return Ok(());
    };

    if config.probes.irqs.is_empty() {
        anyhow::bail!(
            "--irq-latency requires at least one explicit --irq <N>; inspect /proc/interrupts to find the IRQ number for your GPU or device"
        );
    }

    for irq in config.probes.irqs.iter().copied() {
        target_irq_map.insert(irq, 1, 0)?;
        info!("irq_latency_target_added irq={irq}");
    }

    Ok(())
}

#[derive(Clone, Debug, Default)]
struct ScxSnapshot {
    ops: Option<String>,
    state: Option<String>,
    enable_seq: Option<String>,
}

fn scx_snapshot(tracker: &crate::scx::ScxTracker) -> ScxSnapshot {
    ScxSnapshot {
        ops: tracker.current_ops().map(str::to_owned),
        state: tracker.current_state().map(str::to_owned),
        enable_seq: tracker.current_enable_seq().map(str::to_owned),
    }
}

/// First-frame MangoHud alignment between the MangoHud CSV clock and the
/// recorder's monotonic clock.
///
/// The live MangoHud path discovers this once per recording. It is delivered
/// through a `oneshot` because the alignment event is a single initialization
/// value, not a stream.
pub(crate) type MangoHudAlignment = (u64, u64);

pub(crate) type MangoHudAlignmentReceiver = tokio::sync::oneshot::Receiver<MangoHudAlignment>;
pub(crate) type FusedMangoHudAlignmentReceiver = Fuse<MangoHudAlignmentReceiver>;

/// Make the MangoHud alignment receiver safe to keep inside the main select loop.
///
/// `tokio::sync::oneshot::Receiver` is a one-shot future. After it resolves,
/// polling it again is a programmer error and Tokio panics with:
///
/// `called after complete`
///
/// `MonitorSession::run` keeps this receiver in a long-running `tokio::select!`
/// loop. Once MangoHud alignment has been observed, the loop continues recording
/// frames, scheduler events, hwmon samples, foreground changes, and stop signals.
/// Therefore the completed receiver must become inert after its first result.
/// `Fuse` gives exactly that behavior: after completion, future polls return
/// `Pending` instead of panicking.
pub(crate) fn fused_mangohud_alignment_receiver(
    receiver: MangoHudAlignmentReceiver,
) -> FusedMangoHudAlignmentReceiver {
    receiver.fuse()
}

#[cfg(test)]
#[path = "session/tests.rs"]
mod tree_tick_tests;

#[cfg(test)]
#[path = "session/generic_tests.rs"]
mod tests;
