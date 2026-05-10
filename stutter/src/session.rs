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
    cli::{Config, CsvStreamTarget, FocusSource, ForegroundSourceArg},
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
        sinks::{MonitorEventSink, RecorderSink},
        targeting::TargetController,
        ui::{TuiRenderSnapshot, TuiRuntime},
    },
    session_events::MonitorEvent,
    watch::{
        WatchProcessState, add_watch_tree_pid, capture_tree_root_starttimes,
        find_process_by_pattern_at_with_cache, process_root_starttime, remove_stale_tree_roots,
        remove_watch_tree_pid, resolve_watch_process, tree_root_is_stale,
    },
};

#[path = "session/event_bus.rs"]
pub mod event_bus;
#[path = "session/live_telemetry.rs"]
pub mod live_telemetry;
#[path = "session/outputs.rs"]
pub mod outputs;
#[path = "session/probes.rs"]
pub mod probes;
#[path = "session/runtime.rs"]
pub mod runtime;
#[path = "session/sinks.rs"]
pub mod sinks;
#[path = "session/targeting.rs"]
pub mod targeting;

#[path = "session/ui.rs"]
pub mod ui;

const LIVE_DIAGNOSIS_CLUSTER_WINDOW_MS: u64 = 5;
// Keep this aligned with the report default unless live diagnosis gets
// its own CLI/config field.

fn needs_tree_tick_from_parts(
    had_tree_roots: bool,
    watch_process_active: bool,
    cgroupv2_active: bool,
) -> bool {
    had_tree_roots || watch_process_active || cgroupv2_active
}

fn needs_tree_tick(config: &Config, had_tree_roots: bool) -> bool {
    needs_tree_tick_from_parts(
        had_tree_roots,
        config.watch_process.is_some(),
        config.cgroupv2.is_some(),
    )
}

fn foreground_capture_enabled(config: &Config) -> bool {
    config.foreground_window || (config.auto_focus && config.focus_source != FocusSource::Heuristic)
}

fn foreground_provider_for_config(
    config: &Config,
) -> Box<dyn crate::foreground::ForegroundProvider + Send> {
    match config.foreground_source {
        ForegroundSourceArg::Auto => crate::foreground::auto_foreground_provider(),
        ForegroundSourceArg::Sway => Box::new(crate::foreground::SwayForegroundProvider::new()),
        ForegroundSourceArg::Hyprland => {
            Box::new(crate::foreground::UnsupportedForegroundProvider::new(
                "Hyprland foreground provider is not implemented yet; no safe generic Wayland foreground-window API detected",
            ))
        }
        ForegroundSourceArg::X11 => Box::new(crate::foreground::X11ForegroundProvider::new()),
    }
}

