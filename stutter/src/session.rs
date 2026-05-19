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
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use crossterm::event::{Event, KeyCode};
use log::{info, warn};
use tokio::{
    task,
    time::{MissedTickBehavior, interval},
};

use crate::{
    artifacts::ArtifactKind,
    config::{CsvStreamTarget, FocusSource, ForegroundSource, model::MonitorConfig},
    diagnosis::{LiveDiagnosisEntry, diagnose_cluster},
    ebpf_loader,
    focus::{FocusDecision, FocusPolicy, FocusResolver, ResolvedFocus},
    hwmon, mangohud,
    metrics::{collect_interval_summaries_labeled, log_drop_counters, print_session_summaries},
    process_tree::find_auto_target_pids,
    recorder::{self, FinalizeRecordingInput, LiveRecorder, SpikeEvent, SpikeEventBuffer},
    runtime_slices::RuntimeSliceSampler,
    session::{
        event_bus::MonitorEventBus,
        outputs::OutputRuntime,
        probes::ProbeRuntime,
        runtime::MonitorRuntime,
        sinks::MonitorOutputSinks,
        targeting::{TargetController, TargetPolicy},
        ui::{TuiRenderSnapshot, TuiRuntime},
    },
    session_events::MonitorEvent,
    watch::{
        WatchProcessConfig, WatchProcessState, capture_tree_root_starttimes,
        find_process_by_pattern_at_with_cache, resolve_watch_process, tree_root_is_stale,
    },
};

#[path = "session/alerts.rs"]
pub(crate) mod alerts;
#[path = "session/event_bus.rs"]
pub(crate) mod event_bus;
#[path = "session/exporter.rs"]
pub(crate) mod exporter;
#[path = "session/hwmon.rs"]
pub(crate) mod hwmon_stage;
#[path = "session/live_telemetry.rs"]
pub(crate) mod live_telemetry;
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
#[path = "session/target.rs"]
pub(crate) mod target;
#[path = "session/targeting.rs"]
pub(crate) mod targeting;
#[path = "session/ticks/mod.rs"]
pub(crate) mod ticks;

#[path = "session/ui.rs"]
pub(crate) mod ui;

const LIVE_DIAGNOSIS_CLUSTER_WINDOW_MS: u64 = 5;
// Keep this aligned with the report default unless live diagnosis gets
// its own CLI/config field.

#[allow(dead_code)] // Transitional session-stage context; tick extraction will adopt this incrementally.
pub(crate) struct SessionContext<'a> {
    pub config: &'a MonitorConfig,
    pub started: Instant,
}

fn needs_tree_tick_from_parts(
    had_tree_roots: bool,
    watch_process_active: bool,
    cgroupv2_active: bool,
) -> bool {
    had_tree_roots || watch_process_active || cgroupv2_active
}

fn foreground_capture_enabled(config: &MonitorConfig) -> bool {
    config.focus.foreground_window
        || (config.focus.auto_focus && config.focus.focus_source != FocusSource::Heuristic)
}

fn foreground_resolver_from_config(
    config: &MonitorConfig,
) -> crate::foreground::ForegroundResolver {
    let resolver = match config.focus.foreground_source {
        ForegroundSource::Auto => crate::foreground::auto_foreground_resolver(),
        ForegroundSource::Sway => crate::foreground::ForegroundResolver::new(Box::new(
            crate::foreground::SwayForegroundProvider::new(),
        )),
        ForegroundSource::Hyprland => crate::foreground::ForegroundResolver::new(Box::new(
            crate::foreground::HyprlandForegroundProvider::new(),
        )),
        ForegroundSource::X11 => crate::foreground::ForegroundResolver::new(Box::new(
            crate::foreground::X11ForegroundProvider::new(),
        )),
    };

    resolver
        .with_include_title(config.focus.foreground_include_title)
        .with_max_stale_ms(config.focus.foreground_max_stale_ms)
}

fn foreground_identity_changed(
    old: Option<&crate::foreground::ForegroundWindowSnapshot>,
    new: &crate::foreground::ForegroundWindowSnapshot,
) -> bool {
    let Some(old) = old else {
        return true;
    };

    old.source != new.source
        || old.status != new.status
        || old.pid != new.pid
        || old.app_id.as_deref() != new.app_id.as_deref()
        || old.class.as_deref() != new.class.as_deref()
        || old.window_id.as_deref() != new.window_id.as_deref()
        || old.workspace.as_deref() != new.workspace.as_deref()
}

async fn optional_tick(tick: Option<&mut tokio::time::Interval>) {
    if let Some(tick) = tick {
        tick.tick().await;
    } else {
        futures_util::future::pending::<()>().await;
    }
}

struct SessionTargetPlan {
    tree_pids: Vec<u32>,
    watch_config: WatchProcessConfig,
    watch_state: WatchProcessState,
    tree_root_starttimes: BTreeMap<u32, Option<u64>>,
    had_tree_roots: bool,
    focus_resolver: Option<FocusResolver>,
    current_focus: Option<ResolvedFocus>,
    foreground_resolver: Option<crate::foreground::ForegroundResolver>,
    current_foreground: Option<crate::foreground::ForegroundWindowSnapshot>,
    community_rules: crate::community_rules::CommunityRulesStatus,
}

impl SessionTargetPlan {
    async fn resolve(config: &MonitorConfig) -> anyhow::Result<Self> {
        let explicit_target = config.has_explicit_target();
        let mut tree_pids = config.target.tree_pids.clone();

        let mut focus_resolver = None;
        let mut current_focus = None;
        let foreground_enabled = foreground_capture_enabled(config);
        let foreground_resolver =
            foreground_enabled.then(|| foreground_resolver_from_config(config));
        let current_foreground = None;

        let user_config = crate::config_file::load_user_config()?;

        log::info!(
            "monitor_session_config source=monitor_config summary_period_ms={} spike_threshold_ns={} max_tasks={} hwmon={} cpu_freq={} foreground_window={} focus_source={:?} foreground_source={:?}",
            config.timing.summary_period_ms,
            config.timing.spike_threshold_ns,
            config.target.max_tasks,
            config.probes.hwmon,
            config.probes.cpu_freq,
            config.focus.foreground_window,
            config.focus.focus_source,
            config.focus.foreground_source,
        );

        let community_rules_config =
            crate::config_file::community_rules_config_from_user_config(user_config.as_ref());
        let community_rules =
            crate::community_rules::load_community_rules_status(&community_rules_config);
        let community_rules_status = community_rules.label();
        match &community_rules {
            crate::community_rules::CommunityRulesStatus::Loaded { db } => {
                log::info!(
                    "community_rules_status status={} rules={}",
                    community_rules_status,
                    db.rule_count()
                );
            }
            crate::community_rules::CommunityRulesStatus::Disabled => {
                log::info!("community_rules_status status={community_rules_status}");
            }
            crate::community_rules::CommunityRulesStatus::Failed { error } => {
                log::warn!("community_rules_status status={community_rules_status} err={error}");
            }
        }

        if !explicit_target && config.focus.auto_focus {
            let policy = FocusPolicy {
                poll_ms: config.focus.auto_focus_poll_ms,
                min_confidence: config.focus.auto_focus_min_confidence,
                switch_margin: config.focus.auto_focus_switch_margin,
                switch_cooldown_ms: config.focus.auto_focus_switch_cooldown_ms,
                required_winner_polls: config.focus.auto_focus_required_polls,
                max_roots: config.focus.auto_focus_max_roots,
            };

            let mut resolver = FocusResolver::new(policy);
            match resolver.sample(Path::new("/proc"), 0, None, FocusSource::Heuristic) {
                FocusDecision::Switch { new, .. } | FocusDecision::Keep { focus: new } => {
                    tree_pids = new.group.root_pids.clone();
                    info!(
                        "auto_focus_initial_target kind={:?} score={:.3} confidence={:.3} roots={:?} situation={:?}",
                        new.group.kind,
                        new.group.score,
                        new.group.confidence,
                        new.group.root_pids,
                        new.situation
                    );
                    current_focus = Some(new);
                }
                FocusDecision::NoTarget { reason } | FocusDecision::Clear { reason, .. } => {
                    info!("auto_focus_no_initial_target reason={reason}");
                }
            }

            focus_resolver = Some(resolver);
        } else if !explicit_target {
            let auto_targets = find_auto_target_pids(Path::new("/proc"));
            if auto_targets.is_empty() {
                anyhow::bail!(
                    "no target specified and no game launcher (gamescope, pressure-vessel, etc.) detected. \
                     Please provide --pid <PID>, --tree-pid <PID>, --watch-process <COMM>, or --cgroupv2 <PATH>"
                );
            }

            let pids: Vec<_> = auto_targets.iter().map(|(p, _)| *p).collect();
            let class = auto_targets[0].1;
            info!("auto_detected_launcher class={class} pids={pids:?}");
            let stdout_is_machine_stream =
                config.outputs.json_stream || config.csv_streams_to_stdout();
            if !stdout_is_machine_stream {
                println!(
                    "auto-detected game launcher: {class} (PIDs {pids:?}). monitoring tree..."
                );
            }
            tree_pids = pids;
        }

        let watch_config = WatchProcessConfig::from_monitor_config(config);
        let watch_state = match resolve_watch_process(&watch_config, &mut tree_pids).await? {
            Some(pid) => WatchProcessState::Running(pid),
            None => WatchProcessState::None,
        };

        let had_tree_roots = !tree_pids.is_empty();
        let tree_root_starttimes = capture_tree_root_starttimes(&tree_pids);

        Ok(Self {
            tree_pids,
            watch_config,
            watch_state,
            tree_root_starttimes,
            had_tree_roots,
            focus_resolver,
            current_focus,
            foreground_resolver,
            current_foreground,
            community_rules,
        })
    }
}

