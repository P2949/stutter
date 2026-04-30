mod affinity;
mod cli;
mod ebpf_loader;
mod error;
mod hwmon;
mod mangohud;
mod metadata;
mod metrics;
mod process_tree;
mod profiles;
mod recorder;
mod report;
mod scorer;
mod scx;
mod tui;

#[cfg(test)]
mod regression_tests;

use std::{
    collections::BTreeMap,
    fs, future,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use aya::maps::{HashMap as AyaHashMap, MapData};
use cli::{AppCommand, Config, parse_app_command};
use log::{debug, info, warn};
use metrics::{collect_interval_summaries, format_latency, print_event, print_session_summaries};
use process_tree::{TargetDiffAction, TaskFilters, TaskInfo};
use recorder::{
    FinalizeRecordingInput, IntervalCsvWriter, IntervalRecord, IrqEventRecord, JsonArrayWriter,
    SpikeEventBuffer, TreeEvent, finalize_recording, prepare_recording,
};
use crossterm::event::{Event, EventStream, KeyCode};
use futures_util::StreamExt;
use serde::Serialize;
use stutter_common::{EVENT_IRQ_LATENCY, EVENT_RUNNABLE_LATENCY, IrqEvent, SchedulerEvent};
use tokio::{
    signal, task,
    time::{Duration, MissedTickBehavior, interval, sleep},
};

pub const TARGET_PIDS_MAX: usize = 1024;

type TaskExeInodesMap = BTreeMap<u32, (Option<u64>, Option<u64>, Option<u64>)>;

struct RefreshTargetTasksInput<'a> {
    config: &'a Config,
    active_targets: &'a mut BTreeMap<u32, TaskInfo>,
    known_targets: &'a mut BTreeMap<u32, TaskInfo>,
    stats_by_task: &'a mut BTreeMap<u32, metrics::TaskStats>,
    task_exe_inodes: &'a mut TaskExeInodesMap,
    tree_events: &'a mut Vec<TreeEvent>,
    target_pid_map: &'a mut AyaHashMap<MapData, u32, u8>,
    elapsed_ms: u128,
    recording_started: Option<Instant>,
    process_cache: &'a mut process_tree::ProcessCache,
}

struct HandleEventInput<'a> {
    event: &'a SchedulerEvent,
    config: &'a Config,
    started: Instant,
    active_targets: &'a BTreeMap<u32, TaskInfo>,
    known_targets: &'a BTreeMap<u32, TaskInfo>,
    stats_by_task: &'a mut BTreeMap<u32, metrics::TaskStats>,
    monotonic_start_ns: Option<u64>,
    spike_events: Option<&'a mut SpikeEventBuffer>,
}

struct DrainBpfEventsInput<'a> {
    guard: tokio::io::unix::AsyncFdReadyMutGuard<'a, aya::maps::RingBuf<aya::maps::MapData>>,
    config: &'a Config,
    started: Instant,
    active_targets: &'a BTreeMap<u32, TaskInfo>,
    known_targets: &'a BTreeMap<u32, TaskInfo>,
    stats_by_task: &'a mut BTreeMap<u32, metrics::TaskStats>,
    recording_monotonic_start_ns: Option<u64>,
    spike_events: &'a mut Option<SpikeEventBuffer>,
    irq_event_writer: Option<&'a mut JsonArrayWriter>,
    irq_event_count: &'a mut usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    match parse_app_command()? {
        AppCommand::Monitor(config) => run_monitor(*config).await,
        AppCommand::Restore { dry_run } => {
            let path = affinity::default_restore_path();
            if dry_run {
                print_restore_dry_run(&path)?;
            } else {
                let summary = affinity::restore_saved(&path)?;
                println!(
                    "restored {} affinity record(s); skipped_dead={}",
                    summary.restored, summary.skipped_dead
                );
            }
            Ok(())
        }
        AppCommand::ApplyProfile {
            tree_pid,
            profile,
            force,
            dry_run,
            watch,
            keep_applied,
            refresh_ms,
        } => apply_profile_command(tree_pid, profile, force, dry_run, watch, keep_applied, refresh_ms).await,
        AppCommand::InspectTree { tree_pid } => {
            let rendered = process_tree::render_tree(tree_pid)?;
            print!("{rendered}");
            Ok(())
        }
        AppCommand::Report {
            path,
            json,
            html,
            top,
            cluster_window_ms,
            diff,
            filter_class,
        } => {
            if let Some(diff_path) = diff {
                return report::print_diff_report(&path, &diff_path, top, filter_class);
            }
            if let Some(html_path) = html {
                report::write_html_report(&path, &html_path, top, cluster_window_ms, filter_class)?;
            }
            report::print_report(&path, json, top, cluster_window_ms, filter_class)
        }
        AppCommand::Tune {
            tree_pid,
            profiles,
            epoch_seconds,
            warmup_seconds,
            keep_best,
        } => tune_command(tree_pid, profiles, epoch_seconds, warmup_seconds, keep_best).await,
    }
}

fn print_restore_dry_run(path: &Path) -> anyhow::Result<()> {
    let state = affinity::load_restore_state(path)?;
    println!("restore dry-run file={}", path.display());

    for record in state.records {
        match affinity::read_allowed_mask(record.tid) {
            Ok(current) => println!(
                "tid={} alive=true current_mask={} restore_mask={}",
                record.tid,
                current.to_range_string(),
                record.original_mask.to_range_string()
            ),
            Err(err)
                if err
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|err| err.raw_os_error() == Some(libc::ESRCH)) =>
            {
                println!(
                    "tid={} alive=false current_mask=- restore_mask={}",
                    record.tid,
                    record.original_mask.to_range_string()
                )
            }
            Err(err) => println!(
                "tid={} alive=unknown current_mask_error={} restore_mask={}",
                record.tid,
                err,
                record.original_mask.to_range_string()
            ),
        }
    }

    Ok(())
}

