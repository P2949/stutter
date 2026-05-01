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
mod psi;
mod recorder;
mod report;
mod scorer;
mod scx;
mod tui;

#[cfg(test)]
mod regression_tests;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, future,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use aya::maps::{HashMap as AyaHashMap, MapData};
use cli::{AppCommand, Config, parse_app_command};
use crossterm::event::{Event, EventStream, KeyCode};
use futures_util::StreamExt;
use log::{debug, info, warn};
use metrics::{
    collect_interval_summaries_labeled, format_latency, print_event, print_session_summaries,
};
use process_tree::{TargetDiffAction, TaskClass, TaskFilters, TaskInfo};
use recorder::{
    FinalizeRecordingInput, IntervalCsvWriter, IntervalRecord, IrqEventRecord, JsonArrayWriter,
    SpikeEventBuffer, TreeEvent, finalize_recording, prepare_recording,
};
use serde::Serialize;
use stutter_common::{
    BlockIoEvent, CpuFreqEvent, EVENT_BLOCK_IO, EVENT_CPU_FREQ, EVENT_EXEC, EVENT_IRQ_LATENCY,
    EVENT_MIGRATION, EVENT_RUNNABLE_LATENCY, EVENT_STAT_WAIT, ExecEvent, IrqEvent, MigrationEvent,
    SchedulerEvent, StatWaitEvent,
};
use tokio::{
    signal, task,
    time::{Duration, MissedTickBehavior, interval, sleep},
};

pub const TARGET_PIDS_MAX: usize = 1024;
const TUNE_RUN_STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
const TUNE_PROFILE_REFRESH_MS: u64 = 1_000;
const PROFILE_WATCH_VERIFY_MS: u64 = 10_000;

type TaskExeInodesMap = BTreeMap<u32, (Option<u64>, Option<u64>, Option<u64>)>;

struct RefreshTargetTasksInput<'a> {
    config: &'a Config,
    active_targets: &'a mut BTreeMap<u32, TaskInfo>,
    known_targets: &'a mut BTreeMap<u32, TaskInfo>,
    stats_by_task: &'a mut BTreeMap<u32, metrics::TaskStats>,
    task_exe_inodes: &'a mut TaskExeInodesMap,
    tree_events: &'a mut Vec<TreeEvent>,
    target_pid_map: &'a mut AyaHashMap<MapData, u32, u8>,
    prev_faults_map: Option<&'a mut AyaHashMap<MapData, u32, [u64; 2]>>,
    prev_faults_snapshot: &'a mut BTreeMap<u32, (u64, u64)>,
    elapsed_ms: u128,
    recording_started: Option<Instant>,
    process_cache: &'a mut process_tree::ProcessCache,
}

struct HandleEventInput<'a> {
    event: &'a SchedulerEvent,
    config: &'a Config,
    started: Instant,
    active_targets: &'a mut BTreeMap<u32, TaskInfo>,
    known_targets: &'a mut BTreeMap<u32, TaskInfo>,
    stats_by_task: &'a mut BTreeMap<u32, metrics::TaskStats>,
    monotonic_start_ns: Option<u64>,
    spike_events: Option<&'a mut SpikeEventBuffer>,
}

struct DrainBpfEventsInput<'a> {
    guard: tokio::io::unix::AsyncFdReadyMutGuard<'a, aya::maps::RingBuf<aya::maps::MapData>>,
    config: &'a Config,
    started: Instant,
    active_targets: &'a mut BTreeMap<u32, TaskInfo>,
    known_targets: &'a mut BTreeMap<u32, TaskInfo>,
    stats_by_task: &'a mut BTreeMap<u32, metrics::TaskStats>,
    recording_monotonic_start_ns: Option<u64>,
    spike_events: &'a mut Option<SpikeEventBuffer>,
    irq_event_writer: Option<&'a mut JsonArrayWriter>,
    irq_event_count: &'a mut usize,
    migration_event_writer: Option<&'a mut JsonArrayWriter>,
    migration_event_count: &'a mut usize,
    cpu_freq_sample_writer: Option<&'a mut JsonArrayWriter>,
    cpu_freq_sample_count: &'a mut usize,
    block_io_event_writer: Option<&'a mut JsonArrayWriter>,
    block_io_event_count: &'a mut usize,
    block_io_correlation_basis: &'a str,
    cpu_to_pkg: &'a BTreeMap<u32, String>,
    process_cache: &'a mut process_tree::ProcessCache,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    match parse_app_command()? {
        AppCommand::Monitor(config) => run_monitor(*config, None).await,
        AppCommand::Restore { dry_run } => {
            let path = affinity::default_restore_path();
            if dry_run {
                print_restore_dry_run(&path)?;
            } else {
                let summary = affinity::restore_saved(&path)?;
                println!(
                    "restored {} affinity record(s); skipped_dead={} skipped_identity_mismatch={} legacy_unverified={}",
                    summary.restored,
                    summary.skipped_dead,
                    summary.skipped_identity_mismatch,
                    summary.legacy_unverified
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
            enforce,
        } => {
            apply_profile_command(
                tree_pid,
                profile,
                force,
                dry_run,
                watch,
                keep_applied,
                refresh_ms,
                enforce,
            )
            .await
        }
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
            mangohud_log,
            enforce,
        } => {
            tune_command(
                tree_pid,
                profiles,
                epoch_seconds,
                warmup_seconds,
                keep_best,
                mangohud_log,
                enforce,
            )
            .await
        }
    }
}