struct SessionProbePlan {
    loaded: ebpf_loader::LoadedEbpf,
    block_io_correlation_basis: String,
    block_io_correlation_confidence: String,
}

impl SessionProbePlan {
    fn load(config: &MonitorConfig, target_policy: &TargetPolicy) -> anyhow::Result<Self> {
        let mut loaded = ebpf_loader::load_and_attach(config, target_policy)?;
        configure_target_irqs(&mut loaded, config)?;
        let block_io_correlation_basis = loaded.block_io_correlation_basis.as_str().to_owned();
        let block_io_correlation_confidence =
            loaded.block_io_correlation_basis.confidence().to_owned();

        Ok(Self {
            loaded,
            block_io_correlation_basis,
            block_io_correlation_confidence,
        })
    }
}

struct RecordingRuntime;

impl RecordingRuntime {
    fn begin(
        config: &MonitorConfig,
        probe_plan: &SessionProbePlan,
    ) -> anyhow::Result<LiveRecorder> {
        let recording = recorder::prepare_recording(config)?;
        let mut recorder = LiveRecorder {
            run: recording,
            ..Default::default()
        };

        if config.streams.json_stream {
            recorder.enable_stdout_spike_stream();
        }

        recorder.buffers.spike_events = recorder.run.as_ref().map(|_| SpikeEventBuffer::default());

        if let Some(run) = recorder.run.as_mut() {
            if let Some(path) = &config.mangohud.log
                && let Ok(meta) = fs::metadata(path)
            {
                run.mangohud_start_offset = Some(meta.len());
                info!(
                    "mangohud_alignment_init path={} start_offset={}",
                    path.display(),
                    meta.len()
                );
            }

            let registry = &mut recorder.streams;
            let dir = &run.run_dir;

            for kind in probe_plan
                .loaded
                .activation_plan
                .required_stream_artifacts()
            {
                registry.create_stream(dir, kind)?;
            }
        }

        if let Some(csv_stream) = &config.streams.csv {
            recorder.csv_writer = Some(match csv_stream {
                CsvStreamTarget::File(path) => {
                    recorder::IntervalCsvWriter::create_file(path.clone())?
                }
                CsvStreamTarget::Stdout => recorder::IntervalCsvWriter::stdout(),
            });
        }

        Ok(recorder)
    }
}

struct ExporterRuntime {
    prometheus_state: Option<Arc<crate::prometheus::PrometheusState>>,
    prometheus_task: Option<tokio::task::JoinHandle<()>>,
    otel_exporter: Option<crate::otel::OtelExporterHandle>,
}

impl ExporterRuntime {
    async fn begin(config: &MonitorConfig, recorder: &mut LiveRecorder) -> anyhow::Result<Self> {
        let (prometheus_state, prometheus_task) = if let Some(port) = config.outputs.metrics_port {
            let state = Arc::new(crate::prometheus::PrometheusState::new_started_now());
            let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
            let task = crate::prometheus::spawn_metrics_server(addr, state.clone()).await?;
            info!("prometheus metrics listening on http://127.0.0.1:{port}/metrics");
            (Some(state), Some(task))
        } else {
            (None, None)
        };

        recorder.exporters.prometheus_state = prometheus_state.clone();

        let mut otel_exporter = None;
        if let Some(endpoint) = config.outputs.otlp_endpoint.as_ref() {
            let started_at = recorder
                .run
                .as_ref()
                .map(|r| r.started_at)
                .unwrap_or_else(SystemTime::now);
            let monotonic_start_ns = recorder
                .run
                .as_ref()
                .and_then(|r| r.monotonic_start_ns)
                .unwrap_or_else(|| recorder::monotonic_now_ns().unwrap_or(0));

            let otel_config = crate::otel::OtelConfig {
                endpoint: endpoint.clone(),
                service_name: config.outputs.otel_service_name.clone(),
                started_at,
                monotonic_start_ns,
            };

            match crate::otel::spawn_exporter(otel_config) {
                Ok(handle) => {
                    recorder.exporters.otel_spike_tx = Some(handle.tx.clone());
                    recorder.exporters.otel_spans_dropped = Some(handle.dropped.clone());
                    otel_exporter = Some(handle);
                }
                Err(err) => {
                    warn!("failed to start OTel exporter: {err:#}");
                }
            }
        }

        Ok(Self {
            prometheus_state,
            prometheus_task,
            otel_exporter,
        })
    }
}

struct AlertRuntime {
    sender: Option<tokio::sync::mpsc::Sender<crate::alert::AlertPayload>>,
}

impl AlertRuntime {
    fn begin(config: &MonitorConfig) -> Self {
        let sender = if config.alerts.threshold_ns.is_some() {
            let (tx, mut rx) = tokio::sync::mpsc::channel(100);
            let webhook_url = config.alerts.webhook_url.clone();
            let webhook_client = webhook_url.as_ref().map(|_| {
                reqwest::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .build()
            });

            tokio::spawn(async move {
                while let Some(payload) = rx.recv().await {
                    if let Err(err) = crate::alert::send_desktop_alert(&payload).await {
                        warn!("desktop_alert_failed err={err}");
                    }
                    if let Some(url) = &webhook_url {
                        match &webhook_client {
                            Some(Ok(client)) => {
                                if let Err(err) = crate::alert::send_webhook_alert_with_client(
                                    client, url, &payload,
                                )
                                .await
                                {
                                    warn!("webhook_alert_failed url={url} err={err}");
                                }
                            }
                            Some(Err(err)) => {
                                warn!(
                                    "webhook_alert_failed url={url} err=failed to build HTTP client: {err}"
                                );
                            }
                            None => {}
                        }
                    }
                }
            });

            Some(tx)
        } else {
            None
        };

        Self { sender }
    }
}

struct SamplerRuntime {
    cpu_perf_sampler: Option<crate::perf_counters::CpuPerfSampler>,
    runtime_slice_sampler: Option<RuntimeSliceSampler>,
}

impl SamplerRuntime {
    fn begin(config: &MonitorConfig) -> Self {
        let cpu_perf_sampler = if config.probes.cpu_perf {
            Some(crate::perf_counters::CpuPerfSampler::new(
                crate::perf_counters::CpuPerfConfig {
                    include_kernel: config.cpu_perf.include_kernel,
                    max_tasks: config.cpu_perf.max_tasks,
                    collect_cache_refs: config.cpu_perf.collect_cache_refs,
                },
            ))
        } else {
            None
        };

        let runtime_slice_sampler = config.probes.runtime_slices.then(RuntimeSliceSampler::new);

        Self {
            cpu_perf_sampler,
            runtime_slice_sampler,
        }
    }
}

struct UiRuntimeStage;