async fn tune_command(
    tree_pid: u32,
    profiles_path: PathBuf,
    epoch_seconds: u64,
    warmup_seconds: u64,
    keep_best: bool,
) -> anyhow::Result<()> {
    let profiles = profiles::load_profiles(&profiles_path)?;
    if profiles.is_empty() {
        anyhow::bail!(
            "profile file {} did not contain [[profile]]",
            profiles_path.display()
        );
    }

    let measure_seconds = epoch_seconds.saturating_sub(warmup_seconds);
    let mut results = Vec::new();
    let mut best_idx = 0usize;
    let restore_policy = if keep_best {
        "restore-after-each-then-keep-best"
    } else {
        "restore-after-each"
    };

    let shared_hwmon = hwmon::HwmonReader::discover_with_options(None, None, None)
        .map(|r| std::sync::Arc::new(std::sync::Mutex::new(r)));

    for (idx, profile) in profiles.iter().enumerate() {
        println!(
            "tune candidate={} state=CandidateWarmup warmup_seconds={}",
            profile.name, warmup_seconds
        );

        let records =
            match apply_profile_to_tree_blocking(tree_pid, profile.clone(), idx == 0, false).await {
                Ok(records) => records,
                Err(err) => {
                    restore_tune_on_error();
                    return Err(err);
                }
            };

        println!(
            "tune candidate={} state=CandidateMeasure measure_seconds={}",
            profile.name, measure_seconds
        );

        let interval_records =
            match measure_tune_candidate(tree_pid, &profile.name, epoch_seconds, warmup_seconds, shared_hwmon.clone())
                .await
            {
                Ok(records) => records,
                Err(err) => {
                    restore_tune_on_error();
                    return Err(err);
                }
            };

        let score = scorer::score_from_interval_records(&interval_records);
        let result = TuneCandidateSummary {
            profile: profile.name.clone(),
            applied_tasks: records.len(),
            warmup_seconds,
            measure_seconds,
            interval_count: interval_records.len(),
            score_total: score.total,
            over_1ms: score.over_1ms,
            over_2ms: score.over_2ms,
            over_5ms: score.over_5ms,
            max_latency_ns: score.max_latency_ns,
        };

        if results
            .get(best_idx)
            .is_none_or(|current_best| result_is_better(&result, current_best))
        {
            best_idx = results.len();
        }

        results.push(result);

        restore_tune_after_candidate(&profile.name)?;
    }

    let best_profile = results
        .get(best_idx)
        .map(|result| result.profile.clone())
        .unwrap_or_default();

    let summary = TuneSummary {
        schema_version: 1,
        tree_pid,
        profiles_path,
        epoch_seconds,
        warmup_seconds,
        restore_policy: restore_policy.to_owned(),
        best_profile,
        candidates: results,
    };

    if keep_best
        && let Some(profile) = profiles
            .iter()
            .find(|profile| profile.name == summary.best_profile)
    {
        apply_profile_to_tree_blocking(tree_pid, profile.clone(), false, false).await?;
    }

    let summary_path = default_tune_summary_path();
    if let Some(parent) = summary_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(&summary_path, serde_json::to_vec_pretty(&summary)?)?;

    println!(
        "tune complete best_profile={} restore_policy={} summary={}{}",
        summary.best_profile,
        summary.restore_policy,
        summary_path.display(),
        if keep_best {
            " restore_with=\"stutter restore\""
        } else {
            ""
        }
    );

    Ok(())
}

async fn measure_tune_candidate(
    tree_pid: u32,
    profile_name: &str,
    epoch_seconds: u64,
    warmup_seconds: u64,
    shared_hwmon: Option<std::sync::Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
) -> anyhow::Result<Vec<IntervalRecord>> {
    let run_dir = tune_run_dir(profile_name);
    let config = Config {
        target_pids: Vec::new(),
        tree_pids: vec![tree_pid],
        summary_period_ms: 1_000,
        spike_threshold_ns: 1_000_000,
        verbose: false,
        max_tasks: 1024,
        task_filters: TaskFilters::default(),
        keep_missing_pid: false,
        watch_process: None,
        persistent: false,
        watch_poll_ms: 2_000,
        watch_timeout: None,
        csv_path: None,
        irq_latency: false,
        irqs: Vec::new(),
        hwmon: false,
        hwmon_root: None,
        hwmon_drm_card: None,
        hwmon_render_node: None,
        mangohud_log: None,
        tui: false,
        retain_intervals: None,
        recording: Some(cli::RecordingConfig {
            run_name: Some(format!("tune-{profile_name}")),
            out_dir: Some(run_dir.clone()),
        }),
        max_duration: Some(Duration::from_secs(epoch_seconds)),
        shared_hwmon,
    };

    run_monitor(config).await?;

    let interval_path = run_dir.join("interval.json");
    let data = fs::read_to_string(&interval_path).map_err(|err| {
        anyhow::anyhow!(
            "failed to read tune intervals {}: {err}",
            interval_path.display()
        )
    })?;
    let mut records: Vec<IntervalRecord> = serde_json::from_str(&data).map_err(|err| {
        anyhow::anyhow!(
            "failed to parse tune intervals {}: {err}",
            interval_path.display()
        )
    })?;
    let warmup_ms = u128::from(warmup_seconds) * 1_000;
    records.retain(|record| record.elapsed_ms >= warmup_ms);
    fs::remove_dir_all(&run_dir).ok();

    Ok(records)
}

fn tune_run_dir(profile_name: &str) -> PathBuf {
    let sanitized = profile_name
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    std::env::temp_dir().join(format!(
        "stutter-tune-{}-{}-{}",
        std::process::id(),
        sanitized,
        unix_nanos_now()
    ))
}

#[derive(Serialize)]
struct TuneSummary {
    schema_version: u32,
    tree_pid: u32,
    profiles_path: PathBuf,
    epoch_seconds: u64,
    warmup_seconds: u64,
    restore_policy: String,
    best_profile: String,
    candidates: Vec<TuneCandidateSummary>,
}

#[derive(Clone, Serialize)]
struct TuneCandidateSummary {
    profile: String,
    applied_tasks: usize,
    warmup_seconds: u64,
    measure_seconds: u64,
    interval_count: usize,
    score_total: u64,
    over_1ms: u64,
    over_2ms: u64,
    over_5ms: u64,
    max_latency_ns: u64,
}

fn result_is_better(candidate: &TuneCandidateSummary, current_best: &TuneCandidateSummary) -> bool {
    (candidate.score_total, candidate.max_latency_ns)
        < (current_best.score_total, current_best.max_latency_ns)
}

fn restore_tune_on_error() {
    let path = affinity::default_restore_path();
    if path.exists()
        && let Err(err) = affinity::restore_saved(&path)
    {
        warn!("tune_restore_after_error_failed err={err:#}");
    }
}

fn restore_tune_after_candidate(profile_name: &str) -> anyhow::Result<()> {
    let path = affinity::default_restore_path();
    if !path.exists() {
        return Ok(());
    }

    let summary = affinity::restore_saved(&path)?;
    info!(
        "tune_candidate_restored profile={} restored={} skipped_dead={}",
        profile_name, summary.restored, summary.skipped_dead
    );
    Ok(())
}