fn foreground_resolver_from_config(config: &Config) -> crate::foreground::ForegroundResolver {
    crate::foreground::ForegroundResolver::new(foreground_provider_for_config(config))
        .with_include_title(config.foreground_include_title)
        .with_max_stale_ms(config.foreground_max_stale_ms)
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

#[cfg(test)]
mod foreground_session_tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn foreground_snapshot(
        elapsed_ms: u64,
        status: crate::foreground::ForegroundProviderStatus,
        pid: Option<u32>,
        app_id: Option<&str>,
        class: Option<&str>,
        window_id: Option<&str>,
        workspace: Option<&str>,
        confidence: f32,
    ) -> crate::foreground::ForegroundWindowSnapshot {
        crate::foreground::ForegroundWindowSnapshot {
            elapsed_ms,
            source: Some(crate::foreground::ForegroundSource::Sway),
            status,
            pid,
            app_id: app_id.map(str::to_owned),
            class: class.map(str::to_owned),
            title: None,
            window_id: window_id.map(str::to_owned),
            workspace: workspace.map(str::to_owned),
            confidence,
            stale_ms: None,
            reason: "test foreground snapshot".to_owned(),
        }
    }

    #[test]
    fn foreground_identity_records_first_sample() {
        let snapshot = foreground_snapshot(
            100,
            crate::foreground::ForegroundProviderStatus::Available,
            Some(4242),
            Some("steam"),
            Some("Steam"),
            Some("7"),
            Some("games"),
            0.95,
        );

        assert!(foreground_identity_changed(None, &snapshot));
    }

    #[test]
    fn foreground_identity_changes_on_provider_status_transition() {
        let old = foreground_snapshot(
            100,
            crate::foreground::ForegroundProviderStatus::Available,
            Some(4242),
            Some("steam"),
            Some("Steam"),
            Some("7"),
            Some("games"),
            0.95,
        );
        let new = foreground_snapshot(
            200,
            crate::foreground::ForegroundProviderStatus::Error,
            Some(4242),
            Some("steam"),
            Some("Steam"),
            Some("7"),
            Some("games"),
            0.0,
        );

        assert!(foreground_identity_changed(Some(&old), &new));
    }

    #[test]
    fn foreground_identity_changes_on_window_identity_transition() {
        let old = foreground_snapshot(
            100,
            crate::foreground::ForegroundProviderStatus::Available,
            Some(4242),
            Some("steam"),
            Some("Steam"),
            Some("7"),
            Some("games"),
            0.95,
        );
        let new = foreground_snapshot(
            200,
            crate::foreground::ForegroundProviderStatus::Available,
            Some(9000),
            Some("firefox"),
            Some("Firefox"),
            Some("8"),
            Some("web"),
            0.95,
        );

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
    pub config: Arc<Config>,
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
    community_rules: Option<crate::community_rules::CommunityRulesDb>,
}

impl MonitorSession {
    pub async fn new(
        mut config: Config,
        shared_hwmon: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
        event_tx: Option<tokio::sync::mpsc::Sender<MonitorEvent>>,
    ) -> anyhow::Result<Self> {
        let explicit_target = config.has_explicit_target();

        let mut focus_resolver = None;
        let mut current_focus = None;
        let focus_switch_count = 0;
        let foreground_enabled = foreground_capture_enabled(&config);
        let foreground_resolver =
            foreground_enabled.then(|| foreground_resolver_from_config(&config));
        let current_foreground = None;
        let foreground_switch_count = 0;

        let user_config = crate::config_file::load_user_config()?;

        if let Some(layer) = &config.monitor_config_layer {
            log::info!(
                "monitor_session_config_resolved source=presence_aware summary_period_ms={} spike_threshold_ns={} max_tasks={} hwmon={} cpu_freq={} foreground_window={} focus_source={:?} foreground_source={:?}",
                config.summary_period_ms,
                config.spike_threshold_ns,
                config.max_tasks,
                config.hwmon,
                config.cpu_freq,
                config.foreground_window,
                config.focus_source,
                config.foreground_source,
            );
            log::debug!("monitor_config_layer: {:?}", layer);
        } else {
            log::info!(
                "monitor_session_config_legacy source=legacy_cloned summary_period_ms={} spike_threshold_ns={} max_tasks={} hwmon={} cpu_freq={} foreground_window={} focus_source={:?} foreground_source={:?}",
                config.summary_period_ms,
                config.spike_threshold_ns,
                config.max_tasks,
                config.hwmon,
                config.cpu_freq,
                config.foreground_window,
                config.focus_source,
                config.foreground_source,
            );
        }

        let community_rules = crate::community_rules::load_community_rules(
            &crate::config_file::community_rules_config_from_user_config(user_config.as_ref()),
        )
        .map_err(|err| {
            log::warn!("failed_to_load_community_rules err={err:#}");
            err
        })
        .ok();

        if !explicit_target && config.auto_focus {
            let policy = FocusPolicy {
                poll_ms: config.auto_focus_poll_ms,
                min_confidence: config.auto_focus_min_confidence,
                switch_margin: config.auto_focus_switch_margin,
                switch_cooldown_ms: config.auto_focus_switch_cooldown_ms,
                required_winner_polls: config.auto_focus_required_polls,
                max_roots: config.auto_focus_max_roots,
            };

            let mut resolver = FocusResolver::new(policy);
            match resolver.sample(Path::new("/proc"), 0, None, FocusSource::Heuristic) {
                FocusDecision::Switch { new, .. } | FocusDecision::Keep { focus: new } => {
                    config.tree_pids = new.group.root_pids.clone();
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
            let stdout_is_machine_stream = config.json_stream || config.csv_streams_to_stdout();
            if !stdout_is_machine_stream {
                println!(
                    "auto-detected game launcher: {class} (PIDs {pids:?}). monitoring tree..."
                );
            }
            config.tree_pids = pids;
        }

        let mut tree_pids = config.tree_pids.clone();
        let watch_state = match resolve_watch_process(&config, &mut tree_pids).await? {
            Some(pid) => WatchProcessState::Running(pid),
            None => WatchProcessState::None,
        };

        let had_tree_roots = !tree_pids.is_empty();
        let tree_root_starttimes = capture_tree_root_starttimes(&tree_pids);

        let recording = recorder::prepare_recording(&config)?;
        let mut loaded = ebpf_loader::load_and_attach(&config)?;
        configure_target_irqs(&mut loaded, &config)?;
        let block_io_correlation_basis = loaded.block_io_correlation_basis.as_str().to_owned();

        let mut recorder = LiveRecorder {
            run: recording,
            ..Default::default()
        };
        if config.json_stream {
            recorder.enable_stdout_spike_stream();
        }

        let (prometheus_state, prometheus_task) = if let Some(port) = config.metrics_port {
            let state = Arc::new(crate::prometheus::PrometheusState::new_started_now());
            let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
            let task = crate::prometheus::spawn_metrics_server(addr, state.clone()).await?;
            info!("prometheus metrics listening on http://127.0.0.1:{port}/metrics");
            (Some(state), Some(task))
        } else {
            (None, None)
        };

        recorder.exporters.prometheus_state = prometheus_state.clone();
        recorder.buffers.spike_events = recorder.run.as_ref().map(|_| SpikeEventBuffer::default());

        if let Some(run) = recorder.run.as_mut() {
            if let Some(path) = &config.mangohud_log
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

            for kind in loaded.activation_plan.required_stream_artifacts() {
                registry.create_stream(dir, kind)?;
            }
        }

        if let Some(csv_stream) = &config.csv_stream {
            recorder.csv_writer = Some(match csv_stream {
                CsvStreamTarget::File(path) => {
                    recorder::IntervalCsvWriter::create_file(path.clone())?
                }
                CsvStreamTarget::Stdout => recorder::IntervalCsvWriter::stdout(),
            });
        }

        let metadata = crate::metadata::collect_system_metadata();
        let cpu_to_pkg: BTreeMap<u32, String> = metadata
            .cpu_topology
            .iter()
            .map(|c| (c.cpu, c.physical_package_id.clone().unwrap_or_default()))
            .collect();

        let hwmon_reader = if !config.hwmon {
            None
        } else if let Some(shared) = shared_hwmon {
            Some(shared)
        } else {
            hwmon::HwmonReader::discover_with_options(
                config.hwmon_root.as_deref(),
                config.hwmon_drm_card.as_deref(),
                config.hwmon_render_node.as_deref(),
            )
            .map(|r| Arc::new(std::sync::Mutex::new(r)))
        };

        if config.hwmon && hwmon_reader.is_none() {
            warn!("hwmon_requested_but_no_gpu_hwmon_found");
        }

        let started = Instant::now();

        let tui_state = crate::tui::TuiState::default();
        let terminal = if config.tui {
            Some(
                crate::tui::init_terminal()
                    .map_err(|e| anyhow::anyhow!("failed to init terminal: {e}"))?,
            )
        } else {
            None
        };

        let interval_label = if config.epoch_period_ms.is_some() {
            "epoch"
        } else {
            "summary"
        };

        let alert_sender = if config.alert_threshold_ns.is_some() {
            let (tx, mut rx) = tokio::sync::mpsc::channel(100);
            let webhook_url = config.alert_webhook_url.clone();
            let webhook_client = webhook_url.as_ref().map(|_| {
                reqwest::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .build()
            });
            tokio::spawn(async move {
                while let Some(payload) = rx.recv().await {
                    if let Err(err) = crate::events::send_desktop_alert(&payload).await {
                        warn!("desktop_alert_failed err={err}");
                    }
                    if let Some(url) = &webhook_url {
                        match &webhook_client {
                            Some(Ok(client)) => {
                                if let Err(err) = crate::events::send_webhook_alert_with_client(
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

        let cpu_perf_sampler = if config.cpu_perf {
            Some(crate::perf_counters::CpuPerfSampler::new(
                crate::perf_counters::CpuPerfConfig {
                    include_kernel: config.cpu_perf_kernel,
                    max_tasks: config.cpu_perf_max_tasks,
                    collect_cache_refs: config.cpu_perf_cache_refs,
                },
            ))
        } else {
            None
        };
        let runtime_slice_sampler = config.runtime_slices.then(RuntimeSliceSampler::new);

        let probes = ProbeRuntime::new(
            loaded,
            block_io_correlation_basis,
            cpu_perf_sampler,
            runtime_slice_sampler,
        );

        let mut otel_exporter = None;
        if let Some(endpoint) = config.otlp_endpoint.as_ref() {
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
                service_name: config.otel_service_name.clone(),
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

        let targeting =
            TargetController::from_config_parts(tree_pids, watch_state, tree_root_starttimes);

        let outputs = OutputRuntime::from_parts(
            recorder,
            prometheus_state,
            prometheus_task,
            otel_exporter,
            alert_sender,
        );

        let ui = TuiRuntime::from_parts(tui_state, terminal);

        let runtime = MonitorRuntime::from_config_parts(
            probes,
            outputs,
            ui,
            targeting,
            MonitorEventBus::new(event_tx),
        );

        Ok(Self {
            config: Arc::new(config),
            runtime,
            cpu_to_pkg,
            hwmon_reader,
            community_rules,
            focus_resolver,
            current_focus,
            focus_switch_count,
            foreground_resolver,
            current_foreground,
            foreground_switch_count,
            started,
            had_tree_roots,
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

        self.emit(MonitorEvent::TargetSnapshot {
            elapsed_ms,
            active_targets: self.runtime.targeting.tasks.active_targets.clone(),
            removed_targets,
        })
        .await;

        Ok(())
    }

    fn dispatch_recorder_event(&mut self, event: &MonitorEvent) -> anyhow::Result<()> {
        RecorderSink::new(&mut self.runtime.outputs.recorder)
            .on_event(event)
            .map_err(|err| anyhow::anyhow!(err))
    }

    fn foreground_event_for_snapshot(
        &self,
        snapshot: &crate::foreground::ForegroundWindowSnapshot,
    ) -> Option<MonitorEvent> {
        snapshot
            .to_event(self.config.foreground_include_title)
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

        self.dispatch_recorder_event(&event)?;
        self.emit(event).await;

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

        self.dispatch_recorder_event(&event)?;
        self.emit(event).await;

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
            self.config.focus_source,
        );

        match decision {
            FocusDecision::Switch { old, new } => {
                self.runtime
                    .targeting
                    .replace_tree_roots(new.group.root_pids.clone());
                self.had_tree_roots = self.runtime.targeting.has_tree_roots();
                self.current_focus = Some(new.clone());
                self.focus_switch_count = self.focus_switch_count.saturating_add(1);
                self.refresh_tasks_and_emit_snapshot().await?;
                self.emit_focus_changed(elapsed_ms, old.as_ref(), &new)
                    .await?;
            }
            FocusDecision::Clear { old, reason } => {
                self.runtime.targeting.clear_tree_roots();
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
                self.dispatch_recorder_event(&event)?;
                self.emit(event).await;
            }
        }

        self.current_foreground = Some(snapshot);

        Ok(())
    }

    pub async fn run(
        &mut self,
        mut stop_rx: Option<tokio::sync::oneshot::Receiver<()>>,
    ) -> anyhow::Result<String> {
        let mut summary_tick = interval(Duration::from_millis(self.config.summary_period_ms));
        summary_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let epoch_tick_duration = self
            .config
            .epoch_period_ms
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_secs(3600 * 24 * 365));
        let mut epoch_tick = interval(epoch_tick_duration);
        epoch_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut tree_tick = if needs_tree_tick(&self.config, self.had_tree_roots) {
            let mut tick = interval(Duration::from_millis(2_000));
            tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
            Some(tick)
        } else {
            None
        };

        let mut focus_tick = if self.focus_resolver.is_some() {
            let mut tick = interval(Duration::from_millis(self.config.auto_focus_poll_ms));
            tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
            Some(tick)
        } else {
            None
        };

        let mut foreground_tick = if self.foreground_resolver.is_some() {
            let mut tick = interval(Duration::from_millis(self.config.foreground_poll_ms));
            tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
            Some(tick)
        } else {
            None
        };

        let mut watch_tick = interval(Duration::from_millis(self.config.watch_poll_ms));
        watch_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut scx_tick = interval(Duration::from_millis(1_000));
        scx_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut hwmon_tick = interval(Duration::from_millis(1_000));
        hwmon_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut tui_event_reader = if self.config.tui {
            Some(crossterm::event::EventStream::new())
        } else {
            None
        };

        let max_duration = self.config.max_duration;
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
            && let Some(path) = self.config.mangohud_log.clone()
            && let Some(offset) = run.mangohud_start_offset
        {
            let path_clone = path.clone();
            tokio::spawn(async move {
                if let Ok(res) = mangohud::poll_alignment(&path_clone, offset).await {
                    let _ = mangohud_tx.send(res);
                }
            });

            if self.config.mangohud_log_live {
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
                    if let Ok((raw_ms, monotonic_ns)) = res
                        && let Some(run) = self.runtime.outputs.recorder.run.as_mut()
                    {
                        run.mangohud_first_frame_raw_elapsed_ms = Some(raw_ms);
                        run.mangohud_first_frame_monotonic_ns = Some(monotonic_ns);
                        info!(
                            "mangohud_alignment_observed raw_ms={} monotonic_ns={}",
                            raw_ms, monotonic_ns
                        );
                    }
                }
                Some(frame) = frame_rx.recv() => {
                    crate::events::push_artifact_event(&mut self.runtime.outputs.recorder, ArtifactKind::FrameEvents, &frame, "frame_events", |c| {
                        c.frame_event_count += 1;
                    });
                    self.runtime.telemetry.push_frame(frame);
                }
                _ = tokio::signal::ctrl_c() => {
                    return Ok("ctrl_c".to_owned());
                }
                reason = &mut max_duration_future => {
                    return Ok(reason.unwrap());
                }
                _ = async {
                    if let Some(rx) = &mut stop_rx {
                        let _ = rx.await;
                        Some(())
                    } else {
                        futures_util::future::pending().await
                    }
                } => {
                    return Ok("remote_stop".to_owned());
                }

                _ = summary_tick.tick() => {
                    self.handle_summary_tick().await?;
                }

                _ = epoch_tick.tick() => {
                    if self.config.epoch_period_ms.is_some() {
                        return Ok("epoch_ended".to_owned());
                    }
                }

                _ = async {
                    if let Some(tick) = tree_tick.as_mut() {
                        tick.tick().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    if let Some(reason) = self.handle_tree_tick().await? {
                        return Ok(reason);
                    }
                }

                _ = async {
                    if let Some(tick) = focus_tick.as_mut() {
                        tick.tick().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    self.handle_focus_tick().await?;
                }

                _ = async {
                    if let Some(tick) = foreground_tick.as_mut() {
                        tick.tick().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    self.handle_foreground_tick().await?;
                }

                _ = watch_tick.tick() => {
                    self.handle_watch_tick().await?;
                }

                _ = scx_tick.tick() => {
                    self.handle_scx_tick();
                }

                _ = hwmon_tick.tick() => {
                    self.handle_hwmon_tick().await?;
                }

                maybe_event = async {
                    if let Some(reader) = &mut tui_event_reader {
                        futures_util::StreamExt::next(reader).await
                    } else {
                        futures_util::future::pending().await
                    }
                } => {
                    if let Some(Ok(event)) = maybe_event
                        && let Some(reason) = self.handle_tui_event(event)
                    {
                        return Ok(reason);
                    }
                }

                res = self.drain_bpf_events() => {
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

        let current_scx = scx_snapshot(&self.runtime.probes.scx_tracker);

        while let Some(item) = guard.get_inner_mut().next() {
            if item.len() < std::mem::size_of::<u32>() {
                log::warn!("short_bpf_event len={}", item.len());
                continue;
            }

            let kind = unsafe { (item.as_ptr() as *const u32).read_unaligned() };
            match kind {
                stutter_common::EVENT_RUNNABLE_LATENCY => {
                    if let Some(event) =
                        crate::events::read_event_unaligned::<stutter_common::SchedulerEvent>(&item)
                    {
                        let spike = crate::events::handle_event(
                            &event,
                            &self.config,
                            self.started,
                            &mut self.runtime.targeting.tasks,
                            recording_monotonic_start_ns,
                            &mut self.runtime.outputs.recorder,
                            self.runtime.outputs.alert_sender.as_ref(),
                            current_scx.ops.as_deref(),
                            current_scx.state.as_deref(),
                            current_scx.enable_seq.as_deref(),
                        );
                        if let Some(spike) = spike {
                            pending_spikes.push(spike);
                        }
                    } else {
                        log::warn!("short_scheduler_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_IRQ_LATENCY => {
                    if let Some(event) =
                        crate::events::read_event_unaligned::<stutter_common::IrqEvent>(&item)
                    {
                        let record =
                            crate::events::irq_event_record(recording_monotonic_start_ns, &event);
                        crate::events::handle_irq_record(
                            &record,
                            &mut self.runtime.outputs.recorder,
                        );
                        pending_irqs.push(record);
                    } else {
                        log::warn!("short_irq_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_MIGRATION => {
                    if let Some(event) =
                        crate::events::read_event_unaligned::<stutter_common::MigrationEvent>(&item)
                    {
                        crate::events::handle_migration_event(
                            &event,
                            &mut self.runtime.targeting.tasks,
                            &mut self.runtime.outputs.recorder,
                            &self.cpu_to_pkg,
                            self.started,
                        );
                    } else {
                        log::warn!("short_migration_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_CPU_FREQ => {
                    if let Some(event) =
                        crate::events::read_event_unaligned::<stutter_common::CpuFreqEvent>(&item)
                    {
                        crate::events::handle_cpu_freq_event(
                            &event,
                            &mut self.runtime.outputs.recorder,
                            self.started,
                        );
                    } else {
                        log::warn!("short_cpu_freq_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_STAT_WAIT => {
                    if let Some(event) =
                        crate::events::read_event_unaligned::<stutter_common::StatWaitEvent>(&item)
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
                    if let Some(event) =
                        crate::events::read_event_unaligned::<stutter_common::BlockIoEvent>(&item)
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

                        crate::events::handle_block_io_record(
                            &record,
                            &mut self.runtime.outputs.recorder,
                        );
                        pending_ios.push(record);
                    } else {
                        log::warn!("short_block_io_event len={}", item.len());
                    }
                }
                stutter_common::EVENT_EXEC => {
                    if self.config.follow_exec {
                        crate::events::handle_exec_event(&item, &mut self.runtime.targeting.tasks);
                    }
                }
                other => log::warn!("unknown_bpf_event kind={other} len={}", item.len()),
            }
        }

        guard.clear_ready();
        drop(guard);

        for irq in pending_irqs {
            self.runtime.telemetry.push_irq(irq);
        }
        for io in pending_ios {
            self.runtime.telemetry.push_io(io);
        }
        for spike in pending_spikes {
            self.handle_live_spike(spike);
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
            self.runtime.outputs.recorder.counters.interval_record_count += records.len() as u64;

            self.emit(MonitorEvent::Interval {
                elapsed_ms,
                records: records.clone(),
                drop_counters: drop_counters_snapshot.clone(),
            })
            .await;

            if self
                .runtime
                .outputs
                .recorder
                .streams
                .contains(ArtifactKind::Interval)
            {
                for record in &records {
                    let _ = self
                        .runtime
                        .outputs
                        .recorder
                        .streams
                        .push(ArtifactKind::Interval, record);
                }
            } else if self.config.retain_intervals.is_some() || self.config.tui {
                // For TUI sparklines we need interval_records
                for record in &records {
                    self.runtime
                        .outputs
                        .recorder
                        .buffers
                        .interval_records
                        .push(record.clone());
                }

                let max_intervals = self.config.retain_intervals.unwrap_or(120);
                if self.runtime.outputs.recorder.buffers.interval_records.len() > max_intervals {
                    let drop_count = self.runtime.outputs.recorder.buffers.interval_records.len()
                        - max_intervals;
                    self.runtime
                        .outputs
                        .recorder
                        .buffers
                        .interval_records
                        .drain(0..drop_count);
                    if self.config.retain_intervals.is_some() {
                        self.runtime.outputs.recorder.counters.intervals_dropped +=
                            drop_count as u64;
                    }
                }
            }

            if let Some(writer) = self.runtime.outputs.recorder.csv_writer.as_mut() {
                for record in &records {
                    writer.push(record)?;
                }
            }

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
                    self.config.summary_period_ms,
                    self.config.runtime_slices_max_tasks,
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
                        crate::events::push_artifact_event(
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
                foreground_include_title: self.config.foreground_include_title,
            };

            // TUI rendering errors and panics should be logged and dismissed,
            // not propagated, to avoid killing the monitor.
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                term.draw(move |f| {
                    crate::tui::render_tui(
                        f,
                        &snapshot.tui_state,
                        &snapshot.active_targets,
                        &snapshot.stats_by_task,
                        &snapshot.interval_records,
                        &snapshot.recent_diagnoses,
                        snapshot.elapsed_ms.into(),
                        &snapshot.drop_counters,
                        snapshot.current_focus.as_ref(),
                        snapshot.current_foreground.as_ref(),
                        snapshot.focus_switch_count,
                        snapshot.foreground_include_title,
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
            remove_watch_tree_pid(&mut self.runtime.targeting.tree_pids, root_pid);
            self.runtime
                .targeting
                .tree_root_starttimes
                .remove(&root_pid);

            if !self.config.persistent {
                should_exit = Some("watched_process_exit".to_owned());
            } else {
                self.runtime.targeting.watch_state = WatchProcessState::Waiting;
                info!("watch_process_waiting_for_relaunch");
            }
        } else {
            let removed_roots = remove_stale_tree_roots(
                &mut self.runtime.targeting.tree_pids,
                &mut self.runtime.targeting.tree_root_starttimes,
                self.runtime.targeting.watch_state.running_pid(),
            );

            for root in &removed_roots {
                info!("tree_root_removed pid={root}");
            }

            if !removed_roots.is_empty()
                && self.had_tree_roots
                && self.runtime.targeting.tree_pids.is_empty()
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
        let Some(pattern) = self.config.watch_process.clone() else {
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
            add_watch_tree_pid(&mut self.runtime.targeting.tree_pids, pid);
            self.runtime
                .targeting
                .tree_root_starttimes
                .insert(pid, process_root_starttime(pid));
            self.runtime.targeting.watch_state = WatchProcessState::Running(pid);
            info!("watch_process_relaunched pattern={} pid={}", pattern, pid);

            self.refresh_tasks_and_emit_snapshot().await?;
        }

        Ok(())
    }

    pub fn handle_scx_tick(&mut self) {
        if let Some(event) = self
            .runtime
            .probes
            .scx_tracker
            .sample(self.started.elapsed().as_millis() as u64)
        {
            if self
                .runtime
                .outputs
                .recorder
                .streams
                .contains(ArtifactKind::ScxEvents)
            {
                crate::events::push_artifact_event(
                    &mut self.runtime.outputs.recorder,
                    ArtifactKind::ScxEvents,
                    &event,
                    "scx_events",
                    |c| {
                        c.scx_event_count += 1;
                    },
                );
            } else {
                self.runtime.outputs.recorder.buffers.scx_events.push(event);
                self.runtime.outputs.recorder.counters.scx_event_count += 1;
            }
        }
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

                crate::events::push_artifact_event(
                    &mut self.runtime.outputs.recorder,
                    ArtifactKind::GpuSamples,
                    &sample,
                    "gpu_samples",
                    |c| {
                        c.gpu_sample_count += 1;
                    },
                );
            }
        }
        Ok(())
    }

    fn handle_live_spike(&mut self, spike: SpikeEvent) {
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
            return;
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
            self.runtime
                .telemetry
                .diagnoses
                .push_back(LiveDiagnosisEntry {
                    elapsed_ms,
                    cause: diagnosis.cause,
                    confidence: diagnosis.confidence,
                    anchor_class: anchor.class,
                    anchor_comm: anchor.comm.clone(),
                    evidence: diagnosis.evidence.clone(),
                });

            log::info!(
                "live_diagnosis cause={:?} confidence={:?} evidence={:?}",
                diagnosis.cause,
                diagnosis.confidence,
                diagnosis.evidence
            );
        }
    }

    pub async fn refresh_tasks(&mut self) -> anyhow::Result<()> {
        let budget_report = self
            .runtime
            .targeting
            .tasks
            .refresh(crate::tasks::RefreshInput {
                config: &self.config,
                tree_pids: &self.runtime.targeting.tree_pids,
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
                community_rules: self.community_rules.as_ref(),
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
                self.config.json_stream || self.config.csv_streams_to_stdout();
            if count > 0 && !stdout_is_machine_stream {
                println!("OpenTelemetry export: dropped {count} spans due to channel pressure");
            }
        }
        let stdout_is_machine_stream =
            self.config.json_stream || self.config.csv_streams_to_stdout();
        if self.config.epoch_period_ms.is_none() && !stdout_is_machine_stream {
            print_session_summaries(&mut self.runtime.targeting.tasks.stats_by_task);
        }

        if let Some(writer) = self.runtime.outputs.recorder.csv_writer.as_mut() {
            writer.finish()?;
            if let Some(CsvStreamTarget::File(path)) = &self.config.csv_stream
                && !self.config.json_stream
            {
                println!("wrote interval CSV: {}", path.display());
            }
        }

        if self.runtime.outputs.recorder.run.is_some() {
            self.runtime.outputs.recorder.streams.finish_all()?;

            let frame_events = if !self.config.mangohud_log_live
                && let Some(path) = &self.config.mangohud_log
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
                tree_pids: &self.runtime.targeting.tree_pids,
                stop_reason: &stop_reason,
                tasks: &self.runtime.targeting.tasks,
                frame_events: &frame_events,
                block_io_correlation_basis: &self.runtime.probes.block_io_correlation_basis,
                focus_mode: if self.config.auto_focus {
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
    config: impl Into<Arc<Config>>,
    shared_hwmon: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
    event_tx: Option<tokio::sync::mpsc::Sender<MonitorEvent>>,
    stop_rx: Option<tokio::sync::oneshot::Receiver<()>>,
) -> anyhow::Result<String> {
    let config = config.into();
    let config = crate::config::effective::resolve_arc_monitor_config(config)?;
    let mut session = MonitorSession::new((*config).clone(), shared_hwmon, event_tx).await?;
    let stop_reason = session.run(stop_rx).await?;
    session.finalize(stop_reason)
}

pub fn configure_target_irqs(
    loaded: &mut ebpf_loader::LoadedEbpf,
    config: &Config,
) -> anyhow::Result<()> {
    if !config.irq_latency {
        return Ok(());
    }

    let Some(target_irq_map) = loaded.target_irq_map.as_mut() else {
        warn!("irq_latency_requested_but_map_missing");
        return Ok(());
    };

    if config.irqs.is_empty() {
        anyhow::bail!(
            "--irq-latency requires at least one explicit --irq <N>; inspect /proc/interrupts to find the IRQ number for your GPU or device"
        );
    }

    for irq in config.irqs.iter().copied() {
        target_irq_map.insert(irq, 1, 0)?;
        info!("irq_latency_target_added irq={irq}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_tick_not_needed_for_direct_pid_only() {
        assert!(!needs_tree_tick_from_parts(false, false, false));
    }

    #[test]
    fn tree_tick_needed_for_tree_roots() {
        assert!(needs_tree_tick_from_parts(true, false, false));
    }

    #[test]
    fn tree_tick_needed_for_watch_process_even_without_current_root() {
        assert!(needs_tree_tick_from_parts(false, true, false));
    }

    #[test]
    fn tree_tick_needed_for_cgroupv2() {
        assert!(needs_tree_tick_from_parts(false, false, true));
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
