use super::*;

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
        let wayland_presentation_reader = if config.probes.wayland_presentation {
            config
                .wayland_presentation
                .log_path
                .as_ref()
                .map(|path| WaylandPresentationLogReader::open_tail(path))
                .transpose()?
        } else {
            None
        };
        let dmabuf_reader = if config.probes.dmabuf_tracking {
            config
                .dmabuf
                .log_path
                .as_ref()
                .map(|path| DmaBufLogReader::open_tail(path))
                .transpose()?
        } else {
            None
        };

        let ui = UiRuntimeStage::begin(&config)?;
        let event_runtime_config = crate::events::EventRuntimeConfig::from_monitor_config(&config);

        let interval_label = if config.timing.epoch_period_ms.is_some() {
            "epoch"
        } else {
            "summary"
        };

        let ebpf_handles = crate::session::runtime_handles::EbpfHandles {
            loaded: probe_plan.loaded,
        };
        let recorder_handle = Some(crate::session::runtime_handles::RecorderHandle { recorder });
        let exporter_handles = crate::session::runtime_handles::ExporterHandles {
            prometheus_state: exporter_runtime.prometheus_state,
            prometheus_task: exporter_runtime.prometheus_task,
            otel_exporter: exporter_runtime.otel_exporter,
        };
        let target_refresh_handle = crate::session::runtime_handles::TargetRefreshHandle {
            focus_resolver: target_plan.focus_resolver,
            foreground_resolver: target_plan.foreground_resolver,
        };
        let handles = crate::session::runtime_handles::MonitorRuntimeHandles {
            ebpf: ebpf_handles,
            recorder: recorder_handle,
            exporters: exporter_handles,
            target_refresh: target_refresh_handle,
        };

        let probes = ProbeRuntime::new(
            probe_plan.block_io_correlation_basis,
            probe_plan.block_io_correlation_confidence,
            probe_plan.native_cgroup_filter,
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

        let sink_registry = crate::session::sinks::MonitorOutputSinkRegistry::for_runtime(
            event_runtime_config.output,
            &crate::recorder!(handles),
            alert_runtime.sender.as_ref(),
        );
        let outputs = OutputRuntime::from_parts(alert_runtime.sender, sink_registry);
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
            handles,
            runtime,
            cpu_to_pkg,
            hwmon_reader: hwmon_runtime.reader,
            gpu_engine_reader: hwmon_runtime.engine_reader,
            community_rules: target_plan.community_rules,
            current_focus: target_plan.current_focus,
            focus_switch_count: 0,
            current_foreground: target_plan.current_foreground,
            foreground_switch_count: 0,
            wayland_presentation_reader,
            dmabuf_reader,
            started,
            had_tree_roots: target_plan.had_tree_roots,
            interval_label,
        })
    }
}