fn default_tune_summary_path() -> PathBuf {
    let mut path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    path.push(".local");
    path.push("state");
    path.push("stutter");
    path.push(format!("tuning_summary_{}.json", unix_nanos_now()));
    path
}

fn unix_nanos_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

async fn apply_profile_command(
    tree_pid: u32,
    profile_path: PathBuf,
    force: bool,
    dry_run: bool,
    watch: bool,
    keep_applied: bool,
    refresh_ms: u64,
) -> anyhow::Result<()> {
    let profile = profiles::load_first_profile(&profile_path)?;
    let mut cache = profiles::ProfileApplyCache::default();

    let records = if watch {
        match apply_profile_to_tree_cached_blocking(tree_pid, profile.clone(), force, dry_run, cache).await {
            Ok((records, updated_cache)) => {
                cache = updated_cache;
                records
            }
            Err(err) => {
                if !keep_applied && let Err(restore_err) = restore_profile_watch_on_exit() {
                    warn!("profile_watch_restore_after_error_failed err={restore_err:#}");
                }
                return Err(err);
            }
        }
    } else {
        apply_profile_to_tree_blocking(tree_pid, profile.clone(), force, dry_run).await?
    };

    println!(
        "applied profile affinity to {} task(s); restore with: stutter restore",
        records.len()
    );

    if !watch {
        println!("apply-profile is one-shot; use --watch to keep applying to new threads");
        return Ok(());
    }

    let mut tick = interval(Duration::from_millis(refresh_ms));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tick.tick().await;

    loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                if keep_applied {
                    println!("stopped profile watch; restore with: stutter restore");
                } else {
                    restore_profile_watch_on_exit()?;
                }
                return Ok(());
            }
            _ = tick.tick() => {
                let result = apply_profile_to_tree_cached_blocking(
                    tree_pid,
                    profile.clone(),
                    false,
                    dry_run,
                    cache,
                )
                .await;

                let records = match result {
                    Ok((records, updated_cache)) => {
                        cache = updated_cache;
                        records
                    }
                    Err(err) => {
                        if !keep_applied
                            && let Err(restore_err) = restore_profile_watch_on_exit()
                        {
                            warn!("profile_watch_restore_after_error_failed err={restore_err:#}");
                        }
                        return Err(err);
                    }
                };

                if !records.is_empty() {
                    info!("profile_watch_applied tasks={}", records.len());
                }
            }
        }
    }
}

async fn apply_profile_to_tree_blocking(
    tree_pid: u32,
    profile: profiles::Profile,
    force: bool,
    dry_run: bool,
) -> anyhow::Result<Vec<affinity::AffinityRecord>> {
    task::spawn_blocking(move || profiles::apply_profile_to_tree(tree_pid, &profile, force, dry_run))
        .await
        .map_err(|err| anyhow::anyhow!("profile apply worker failed: {err}"))?
}

async fn apply_profile_to_tree_cached_blocking(
    tree_pid: u32,
    profile: profiles::Profile,
    force: bool,
    dry_run: bool,
    mut cache: profiles::ProfileApplyCache,
) -> anyhow::Result<(Vec<affinity::AffinityRecord>, profiles::ProfileApplyCache)> {
    task::spawn_blocking(move || {
        profiles::apply_profile_to_tree_cached(tree_pid, &profile, force, dry_run, &mut cache)
            .map(|records| (records, cache))
    })
    .await
    .map_err(|err| anyhow::anyhow!("profile apply worker failed: {err}"))?
}

fn restore_profile_watch_on_exit() -> anyhow::Result<()> {
    let path = affinity::default_restore_path();
    if !path.exists() {
        println!("stopped profile watch; no restore file was written");
        return Ok(());
    }

    let summary = affinity::restore_saved(&path)?;
    println!(
        "stopped profile watch; restored {} affinity record(s); skipped_dead={}",
        summary.restored, summary.skipped_dead
    );

    Ok(())
}