impl UiRuntimeStage {
    fn begin(config: &MonitorConfig) -> anyhow::Result<TuiRuntime> {
        let tui_state = crate::tui::TuiState::default();
        let terminal = if config.ui.tui {
            Some(
                crate::tui::init_terminal()
                    .map_err(|e| anyhow::anyhow!("failed to init terminal: {e}"))?,
            )
        } else {
            None
        };

        Ok(TuiRuntime::from_parts(tui_state, terminal))
    }
}

struct HwmonRuntime {
    reader: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
}

impl HwmonRuntime {
    fn begin(
        config: &MonitorConfig,
        shared_hwmon: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
    ) -> Self {
        let reader = if !config.probes.hwmon {
            None
        } else if let Some(shared) = shared_hwmon {
            Some(shared)
        } else {
            hwmon::HwmonReader::discover_with_options(
                config.hwmon.root.as_deref(),
                config.hwmon.drm_card.as_deref(),
                config.hwmon.render_node.as_deref(),
            )
            .map(|r| Arc::new(std::sync::Mutex::new(r)))
        };

        if config.probes.hwmon && reader.is_none() {
            warn!("hwmon_requested_but_no_gpu_hwmon_found");
        }

        Self { reader }
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
struct FocusTickContext;

#[derive(Debug, Clone, Copy)]
struct ForegroundTickContext;

#[derive(Debug, Clone, Copy)]
struct SummaryTickContext;

#[derive(Debug, Clone, Copy)]
struct ProbeDrainContext;

#[derive(Debug)]
struct FrameTickContext {
    frame: recorder::FrameEvent,
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
mod foreground_session_tests {
    use super::*;

    struct ForegroundSnapshotTestInput<'a> {
        elapsed_ms: u64,
        status: crate::foreground::ForegroundProviderStatus,
        pid: Option<u32>,
        app_id: Option<&'a str>,
        class: Option<&'a str>,
        window_id: Option<&'a str>,
        workspace: Option<&'a str>,
        confidence: f32,
    }

    fn foreground_snapshot(
        input: ForegroundSnapshotTestInput<'_>,
    ) -> crate::foreground::ForegroundWindowSnapshot {
        crate::foreground::ForegroundWindowSnapshot {
            elapsed_ms: input.elapsed_ms,
            source: Some(crate::foreground::ForegroundSource::Sway),
            status: input.status,
            pid: input.pid,
            app_id: input.app_id.map(str::to_owned),
            class: input.class.map(str::to_owned),
            title: None,
            window_id: input.window_id.map(str::to_owned),
            workspace: input.workspace.map(str::to_owned),
            confidence: input.confidence,
            stale_ms: None,
            reason: "test foreground snapshot".to_owned(),
        }
    }

    #[test]
    fn foreground_identity_records_first_sample() {
        let snapshot = foreground_snapshot(ForegroundSnapshotTestInput {
            elapsed_ms: 100,
            status: crate::foreground::ForegroundProviderStatus::Available,
            pid: Some(4242),
            app_id: Some("steam"),
            class: Some("Steam"),
            window_id: Some("7"),
            workspace: Some("games"),
            confidence: 0.95,
        });

        assert!(foreground_identity_changed(None, &snapshot));
    }

    #[test]
    fn foreground_identity_changes_on_provider_status_transition() {
        let old = foreground_snapshot(ForegroundSnapshotTestInput {
            elapsed_ms: 100,
            status: crate::foreground::ForegroundProviderStatus::Available,
            pid: Some(4242),
            app_id: Some("steam"),
            class: Some("Steam"),
            window_id: Some("7"),
            workspace: Some("games"),
            confidence: 0.95,
        });
        let new = foreground_snapshot(ForegroundSnapshotTestInput {
            elapsed_ms: 200,
            status: crate::foreground::ForegroundProviderStatus::Error,
            pid: Some(4242),
            app_id: Some("steam"),
            class: Some("Steam"),
            window_id: Some("7"),
            workspace: Some("games"),
            confidence: 0.0,
        });

        assert!(foreground_identity_changed(Some(&old), &new));
    }

    #[test]
    fn foreground_identity_changes_on_window_identity_transition() {
        let old = foreground_snapshot(ForegroundSnapshotTestInput {
            elapsed_ms: 100,
            status: crate::foreground::ForegroundProviderStatus::Available,
            pid: Some(4242),
            app_id: Some("steam"),
            class: Some("Steam"),
            window_id: Some("7"),
            workspace: Some("games"),
            confidence: 0.95,
        });
        let new = foreground_snapshot(ForegroundSnapshotTestInput {
            elapsed_ms: 200,
            status: crate::foreground::ForegroundProviderStatus::Available,
            pid: Some(9000),
            app_id: Some("firefox"),
            class: Some("Firefox"),
            window_id: Some("8"),
            workspace: Some("web"),
            confidence: 0.95,
        });

        assert!(foreground_identity_changed(Some(&old), &new));
    }

    #[test]
    fn foreground_identity_ignores_elapsed_title_reason_and_confidence_only_changes() {
        let old = crate::foreground::ForegroundWindowSnapshot {
            elapsed_ms: 100,
            source: Some(crate::foreground::ForegroundSource::X11),
            status: crate::foreground::ForegroundProviderStatus::Available,
            pid: Some(4242),
            app_id: Some("Navigator".to_owned()),
            class: Some("Firefox".to_owned()),
            title: Some("old private title".to_owned()),
            window_id: Some("0x1200007".to_owned()),
            workspace: None,
            confidence: 0.90,
            stale_ms: None,
            reason: "old reason".to_owned(),
        };
        let new = crate::foreground::ForegroundWindowSnapshot {
            elapsed_ms: 250,
            source: Some(crate::foreground::ForegroundSource::X11),
            status: crate::foreground::ForegroundProviderStatus::Available,
            pid: Some(4242),
            app_id: Some("Navigator".to_owned()),
            class: Some("Firefox".to_owned()),
            title: Some("new private title".to_owned()),
            window_id: Some("0x1200007".to_owned()),
            workspace: None,
            confidence: 0.50,
            stale_ms: Some(150),
            reason: "new reason".to_owned(),
        };

        assert!(!foreground_identity_changed(Some(&old), &new));
    }
}

pub struct MonitorSession {
    pub config: Arc<MonitorConfig>,
    pub runtime: MonitorRuntime,

    pub cpu_to_pkg: BTreeMap<u32, String>,

    pub hwmon_reader: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
    pub focus_resolver: Option<FocusResolver>,
    pub current_focus: Option<ResolvedFocus>,
    pub focus_switch_count: u64,
    pub foreground_resolver: Option<crate::foreground::ForegroundResolver>,
    pub current_foreground: Option<crate::foreground::ForegroundWindowSnapshot>,
    pub foreground_switch_count: u64,

    pub started: Instant,
    pub had_tree_roots: bool,
    pub interval_label: &'static str,
    community_rules: crate::community_rules::CommunityRulesStatus,
}

impl MonitorSession {
    pub async fn new(
        config: MonitorConfig,
        shared_hwmon: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
        event_tx: Option<tokio::sync::mpsc::Sender<MonitorEvent>>,
    ) -> anyhow::Result<Self> {
        let target_policy = TargetPolicy::from_monitor_config(&config)?;
        let target_plan = SessionTargetPlan::resolve(&config).await?;
        let probe_plan = SessionProbePlan::load(&config, &target_policy)?;
        let mut recorder = RecordingRuntime::begin(&config, &probe_plan)?;
        let exporter_runtime = ExporterRuntime::begin(&config, &mut recorder).await?;
        let alert_runtime = AlertRuntime::begin(&config);
        let sampler_runtime = SamplerRuntime::begin(&config);

        let metadata = crate::metadata::collect_system_metadata();
        let cpu_to_pkg: BTreeMap<u32, String> = metadata
            .cpu_topology
            .iter()
            .map(|c| (c.cpu, c.physical_package_id.clone().unwrap_or_default()))
            .collect();

        let hwmon_runtime = HwmonRuntime::begin(&config, shared_hwmon);
        let started = Instant::now();

        let ui = UiRuntimeStage::begin(&config)?;
        let event_runtime_config = crate::events::EventRuntimeConfig::from_monitor_config(&config);

        let interval_label = if config.timing.epoch_period_ms.is_some() {
            "epoch"
        } else {
            "summary"
        };

        let probes = ProbeRuntime::new(
            probe_plan.loaded,
            probe_plan.block_io_correlation_basis,
            probe_plan.block_io_correlation_confidence,
            sampler_runtime.cpu_perf_sampler,
            sampler_runtime.runtime_slice_sampler,
        );

        let targeting = TargetController::from_policy_parts(
            target_policy,
            target_plan.watch_config,
            target_plan.tree_pids,
            target_plan.watch_state,
            target_plan.tree_root_starttimes,
        );

        let outputs = OutputRuntime::from_parts(
            recorder,
            exporter_runtime.prometheus_state,
            exporter_runtime.prometheus_task,
            exporter_runtime.otel_exporter,
            alert_runtime.sender,
            event_runtime_config.output,
        );

        let runtime = MonitorRuntime::from_config_parts(
            probes,
            outputs,
            ui,
            targeting,
            MonitorEventBus::new(event_tx),
            event_runtime_config,
        );

        Ok(Self {
            config: Arc::new(config),
            runtime,
            cpu_to_pkg,
            hwmon_reader: hwmon_runtime.reader,
            community_rules: target_plan.community_rules,
            focus_resolver: target_plan.focus_resolver,
            current_focus: target_plan.current_focus,
            focus_switch_count: 0,
            foreground_resolver: target_plan.foreground_resolver,
            current_foreground: target_plan.current_foreground,
            foreground_switch_count: 0,
            started,
            had_tree_roots: target_plan.had_tree_roots,
            interval_label,
        })
    }

    pub fn emit(
        &mut self,
        event: MonitorEvent,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        self.runtime.bus.emit(event)
    }

    async fn refresh_tasks_and_emit_snapshot(&mut self) -> anyhow::Result<()> {
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        let previous_active_targets: BTreeSet<u32> = self
            .runtime
            .targeting
            .tasks
            .active_targets
            .keys()
            .copied()
            .collect();

        self.refresh_tasks().await?;

        let removed_targets = previous_active_targets
            .into_iter()
            .filter(|tid| {
                !self
                    .runtime
                    .targeting
                    .tasks
                    .active_targets
                    .contains_key(tid)
            })
            .collect::<Vec<_>>();

        self.dispatch_monitor_event(MonitorEvent::TargetSnapshot {
            elapsed_ms,
            active_targets: self.runtime.targeting.tasks.active_targets.clone(),
            removed_targets,
        })
        .await?;

        Ok(())
    }

    async fn dispatch_monitor_event(&mut self, event: MonitorEvent) -> anyhow::Result<()> {
        let output = self.runtime.event_runtime_config.output;
        let outputs = &mut self.runtime.outputs;
        let mut sinks = MonitorOutputSinks::new(
            output,
            &mut outputs.recorder,
            outputs.alert_sender.as_ref(),
            &mut outputs.sink_registry,
        );

        if let Err(err) = sinks.dispatch(&event) {
            warn!("monitor_event_sink_failed err={err}");
        }

        self.emit(event).await;
        Ok(())
    }

    fn foreground_event_for_snapshot(
        &self,
        snapshot: &crate::foreground::ForegroundWindowSnapshot,
    ) -> Option<MonitorEvent> {
        snapshot
            .to_event(self.config.focus.foreground_include_title)
            .map(MonitorEvent::from)
    }

    async fn emit_focus_changed(
        &mut self,
        elapsed_ms: u64,
        old: Option<&ResolvedFocus>,
        new: &ResolvedFocus,
    ) -> anyhow::Result<()> {
        info!(
            "auto_focus_changed elapsed_ms={} old_kind={:?} new_kind={:?} score={:.3} confidence={:.3} roots={:?} situation={:?}",
            elapsed_ms,
            old.map(|focus| focus.group.kind),
            new.group.kind,
            new.group.score,
            new.group.confidence,
            new.group.root_pids,
            new.situation
        );

        let event = MonitorEvent::FocusChanged {
            elapsed_ms,
            old_kind: old.map(|focus| focus.group.kind),
            new_kind: new.group.kind,
            root_pids: new.group.root_pids.clone(),
            member_pids: new.group.member_pids.clone(),
            confidence: new.group.confidence,
            score: new.group.score,
            situation: new.situation,
            reasons: new.group.reasons.clone(),
        };

        self.dispatch_monitor_event(event).await?;

        Ok(())
    }

    async fn emit_focus_cleared(
        &mut self,
        elapsed_ms: u64,
        old: Option<&ResolvedFocus>,
        reason: String,
    ) -> anyhow::Result<()> {
        info!(
            "auto_focus_cleared elapsed_ms={} old_kind={:?} reason={}",
            elapsed_ms,
            old.map(|focus| focus.group.kind),
            reason
        );

        let event = MonitorEvent::FocusCleared {
            elapsed_ms,
            old_kind: old.map(|focus| focus.group.kind),
            reason,
        };

        self.dispatch_monitor_event(event).await?;

        Ok(())
    }

    async fn handle_focus_tick(&mut self) -> anyhow::Result<()> {
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        let Some(resolver) = self.focus_resolver.as_mut() else {
            return Ok(());
        };

        let foreground = self.current_foreground.clone();
        let decision = resolver.sample(
            Path::new("/proc"),
            elapsed_ms,
            foreground.as_ref(),
            self.config.focus.focus_source,
        );

        match decision {
            FocusDecision::Switch { old, new } => {
                self.runtime
                    .targeting
                    .replace_dynamic_tree_roots(new.group.root_pids.clone());
                self.had_tree_roots = self.runtime.targeting.has_tree_roots();
                self.current_focus = Some(new.clone());
                self.focus_switch_count = self.focus_switch_count.saturating_add(1);
                self.refresh_tasks_and_emit_snapshot().await?;
                self.emit_focus_changed(elapsed_ms, old.as_ref(), &new)
                    .await?;
            }
            FocusDecision::Clear { old, reason } => {
                self.runtime.targeting.clear_dynamic_tree_roots();
                self.had_tree_roots = false;
                self.current_focus = None;
                self.refresh_tasks_and_emit_snapshot().await?;
                self.emit_focus_cleared(elapsed_ms, old.as_ref(), reason)
                    .await?;
            }
            FocusDecision::Keep { focus } => {
                self.current_focus = Some(focus);
            }
            FocusDecision::NoTarget { .. } => {}
        }

        Ok(())
    }

    async fn handle_foreground_tick(&mut self) -> anyhow::Result<()> {
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        let Some(resolver) = self.foreground_resolver.as_mut() else {
            return Ok(());
        };

        let snapshot = resolver.sample(elapsed_ms);
        let changed = foreground_identity_changed(self.current_foreground.as_ref(), &snapshot);

        if changed {
            if self.current_foreground.is_some() {
                self.foreground_switch_count = self.foreground_switch_count.saturating_add(1);
            }
            if let Some(event) = self.foreground_event_for_snapshot(&snapshot) {
                self.dispatch_monitor_event(event).await?;
            }
        }

        self.current_foreground = Some(snapshot);

        Ok(())
    }

    fn handle_ctrl_c_stop(&self) -> String {
        "ctrl_c".to_owned()
    }

    fn handle_max_duration_stop(&self, reason: Option<String>) -> String {
        reason.expect("max duration future must resolve with a stop reason")
    }

    fn handle_remote_stop(&self) -> String {
        "remote_stop".to_owned()
    }

    fn handle_epoch_tick(&self) -> Option<String> {
        self.config
            .timing
            .epoch_period_ms
            .is_some()
            .then(|| "epoch_ended".to_owned())
    }

    async fn handle_target_tick(
        &mut self,
        context: TargetTickContext,
    ) -> anyhow::Result<Option<String>> {
        match context.event {
            TargetTickEvent::Tree => self.handle_tree_tick().await,
            TargetTickEvent::Watch => {
                self.handle_watch_tick().await?;
                Ok(None)
            }
        }
    }

    async fn handle_focus_context_tick(
        &mut self,
        _context: FocusTickContext,
    ) -> anyhow::Result<()> {
        self.handle_focus_tick().await
    }

    async fn handle_foreground_context_tick(
        &mut self,
        _context: ForegroundTickContext,
    ) -> anyhow::Result<()> {
        self.handle_foreground_tick().await
    }

    async fn handle_summary_context_tick(
        &mut self,
        _context: SummaryTickContext,
    ) -> anyhow::Result<()> {
        self.handle_summary_tick().await
    }

    async fn handle_probe_drain(&mut self, _context: ProbeDrainContext) -> anyhow::Result<()> {
        self.drain_bpf_events().await
    }

    async fn handle_frame_tick(&mut self, context: FrameTickContext) -> anyhow::Result<()> {
        let frame = context.frame;
        self.dispatch_monitor_event(MonitorEvent::Frame {
            event: Box::new(frame.clone()),
        })
        .await?;
        self.runtime.telemetry.push_frame(frame);
        Ok(())
    }

    async fn handle_telemetry_tick(&mut self, context: TelemetryTickContext) -> anyhow::Result<()> {
        match context.event {
            TelemetryTickEvent::MangoHudAlignment {
                raw_ms,
                monotonic_ns,
            } => {
                if let Some(run) = self.runtime.outputs.recorder.run.as_mut() {
                    run.mangohud_first_frame_raw_elapsed_ms = Some(raw_ms);
                    run.mangohud_first_frame_monotonic_ns = Some(monotonic_ns);
                    info!(
                        "mangohud_alignment_observed raw_ms={} monotonic_ns={}",
                        raw_ms, monotonic_ns
                    );
                }
                Ok(())
            }
            TelemetryTickEvent::Scx => self.handle_scx_tick().await,
            TelemetryTickEvent::Hwmon => self.handle_hwmon_tick().await,
        }
    }

    fn handle_ui_tick(&mut self, context: UiTickContext) -> Option<String> {
        self.handle_tui_event(context.event)
    }

    pub async fn run(
        &mut self,
        mut stop_rx: Option<tokio::sync::oneshot::Receiver<()>>,
    ) -> anyhow::Result<String> {
        let mut summary_tick =
            interval(Duration::from_millis(self.config.timing.summary_period_ms));
        summary_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let epoch_tick_duration = self
            .config
            .timing
            .epoch_period_ms
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_secs(3600 * 24 * 365));
        let mut epoch_tick = interval(epoch_tick_duration);
        epoch_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut tree_tick = if needs_tree_tick_from_parts(
            self.had_tree_roots,
            self.runtime.targeting.watch_config.is_active(),
            self.config.target.cgroupv2.is_some(),
        ) {
            let mut tick = interval(Duration::from_millis(2_000));
            tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
            Some(tick)
        } else {
            None
        };

        let mut focus_tick = if self.focus_resolver.is_some() {
            let mut tick = interval(Duration::from_millis(self.config.focus.auto_focus_poll_ms));
            tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
            Some(tick)
        } else {
            None
        };

        let mut foreground_tick = if self.foreground_resolver.is_some() {
            let mut tick = interval(Duration::from_millis(self.config.focus.foreground_poll_ms));
            tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
            Some(tick)
        } else {
            None
        };

        let mut watch_tick = interval(Duration::from_millis(
            self.runtime.targeting.watch_config.poll_ms,
        ));
        watch_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut scx_tick = interval(Duration::from_millis(1_000));
        scx_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut hwmon_tick = interval(Duration::from_millis(1_000));
        hwmon_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut tui_event_reader = if self.config.ui.tui {
            Some(crossterm::event::EventStream::new())
        } else {
            None
        };

        let max_duration = self.config.timing.max_duration;
        let max_duration_future = async move {
            if let Some(duration) = max_duration {
                tokio::time::sleep(duration).await;
                Some("max_duration_reached".to_owned())
            } else {
                futures_util::future::pending().await
            }
        };
        tokio::pin!(max_duration_future);

        self.refresh_tasks_and_emit_snapshot().await?;

        let (mangohud_tx, mut mangohud_rx) = tokio::sync::oneshot::channel::<(u64, u64)>();
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(1024);

        if let Some(run) = self.runtime.outputs.recorder.run.as_ref()
            && let Some(path) = self.config.mangohud.log.clone()
            && let Some(offset) = run.mangohud_start_offset
        {
            let path_clone = path.clone();
            tokio::spawn(async move {
                if let Ok(res) = mangohud::poll_alignment(&path_clone, offset).await {
                    let _ = mangohud_tx.send(res);
                }
            });

            if self.config.mangohud.log_live {
                tokio::spawn(async move {
                    if let Err(err) = mangohud::tail_frames(path, offset, frame_tx).await {
                        log::warn!("mangohud_tail_failed err={err:#}");
                    }
                });
            }
        }

        loop {
            tokio::select! {
                res = &mut mangohud_rx => {
                    if let Ok((raw_ms, monotonic_ns)) = res {
                        self.handle_telemetry_tick(TelemetryTickContext {
                            event: TelemetryTickEvent::MangoHudAlignment {
                                raw_ms,
                                monotonic_ns,
                            },
                        })
                        .await?;
                    }
                }
                Some(frame) = frame_rx.recv() => {
                    self.handle_frame_tick(FrameTickContext { frame }).await?;
                }
                _ = tokio::signal::ctrl_c() => {
                    return Ok(self.handle_ctrl_c_stop());
                }
                reason = &mut max_duration_future => {
                    return Ok(self.handle_max_duration_stop(reason));
                }
                _ = async {
                    if let Some(rx) = &mut stop_rx {
                        let _ = rx.await;
                        Some(())
                    } else {
                        futures_util::future::pending().await
                    }
                } => {
                    return Ok(self.handle_remote_stop());
                }

                _ = summary_tick.tick() => {
                    self.handle_summary_context_tick(SummaryTickContext).await?;
                }

                _ = epoch_tick.tick() => {
                    if let Some(reason) = self.handle_epoch_tick() {
                        return Ok(reason);
                    }
                }

                _ = optional_tick(tree_tick.as_mut()) => {
                    if let Some(reason) = self.handle_target_tick(TargetTickContext {
                        event: TargetTickEvent::Tree,
                    }).await? {
                        return Ok(reason);
                    }
                }

                _ = optional_tick(focus_tick.as_mut()) => {
                    self.handle_focus_context_tick(FocusTickContext).await?;
                }

                _ = optional_tick(foreground_tick.as_mut()) => {
                    self.handle_foreground_context_tick(ForegroundTickContext).await?;
                }

                _ = watch_tick.tick() => {
                    self.handle_target_tick(TargetTickContext {
                        event: TargetTickEvent::Watch,
                    }).await?;
                }

                _ = scx_tick.tick() => {
                    self.handle_telemetry_tick(TelemetryTickContext {
                        event: TelemetryTickEvent::Scx,
                    }).await?;
                }

                _ = hwmon_tick.tick() => {
                    self.handle_telemetry_tick(TelemetryTickContext {
                        event: TelemetryTickEvent::Hwmon,
                    }).await?;
                }

                maybe_event = async {
                    if let Some(reader) = &mut tui_event_reader {
                        futures_util::StreamExt::next(reader).await
                    } else {
                        futures_util::future::pending().await
                    }
                } => {
                    if let Some(Ok(event)) = maybe_event
                        && let Some(reason) = self.handle_ui_tick(UiTickContext { event })
                    {
                        return Ok(reason);
                    }
                }

                res = self.handle_probe_drain(ProbeDrainContext) => {
                    res?;
                }
            }
        }
    }

    async fn drain_bpf_events(&mut self) -> anyhow::Result<()> {
        let mut guard = self.runtime.probes.loaded.events.readable_mut().await?;
        let recording_monotonic_start_ns = self
            .runtime
            .outputs
            .recorder
            .run
            .as_ref()
            .and_then(|r| r.monotonic_start_ns);

        let mut pending_spikes = Vec::new();
        let mut pending_irqs = Vec::new();
        let mut pending_ios = Vec::new();
        let mut pending_monitor_events = Vec::new();

        let current_scx = scx_snapshot(&self.runtime.probes.scx_tracker);

        while let Some(item) = guard.get_inner_mut().next() {
            if item.len() < std::mem::size_of::<u32>() {
                log::warn!("short_bpf_event len={}", item.len());
                continue;
            }

            let kind = unsafe { (item.as_ptr() as *const u32).read_unaligned() };
            match kind {
                stutter_common::EVENT_RUNNABLE_LATENCY => {
                    if let Some(event) = crate::events::decode::read_event_unaligned::<
                        stutter_common::SchedulerEvent,
                    >(&item)
                    {
                        let update = crate::events::handle_event_with_runtime_config(
                            &event,
                            crate::events::EventHandlingContext {
                                config: &self.runtime.event_runtime_config,
                                started: self.started,
                                tasks: &mut self.runtime.targeting.tasks,
                                monotonic_start_ns: recording_monotonic_start_ns,
                                diagnostics: crate::events::interpret::SchedulerEventDiagnostics {
                                    scx_ops: current_scx.ops.as_deref(),
                                    scx_state: current_scx.state.as_deref(),
                                    scx_enable_seq: current_scx.enable_seq.as_deref(),
                                },
                            },
                        );
                        let crate::events::interpret::SchedulerSampleUpdate {
                            events,
                            spike_event,
                        } = update;
                        if let Some(spike) = spike_event {
                            pending_spikes.push(spike);
                        }
                        pending_monitor_events.extend(events);
                    } else {
                        log::warn!("short_scheduler_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_IRQ_LATENCY => {
                    if let Some(event) = crate::events::decode::read_event_unaligned::<
                        stutter_common::IrqEvent,
                    >(&item)
                    {
                        let record =
                            crate::events::irq_event_record(recording_monotonic_start_ns, &event);
                        pending_monitor_events.push(crate::events::handle_irq_record(&record));
                        pending_irqs.push(record);
                    } else {
                        log::warn!("short_irq_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_MIGRATION => {
                    if let Some(event) = crate::events::decode::read_event_unaligned::<
                        stutter_common::MigrationEvent,
                    >(&item)
                    {
                        pending_monitor_events.push(crate::events::handle_migration_event(
                            &event,
                            &mut self.runtime.targeting.tasks,
                            &self.cpu_to_pkg,
                            self.started,
                        ));
                    } else {
                        log::warn!("short_migration_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_CPU_FREQ => {
                    if let Some(event) = crate::events::decode::read_event_unaligned::<
                        stutter_common::CpuFreqEvent,
                    >(&item)
                    {
                        pending_monitor_events
                            .push(crate::events::handle_cpu_freq_event(&event, self.started));
                    } else {
                        log::warn!("short_cpu_freq_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_STAT_WAIT => {
                    if let Some(event) = crate::events::decode::read_event_unaligned::<
                        stutter_common::StatWaitEvent,
                    >(&item)
                    {
                        if let Some(stats) = self
                            .runtime
                            .targeting
                            .tasks
                            .stats_by_task
                            .get_mut(&event.tid)
                        {
                            stats.stat_wait_sum_ns += event.delay_ns as u128;
                            stats.stat_wait_count += 1;
                        }
                    } else {
                        log::warn!("short_stat_wait_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_BLOCK_IO => {
                    if let Some(event) = crate::events::decode::read_event_unaligned::<
                        stutter_common::BlockIoEvent,
                    >(&item)
                    {
                        let record = crate::events::block_io_event_record(
                            &event,
                            self.runtime
                                .probes
                                .loaded
                                .block_io_correlation_basis
                                .as_str(),
                            self.started,
                        );

                        pending_monitor_events.push(crate::events::handle_block_io_record(&record));
                        pending_ios.push(record);
                    } else {
                        log::warn!("short_block_io_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_EXEC => {
                    if self.config.safety.follow_exec {
                        let elapsed_ms = self.started.elapsed().as_millis() as u64;
                        if let Some(event) = crate::events::handle_exec_event(
                            &item,
                            &mut self.runtime.targeting.tasks,
                            elapsed_ms,
                        ) {
                            pending_monitor_events.push(event);
                        }
                    }
                }
                other => log::warn!("unknown_bpf_event kind={other} len={}", item.len()),
            }
        }

        guard.clear_ready();
        drop(guard);

        for event in pending_monitor_events {
            self.dispatch_monitor_event(event).await?;
        }

        for irq in pending_irqs {
            self.runtime.telemetry.push_irq(irq);
        }
        for io in pending_ios {
            self.runtime.telemetry.push_io(io);
        }
        for spike in pending_spikes {
            self.handle_live_spike(spike).await?;
        }

        Ok(())
    }

    pub fn handle_tui_event(&mut self, event: Event) -> Option<String> {
        if let Event::Key(key) = event {
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('Q') => return Some("quit".to_owned()),
                KeyCode::Char('p') | KeyCode::Char('P') => {
                    self.runtime.ui.tui_state.paused = !self.runtime.ui.tui_state.paused;
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    self.runtime.ui.tui_state.sort_field =
                        self.runtime.ui.tui_state.sort_field.next();
                }
                KeyCode::Char('f') | KeyCode::Char('F') => {
                    self.runtime.ui.tui_state.next_filter_class();
                }
                _ => {}
            }
        }
        None
    }

    pub async fn handle_summary_tick(&mut self) -> anyhow::Result<()> {
        if !self.runtime.ui.tui_state.paused {
            let elapsed_ms = self.started.elapsed().as_millis() as u64;
            if let Some(sampler) = self.runtime.probes.cpu_perf_sampler.as_mut() {
                let deltas = sampler.sample_interval();
                for (tid, delta) in deltas {
                    if let Some(stats) = self.runtime.targeting.tasks.stats_by_task.get_mut(&tid) {
                        stats.record_cpu_perf(&delta);
                    }
                }
            }

            let drop_counters_snapshot = self.runtime.probes.loaded.snapshot_drop_counters();
            let psi_snapshot = self.runtime.probes.psi_reader.read().ok();
            let records = collect_interval_summaries_labeled(
                self.interval_label,
                &mut self.runtime.targeting.tasks.stats_by_task,
                elapsed_ms,
                &drop_counters_snapshot,
                self.runtime.probes.loaded.prev_faults_map.as_ref(),
                psi_snapshot.as_ref(),
                &mut self.runtime.targeting.tasks.prev_faults_snapshot,
            );

            self.dispatch_monitor_event(MonitorEvent::Interval {
                elapsed_ms,
                records: records.clone(),
                drop_counters: drop_counters_snapshot.clone(),
            })
            .await?;

            if let Some(sampler) = self.runtime.probes.runtime_slice_sampler.as_mut() {
                let tasks = self
                    .runtime
                    .targeting
                    .tasks
                    .active_targets
                    .values()
                    .cloned()
                    .collect::<Vec<_>>();
                let batch = sampler.collect(
                    &tasks,
                    elapsed_ms,
                    self.config.timing.summary_period_ms,
                    self.config.runtime_slices.max_tasks,
                );
                self.runtime
                    .outputs
                    .recorder
                    .counters
                    .runtime_slice_read_errors = self
                    .runtime
                    .outputs
                    .recorder
                    .counters
                    .runtime_slice_read_errors
                    .saturating_add(batch.read_errors);
                self.runtime
                    .outputs
                    .recorder
                    .counters
                    .runtime_slice_skipped_tasks = self
                    .runtime
                    .outputs
                    .recorder
                    .counters
                    .runtime_slice_skipped_tasks
                    .saturating_add(batch.skipped_tasks as u64);

                if self
                    .runtime
                    .outputs
                    .recorder
                    .streams
                    .contains(ArtifactKind::RuntimeSlices)
                {
                    for record in &batch.records {
                        crate::artifacts::push_artifact_event(
                            &mut self.runtime.outputs.recorder,
                            ArtifactKind::RuntimeSlices,
                            record,
                            "runtime_slices",
                            |c| {
                                c.runtime_slice_count += 1;
                            },
                        );
                    }
                } else {
                    self.runtime.outputs.recorder.counters.runtime_slice_count = self
                        .runtime
                        .outputs
                        .recorder
                        .counters
                        .runtime_slice_count
                        .saturating_add(batch.records.len() as u64);
                }
            }

            if let Some(state) = self.runtime.outputs.prometheus_state.as_ref() {
                let max_p99 = records.iter().map(|r| r.p99_ns).max().unwrap_or(0);
                state.set_latest_p99_ns(max_p99);
                state.set_active_targets(self.runtime.targeting.tasks.active_targets.len() as u64);
                state.set_event_stream_write_errors(
                    self.runtime
                        .outputs
                        .recorder
                        .counters
                        .event_stream_write_errors,
                );
                state.set_ebpf_ringbuf_drops(drop_counters_snapshot.total());
            }
        }

        if let Some(term) = self.runtime.ui.terminal.as_mut() {
            let elapsed_ms = self.started.elapsed().as_millis() as u64;
            let drop_counters_snapshot = self.runtime.probes.loaded.snapshot_drop_counters();

            let snapshot = TuiRenderSnapshot {
                elapsed_ms,
                drop_counters: drop_counters_snapshot,
                tui_state: self.runtime.ui.tui_state.clone(),
                active_targets: self.runtime.targeting.tasks.active_targets.clone(),
                stats_by_task: self.runtime.targeting.tasks.stats_by_task.clone(),
                interval_records: self
                    .runtime
                    .outputs
                    .recorder
                    .buffers
                    .interval_records
                    .clone(),
                recent_diagnoses: self.runtime.telemetry.diagnoses.clone(),
                current_focus: self.current_focus.clone(),
                current_foreground: self.current_foreground.clone(),
                focus_switch_count: self.focus_switch_count,
                foreground_include_title: self.config.focus.foreground_include_title,
            };

            // TUI rendering errors and panics should be logged and dismissed,
            // not propagated, to avoid killing the monitor.
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                term.draw(move |f| {
                    crate::tui::render_tui(
                        f,
                        crate::tui::TuiRenderInput {
                            state: &snapshot.tui_state,
                            active_targets: &snapshot.active_targets,
                            stats_by_task: &snapshot.stats_by_task,
                            interval_records: &snapshot.interval_records,
                            recent_diagnoses: &snapshot.recent_diagnoses,
                            elapsed_ms: snapshot.elapsed_ms.into(),
                            drop_counters: &snapshot.drop_counters,
                            current_focus: snapshot.current_focus.as_ref(),
                            current_foreground: snapshot.current_foreground.as_ref(),
                            focus_switch_count: snapshot.focus_switch_count,
                            foreground_include_title: snapshot.foreground_include_title,
                        },
                    );
                })
            }));

            match res {
                Ok(Ok(_)) => {}
                Ok(Err(err)) => warn!("tui_render_failed err={err:#}"),
                Err(_) => {
                    warn!("tui_render_panic");
                }
            }
        }

        Ok(())
    }

    pub async fn handle_tree_tick(&mut self) -> anyhow::Result<Option<String>> {
        let mut should_exit = None;

        if let Some(root_pid) = self.runtime.targeting.watch_state.running_pid()
            && tree_root_is_stale(root_pid, &self.runtime.targeting.tree_root_starttimes)
        {
            self.runtime.targeting.remove_watch_root(root_pid);

            if !self.config.target.persistent {
                should_exit = Some("watched_process_exit".to_owned());
            } else {
                self.runtime.targeting.watch_state = WatchProcessState::Waiting;
                info!("watch_process_waiting_for_relaunch");
            }
        } else {
            let removed_roots = self.runtime.targeting.remove_stale_dynamic_tree_roots();

            for root in &removed_roots {
                info!("tree_root_removed pid={root}");
            }

            if !removed_roots.is_empty()
                && self.had_tree_roots
                && self.runtime.targeting.effective_tree_pids().is_empty()
                && !matches!(
                    self.runtime.targeting.watch_state,
                    WatchProcessState::Waiting
                )
            {
                should_exit = Some("tree_root_exit".to_owned());
            }
        }

        self.refresh_tasks_and_emit_snapshot().await?;

        // Belt-and-suspenders cleanup in case a refresh path exits before
        // emitting per-task removal diffs.
        self.runtime
            .targeting
            .tasks
            .prev_faults_snapshot
            .retain(|tid, _| {
                self.runtime
                    .targeting
                    .tasks
                    .active_targets
                    .contains_key(tid)
            });

        Ok(should_exit)
    }

    pub async fn handle_watch_tick(&mut self) -> anyhow::Result<()> {
        let Some(pattern) = self.runtime.targeting.watch_config.pattern.clone() else {
            return Ok(());
        };

        if !self.runtime.targeting.watch_state.should_poll() {
            return Ok(());
        }

        if let Some(pid) = find_process_by_pattern_at_with_cache(
            Path::new("/proc"),
            &pattern,
            &mut self.runtime.targeting.process_cache,
        ) {
            self.runtime.targeting.add_watch_root(pid);
            self.runtime.targeting.watch_state = WatchProcessState::Running(pid);
            info!("watch_process_relaunched pattern={} pid={}", pattern, pid);

            self.refresh_tasks_and_emit_snapshot().await?;
        }

        Ok(())
    }

    pub async fn handle_scx_tick(&mut self) -> anyhow::Result<()> {
        if let Some(event) = self
            .runtime
            .probes
            .scx_tracker
            .sample(self.started.elapsed().as_millis() as u64)
        {
            self.dispatch_monitor_event(MonitorEvent::ScxEvent {
                event: Box::new(event),
            })
            .await?;
        }

        Ok(())
    }

    pub async fn handle_hwmon_tick(&mut self) -> anyhow::Result<()> {
        if let Some(reader_arc) = &self.hwmon_reader {
            let elapsed = self.started.elapsed().as_millis() as u64;
            let reader_arc_clone = reader_arc.clone();

            let sample_opt = task::spawn_blocking(move || {
                if let Ok(mut reader) = reader_arc_clone.lock() {
                    Some(reader.sample(elapsed))
                } else {
                    None
                }
            })
            .await
            .map_err(|err| anyhow::anyhow!("hwmon worker failed: {err}"))?;

            if let Some(sample) = sample_opt {
                self.runtime.telemetry.push_gpu(sample.clone());
                self.dispatch_monitor_event(MonitorEvent::GpuSample {
                    sample: Box::new(sample),
                })
                .await?;
            }
        }
        Ok(())
    }

    async fn handle_live_spike(&mut self, spike: SpikeEvent) -> anyhow::Result<()> {
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        self.runtime.telemetry.push_spike(spike);
        // Prune old telemetry (history window controlled by LiveTelemetry::max_age_ms)
        self.runtime.telemetry.prune(elapsed_ms.into());

        // Form a cluster from spikes within cluster_window_ms
        let cluster_window_ms = LIVE_DIAGNOSIS_CLUSTER_WINDOW_MS;
        let cluster_window_ns = cluster_window_ms * 1_000_000;

        let recent_spikes: Vec<_> = self
            .runtime
            .telemetry
            .spikes
            .iter()
            .filter(|s| elapsed_ms.saturating_sub(s.elapsed_ms.unwrap_or(0)) <= cluster_window_ms)
            .cloned()
            .collect();

        if recent_spikes.is_empty() {
            return Ok(());
        }

        let mut points = Vec::new();
        for s in &recent_spikes {
            points.push(crate::spike::SpikePoint {
                task: s.task,
                class: s.class,
                process_pid: s.process_pid,
                comm: s.comm.clone(),
                cpu: s.cpu,
                wakeup_target_cpu: s.wakeup_target_cpu,
                latency_ns: s.latency_ns,
                wakeup_ns: s.wakeup_ns,
                switch_ns: s.switch_ns,
                target_pending_wakeups: s.target_pending_wakeups,
                observed_runnable_depth: s.observed_runnable_depth,
                switch_prev_pid: s.switch_prev_pid,
                switch_prev_state: s.switch_prev_state,
                elapsed_ms: s.elapsed_ms,
                scx_ops: s.scx_ops.clone(),
                scx_state: s.scx_state.clone(),
                cause_tags: s.cause_tags.clone(),
                primary_cause: s.primary_cause.clone(),
                ..Default::default()
            });
        }

        let distinct_tasks = points.iter().map(|p| p.task).collect::<HashSet<_>>().len();
        let min_switch_ns = points.iter().map(|p| p.switch_ns).min().unwrap_or(0);
        let max_switch_ns = points.iter().map(|p| p.switch_ns).max().unwrap_or(0);
        let max_latency_ns = points.iter().map(|p| p.latency_ns).max().unwrap_or(0);

        let mut cluster = crate::spike::SpikeCluster {
            points,
            distinct_tasks,
            min_switch_ns,
            max_switch_ns,
            max_latency_ns,
            ..Default::default()
        };

        // Build a RunArtifacts-like snapshot from recent telemetry and let
        // `diagnose_cluster` perform time filtering itself.
        let artifacts = crate::session_io::RunArtifacts {
            irq_events: self.runtime.telemetry.irq_events.iter().cloned().collect(),
            gpu_samples: self.runtime.telemetry.gpu_samples.iter().cloned().collect(),
            block_io_events: self.runtime.telemetry.io_events.iter().cloned().collect(),
            intervals: self
                .runtime
                .outputs
                .recorder
                .buffers
                .interval_records
                .clone(),
            ..Default::default()
        };

        let diagnosis = diagnose_cluster(&cluster, &artifacts, cluster_window_ns);
        let anchor = crate::diagnosis::select_anchor(&cluster);
        cluster.anchor_task = Some(anchor.task);
        cluster.anchor_class = Some(anchor.class);
        cluster.anchor_comm = Some(anchor.comm.clone());
        cluster.anchor_kind = Some(anchor.kind);

        if diagnosis.cause != crate::diagnosis::StutterCause::Unknown {
            let entry = LiveDiagnosisEntry {
                elapsed_ms,
                cause: diagnosis.cause,
                confidence: diagnosis.confidence,
                anchor_class: anchor.class,
                anchor_comm: anchor.comm.clone(),
                evidence: diagnosis.evidence.clone(),
            };

            self.runtime.telemetry.diagnoses.push_back(entry.clone());
            self.dispatch_monitor_event(MonitorEvent::LiveDiagnosis {
                entry: Box::new(entry),
            })
            .await?;

            log::info!(
                "live_diagnosis cause={:?} confidence={:?} evidence={:?}",
                diagnosis.cause,
                diagnosis.confidence,
                diagnosis.evidence
            );
        }

        Ok(())
    }

    pub async fn refresh_tasks(&mut self) -> anyhow::Result<()> {
        let targeting = &mut self.runtime.targeting;
        let policy = &targeting.policy;
        let dynamic_tree_pids = &targeting.dynamic_tree_pids;
        let process_cache = &mut targeting.process_cache;
        let tasks = &mut targeting.tasks;
        let target_snapshot_input = TargetController::target_snapshot_input_from_parts(
            policy,
            dynamic_tree_pids,
            process_cache,
            self.community_rules.as_db(),
        );

        let budget_report = tasks
            .refresh(crate::tasks::RefreshInput {
                target_snapshot_input,
                max_tasks: policy.max_tasks,
                tree_events: &mut self.runtime.outputs.recorder.buffers.tree_events,
                target_pid_map: &mut self.runtime.probes.loaded.target_pid_map,
                prev_faults_map: self.runtime.probes.loaded.prev_faults_map.as_mut(),
                elapsed_ms: self.started.elapsed().as_millis() as u64,
                recording_started: self
                    .runtime
                    .outputs
                    .recorder
                    .run
                    .as_ref()
                    .map(|run| run.started_instant),
            })
            .await?;

        if budget_report.scan_timed_out {
            self.runtime
                .outputs
                .recorder
                .counters
                .process_scan_budget_exceeded_count += 1;
        }

        self.runtime
            .outputs
            .recorder
            .counters
            .thread_scan_limited_count += budget_report.processes_thread_limited as u64;

        if let Some(sampler) = self.runtime.probes.cpu_perf_sampler.as_mut() {
            sampler.sync_targets(
                &self.runtime.targeting.tasks.active_targets,
                &self.runtime.targeting.tasks.stats_by_task,
            );
        }

        Ok(())
    }

    pub fn finalize(mut self, stop_reason: String) -> anyhow::Result<String> {
        if let Some(term) = self.runtime.ui.terminal.as_mut() {
            let _ = crate::tui::restore_terminal(term);
        }

        let drop_counters = self.runtime.probes.loaded.snapshot_drop_counters();
        log_drop_counters(&drop_counters);

        if let Some(dropped) = self
            .runtime
            .outputs
            .recorder
            .exporters
            .otel_spans_dropped
            .as_ref()
        {
            let count = dropped.load(std::sync::atomic::Ordering::Relaxed);
            let stdout_is_machine_stream =
                self.config.outputs.json_stream || self.config.csv_streams_to_stdout();
            if count > 0 && !stdout_is_machine_stream {
                println!("OpenTelemetry export: dropped {count} spans due to channel pressure");
            }
        }
        let stdout_is_machine_stream =
            self.config.outputs.json_stream || self.config.csv_streams_to_stdout();
        if self.config.timing.epoch_period_ms.is_none() && !stdout_is_machine_stream {
            print_session_summaries(&mut self.runtime.targeting.tasks.stats_by_task);
        }

        if let Some(writer) = self.runtime.outputs.recorder.csv_writer.as_mut() {
            writer.finish()?;
            if let Some(CsvStreamTarget::File(path)) = &self.config.streams.csv
                && !self.config.outputs.json_stream
            {
                println!("wrote interval CSV: {}", path.display());
            }
        }

        if self.runtime.outputs.recorder.run.is_some() {
            self.runtime.outputs.recorder.streams.finish_all()?;

            let frame_events = if !self.config.mangohud.log_live
                && let Some(path) = &self.config.mangohud.log
            {
                let (alignment_monotonic_ns, alignment_raw_elapsed_ms, mangohud_ignore_offset) =
                    if let Some(run) = self.runtime.outputs.recorder.run.as_ref() {
                        (
                            run.mangohud_first_frame_monotonic_ns,
                            run.mangohud_first_frame_raw_elapsed_ms,
                            run.mangohud_start_offset.unwrap_or(0),
                        )
                    } else {
                        (None, None, 0)
                    };

                match mangohud::read_frame_events(
                    path,
                    mangohud_ignore_offset,
                    alignment_monotonic_ns,
                    alignment_raw_elapsed_ms,
                    self.runtime
                        .outputs
                        .recorder
                        .run
                        .as_ref()
                        .and_then(|r| r.monotonic_start_ns),
                ) {
                    Ok(events) => events,
                    Err(err) => {
                        warn!(
                            "mangohud_log_read_failed path={} err={err:#}",
                            path.display()
                        );
                        Vec::new()
                    }
                }
            } else {
                Vec::new()
            };

            recorder::finalize_recording(FinalizeRecordingInput {
                recorder: &self.runtime.outputs.recorder,
                config: &self.config,
                tree_pids: self.runtime.targeting.effective_tree_pids(),
                stop_reason: &stop_reason,
                tasks: &self.runtime.targeting.tasks,
                frame_events: &frame_events,
                block_io_correlation_basis: self.runtime.probes.block_io_correlation_basis.clone(),
                block_io_correlation_confidence: self
                    .runtime
                    .probes
                    .block_io_correlation_confidence
                    .clone(),
                focus_mode: if self.config.focus.auto_focus {
                    Some("auto".to_owned())
                } else if self.config.has_explicit_target() {
                    Some("explicit".to_owned())
                } else {
                    Some("legacy-auto-detect".to_owned())
                },
                final_focus_kind: self
                    .current_focus
                    .as_ref()
                    .map(|focus| format!("{:?}", focus.group.kind)),
                focus_switch_count: self.focus_switch_count,
                current_focus: self.current_focus.clone(),
                final_foreground_event: self.runtime.outputs.recorder.last_foreground_event.clone(),
                drop_counters,
                cpu_perf_status: self
                    .runtime
                    .probes
                    .cpu_perf_sampler
                    .as_ref()
                    .map(|sampler| recorder::CpuPerfStatus {
                        sample_count: sampler.total_samples(),
                        active_counter_tasks: sampler.active_counter_tasks() as u64,
                        skipped_counter_tasks: sampler.skipped_counter_tasks() as u64,
                        open_errors: sampler.total_open_errors(),
                        read_errors: sampler.total_read_errors(),
                        last_error: sampler.last_error().map(str::to_owned),
                    }),
            })?;

            recorder::print_recording_warnings(&self.runtime.outputs.recorder);
        }

        info!("exiting stop_reason={stop_reason}");
        Ok(stop_reason)
    }
}

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

#[cfg(test)]
#[path = "session/tests.rs"]
mod tree_tick_tests;

#[cfg(test)]
mod tests {
    #[test]
    fn session_child_modules_are_not_public_submodules() {
        let source = include_str!("session.rs");

        let public_child_modules: Vec<&str> = source
            .lines()
            .map(str::trim_start)
            .filter(|line| line.starts_with("pub mod "))
            .collect();

        assert!(
            public_child_modules.is_empty(),
            "session child modules must stay crate-private and be exposed intentionally through api::session: {public_child_modules:?}"
        );
    }
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