fn print_restore_dry_run(path: &Path) -> anyhow::Result<()> {
    let state = affinity::load_restore_state(path)?;
    println!("restore dry-run file={}", path.display());

    for record in state.records {
        match affinity::restore_record_status(&record) {
            Ok(status @ affinity::RestoreRecordStatus::Verified)
            | Ok(status @ affinity::RestoreRecordStatus::LegacyUnverified) => {
                let identity_status = match status {
                    affinity::RestoreRecordStatus::Verified => "verified",
                    affinity::RestoreRecordStatus::LegacyUnverified => "legacy_unverified",
                    affinity::RestoreRecordStatus::Dead
                    | affinity::RestoreRecordStatus::IdentityMismatch => unreachable!(),
                };
                match affinity::read_allowed_mask_raw(record.tid) {
                    Ok(current) => println!(
                        "tid={} alive=true identity={} current_mask={} restore_mask={}",
                        record.tid,
                        identity_status,
                        current.to_range_string(),
                        record.original_mask.to_range_string()
                    ),
                    Err(err) if err.raw_os_error() == Some(libc::ESRCH) => println!(
                        "tid={} alive=false identity={} current_mask=- restore_mask={}",
                        record.tid,
                        identity_status,
                        record.original_mask.to_range_string()
                    ),
                    Err(err) => println!(
                        "tid={} alive=unknown identity={} current_mask_error={} restore_mask={}",
                        record.tid,
                        identity_status,
                        err,
                        record.original_mask.to_range_string()
                    ),
                }
            }
            Ok(affinity::RestoreRecordStatus::Dead) => {
                println!(
                    "tid={} alive=false identity=dead current_mask=- restore_mask={}",
                    record.tid,
                    record.original_mask.to_range_string()
                )
            }
            Ok(affinity::RestoreRecordStatus::IdentityMismatch) => println!(
                "tid={} alive=unknown identity=mismatch current_mask=- restore_mask={}",
                record.tid,
                record.original_mask.to_range_string()
            ),
            Err(err) => println!(
                "tid={} alive=unknown identity=error current_mask_error={} restore_mask={}",
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
    mangohud_log: Option<PathBuf>,
    enforce: bool,
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
    let tune_output_dir = default_tune_output_dir();

    let shared_hwmon = hwmon::HwmonReader::discover_with_options(None, None, None)
        .map(|r| std::sync::Arc::new(std::sync::Mutex::new(r)));

    for (idx, profile) in profiles.iter().enumerate() {
        println!(
            "tune candidate={} state=CandidateWarmup warmup_seconds={}",
            profile.name, warmup_seconds
        );

        println!(
            "tune candidate={} state=CandidateMeasure measure_seconds={}",
            profile.name, measure_seconds
        );

        let TuneMeasureResult {
            applied_tasks,
            run_dir,
            interval_records,
            frame_events,
            coverage,
        } = match measure_tune_candidate(TuneMeasureInput {
            tree_pid,
            profile: profile.clone(),
            epoch_seconds,
            warmup_seconds,
            enforce,
            shared_hwmon: shared_hwmon.clone(),
            mangohud_log: mangohud_log.clone(),
            force_restore_overwrite: idx == 0,
            tune_output_dir: tune_output_dir.clone(),
        })
        .await
        {
            Ok(res) => res,
            Err(err) => {
                restore_tune_on_error();
                return Err(err);
            }
        };

        let mut score =
            scorer::score_from_interval_records_and_frames(&interval_records, &frame_events);
        let frame_max = score.frame_max_ms;
        let frame_p99 = score.frame_p99_ms;

        // Reject / penalize candidates that did not gather enough data to be
        // meaningfully comparable. These thresholds are conservative: at
        // minimum require a couple of intervals and a modest number of
        // scheduler samples.
        const TUNE_MIN_INTERVALS: usize = 2;
        const TUNE_MIN_SAMPLES: u64 = 50;
        let interval_count = interval_records.len();
        let sample_count: u64 = interval_records.iter().map(|r| r.samples).sum();
        let (scored_interval_count, scored_sample_count) =
            tune_scored_record_counts(&interval_records);
        let mut valid =
            scored_interval_count >= TUNE_MIN_INTERVALS && scored_sample_count >= TUNE_MIN_SAMPLES;
        if !valid {
            warn!(
                "tune_candidate_insufficient_scored_data profile={} scored_intervals={} scored_samples={} total_intervals={} total_samples={}",
                profile.name,
                scored_interval_count,
                scored_sample_count,
                interval_count,
                sample_count
            );
            // Inflate the score so this candidate loses to better-measured ones.
            score.total = u64::MAX / 4;
        }

        if coverage.unique_scored_tasks == 0 {
            warn!(
                "tune_candidate_no_scored_tasks profile={} tracked_tasks={}",
                profile.name, coverage.unique_tracked_tasks
            );
            score.total = u64::MAX / 4;
            valid = false;
        }
        if coverage.drop_counter_total > 0 {
            warn!(
                "tune_candidate_drop_counters_nonzero profile={} drops={}",
                profile.name, coverage.drop_counter_total
            );
        }

        let result = TuneCandidateSummary {
            profile: profile.name.clone(),
            run_dir,
            applied_tasks,
            warmup_seconds,
            measure_seconds,
            interval_count: interval_records.len(),
            samples: sample_count,
            scored_samples: scored_sample_count,
            score_total: score.total,
            over_1ms: score.over_1ms,
            over_2ms: score.over_2ms,
            over_5ms: score.over_5ms,
            max_latency_ns: score.max_latency_ns,
            frame_count: frame_events.len(),
            frame_max_ms: frame_max,
            frame_p99_ms: frame_p99,
            coverage,
            valid,
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

    let any_valid = results.iter().any(|r| r.valid);
    if !any_valid {
        restore_tune_on_error();
        anyhow::bail!("no tune candidate collected enough data; no best profile selected");
    }

    // Check for large disparities in sample counts across candidates and
    // warn when candidates are not meaningfully comparable.
    let min_samples = results.iter().map(|r| r.scored_samples).min().unwrap_or(0);
    let max_samples = results.iter().map(|r| r.scored_samples).max().unwrap_or(0);
    if min_samples == 0 && max_samples > 0 {
        anyhow::bail!(
            "tune candidates are not comparable: some candidates gathered no scored samples while others did (max_scored_samples={})",
            max_samples
        );
    } else if min_samples > 0 {
        let ratio = (max_samples as f64) / (min_samples as f64);
        if ratio > 2.0 {
            anyhow::bail!(
                "tune candidates are not comparable: scored sample count varies by more than 2x across candidates (min={} max={} ratio={:.2})",
                min_samples,
                max_samples,
                ratio
            );
        }
    }
    check_tune_coverage_comparability(&results)?;

    let min_frames = results.iter().map(|r| r.frame_count).min().unwrap_or(0);
    let max_frames = results.iter().map(|r| r.frame_count).max().unwrap_or(0);
    if min_frames > 0 {
        let ratio = (max_frames as f64) / (min_frames as f64);
        if ratio > 1.5 {
            warn!(
                "tune_candidates_unbalanced_frames min={} max={} ratio={:.2}",
                min_frames, max_frames, ratio
            );
        }
    }

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
        apply_profile_to_tree_blocking(tree_pid, profile.clone(), false, false, enforce).await?;
    }

    let summary_path = tune_output_dir.join("tuning_summary.json");
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

struct TuneMeasureInput {
    tree_pid: u32,
    profile: profiles::Profile,
    epoch_seconds: u64,
    warmup_seconds: u64,
    enforce: bool,
    shared_hwmon: Option<std::sync::Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
    mangohud_log: Option<PathBuf>,
    force_restore_overwrite: bool,
    tune_output_dir: PathBuf,
}

struct TuneMeasureResult {
    applied_tasks: usize,
    run_dir: PathBuf,
    interval_records: Vec<IntervalRecord>,
    frame_events: Vec<crate::recorder::FrameEvent>,
    coverage: TuneCoverageMetrics,
}

async fn measure_tune_candidate(input: TuneMeasureInput) -> anyhow::Result<TuneMeasureResult> {
    let TuneMeasureInput {
        tree_pid,
        profile,
        epoch_seconds,
        warmup_seconds,
        enforce,
        shared_hwmon,
        mangohud_log,
        force_restore_overwrite,
        tune_output_dir,
    } = input;
    let profile_name = profile.name.clone();
    let run_dir = tune_run_dir(&tune_output_dir, &profile_name);
    let config = Config {
        target_pids: Vec::new(),
        tree_pids: vec![tree_pid],
        summary_period_ms: 1_000,
        epoch_period_ms: None,
        spike_threshold_ns: 1_000_000,
        alert_threshold_ns: None,
        alert_webhook_url: None,
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
        mangohud_log,
        tui: false,
        retain_intervals: None,
        recording: Some(cli::RecordingConfig {
            run_name: Some(format!("tune-{profile_name}")),
            out_dir: Some(run_dir.clone()),
        }),
        max_duration: Some(Duration::from_secs(epoch_seconds)),
        cgroupv2: None,
        follow_exec: true,
        exclude_tree_pids: Vec::new(),
    };

    let cache = profiles::ProfileApplyCache::default();
    let (initial_records, cache) = apply_profile_to_tree_cached_blocking(
        tree_pid,
        profile.clone(),
        force_restore_overwrite,
        false,
        cache,
    )
    .await?;
    let initial_applied_tasks = initial_records.len();
    let should_force_refresh = force_restore_overwrite && initial_records.is_empty();

    let stop_refresh = Arc::new(AtomicBool::new(false));
    let refreshed_applied_tasks = Arc::new(AtomicUsize::new(0));
    let mut profile_refresh = tokio::spawn(tune_profile_refresh_loop(
        tree_pid,
        profile,
        cache,
        should_force_refresh,
        TUNE_PROFILE_REFRESH_MS,
        stop_refresh.clone(),
        refreshed_applied_tasks.clone(),
        enforce,
    ));

    let mut profile_refresh_finished = false;
    let monitor_result = tokio::select! {
        result = run_monitor(config, shared_hwmon) => result,
        refresh_result = &mut profile_refresh => {
            profile_refresh_finished = true;
            match refresh_result {
                Ok(Ok(())) => Err(anyhow::anyhow!("tune profile refresh stopped before monitor epoch ended")),
                Ok(Err(err)) => Err(err.context("tune profile refresh failed")),
                Err(err) => Err(anyhow::anyhow!("tune profile refresh worker failed: {err}")),
            }
        }
    };

    stop_refresh.store(true, Ordering::Relaxed);
    if !profile_refresh_finished {
        match profile_refresh.await {
            Ok(Ok(())) => {}
            Ok(Err(err)) if monitor_result.is_ok() => return Err(err),
            Ok(Err(err)) => warn!("tune_profile_refresh_stop_failed err={err:#}"),
            Err(err) if monitor_result.is_ok() => {
                return Err(anyhow::anyhow!("tune profile refresh worker failed: {err}"));
            }
            Err(err) => warn!("tune_profile_refresh_join_failed err={err}"),
        }
    };
    let applied_tasks =
        initial_applied_tasks.saturating_add(refreshed_applied_tasks.load(Ordering::Relaxed));

    monitor_result?;

    let interval_path = run_dir.join("interval.json");
    let data = fs::read_to_string(&interval_path)
        .with_context(|| format!("failed to read interval.json from {}", run_dir.display()))?;
    let mut interval_records: Vec<IntervalRecord> = serde_json::from_str(&data)?;
    let warmup_ms = u128::from(warmup_seconds) * 1_000;
    interval_records.retain(|r| r.elapsed_ms >= warmup_ms);

    let frame_path = run_dir.join("frame_correlation.json");
    let mut frame_events: Vec<crate::recorder::FrameEvent> = if frame_path.exists() {
        let data = fs::read_to_string(&frame_path)?;
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Vec::new()
    };
    frame_events.retain(|f| f.elapsed_ms >= warmup_ms);

    let session_path = run_dir.join("session.json");
    let session_data = fs::read_to_string(&session_path)
        .with_context(|| format!("failed to read session.json from {}", run_dir.display()))?;
    let session: recorder::SessionFile = serde_json::from_str(&session_data)?;
    let coverage = tune_coverage_metrics(&session, &interval_records);

    Ok(TuneMeasureResult {
        applied_tasks,
        run_dir,
        interval_records,
        frame_events,
        coverage,
    })
}

#[allow(clippy::too_many_arguments)]
async fn tune_profile_refresh_loop(
    tree_pid: u32,
    profile: profiles::Profile,
    mut cache: profiles::ProfileApplyCache,
    force_restore_overwrite: bool,
    refresh_ms: u64,
    stop_refresh: Arc<AtomicBool>,
    applied_tasks: Arc<AtomicUsize>,
    enforce: bool,
) -> anyhow::Result<()> {
    let mut should_force = force_restore_overwrite;
    let refresh_interval = Duration::from_millis(refresh_ms);
    let verify_interval = Duration::from_millis(PROFILE_WATCH_VERIFY_MS);
    let mut next_verify = Instant::now() + verify_interval;

    loop {
        if stop_refresh.load(Ordering::Relaxed) {
            return Ok(());
        }

        if enforce || Instant::now() >= next_verify {
            cache.clear();
            next_verify = Instant::now() + verify_interval;
            debug!("tune_profile_refresh_cache_invalidated_for_full_verify enforce={enforce}");
        }

        let (records, updated_cache) = apply_profile_to_tree_cached_blocking(
            tree_pid,
            profile.clone(),
            should_force,
            false,
            cache,
        )
        .await?;
        cache = updated_cache;

        if !records.is_empty() {
            applied_tasks.fetch_add(records.len(), Ordering::Relaxed);
            should_force = false;
            info!(
                "tune_profile_refresh_applied profile={} tasks={}",
                profile.name,
                records.len()
            );
        }

        sleep(refresh_interval).await;
    }
}

fn tune_run_dir(tune_output_dir: &Path, profile_name: &str) -> PathBuf {
    let sanitized = profile_name
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    tune_output_dir
        .join("candidates")
        .join(format!("{}-{}", sanitized, unix_nanos_now()))
}

fn cleanup_stale_tune_run_dirs(state_dir: &Path) {
    let Ok(entries) = fs::read_dir(state_dir) else {
        return;
    };

    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("tune-") {
            continue;
        }

        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let Ok(age) = now.duration_since(modified) else {
            continue;
        };
        if age <= TUNE_RUN_STALE_AFTER {
            continue;
        }

        if let Err(err) = fs::remove_dir_all(&path) {
            warn!(
                "stale_tune_run_cleanup_failed path={} err={err}",
                path.display()
            );
        }
    }
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
    run_dir: PathBuf,
    applied_tasks: usize,
    warmup_seconds: u64,
    measure_seconds: u64,
    interval_count: usize,
    samples: u64,
    scored_samples: u64,
    score_total: u64,
    over_1ms: u64,
    over_2ms: u64,
    over_5ms: u64,
    max_latency_ns: u64,
    frame_count: usize,
    frame_max_ms: f64,
    frame_p99_ms: f64,
    coverage: TuneCoverageMetrics,
    valid: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
struct TaskIdentity {
    class: TaskClass,
    process_comm: String,
    comm: String,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
    exe_dev: Option<u64>,
    exe_ino: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize)]
struct TuneCoverageMetrics {
    unique_tracked_tasks: usize,
    unique_scored_tasks: usize,
    active_target_min: usize,
    active_target_max: usize,
    removed_task_count: usize,
    drop_counter_total: u64,
    #[serde(skip_serializing)]
    scored_identity_counts: BTreeMap<TaskIdentity, usize>,
}

fn result_is_better(candidate: &TuneCandidateSummary, current_best: &TuneCandidateSummary) -> bool {
    (candidate.score_total, candidate.max_latency_ns)
        < (current_best.score_total, current_best.max_latency_ns)
}

fn tune_scored_record_counts(records: &[IntervalRecord]) -> (usize, u64) {
    records
        .iter()
        .filter(|record| scorer::class_contributes_to_score(record.class))
        .fold((0usize, 0u64), |(intervals, samples), record| {
            (intervals + 1, samples.saturating_add(record.samples))
        })
}

fn tune_coverage_metrics(
    session: &recorder::SessionFile,
    interval_records: &[IntervalRecord],
) -> TuneCoverageMetrics {
    let unique_tracked_tasks = session.tasks.len();
    let scored_task_ids = interval_records
        .iter()
        .filter(|record| scorer::class_contributes_to_score(record.class) && record.samples > 0)
        .map(|record| record.task)
        .collect::<BTreeSet<_>>();
    let unique_scored_tasks = scored_task_ids.len();
    let removed_task_count = session
        .tasks
        .iter()
        .filter(|task| !task.active || task.removed_ms.is_some())
        .count();

    let mut active_by_elapsed: BTreeMap<u128, usize> = BTreeMap::new();
    for record in interval_records.iter().filter(|record| record.active) {
        *active_by_elapsed.entry(record.elapsed_ms).or_default() += 1;
    }
    let (active_target_min, active_target_max) = if active_by_elapsed.is_empty() {
        (
            session.active_target_pids_count,
            session.active_target_pids_count,
        )
    } else {
        (
            active_by_elapsed.values().copied().min().unwrap_or(0),
            active_by_elapsed.values().copied().max().unwrap_or(0),
        )
    };

    let tasks_by_tid = session
        .tasks
        .iter()
        .map(|task| (task.task, task))
        .collect::<BTreeMap<_, _>>();
    let mut scored_identity_counts = BTreeMap::<TaskIdentity, usize>::new();
    for tid in scored_task_ids {
        let identity = if let Some(task) = tasks_by_tid.get(&tid) {
            TaskIdentity {
                class: task.class,
                process_comm: task.process_comm.to_string(),
                comm: task.comm.clone(),
                process_starttime_ticks: task.process_starttime_ticks,
                task_starttime_ticks: task.task_starttime_ticks,
                exe_dev: task.exe_dev,
                exe_ino: task.exe_ino,
            }
        } else if let Some(record) = interval_records.iter().find(|record| record.task == tid) {
            TaskIdentity {
                class: record.class,
                process_comm: record.process_comm.to_string(),
                comm: record.comm.clone(),
                process_starttime_ticks: None,
                task_starttime_ticks: None,
                exe_dev: None,
                exe_ino: None,
            }
        } else {
            continue;
        };
        *scored_identity_counts.entry(identity).or_default() += 1;
    }

    TuneCoverageMetrics {
        unique_tracked_tasks,
        unique_scored_tasks,
        active_target_min,
        active_target_max,
        removed_task_count,
        drop_counter_total: session.drop_counters.total(),
        scored_identity_counts,
    }
}

fn check_tune_coverage_comparability(results: &[TuneCandidateSummary]) -> anyhow::Result<()> {
    check_tune_metric_ratio(
        "unique tracked tasks",
        results
            .iter()
            .map(|result| result.coverage.unique_tracked_tasks),
    )?;
    check_tune_metric_ratio(
        "unique scored tasks",
        results
            .iter()
            .map(|result| result.coverage.unique_scored_tasks),
    )?;

    // Verify task identity stability. If the set of scored tasks shifts
    // significantly between candidates, the comparison is unsafe.
    if let Some(first) = results.first() {
        for other in results.iter().skip(1) {
            let common = scored_identity_overlap(
                &first.coverage.scored_identity_counts,
                &other.coverage.scored_identity_counts,
                usize::min,
            );
            let total = scored_identity_overlap(
                &first.coverage.scored_identity_counts,
                &other.coverage.scored_identity_counts,
                usize::max,
            );

            let overlap_ratio = if total > 0 {
                common as f64 / total as f64
            } else {
                1.0
            };

            if overlap_ratio < 0.75 {
                anyhow::bail!(
                    "scored task identity mismatch (overlap={:.1}%); candidates are not comparable (major thread topology shift)",
                    overlap_ratio * 100.0
                );
            }
        }
    }

    check_tune_metric_ratio(
        "active target minimum",
        results
            .iter()
            .map(|result| result.coverage.active_target_min),
    )?;
    check_tune_metric_ratio(
        "active target maximum",
        results
            .iter()
            .map(|result| result.coverage.active_target_max),
    )?;

    let min_removed = results
        .iter()
        .map(|result| result.coverage.removed_task_count)
        .min()
        .unwrap_or(0);
    let max_removed = results
        .iter()
        .map(|result| result.coverage.removed_task_count)
        .max()
        .unwrap_or(0);
    if max_removed > min_removed {
        warn!(
            "tune_candidates_removed_task_counts_differ min={} max={}",
            min_removed, max_removed
        );
    }

    let max_drops = results
        .iter()
        .map(|result| result.coverage.drop_counter_total)
        .max()
        .unwrap_or(0);
    if max_drops > 0 {
        warn!("tune_candidates_drop_counters_nonzero max_drops={max_drops}");
    }

    Ok(())
}

fn scored_identity_overlap(
    left: &BTreeMap<TaskIdentity, usize>,
    right: &BTreeMap<TaskIdentity, usize>,
    combine: fn(usize, usize) -> usize,
) -> usize {
    left.keys()
        .chain(right.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|identity| {
            combine(
                left.get(identity).copied().unwrap_or(0),
                right.get(identity).copied().unwrap_or(0),
            )
        })
        .sum()
}

fn check_tune_metric_ratio(label: &str, values: impl Iterator<Item = usize>) -> anyhow::Result<()> {
    let values = values.collect::<Vec<_>>();
    let min_value = values.iter().copied().min().unwrap_or(0);
    let max_value = values.iter().copied().max().unwrap_or(0);
    if min_value == 0 && max_value > 0 {
        anyhow::bail!(
            "tune candidates are not comparable: {} is zero for some candidates but nonzero for others (max={})",
            label,
            max_value
        );
    }
    if min_value > 0 {
        let ratio = (max_value as f64) / (min_value as f64);
        if ratio > 2.0 {
            anyhow::bail!(
                "tune candidates are not comparable: {} varies by more than 2x across candidates (min={} max={} ratio={:.2})",
                label,
                min_value,
                max_value,
                ratio
            );
        }
    }
    Ok(())
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
        "tune_candidate_restored profile={} restored={} skipped_dead={} skipped_identity_mismatch={} legacy_unverified={}",
        profile_name,
        summary.restored,
        summary.skipped_dead,
        summary.skipped_identity_mismatch,
        summary.legacy_unverified
    );
    Ok(())
}

fn default_tune_output_dir() -> PathBuf {
    let mut path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    path.push(".local");
    path.push("state");
    path.push("stutter");
    cleanup_stale_tune_run_dirs(&path);
    path.push(format!("tune-{}", unix_nanos_now()));
    path
}

fn unix_nanos_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[allow(clippy::too_many_arguments)]
async fn apply_profile_command(
    tree_pid: u32,
    profile_path: PathBuf,
    force: bool,
    dry_run: bool,
    watch: bool,
    keep_applied: bool,
    refresh_ms: u64,
    enforce: bool,
) -> anyhow::Result<()> {
    let profile = profiles::load_first_profile(&profile_path)?;
    let mut cache = profiles::ProfileApplyCache::default();

    let records = if watch {
        match apply_profile_to_tree_cached_blocking(
            tree_pid,
            profile.clone(),
            force,
            dry_run,
            cache,
        )
        .await
        {
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
        apply_profile_to_tree_blocking(tree_pid, profile.clone(), force, dry_run, enforce).await?
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
    let verify_interval = Duration::from_millis(PROFILE_WATCH_VERIFY_MS);
    let mut next_verify = Instant::now() + verify_interval;

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
                if enforce || Instant::now() >= next_verify {
                    cache.clear();
                    next_verify = Instant::now() + verify_interval;
                    debug!("profile_watch_cache_invalidated_for_full_verify enforce={enforce}");
                }

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
    _enforce: bool,
) -> anyhow::Result<Vec<affinity::AffinityRecord>> {
    task::spawn_blocking(move || {
        // Enforce is handled by the caller clearing the cache in watch mode.
        // Blocking one-shot always verifies.
        profiles::apply_profile_to_tree(tree_pid, &profile, force, dry_run)
    })
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
        "stopped profile watch; restored {} affinity record(s); skipped_dead={} skipped_identity_mismatch={} legacy_unverified={}",
        summary.restored,
        summary.skipped_dead,
        summary.skipped_identity_mismatch,
        summary.legacy_unverified
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

async fn run_monitor(
    mut config: Config,
    shared_hwmon: Option<std::sync::Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
) -> anyhow::Result<()> {
    if config.target_pids.is_empty()
        && config.tree_pids.is_empty()
        && config.watch_process.is_none()
        && config.cgroupv2.is_none()
    {
        let auto_targets = process_tree::find_auto_target_pids(Path::new("/proc"));
        if auto_targets.is_empty() {
            anyhow::bail!(
                "no target specified and no game launcher (gamescope, pressure-vessel, etc.) detected. \
                 Please provide --pid <PID>, --tree-pid <PID>, --watch-process <COMM>, or --cgroupv2 <PATH>"
            );
        }

        let pids: Vec<_> = auto_targets.iter().map(|(p, _)| *p).collect();
        let class = auto_targets[0].1;
        info!("auto_detected_launcher class={class} pids={pids:?}");
        println!("auto-detected game launcher: {class} (PIDs {pids:?}). monitoring tree...");
        config.tree_pids = pids;
    }

    let mut watch_state = match resolve_watch_process(&mut config).await? {
        Some(pid) => WatchProcessState::Running(pid),
        None => WatchProcessState::None,
    };

    let had_tree_roots = !config.tree_pids.is_empty();
    let mut tree_root_starttimes = capture_tree_root_starttimes(&config.tree_pids);

    let recording = prepare_recording(&config)?;
    let mut loaded = ebpf_loader::load_and_attach(&config).map_err(anyhow::Error::new)?;
    configure_target_irqs(&mut loaded, &config)?;
    let block_io_correlation_basis = loaded.block_io_correlation_basis.as_str().to_owned();

    let mut active_targets: BTreeMap<u32, TaskInfo> = BTreeMap::new();
    let mut known_targets: BTreeMap<u32, TaskInfo> = BTreeMap::new();
    let mut stats_by_task: BTreeMap<u32, metrics::TaskStats> = BTreeMap::new();
    let mut prev_faults_snapshot: BTreeMap<u32, (u64, u64)> = BTreeMap::new();
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
    let mut migration_event_writer = recording
        .as_ref()
        .map(|run| JsonArrayWriter::create(run.run_dir.join("migration_events.json")))
        .transpose()?;
    let mut cpu_freq_sample_writer = recording
        .as_ref()
        .map(|run| JsonArrayWriter::create(run.run_dir.join("cpu_freq_samples.json")))
        .transpose()?;
    let mut gpu_sample_writer = recording
        .as_ref()
        .map(|run| JsonArrayWriter::create(run.run_dir.join("gpu_samples.json")))
        .transpose()?;
    let mut block_io_event_writer = recording
        .as_ref()
        .map(|run| JsonArrayWriter::create(run.run_dir.join("io_events.json")))
        .transpose()?;
    let mut csv_writer = config
        .csv_path
        .as_ref()
        .map(|path| IntervalCsvWriter::create(path.clone()))
        .transpose()?;
    let mut streamed_interval_record_count = 0usize;
    let mut streamed_irq_event_count = 0usize;
    let mut streamed_migration_event_count = 0usize;
    let mut streamed_cpu_freq_sample_count = 0usize;
    let mut streamed_gpu_sample_count = 0usize;
    let mut streamed_block_io_event_count = 0usize;

    let metadata = metadata::collect_system_metadata();
    let cpu_to_pkg: BTreeMap<u32, String> = metadata
        .cpu_topology
        .iter()
        .map(|c| (c.cpu, c.physical_package_id.clone().unwrap_or_default()))
        .collect();

    let psi_reader = psi::PsiReader::new();

    // These mutable collections are intentionally confined to the main monitoring task.
    // Blocking work returns state to this task and does not mutate these collections
    // concurrently. Future background mutation should use Arc<Mutex<_>> or messages.
    let mut scx_tracker = scx::ScxTracker::default();
    let recording_monotonic_start_ns = recording.as_ref().and_then(|run| run.monotonic_start_ns);

    let hwmon_reader = if let Some(shared) = &shared_hwmon {
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
    let mut watch_process_cache = process_tree::ProcessCache::default();

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
        prev_faults_map: loaded.prev_faults_map.as_mut(),
        prev_faults_snapshot: &mut prev_faults_snapshot,
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
        Some(
            crate::tui::init_terminal()
                .map_err(|e| anyhow::anyhow!("failed to init terminal: {e}"))?,
        )
    } else {
        None
    };
    let mut crossterm_events = EventStream::new();
    let interval_label = if config.epoch_period_ms.is_some() {
        "epoch"
    } else {
        "summary"
    };

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
                    let psi_snapshot = psi_reader.read().ok();
                    let records = collect_interval_summaries_labeled(
                        interval_label,
                        &mut stats_by_task,
                        elapsed_ms,
                        &drop_counters_snapshot,
                        loaded.prev_faults_map.as_ref(),
                        psi_snapshot.as_ref(),
                        &mut prev_faults_snapshot,
                    );
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

            _ = tree_tick.tick(), if !config.tree_pids.is_empty() || config.cgroupv2.is_some() => {
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
                        prev_faults_map: loaded.prev_faults_map.as_mut(),
                        prev_faults_snapshot: &mut prev_faults_snapshot,
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
                        prev_faults_map: loaded.prev_faults_map.as_mut(),
                        prev_faults_snapshot: &mut prev_faults_snapshot,
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
                    prev_faults_map: loaded.prev_faults_map.as_mut(),
                    prev_faults_snapshot: &mut prev_faults_snapshot,
                    elapsed_ms: started.elapsed().as_millis(),
                    recording_started: recording.as_ref().map(|run| run.started_instant),
                    process_cache: &mut process_cache,
                })
                .await?;

                // Belt-and-suspenders cleanup in case a refresh path exits before
                // emitting per-task removal diffs.
                prev_faults_snapshot.retain(|tid, _| active_targets.contains_key(tid));
            }

            _ = watch_tick.tick(), if matches!(watch_state, WatchProcessState::Waiting) => {
                let Some(pattern) = config.watch_process.clone() else {
                    continue;
                };

                if let Some(pid) = find_process_by_pattern_at_with_cache(
                    Path::new("/proc"),
                    &pattern,
                    &mut watch_process_cache,
                ) {
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
                        prev_faults_map: loaded.prev_faults_map.as_mut(),
                        prev_faults_snapshot: &mut prev_faults_snapshot,
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
                    active_targets: &mut active_targets,
                    known_targets: &mut known_targets,
                    stats_by_task: &mut stats_by_task,
                    recording_monotonic_start_ns,
                    spike_events: &mut spike_events,
                    irq_event_writer: irq_event_writer.as_mut(),
                    irq_event_count: &mut streamed_irq_event_count,
                    migration_event_writer: migration_event_writer.as_mut(),
                    migration_event_count: &mut streamed_migration_event_count,
                    cpu_freq_sample_writer: cpu_freq_sample_writer.as_mut(),
                    cpu_freq_sample_count: &mut streamed_cpu_freq_sample_count,
                    block_io_event_writer: block_io_event_writer.as_mut(),
                    block_io_event_count: &mut streamed_block_io_event_count,
                    block_io_correlation_basis: &block_io_correlation_basis,
                    cpu_to_pkg: &cpu_to_pkg,
                    process_cache: &mut process_cache,
                });
            }
        }
    };

    if let Some(term) = terminal.as_mut() {
        let _ = crate::tui::restore_terminal(term);
    }

    let drop_counters = loaded.snapshot_drop_counters();
    log_drop_counters(&drop_counters);
    if config.epoch_period_ms.is_none() {
        print_session_summaries(&mut stats_by_task);
    }

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
        if let Some(writer) = block_io_event_writer.as_mut() {
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
            migration_event_count: migration_event_writer
                .is_some()
                .then_some(streamed_migration_event_count),
            cpu_freq_sample_count: cpu_freq_sample_writer
                .is_some()
                .then_some(streamed_cpu_freq_sample_count),
            gpu_samples: &gpu_samples,
            streamed_gpu_sample_count: gpu_sample_writer
                .is_some()
                .then_some(streamed_gpu_sample_count),
            block_io_event_count: streamed_block_io_event_count,
            block_io_correlation_basis: &block_io_correlation_basis,
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

    let mut cache = process_tree::ProcessCache::default();
    if let Some(pid) =
        find_process_by_pattern_at_with_cache(Path::new("/proc"), &pattern, &mut cache)
    {
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
    let mut cache = process_tree::ProcessCache::default();

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
                if let Some(pid) = find_process_by_pattern_at_with_cache(
                    Path::new("/proc"),
                    &pattern,
                    &mut cache,
                ) {
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

#[cfg(test)]
fn find_process_by_pattern_at(proc_root: &Path, pattern: &str) -> Option<u32> {
    let mut cache = process_tree::ProcessCache::default();
    find_process_by_pattern_at_with_cache(proc_root, pattern, &mut cache)
}

fn find_process_by_pattern_at_with_cache(
    proc_root: &Path,
    pattern: &str,
    cache: &mut process_tree::ProcessCache,
) -> Option<u32> {
    let pattern_lower = normalize_process_match_text(pattern);

    process_tree::scan_processes_at(proc_root, cache)
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

fn drain_bpf_events(input: DrainBpfEventsInput<'_>) {
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
        mut migration_event_writer,
        migration_event_count,
        mut cpu_freq_sample_writer,
        cpu_freq_sample_count,
        mut block_io_event_writer,
        block_io_event_count,
        block_io_correlation_basis,
        cpu_to_pkg,
        process_cache,
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
                    push_json_stream_event(writer, &record, irq_event_count, "irq_events");
                }
                log_irq_event(event);
            }
            EVENT_MIGRATION => {
                if item.len() < std::mem::size_of::<MigrationEvent>() {
                    warn!("short_migration_event len={}", item.len());
                    continue;
                }
                let event = unsafe { &*(item.as_ptr() as *const MigrationEvent) };
                let elapsed_ms = started.elapsed().as_millis();

                if let Some(stats) = stats_by_task.get_mut(&event.tid) {
                    stats.migration_count += 1;

                    let from_pkg = cpu_to_pkg.get(&event.from_cpu);
                    let to_pkg = cpu_to_pkg.get(&event.to_cpu);
                    if let (Some(f), Some(t)) = (from_pkg, to_pkg)
                        && f != t
                    {
                        stats.cross_numa_migrations += 1;
                    }
                }

                if let Some(writer) = migration_event_writer.as_deref_mut() {
                    let record = recorder::MigrationEventRecord {
                        elapsed_ms,
                        tid: event.tid,
                        from_cpu: event.from_cpu,
                        to_cpu: event.to_cpu,
                        timestamp_ns: event.timestamp_ns,
                    };
                    push_json_stream_event(
                        writer,
                        &record,
                        migration_event_count,
                        "migration_events",
                    );
                }
            }
            EVENT_CPU_FREQ => {
                if item.len() < std::mem::size_of::<CpuFreqEvent>() {
                    warn!("short_cpu_freq_event len={}", item.len());
                    continue;
                }
                let event = unsafe { &*(item.as_ptr() as *const CpuFreqEvent) };
                let elapsed_ms = started.elapsed().as_millis();

                if let Some(writer) = cpu_freq_sample_writer.as_deref_mut() {
                    let record = recorder::CpuFreqRecord {
                        elapsed_ms,
                        cpu: event.cpu,
                        freq_khz: event.state, // state field contains freq in kHz
                        timestamp_ns: event.timestamp_ns,
                    };
                    push_json_stream_event(
                        writer,
                        &record,
                        cpu_freq_sample_count,
                        "cpu_freq_samples",
                    );
                }
            }
            EVENT_STAT_WAIT => {
                if item.len() < std::mem::size_of::<StatWaitEvent>() {
                    warn!("short_stat_wait_event len={}", item.len());
                    continue;
                }
                let event = unsafe { &*(item.as_ptr() as *const StatWaitEvent) };
                if let Some(stats) = stats_by_task.get_mut(&event.tid) {
                    stats.stat_wait_sum_ns += event.delay_ns as u128;
                    stats.stat_wait_count += 1;
                }
            }
            EVENT_BLOCK_IO => {
                if item.len() < std::mem::size_of::<BlockIoEvent>() {
                    warn!("short_block_io_event len={}", item.len());
                    continue;
                }
                let event = unsafe { &*(item.as_ptr() as *const BlockIoEvent) };
                let elapsed_ms = started.elapsed().as_millis();

                if let Some(writer) = block_io_event_writer.as_deref_mut() {
                    let record = recorder::BlockIoRecord {
                        elapsed_ms,
                        tid: event.tid,
                        correlation_basis: block_io_correlation_basis.to_owned(),
                        dev: event.dev,
                        nr_sector: event.nr_sector,
                        sector: event.sector,
                        duration_ns: event.duration_ns,
                        timestamp_ns: event.timestamp_ns,
                        rwbs: String::from_utf8_lossy(&event.rwbs)
                            .trim_matches(char::from(0))
                            .to_owned(),
                    };
                    push_json_stream_event(writer, &record, block_io_event_count, "io_events");
                }
            }
            EVENT_EXEC => {
                if !config.follow_exec {
                    continue;
                }
                if item.len() < std::mem::size_of::<ExecEvent>() {
                    warn!("short_exec_event len={}", item.len());
                    continue;
                }
                let event = unsafe { &*(item.as_ptr() as *const ExecEvent) };
                let comm = metrics::comm_to_string(&event.comm);
                process_cache.invalidate(event.pid);
                process_cache.invalidate(event.tid);

                info!(
                    "process_exec pid={} tid={} comm={}",
                    event.pid, event.tid, comm
                );

                if let Some(stats) = stats_by_task.get_mut(&event.tid) {
                    stats.comm = comm.clone();
                    stats.class = process_tree::classify_task(&comm, &comm, "");
                }

                if let Some(info) = active_targets.get_mut(&event.tid) {
                    info.comm = comm.clone();
                    info.class = process_tree::classify_task(&comm, &comm, "");
                }
            }
            other => warn!("unknown_bpf_event kind={other} len={}", item.len()),
        }
    }

    guard.clear_ready();
}

fn push_json_stream_event<T: Serialize>(
    writer: &mut JsonArrayWriter,
    value: &T,
    count: &mut usize,
    stream_name: &str,
) {
    match writer.push(value) {
        Ok(()) => *count += 1,
        Err(err) => warn!("json_stream_write_failed stream={stream_name} err={err:#}"),
    }
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
            } else if config.cgroupv2.is_some() {
                stats.active = true;
            }

            stats.record(event, config.spike_threshold_ns, elapsed_ms);

            let alert_payload = if config
                .alert_threshold_ns
                .is_some_and(|threshold| event.latency_ns >= threshold)
            {
                Some(AlertPayload::from_task_stats(stats, event, elapsed_ms))
            } else {
                None
            };

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

            if let Some(alert_payload) = alert_payload {
                dispatch_alert(config, alert_payload);
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

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct AlertPayload {
    title: String,
    message: String,
    task: u32,
    active: bool,
    class: TaskClass,
    comm: String,
    process_pid: Option<u32>,
    process_comm: String,
    latency_ns: u64,
    latency_ms: u64,
    cpu: u32,
    prio: i32,
    wakeup_ns: u64,
    switch_ns: u64,
    elapsed_ms: u128,
}

impl AlertPayload {
    fn from_task_stats(
        stats: &metrics::TaskStats,
        event: &SchedulerEvent,
        elapsed_ms: u128,
    ) -> Self {
        let latency_ms = event.latency_ns / 1_000_000;
        let title = "stutter latency alert".to_owned();
        let message = format!(
            "task={} comm={} latency={} cpu={} process_pid={:?} process_comm={}",
            event.pid,
            stats.comm,
            format_latency(event.latency_ns),
            event.cpu,
            stats.process_pid,
            stats.process_comm
        );

        Self {
            title,
            message,
            task: event.pid,
            active: stats.active,
            class: stats.class,
            comm: stats.comm.clone(),
            process_pid: stats.process_pid,
            process_comm: stats.process_comm.to_string(),
            latency_ns: event.latency_ns,
            latency_ms,
            cpu: event.cpu,
            prio: event.prio,
            wakeup_ns: event.wakeup_ns,
            switch_ns: event.switch_ns,
            elapsed_ms,
        }
    }
}

fn dispatch_alert(config: &Config, payload: AlertPayload) {
    let webhook_url = config.alert_webhook_url.clone();
    let spawn_result = std::thread::Builder::new()
        .name("stutter-alert".to_owned())
        .spawn(move || {
            let result = if let Some(url) = webhook_url {
                send_webhook_alert(&url, &payload)
            } else {
                send_desktop_alert(&payload)
            };

            if let Err(err) = result {
                warn!("alert_delivery_failed err={err}");
            }
        });

    if let Err(err) = spawn_result {
        warn!("alert_thread_spawn_failed err={err}");
    }
}

fn send_desktop_alert(payload: &AlertPayload) -> Result<(), String> {
    let status = std::process::Command::new("notify-send")
        .args([
            "--urgency=critical",
            payload.title.as_str(),
            payload.message.as_str(),
        ])
        .status()
        .map_err(|err| format!("failed to run notify-send: {err}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("notify-send exited with {status}"))
    }
}

fn send_webhook_alert(url: &str, payload: &AlertPayload) -> Result<(), String> {
    let body = serde_json::to_string(payload)
        .map_err(|err| format!("failed to serialize alert payload: {err}"))?;
    let status = std::process::Command::new("curl")
        .args([
            "-fsS",
            "--max-time",
            "10",
            "-H",
            "Content-Type: application/json",
            "-X",
            "POST",
            "--data-binary",
            body.as_str(),
            url,
        ])
        .status()
        .map_err(|err| format!("failed to run curl webhook POST: {err}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("curl webhook POST exited with {status}"))
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
        mut prev_faults_map,
        prev_faults_snapshot,
        elapsed_ms,
        recording_started,
        process_cache,
    } = input;

    let target_pids = config.target_pids.clone();
    let cgroup_path = config.cgroupv2.clone();
    let tree_pids = config.tree_pids.clone();
    let exclude_tree_pids = config.exclude_tree_pids.clone();
    let filters = config.task_filters.clone();
    let keep_missing_pid = config.keep_missing_pid;
    let max_tasks = config.max_tasks;

    let mut process_cache_owned = std::mem::take(process_cache);
    let active_targets_clone = active_targets.clone();

    let (snapshot, returned_cache) = task::spawn_blocking(move || {
        let snap = process_tree::target_snapshot_with_options(
            &target_pids,
            &tree_pids,
            cgroup_path.as_deref(),
            &exclude_tree_pids,
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
            "too many target tasks after tree/cgroup/thread expansion: got {}, but --max-tasks is set to {}; try a narrower --tree-pid/--cgroupv2 target or increase --max-tasks",
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
        &mut prev_faults_map,
        prev_faults_snapshot,
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
                    tree_events.push(TreeEvent::from_task(
                        started,
                        target_event_action(diff.task.from_cgroup, "added"),
                        diff.task,
                    ));
                }

                reactivate_or_reset_stats_inner(
                    stats_by_task,
                    Some(&mut *task_exe_inodes),
                    tid,
                    diff.task,
                    elapsed_ms,
                );

                info!(
                    "{} tid={} process_pid={} ppid={} comm={} class={}",
                    if diff.task.from_cgroup {
                        "cgroup_target_added"
                    } else {
                        "tree_target_added"
                    },
                    tid,
                    diff.task.process_pid,
                    diff.task.process_ppid,
                    diff.task.comm,
                    diff.task.class
                );
            }
            TargetDiffAction::Removed => {
                let tid = diff.task.tid;

                let remove_label = if diff.task.from_cgroup {
                    "cgroup_target_removed"
                } else {
                    "tree_target_removed"
                };
                let remove_failed_label = if diff.task.from_cgroup {
                    "cgroup_target_remove_failed"
                } else {
                    "tree_target_remove_failed"
                };
                match target_pid_map.remove(&tid) {
                    Ok(()) => info!("{remove_label} tid={tid} class={}", diff.task.class),
                    Err(e) => warn!("{remove_failed_label} tid={tid} err={e}"),
                }

                if let Some(started) = recording_started {
                    tree_events.push(TreeEvent::from_task(
                        started,
                        target_event_action(diff.task.from_cgroup, "removed"),
                        diff.task,
                    ));
                }

                remove_prev_faults_state(tid, &mut prev_faults_map, prev_faults_snapshot);

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
        "target_refresh_complete active_tasks={} manual_roots={} tree_roots={} cgroupv2={} process_count={}",
        active_targets.len(),
        config.target_pids.len(),
        config.tree_pids.len(),
        config
            .cgroupv2
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_owned()),
        snapshot.process_roots.len(),
    );

    Ok(())
}

fn target_event_action(from_cgroup: bool, action: &'static str) -> &'static str {
    if from_cgroup {
        match action {
            "added" => "cgroup_added",
            "removed" => "cgroup_removed",
            "replaced" => "cgroup_replaced",
            _ => action,
        }
    } else {
        action
    }
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
    prev_faults_map: &mut Option<&mut AyaHashMap<MapData, u32, [u64; 2]>>,
    prev_faults_snapshot: &mut BTreeMap<u32, (u64, u64)>,
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
        remove_prev_faults_state(*tid, prev_faults_map, prev_faults_snapshot);
        reset_stats_for_task_change(
            stats_by_task,
            task_exe_inodes,
            *tid,
            desired_task,
            elapsed_ms,
        );

        if let Some(started) = recording_started {
            tree_events.push(TreeEvent::from_task(
                started,
                target_event_action(desired_task.from_cgroup, "replaced"),
                desired_task,
            ));
        }

        warn!(
            "{} tid={} old_process_pid={} old_comm={} old_class={} new_process_pid={} new_comm={} new_class={}",
            if desired_task.from_cgroup {
                "cgroup_target_replaced"
            } else {
                "tree_target_replaced"
            },
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

fn remove_prev_faults_state(
    tid: u32,
    prev_faults_map: &mut Option<&mut AyaHashMap<MapData, u32, [u64; 2]>>,
    prev_faults_snapshot: &mut BTreeMap<u32, (u64, u64)>,
) {
    prev_faults_snapshot.remove(&tid);

    if let Some(map) = prev_faults_map.as_mut()
        && let Err(err) = map.remove(&tid)
    {
        debug!("prev_faults_remove_failed tid={tid} err={err}");
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
        "ebpf_drop_counters cumulative_total={} wakeup_data_insert_failed={} ringbuf_reserve_failed={} irq_start_times_insert_failed={} block_start_insert_failed={}",
        drop_counters.total(),
        drop_counters.wakeup_data_insert_failed,
        drop_counters.ringbuf_reserve_failed,
        drop_counters.irq_start_times_insert_failed,
        drop_counters.block_start_insert_failed,
    );
}