fn configure_target_irqs(
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WatchProcessState {
    None,
    Running(u32),
    Waiting,
}

impl WatchProcessState {
    fn running_pid(self) -> Option<u32> {
        match self {
            Self::Running(pid) => Some(pid),
            Self::None | Self::Waiting => None,
        }
    }
}

async fn run_monitor(mut config: Config) -> anyhow::Result<()> {
    let mut watch_state = match resolve_watch_process(&mut config).await? {
        Some(pid) => WatchProcessState::Running(pid),
        None => WatchProcessState::None,
    };

    let had_tree_roots = !config.tree_pids.is_empty();
    let mut tree_root_starttimes = capture_tree_root_starttimes(&config.tree_pids);

    let recording = prepare_recording(&config)?;
    let mut loaded = ebpf_loader::load_and_attach().map_err(anyhow::Error::new)?;
    configure_target_irqs(&mut loaded, &config)?;

    let mut active_targets: BTreeMap<u32, TaskInfo> = BTreeMap::new();
    let mut known_targets: BTreeMap<u32, TaskInfo> = BTreeMap::new();
    let mut stats_by_task: BTreeMap<u32, metrics::TaskStats> = BTreeMap::new();
    let mut task_exe_inodes: TaskExeInodesMap = BTreeMap::new();
    let mut interval_records: Vec<IntervalRecord> = Vec::new();
    let mut tree_events: Vec<TreeEvent> = Vec::new();
    let mut spike_events = recording.as_ref().map(|_| SpikeEventBuffer::default());
    let irq_events: Vec<IrqEventRecord> = Vec::new();
    let gpu_samples = Vec::new();
    
    let mut interval_writer = if config.retain_intervals.is_none() {
        recording
            .as_ref()
            .map(|run| JsonArrayWriter::create(run.run_dir.join("interval.json")))
            .transpose()?
    } else {
        None
    };
    let mut intervals_dropped = 0usize;
    let mut irq_event_writer = recording
        .as_ref()
        .map(|run| JsonArrayWriter::create(run.run_dir.join("irq_events.json")))
        .transpose()?;
    let mut gpu_sample_writer = recording
        .as_ref()
        .map(|run| JsonArrayWriter::create(run.run_dir.join("gpu_samples.json")))
        .transpose()?;
    let mut csv_writer = config
        .csv_path
        .as_ref()
        .map(|path| IntervalCsvWriter::create(path.clone()))
        .transpose()?;
    let mut streamed_interval_record_count = 0usize;
    let mut streamed_irq_event_count = 0usize;
    let mut streamed_gpu_sample_count = 0usize;

    // These mutable collections are intentionally confined to the main monitoring task.
    // Blocking work returns state to this task and does not mutate these collections
    // concurrently. Future background mutation should use Arc<Mutex<_>> or messages.
    let mut scx_tracker = scx::ScxTracker::default();
    let recording_monotonic_start_ns = recording.as_ref().and_then(|run| run.monotonic_start_ns);

    let hwmon_reader = if let Some(shared) = &config.shared_hwmon {
        Some(shared.clone())
    } else if config.hwmon {
        hwmon::HwmonReader::discover_with_options(
            config.hwmon_root.as_deref(),
            config.hwmon_drm_card.as_deref(),
            config.hwmon_render_node.as_deref(),
        )
        .map(|r| std::sync::Arc::new(std::sync::Mutex::new(r)))
    } else {
        None
    };

    if config.hwmon && hwmon_reader.is_none() {
        warn!("hwmon_requested_but_no_gpu_hwmon_found");
    }

    let mut process_cache = process_tree::ProcessCache::default();

    let started = Instant::now();
    scx_tracker.sample(0);

    refresh_target_tasks(RefreshTargetTasksInput {
        config: &config,
        active_targets: &mut active_targets,
        known_targets: &mut known_targets,
        stats_by_task: &mut stats_by_task,
        task_exe_inodes: &mut task_exe_inodes,
        tree_events: &mut tree_events,
        target_pid_map: &mut loaded.target_pid_map,
        elapsed_ms: started.elapsed().as_millis(),
        recording_started: recording.as_ref().map(|run| run.started_instant),
        process_cache: &mut process_cache,
    })
    .await?;

    let mut summary_tick = interval(Duration::from_millis(config.summary_period_ms));
    summary_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    summary_tick.tick().await;

    let mut tree_tick = interval(Duration::from_secs(1));
    tree_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tree_tick.tick().await;

    let mut watch_tick = interval(Duration::from_millis(config.watch_poll_ms));
    watch_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    watch_tick.tick().await;

    let mut scx_tick = interval(Duration::from_secs(1));
    scx_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    scx_tick.tick().await;

    let mut hwmon_tick = interval(Duration::from_millis(100));
    hwmon_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    hwmon_tick.tick().await;

    let max_duration = config.max_duration;
    let duration_future = async move {
        if let Some(max_duration) = max_duration {
            sleep(max_duration).await;
        } else {
            future::pending::<()>().await;
        }
    };
    tokio::pin!(duration_future);

    info!(
        "attached target_tasks={} tree_roots={} summary_ms={} spike_threshold={}",
        active_targets.len(),
        config.tree_pids.len(),
        config.summary_period_ms,
        format_latency(config.spike_threshold_ns),
    );

    let mut tui_state = crate::tui::TuiState::default();
    let mut terminal = if config.tui {
        Some(crate::tui::init_terminal().map_err(|e| anyhow::anyhow!("failed to init terminal: {e}"))?)
    } else {
        None
    };
    let mut crossterm_events = EventStream::new();

    let stop_reason = loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                break "ctrl_c".to_owned();
            }
            
            Some(Ok(event)) = crossterm_events.next(), if config.tui => {
                if let Event::Key(key) = event {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') => break "quit".to_owned(),
                        KeyCode::Char('p') | KeyCode::Char('P') => {
                            tui_state.paused = !tui_state.paused;
                        }
                        KeyCode::Char('s') | KeyCode::Char('S') => {
                            tui_state.sort_field = tui_state.sort_field.next();
                        }
                        KeyCode::Char('f') | KeyCode::Char('F') => {
                            tui_state.next_filter_class();
                        }
                        _ => {}
                    }
                }
            }

            _ = &mut duration_future => {
                break "duration".to_owned();
            }

            _ = summary_tick.tick() => {
                if !tui_state.paused {
                    let elapsed_ms = started.elapsed().as_millis();
                    let drop_counters_snapshot = loaded.snapshot_drop_counters();
                    let records = collect_interval_summaries(&mut stats_by_task, elapsed_ms, &drop_counters_snapshot);
                    streamed_interval_record_count += records.len();

                    if let Some(writer) = interval_writer.as_mut() {
                        for record in &records {
                            writer.push(record)?;
                        }
                    } else if config.retain_intervals.is_some() || config.tui {
                        // For TUI sparklines we need interval_records
                        for record in &records {
                            interval_records.push(record.clone());
                        }
                        
                        let max_intervals = config.retain_intervals.unwrap_or(120);
                        if interval_records.len() > max_intervals {
                            let drop_count = interval_records.len() - max_intervals;
                            interval_records.drain(0..drop_count);
                            if config.retain_intervals.is_some() {
                                intervals_dropped += drop_count;
                            }
                        }
                    }

                    if let Some(writer) = csv_writer.as_mut() {
                        for record in &records {
                            writer.push(record)?;
                        }
                    }
                }

                if let Some(term) = terminal.as_mut() {
                    let elapsed_ms = started.elapsed().as_millis();
                    let drop_counters_snapshot = loaded.snapshot_drop_counters();
                    term.draw(|f| {
                        crate::tui::render_tui(
                            f,
                            &tui_state,
                            &active_targets,
                            &stats_by_task,
                            &interval_records,
                            elapsed_ms,
                            &drop_counters_snapshot,
                        );
                    })?;
                }
            }

            _ = tree_tick.tick(), if !config.tree_pids.is_empty() => {
                if let Some(root_pid) = watch_state.running_pid()
                    && tree_root_is_stale(root_pid, &tree_root_starttimes)
                {
                    remove_watch_tree_pid(&mut config, root_pid);
                    tree_root_starttimes.remove(&root_pid);

                    refresh_target_tasks(RefreshTargetTasksInput {
                        config: &config,
                        active_targets: &mut active_targets,
                        known_targets: &mut known_targets,
                        stats_by_task: &mut stats_by_task,
                        task_exe_inodes: &mut task_exe_inodes,
                        tree_events: &mut tree_events,
                        target_pid_map: &mut loaded.target_pid_map,
                        elapsed_ms: started.elapsed().as_millis(),
                        recording_started: recording.as_ref().map(|run| run.started_instant),
                        process_cache: &mut process_cache,
                    })
                    .await?;

                    if !config.persistent {
                        break "watched_process_exit".to_owned();
                    }

                    watch_state = WatchProcessState::Waiting;
                    info!("watch_process_waiting_for_relaunch");
                    continue;
                }

                let removed_roots = remove_stale_tree_roots(
                    &mut config,
                    &mut tree_root_starttimes,
                    watch_state.running_pid(),
                );

                if !removed_roots.is_empty() {
                    for root in &removed_roots {
                        info!("tree_root_removed pid={root}");
                    }

                    refresh_target_tasks(RefreshTargetTasksInput {
                        config: &config,
                        active_targets: &mut active_targets,
                        known_targets: &mut known_targets,
                        stats_by_task: &mut stats_by_task,
                        task_exe_inodes: &mut task_exe_inodes,
                        tree_events: &mut tree_events,
                        target_pid_map: &mut loaded.target_pid_map,
                        elapsed_ms: started.elapsed().as_millis(),
                        recording_started: recording.as_ref().map(|run| run.started_instant),
                        process_cache: &mut process_cache,
                    })
                    .await?;

                    if had_tree_roots
                        && config.tree_pids.is_empty()
                        && !matches!(watch_state, WatchProcessState::Waiting)
                    {
                        break "tree_root_exit".to_owned();
                    }
                }

                refresh_target_tasks(RefreshTargetTasksInput {
                    config: &config,
                    active_targets: &mut active_targets,
                    known_targets: &mut known_targets,
                    stats_by_task: &mut stats_by_task,
                    task_exe_inodes: &mut task_exe_inodes,
                    tree_events: &mut tree_events,
                    target_pid_map: &mut loaded.target_pid_map,
                    elapsed_ms: started.elapsed().as_millis(),
                    recording_started: recording.as_ref().map(|run| run.started_instant),
                    process_cache: &mut process_cache,
                })
                .await?;
            }

            _ = watch_tick.tick(), if matches!(watch_state, WatchProcessState::Waiting) => {
                let Some(pattern) = config.watch_process.clone() else {
                    continue;
                };

                if let Some(pid) = find_process_by_pattern_at(Path::new("/proc"), &pattern) {
                    add_watch_tree_pid(&mut config, pid);
                    tree_root_starttimes.insert(pid, process_root_starttime(pid));
                    watch_state = WatchProcessState::Running(pid);
                    info!("watch_process_relaunched pattern={} pid={}", pattern, pid);

                    refresh_target_tasks(RefreshTargetTasksInput {
                        config: &config,
                        active_targets: &mut active_targets,
                        known_targets: &mut known_targets,
                        stats_by_task: &mut stats_by_task,
                        task_exe_inodes: &mut task_exe_inodes,
                        tree_events: &mut tree_events,
                        target_pid_map: &mut loaded.target_pid_map,
                        elapsed_ms: started.elapsed().as_millis(),
                        recording_started: recording.as_ref().map(|run| run.started_instant),
                        process_cache: &mut process_cache,
                    })
                    .await?;
                }
            }

            _ = scx_tick.tick() => {
                scx_tracker.sample(started.elapsed().as_millis());
            }

            _ = hwmon_tick.tick(), if hwmon_reader.is_some() => {
                if let Some(reader_arc) = &hwmon_reader {
                    let elapsed = started.elapsed().as_millis();
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

                    if let Some(sample) = sample_opt
                        && let Some(writer) = gpu_sample_writer.as_mut()
                    {
                        writer.push(&sample)?;
                        streamed_gpu_sample_count += 1;
                    }
                }
            }

            ready = loaded.events.readable_mut() => {
                let guard = ready?;
                drain_bpf_events(DrainBpfEventsInput {
                    guard,
                    config: &config,
                    started,
                    active_targets: &active_targets,
                    known_targets: &known_targets,
                    stats_by_task: &mut stats_by_task,
                    recording_monotonic_start_ns,
                    spike_events: &mut spike_events,
                    irq_event_writer: irq_event_writer.as_mut(),
                    irq_event_count: &mut streamed_irq_event_count,
                })?;
            }
        }
    };

    if let Some(term) = terminal.as_mut() {
        let _ = crate::tui::restore_terminal(term);
    }

    let drop_counters = loaded.snapshot_drop_counters();
    log_drop_counters(&drop_counters);
    print_session_summaries(&mut stats_by_task);

    if let Some(writer) = csv_writer.as_mut() {
        writer.finish()?;
        if let Some(path) = &config.csv_path {
            println!("wrote interval CSV: {}", path.display());
        }
    }

    if let Some(recording) = recording {
        if let Some(writer) = interval_writer.as_mut() {
            writer.finish()?;
        }
        if let Some(writer) = irq_event_writer.as_mut() {
            writer.finish()?;
        }
        if let Some(writer) = gpu_sample_writer.as_mut() {
            writer.finish()?;
        }

        let spike_events_slice = spike_events
            .as_ref()
            .map(SpikeEventBuffer::as_slice)
            .unwrap_or(&[]);
        let spike_events_truncated = spike_events
            .as_ref()
            .map(SpikeEventBuffer::truncated)
            .unwrap_or(false);

        let frame_events = if let Some(path) = &config.mangohud_log {
            match mangohud::read_frame_events(path) {
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

        finalize_recording(FinalizeRecordingInput {
            recording: &recording,
            config: &config,
            stop_reason: &stop_reason,
            active_targets: &active_targets,
            stats_by_task: &stats_by_task,
            interval_records: &interval_records,
            streamed_interval_record_count: interval_writer
                .is_some()
                .then_some(streamed_interval_record_count),
            intervals_dropped,
            tree_events: &tree_events,
            spike_events: spike_events_slice,
            spike_events_truncated,
            scx_events: scx_tracker.events(),
            irq_events: &irq_events,
            streamed_irq_event_count: irq_event_writer
                .is_some()
                .then_some(streamed_irq_event_count),
            gpu_samples: &gpu_samples,
            streamed_gpu_sample_count: gpu_sample_writer
                .is_some()
                .then_some(streamed_gpu_sample_count),
            frame_events: &frame_events,
            drop_counters,
        })?;
    }

    info!("exiting stop_reason={stop_reason}");
    Ok(())
}

async fn resolve_watch_process(config: &mut Config) -> anyhow::Result<Option<u32>> {
    let Some(pattern) = config.watch_process.clone() else {
        return Ok(None);
    };

    if let Some(pid) = find_process_by_pattern_at(Path::new("/proc"), &pattern) {
        add_watch_tree_pid(config, pid);
        return Ok(Some(pid));
    }

    wait_for_watch_process(config)
        .await?
        .ok_or_else(|| anyhow::anyhow!("stopped while waiting for --watch-process {pattern}"))
        .map(Some)
}

async fn wait_for_watch_process(config: &mut Config) -> anyhow::Result<Option<u32>> {
    let pattern = config
        .watch_process
        .clone()
        .ok_or_else(|| anyhow::anyhow!("internal error: watch_process missing"))?;

    info!(
        "watch_process_waiting pattern={} persistent={}",
        pattern, config.persistent
    );

    let mut tick = interval(Duration::from_millis(config.watch_poll_ms));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let watch_timeout = config.watch_timeout;
    let timeout_future = async move {
        if let Some(timeout) = watch_timeout {
            sleep(timeout).await;
        } else {
            future::pending::<()>().await;
        }
    };
    tokio::pin!(timeout_future);

    loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                return Ok(None);
            }
            _ = &mut timeout_future => {
                anyhow::bail!(
                    "timed out waiting for --watch-process {pattern} after {}ms",
                    watch_timeout.map(|timeout| timeout.as_millis()).unwrap_or(0)
                );
            }
            _ = tick.tick() => {
                if let Some(pid) = find_process_by_pattern_at(Path::new("/proc"), &pattern) {
                    add_watch_tree_pid(config, pid);
                    info!("watch_process_found pattern={} pid={}", pattern, pid);
                    return Ok(Some(pid));
                }
            }
        }
    }
}

fn add_watch_tree_pid(config: &mut Config, pid: u32) {
    config.tree_pids.push(pid);
    config.tree_pids.sort_unstable();
    config.tree_pids.dedup();
}

fn remove_watch_tree_pid(config: &mut Config, pid: u32) {
    config.tree_pids.retain(|tree_pid| *tree_pid != pid);
}

fn capture_tree_root_starttimes(tree_pids: &[u32]) -> BTreeMap<u32, Option<u64>> {
    tree_pids
        .iter()
        .map(|pid| (*pid, process_root_starttime(*pid)))
        .collect()
}

fn process_root_starttime(pid: u32) -> Option<u64> {
    process_tree::process_starttime_at(Path::new("/proc"), pid)
}

fn tree_root_is_stale(pid: u32, root_starttimes: &BTreeMap<u32, Option<u64>>) -> bool {
    let current = process_root_starttime(pid);
    let expected = root_starttimes.get(&pid).copied().flatten();

    current.is_none() || expected.is_some_and(|expected| current != Some(expected))
}

fn remove_stale_tree_roots(
    config: &mut Config,
    root_starttimes: &mut BTreeMap<u32, Option<u64>>,
    watched_pid: Option<u32>,
) -> Vec<u32> {
    let mut removed = Vec::new();

    for pid in config.tree_pids.clone() {
        if Some(pid) == watched_pid {
            continue;
        }

        if tree_root_is_stale(pid, root_starttimes) {
            removed.push(pid);
            root_starttimes.remove(&pid);
        }
    }

    if !removed.is_empty() {
        config
            .tree_pids
            .retain(|tree_pid| !removed.contains(tree_pid));
    }

    removed
}

fn find_process_by_pattern_at(proc_root: &Path, pattern: &str) -> Option<u32> {
    let pattern_lower = normalize_process_match_text(pattern);

    let mut cache = process_tree::ProcessCache::default();
    process_tree::scan_processes_at(proc_root, &mut cache)
        .into_iter()
        .filter_map(|(pid, process)| {
            let score =
                process_match_score(pattern, &pattern_lower, &process.comm, &process.cmdline)?;
            Some((score, pid))
        })
        .max_by_key(|(score, pid)| (*score, *pid))
        .map(|(_, pid)| pid)
}

fn process_match_score(
    pattern: &str,
    pattern_lower: &str,
    comm: &str,
    cmdline: &str,
) -> Option<u8> {
    if comm == pattern {
        return Some(4);
    }

    let comm_lower = normalize_process_match_text(comm);
    if comm_lower == pattern_lower {
        return Some(3);
    }

    let cmdline_lower = normalize_process_match_text(cmdline);
    let exe_basename_lower = cmdline_executable_basename_lower(cmdline);
    if exe_basename_lower.as_deref() == Some(pattern_lower) {
        return Some(2);
    }

    (comm_lower.contains(pattern_lower) || cmdline_lower.contains(pattern_lower)).then_some(1)
}

fn normalize_process_match_text(value: &str) -> String {
    value.replace('\\', "/").to_ascii_lowercase()
}

fn cmdline_executable_basename_lower(cmdline: &str) -> Option<String> {
    let executable = cmdline.split_whitespace().next()?;
    let executable = normalize_process_match_text(executable);

    PathBuf::from(executable)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

fn drain_bpf_events(input: DrainBpfEventsInput<'_>) -> anyhow::Result<()> {
    let DrainBpfEventsInput {
        mut guard,
        config,
        started,
        active_targets,
        known_targets,
        stats_by_task,
        recording_monotonic_start_ns,
        spike_events,
        mut irq_event_writer,
        irq_event_count,
    } = input;

    while let Some(item) = guard.get_inner_mut().next() {
        if item.len() < std::mem::size_of::<u32>() {
            warn!("short_bpf_event len={}", item.len());
            continue;
        }

        let kind = unsafe { (item.as_ptr() as *const u32).read_unaligned() };
        match kind {
            EVENT_RUNNABLE_LATENCY => {
                if item.len() < std::mem::size_of::<SchedulerEvent>() {
                    warn!("short_scheduler_event len={}", item.len());
                    continue;
                }

                let event = unsafe { &*(item.as_ptr() as *const SchedulerEvent) };

                handle_event(HandleEventInput {
                    event,
                    config,
                    started,
                    active_targets,
                    known_targets,
                    stats_by_task,
                    monotonic_start_ns: recording_monotonic_start_ns,
                    spike_events: spike_events.as_mut(),
                });
            }
            EVENT_IRQ_LATENCY => {
                if item.len() < std::mem::size_of::<IrqEvent>() {
                    warn!("short_irq_event len={}", item.len());
                    continue;
                }

                let event = unsafe { &*(item.as_ptr() as *const IrqEvent) };
                let record = irq_event_record(recording_monotonic_start_ns, event);
                if let Some(writer) = irq_event_writer.as_deref_mut() {
                    writer.push(&record)?;
                    *irq_event_count += 1;
                }
                log_irq_event(event);
            }
            other => warn!("unknown_bpf_event kind={other} len={}", item.len()),
        }
    }

    guard.clear_ready();
    Ok(())
}

fn handle_event(input: HandleEventInput<'_>) {
    let HandleEventInput {
        event,
        config,
        started,
        active_targets,
        known_targets,
        stats_by_task,
        monotonic_start_ns,
        spike_events,
    } = input;

    match event.kind {
        EVENT_RUNNABLE_LATENCY => {
            let comm = metrics::comm_to_string(&event.comm);
            let elapsed_ms = started.elapsed().as_millis();

            let task_info = active_targets
                .get(&event.pid)
                .or_else(|| known_targets.get(&event.pid));

            let stats = stats_by_task
                .entry(event.pid)
                .or_insert_with(|| metrics::TaskStats::new(event.pid, comm.clone(), elapsed_ms));

            if should_replace_unknown_comm(&stats.comm, &comm) {
                stats.comm = comm.clone();
            }

            if let Some(task_info) = task_info {
                stats.apply_task_info(task_info);
                stats.active = active_targets.contains_key(&event.pid);
            }

            stats.record(event, config.spike_threshold_ns, elapsed_ms);

            if event.latency_ns >= config.spike_threshold_ns
                && let Some(spike_events) = spike_events
            {
                spike_events.push(recorder::SpikeEvent::from_task_stats(
                    monotonic_start_ns,
                    stats,
                    event,
                ));
            }

            if config.verbose {
                print_event(event, &comm, "sample");
            } else if event.latency_ns >= config.spike_threshold_ns {
                print_event(event, &comm, "spike");
            }
        }
        other => {
            let comm = metrics::comm_to_string(&event.comm);
            info!(
                "unknown_event kind={} task={} cpu={} comm={}",
                other, event.pid, event.cpu, comm
            );
        }
    }
}

fn irq_event_record(monotonic_start_ns: Option<u64>, event: &IrqEvent) -> IrqEventRecord {
    let elapsed_ms = monotonic_start_ns
        .and_then(|start_ns| event.exit_ns.checked_sub(start_ns))
        .map(|elapsed_ns| u128::from(elapsed_ns / 1_000_000));

    IrqEventRecord {
        elapsed_ms,
        irq: event.irq,
        cpu: event.cpu,
        enter_ns: event.enter_ns,
        exit_ns: event.exit_ns,
        duration_ns: event.duration_ns,
    }
}

fn log_irq_event(event: &IrqEvent) {
    debug!(
        "irq_latency irq={} cpu={} duration={}",
        event.irq,
        event.cpu,
        format_latency(event.duration_ns)
    );
}

async fn refresh_target_tasks(input: RefreshTargetTasksInput<'_>) -> anyhow::Result<()> {
    let RefreshTargetTasksInput {
        config,
        active_targets,
        known_targets,
        stats_by_task,
        task_exe_inodes,
        tree_events,
        target_pid_map,
        elapsed_ms,
        recording_started,
        process_cache,
    } = input;

    let target_pids = config.target_pids.clone();
    let tree_pids = config.tree_pids.clone();
    let filters = config.task_filters.clone();
    let keep_missing_pid = config.keep_missing_pid;
    let max_tasks = config.max_tasks;
    
    let mut process_cache_owned = std::mem::take(process_cache);
    let active_targets_clone = active_targets.clone();

    let (snapshot, returned_cache) = task::spawn_blocking(move || {
        let snap = process_tree::target_snapshot_with_options(
            &target_pids,
            &tree_pids,
            &filters,
            keep_missing_pid,
            &mut process_cache_owned,
            Some(&active_targets_clone),
        );
        (snap, process_cache_owned)
    })
    .await
    .map_err(|err| anyhow::anyhow!("target snapshot worker failed: {err}"))?;

    *process_cache = returned_cache;
    let desired_tasks = snapshot.tasks;

    if desired_tasks.len() > max_tasks {
        anyhow::bail!(
            "too many target tasks after tree/thread expansion: got {}, but --max-tasks is set to {}; try a narrower --tree-pid root or increase --max-tasks",
            desired_tasks.len(),
            max_tasks
        );
    }

    handle_same_tid_replacements(
        active_targets,
        &desired_tasks,
        known_targets,
        stats_by_task,
        task_exe_inodes,
        tree_events,
        elapsed_ms,
        recording_started,
    );

    for diff in process_tree::diff_tasks_ref(active_targets, &desired_tasks) {
        match diff.action {
            TargetDiffAction::Added => {
                let tid = diff.task.tid;
                target_pid_map.insert(tid, 1, 0).map_err(|e| {
                    anyhow::anyhow!(
                        "failed to insert TID {tid} into TARGET_PIDS during tree refresh \
                         (map full? eBPF TARGET_PIDS max_entries is {TARGET_PIDS_MAX}): {e}"
                    )
                })?;

                known_targets.insert(tid, diff.task.clone());

                if let Some(started) = recording_started {
                    tree_events.push(TreeEvent::from_task(started, "added", diff.task));
                }

                reactivate_or_reset_stats_inner(
                    stats_by_task,
                    Some(&mut *task_exe_inodes),
                    tid,
                    diff.task,
                    elapsed_ms,
                );

                info!(
                    "tree_target_added tid={} process_pid={} ppid={} comm={} class={}",
                    tid,
                    diff.task.process_pid,
                    diff.task.process_ppid,
                    diff.task.comm,
                    diff.task.class
                );
            }
            TargetDiffAction::Removed => {
                let tid = diff.task.tid;

                match target_pid_map.remove(&tid) {
                    Ok(()) => info!("tree_target_removed tid={tid} class={}", diff.task.class),
                    Err(e) => warn!("tree_target_remove_failed tid={tid} err={e}"),
                }

                if let Some(started) = recording_started {
                    tree_events.push(TreeEvent::from_task(started, "removed", diff.task));
                }

                if let Some(stats) = stats_by_task.get_mut(&tid) {
                    stats.active = false;
                    stats.removed_ms =
                        recording_started.map(|started| started.elapsed().as_millis());
                }
            }
        }
    }

    *active_targets = desired_tasks;

    debug!(
        "tree_refresh_complete active_tasks={} manual_roots={} tree_roots={} process_count={}",
        active_targets.len(),
        config.target_pids.len(),
        config.tree_pids.len(),
        snapshot.process_roots.len(),
    );

    Ok(())
}

fn should_replace_unknown_comm(current: &str, incoming: &str) -> bool {
    (current == "?" || current.is_empty()) && incoming != "?" && !incoming.is_empty()
}

#[allow(clippy::too_many_arguments)]
fn handle_same_tid_replacements(
    active_targets: &BTreeMap<u32, TaskInfo>,
    desired_tasks: &BTreeMap<u32, TaskInfo>,
    known_targets: &mut BTreeMap<u32, TaskInfo>,
    stats_by_task: &mut BTreeMap<u32, metrics::TaskStats>,
    task_exe_inodes: &mut TaskExeInodesMap,
    tree_events: &mut Vec<TreeEvent>,
    elapsed_ms: u128,
    recording_started: Option<Instant>,
) {
    for (tid, desired_task) in desired_tasks {
        let Some(active_task) = active_targets.get(tid) else {
            continue;
        };

        if same_task_info(active_task, desired_task) {
            continue;
        }

        known_targets.insert(*tid, desired_task.clone());
        reset_stats_for_task_change(
            stats_by_task,
            task_exe_inodes,
            *tid,
            desired_task,
            elapsed_ms,
        );

        if let Some(started) = recording_started {
            tree_events.push(TreeEvent::from_task(started, "replaced", desired_task));
        }

        warn!(
            "tree_target_replaced tid={} old_process_pid={} old_comm={} old_class={} new_process_pid={} new_comm={} new_class={}",
            tid,
            active_task.process_pid,
            active_task.comm,
            active_task.class,
            desired_task.process_pid,
            desired_task.comm,
            desired_task.class,
        );
    }
}

fn reset_stats_for_task_change(
    stats_by_task: &mut BTreeMap<u32, metrics::TaskStats>,
    task_exe_inodes: &mut TaskExeInodesMap,
    tid: u32,
    task_info: &TaskInfo,
    elapsed_ms: u128,
) {
    let mut stats = metrics::TaskStats::new(tid, task_info.comm.clone(), elapsed_ms);
    stats.apply_task_info(task_info);
    stats.active = true;
    stats_by_task.insert(tid, stats);

    update_task_exe_info(task_exe_inodes, tid, task_info);
}

fn reactivate_or_reset_stats_inner(
    stats_by_task: &mut BTreeMap<u32, metrics::TaskStats>,
    task_exe_inodes: Option<&mut TaskExeInodesMap>,
    tid: u32,
    task_info: &TaskInfo,
    elapsed_ms: u128,
) {
    let should_reset = stats_by_task.get(&tid).is_some_and(|stats| {
        stats.removed_ms.is_some()
            && !same_logical_task(stats, task_info, task_exe_inodes.as_deref())
    });

    if should_reset {
        let mut stats = metrics::TaskStats::new(tid, task_info.comm.clone(), elapsed_ms);
        stats.apply_task_info(task_info);
        stats_by_task.insert(tid, stats);
    }

    if let Some(stats) = stats_by_task.get_mut(&tid) {
        stats.apply_task_info(task_info);
        stats.active = true;
        stats.removed_ms = None;
    }

    if let Some(inodes) = task_exe_inodes {
        update_task_exe_info(inodes, tid, task_info);
    }
}

fn same_logical_task(
    stats: &metrics::TaskStats,
    task_info: &TaskInfo,
    task_exe_inodes: Option<&TaskExeInodesMap>,
) -> bool {
    if stats.task != task_info.tid {
        return false;
    }

    if stats.process_pid != Some(task_info.process_pid) {
        return false;
    }

    if let (Some(left), Some(right)) = (
        stats.process_starttime_ticks,
        task_info.process_starttime_ticks,
    ) && left != right
    {
        return false;
    }

    if let (Some(left), Some(right)) = (stats.task_starttime_ticks, task_info.task_starttime_ticks)
        && left != right
    {
        return false;
    }

    let has_strong_starttime_match = stats
        .process_starttime_ticks
        .zip(task_info.process_starttime_ticks)
        .is_some()
        || stats
            .task_starttime_ticks
            .zip(task_info.task_starttime_ticks)
            .is_some();

    if has_strong_starttime_match {
        return true;
    }

    if stats.comm != task_info.comm
        || stats.process_comm != task_info.process_comm
        || stats.class != task_info.class
    {
        return false;
    }

    if let Some(inodes) = task_exe_inodes
        && let Some(&(dev, ino, previous_start)) = inodes.get(&stats.task)
    {
        if let (Some(previous_dev), Some(current_dev)) = (dev, task_info.exe_dev)
            && previous_dev != current_dev
        {
            return false;
        }

        if let (Some(previous_ino), Some(current_ino)) = (ino, task_info.exe_ino)
            && previous_ino != current_ino
        {
            return false;
        }

        let current_start = task_info
            .process_starttime_ticks
            .or(task_info.task_starttime_ticks);

        if let (Some(previous), Some(current)) = (previous_start, current_start)
            && previous != current
        {
            return false;
        }
    }

    true
}

fn update_task_exe_info(task_exe_inodes: &mut TaskExeInodesMap, tid: u32, task_info: &TaskInfo) {
    if task_info.exe_dev.is_some() || task_info.exe_ino.is_some() {
        task_exe_inodes.insert(
            tid,
            (
                task_info.exe_dev,
                task_info.exe_ino,
                task_info
                    .process_starttime_ticks
                    .or(task_info.task_starttime_ticks),
            ),
        );
    } else {
        task_exe_inodes.remove(&tid);
    }
}

fn same_task_info(left: &TaskInfo, right: &TaskInfo) -> bool {
    if left.tid != right.tid || left.process_pid != right.process_pid {
        return false;
    }

    if let (Some(left_start), Some(right_start)) =
        (left.process_starttime_ticks, right.process_starttime_ticks)
        && left_start != right_start
    {
        return false;
    }

    if let (Some(left_start), Some(right_start)) =
        (left.task_starttime_ticks, right.task_starttime_ticks)
        && left_start != right_start
    {
        return false;
    }

    let has_strong_starttime_match = left
        .process_starttime_ticks
        .zip(right.process_starttime_ticks)
        .is_some()
        || left
            .task_starttime_ticks
            .zip(right.task_starttime_ticks)
            .is_some();

    // If proc stat start-times are unavailable for both snapshots, same-name
    // siblings under the same parent are indistinguishable. Prefer continuity
    // in that narrow case and rely on exe inode checks for reactivation paths.
    has_strong_starttime_match
        || (left.process_ppid == right.process_ppid
            && left.comm == right.comm
            && left.process_comm == right.process_comm
            && left.class == right.class)
}

fn log_drop_counters(drop_counters: &ebpf_loader::DropCountersSnapshot) {
    if drop_counters.total() == 0 {
        debug!("ebpf_drop_counters total=0");
        return;
    }

    warn!(
        "ebpf_drop_counters total={} wakeup_times_insert_failed={} ringbuf_reserve_failed={} irq_start_times_insert_failed={}",
        drop_counters.total(),
        drop_counters.wakeup_times_insert_failed,
        drop_counters.ringbuf_reserve_failed,
        drop_counters.irq_start_times_insert_failed,
    );
}
