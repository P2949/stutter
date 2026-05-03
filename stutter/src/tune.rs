use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use log::{debug, info, warn};
use serde::Serialize;
use tokio::time::sleep;

use crate::{
    affinity,
    cli::{self, Config},
    hwmon,
    process_tree::{TaskClass, TaskFilters},
    profiles,
    recorder::{self, IntervalRecord},
    scorer,
    session::run_monitor,
};

pub const TUNE_RUN_STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
pub const TUNE_PROFILE_REFRESH_MS: u64 = 1_000;

#[derive(Serialize)]
pub struct TuneSummary {
    pub schema_version: u32,
    pub tree_pid: u32,
    pub profiles_path: PathBuf,
    pub runs: u32,
    pub epoch_seconds: u64,
    pub warmup_seconds: u64,
    pub restore_policy: String,
    pub best_profile: String,
    pub candidates: Vec<TuneCandidateSummary>,
}

#[derive(Clone, Serialize)]
pub struct TuneCandidateSummary {
    pub profile: String,
    pub iteration: u32,
    pub run_dir: PathBuf,
    pub applied_tasks: usize,
    pub warmup_seconds: u64,
    pub measure_seconds: u64,
    pub interval_count: usize,
    pub samples: u64,
    pub scored_samples: u64,
    pub score_total: u64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
    pub max_latency_ns: u64,
    pub frame_count: usize,
    pub frame_max_ms: f64,
    pub frame_p99_ms: f64,
    pub frame_over_16ms: u64,
    pub frame_over_33ms: u64,
    pub frame_over_50ms: u64,
    pub coverage: TuneCoverageMetrics,
    pub valid: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct TaskIdentity {
    pub class: TaskClass,
    pub process_comm: String,
    pub comm: String,
    pub process_starttime_ticks: Option<u64>,
    pub task_starttime_ticks: Option<u64>,
    pub exe_dev: Option<u64>,
    pub exe_ino: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TuneCoverageMetrics {
    pub unique_tracked_tasks: usize,
    pub unique_scored_tasks: usize,
    pub active_target_min: usize,
    pub active_target_max: usize,
    pub removed_task_count: usize,
    pub drop_counter_total: u64,
    pub scored_identity_counts: BTreeMap<TaskIdentity, usize>,
}

pub struct TuneCommandInput {
    pub tree_pid: u32,
    pub profiles_path: PathBuf,
    pub epoch_seconds: u64,
    pub warmup_seconds: u64,
    pub runs: u32,
    pub keep_best: bool,
    pub mangohud_log: Option<PathBuf>,
    pub enforce: bool,
}

pub struct TuneControl {
    pub stop_refresh: Arc<AtomicBool>,
    pub applied_tasks: Arc<AtomicUsize>,
}

pub struct TuneMeasureResult {
    pub applied_tasks: usize,
    pub run_dir: PathBuf,
    pub interval_records: Vec<IntervalRecord>,
    pub frame_events: Vec<crate::recorder::FrameEvent>,
    pub coverage: TuneCoverageMetrics,
}


pub async fn tune_command(input: TuneCommandInput) -> anyhow::Result<()> {
    let TuneCommandInput {
        tree_pid,
        profiles_path,
        epoch_seconds,
        warmup_seconds,
        runs,
        keep_best,
        mangohud_log,
        enforce,
    } = input;
    let profiles = profiles::load_profiles(&profiles_path)?;
    if profiles.is_empty() {
        anyhow::bail!(
            "profile file {} did not contain [[profile]]",
            profiles_path.display()
        );
    }

    if runs < 3 {
        warn!(
            "tune_low_run_count_warning: ranking is count-based and workload-sensitive; --runs {} may be too low for reliable results. --runs 3 or higher is recommended for stable ranking.",
            runs
        );
    } else {
        info!(
            "tune_ranking_info: ranking is count-based and workload-sensitive; assumes comparable route/scene/load across epochs."
        );
    }

    let measure_seconds = epoch_seconds.saturating_sub(warmup_seconds);
    let mut results = Vec::new();
    let restore_policy = if keep_best {
        "restore-after-each-then-keep-best"
    } else {
        "restore-after-each"
    };
    let tune_output_dir = default_tune_output_dir()?;

    let shared_hwmon = hwmon::HwmonReader::discover_with_options(None, None, None)
        .map(|r| std::sync::Arc::new(std::sync::Mutex::new(r)));

    for iteration in 1..=runs {
        if runs > 1 {
            println!("tune iteration={} status=Starting", iteration);
        }

        for profile in profiles.iter() {
            println!(
                "tune iteration={} candidate={} state=CandidateWarmup warmup_seconds={}",
                iteration, profile.name, warmup_seconds
            );

            println!(
                "tune iteration={} candidate={} state=CandidateMeasure measure_seconds={}",
                iteration, profile.name, measure_seconds
            );

            let TuneMeasureResult {
                applied_tasks,
                run_dir,
                interval_records,
                frame_events,
                coverage,
            } = match measure_tune_candidate(
                Config {
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
                    mangohud_log: mangohud_log.clone(),
                    tui: false,
                    retain_intervals: None,
                    recording: Some(cli::RecordingConfig {
                        run_name: Some(format!("tune-{}", profile.name)),
                        out_dir: Some(tune_run_dir(&tune_output_dir, &profile.name)),
                    }),
                    max_duration: Some(Duration::from_secs(epoch_seconds)),
                    cgroupv2: None,
                    follow_exec: true,
                    exclude_tree_pids: Vec::new(),
                    cpu_freq: true,
                    mangohud_ignore_offset: {
                        if let Some(path) = &mangohud_log {
                            std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
                        } else {
                            0
                        }
                    },
                    faults: false,
                    block_io: false,
                    stat_wait: false,
                },
                profile.clone(),
                enforce,
                shared_hwmon.clone(),
                results.is_empty(),
                tune_output_dir.clone(),
            )
            .await
            {
                Ok(res) => res,
                Err(err) => {
                    restore_tune_on_error();
                    return Err(err);
                }
            };

            let score =
                scorer::score_from_interval_records_and_frames(&interval_records, &frame_events);

            const TUNE_MIN_INTERVALS: usize = 2;
            const TUNE_MIN_SAMPLES: u64 = 50;
            let interval_count = interval_records.len();
            let sample_count: u64 = interval_records.iter().map(|r| r.samples).sum();
            let (scored_interval_count, scored_sample_count) =
                tune_scored_record_counts(&interval_records);
            let mut valid = scored_interval_count >= TUNE_MIN_INTERVALS
                && scored_sample_count >= TUNE_MIN_SAMPLES;
            if !valid {
                warn!(
                    "tune_candidate_insufficient_scored_data iteration={} profile={} scored_intervals={} scored_samples={} total_intervals={} total_samples={}",
                    iteration,
                    profile.name,
                    scored_interval_count,
                    scored_sample_count,
                    interval_count,
                    sample_count
                );
                valid = false;
            }

            if coverage.unique_scored_tasks == 0 {
                warn!(
                    "tune_candidate_no_scored_tasks iteration={} profile={} tracked_tasks={}",
                    iteration, profile.name, coverage.unique_tracked_tasks
                );
                valid = false;
            }
            if coverage.drop_counter_total > 0 {
                warn!(
                    "tune_candidate_drop_counters_nonzero iteration={} profile={} drops={}",
                    iteration, profile.name, coverage.drop_counter_total
                );
            }

            let result = TuneCandidateSummary {
                profile: profile.name.clone(),
                iteration,
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
                frame_max_ms: score.frame_max_ms,
                frame_p99_ms: score.frame_p99_ms,
                frame_over_16ms: score.frame_over_16ms,
                frame_over_33ms: score.frame_over_33ms,
                frame_over_50ms: score.frame_over_50ms,
                coverage,
                valid,
            };

            results.push(result);

            restore_tune_after_candidate(&profile.name)?;
        }
    }

    let mut grouped: BTreeMap<String, Vec<TuneCandidateSummary>> = BTreeMap::new();
    for r in &results {
        grouped
            .entry(r.profile.clone())
            .or_default()
            .push(r.clone());
    }

    let best_profile = grouped
        .iter()
        .filter(|(_, runs)| runs.iter().any(|r| r.valid))
        .min_by_key(|(_, runs)| aggregate_profile_rank(runs))
        .map(|(name, _)| name.clone())
        .unwrap_or_default();

    let any_valid = results.iter().any(|r| r.valid);
    if !any_valid {
        restore_tune_on_error();
        anyhow::bail!("no tune candidate collected enough data; no best profile selected");
    }

    let profile_medians: Vec<_> = grouped
        .values()
        .map(|runs| median_u64(runs.iter().map(|r| r.scored_samples).collect()))
        .collect();
    let min_samples = profile_medians.iter().copied().min().unwrap_or(0);
    let max_samples = profile_medians.iter().copied().max().unwrap_or(0);

    if min_samples == 0 && max_samples > 0 {
        anyhow::bail!(
            "tune candidates are not comparable: some profiles gathered no scored samples while others did (max_median_scored_samples={})",
            max_samples
        );
    } else if min_samples > 0 {
        let ratio = (max_samples as f64) / (min_samples as f64);
        if ratio > 2.0 {
            anyhow::bail!(
                "tune candidates are not comparable: median scored sample count varies by more than 2x across profiles (min={} max={} ratio={:.2})",
                min_samples,
                max_samples,
                ratio
            );
        }
    }

    check_tune_coverage_comparability(&grouped)?;

    let profile_frame_medians: Vec<_> = grouped
        .values()
        .map(|runs| median_u64(runs.iter().map(|r| r.frame_count as u64).collect()))
        .collect();
    let min_frames = profile_frame_medians.iter().copied().min().unwrap_or(0);
    let max_frames = profile_frame_medians.iter().copied().max().unwrap_or(0);

    if max_frames > 0 && min_frames == 0 {
        anyhow::bail!(
            "tune candidates are not comparable: some profiles produced MangoHud frame events and some produced none"
        );
    } else if min_frames > 0 {
        let ratio = (max_frames as f64) / (min_frames as f64);
        if ratio > 1.5 {
            warn!(
                "tune_candidates_unbalanced_frames min_median={} max_median={} ratio={:.2}",
                min_frames, max_frames, ratio
            );
        }
    }

    let summary = TuneSummary {
        schema_version: 1,
        tree_pid,
        profiles_path,
        runs,
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
        crate::watch::apply_profile_to_tree_blocking(
            tree_pid,
            profile.clone(),
            false,
            false,
            enforce,
        )
        .await?;
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

    warn!(
        "tune_ranking_is_workload_sensitive: decisions are not final truth and depend on comparable workload; repeated runs showing low variance are recommended."
    );

    Ok(())
}

pub async fn measure_tune_candidate(
    config: Config,
    profile: profiles::Profile,
    enforce: bool,
    shared_hwmon: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
    force_restore_overwrite: bool,
    _tune_output_dir: PathBuf,
) -> anyhow::Result<TuneMeasureResult> {
    let tree_pid = config.tree_pids[0];
    let run_dir = config.recording.as_ref().unwrap().out_dir.as_ref().unwrap().clone();

    let cache = profiles::ProfileApplyCache::default();
    let (initial_records, cache) = crate::watch::apply_profile_to_tree_cached_blocking(
        tree_pid,
        profile.clone(),
        force_restore_overwrite,
        false,
        cache,
    )
    .await?;
    let initial_applied_tasks = initial_records.len();
    let should_force_refresh = force_restore_overwrite && initial_records.is_empty();

    let control = TuneControl {
        stop_refresh: Arc::new(AtomicBool::new(false)),
        applied_tasks: Arc::new(AtomicUsize::new(0)),
    };
    let stop_refresh = control.stop_refresh.clone();
    let refreshed_applied_tasks = control.applied_tasks.clone();

    let mut profile_refresh = tokio::spawn(tune_profile_refresh_loop(
        tree_pid,
        profile,
        cache,
        should_force_refresh,
        TUNE_PROFILE_REFRESH_MS,
        control,
        enforce,
    ));

    let mut profile_refresh_finished = false;
    let monitor_result = tokio::select! {
        result = run_monitor(config.clone(), shared_hwmon) => result,
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
    let interval_data = fs::read_to_string(&interval_path)
        .with_context(|| format!("failed to read interval.json from {}", run_dir.display()))?;
    let interval_records: Vec<IntervalRecord> = serde_json::Deserializer::from_str(&interval_data)
        .into_iter::<IntervalRecord>()
        .collect::<Result<Vec<_>, _>>()?;
    let mut interval_records = interval_records;
    let warmup_ms = config.max_duration.map(|d| d.as_millis()).unwrap_or(0); // This is actually epoch_seconds in the original, we should use config instead
    interval_records.retain(|r| r.elapsed_ms >= warmup_ms);

    let frame_path = run_dir.join("frame_correlation.json");
    let mut frame_events: Vec<crate::recorder::FrameEvent> = if frame_path.exists() {
        let frame_data = fs::read_to_string(&frame_path)?;
        serde_json::Deserializer::from_str(&frame_data)
            .into_iter::<crate::recorder::FrameEvent>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_default()
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

pub async fn tune_profile_refresh_loop(
    tree_pid: u32,
    profile: profiles::Profile,
    mut cache: profiles::ProfileApplyCache,
    force_restore_overwrite: bool,
    refresh_ms: u64,
    control: TuneControl,
    enforce: bool,
) -> anyhow::Result<()> {
    let mut should_force = force_restore_overwrite;
    let refresh_interval = Duration::from_millis(refresh_ms);
    let verify_interval = Duration::from_millis(crate::watch::PROFILE_WATCH_VERIFY_MS);
    let mut next_verify = Instant::now() + verify_interval;

    loop {
        if control.stop_refresh.load(Ordering::Relaxed) {
            return Ok(());
        }

        if enforce || Instant::now() >= next_verify {
            cache.clear();
            next_verify = Instant::now() + verify_interval;
            debug!("tune_profile_refresh_cache_invalidated_for_full_verify enforce={enforce}");
        }

        let (records, updated_cache) = crate::watch::apply_profile_to_tree_cached_blocking(
            tree_pid,
            profile.clone(),
            should_force,
            false,
            cache,
        )
        .await?;
        cache = updated_cache;

        if !records.is_empty() {
            control.applied_tasks.fetch_add(records.len(), Ordering::Relaxed);
        }

        should_force = false;
        sleep(refresh_interval).await;
    }
}

pub fn tune_run_dir(tune_output_dir: &Path, profile_name: &str) -> PathBuf {
    tune_output_dir.join(profile_name)
}

pub fn cleanup_stale_tune_run_dirs(state_dir: &Path) -> anyhow::Result<()> {
    if !state_dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(state_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("tune-") {
            continue;
        }

        let timestamp_str = &name[5..];
        let Ok(nanos) = timestamp_str.parse::<u128>() else {
            continue;
        };

        let created_at = UNIX_EPOCH + Duration::from_nanos(nanos as u64);
        if let Ok(elapsed) = SystemTime::now().duration_since(created_at)
            && elapsed > TUNE_RUN_STALE_AFTER
        {
            info!("tune_cleanup_stale_dir path={}", path.display());
            fs::remove_dir_all(&path)?;
        }
    }

    Ok(())
}

pub fn aggregate_profile_rank(runs: &[TuneCandidateSummary]) -> impl Ord {
    let invalid_run_count = runs.iter().filter(|r| !r.valid).count();
    let valid_runs: Vec<&TuneCandidateSummary> = runs.iter().filter(|r| r.valid).collect();

    let score_totals: Vec<u64> = valid_runs.iter().map(|r| r.score_total).collect();
    let over_5ms: Vec<u64> = valid_runs.iter().map(|r| r.over_5ms).collect();
    let over_2ms: Vec<u64> = valid_runs.iter().map(|r| r.over_2ms).collect();
    let over_1ms: Vec<u64> = valid_runs.iter().map(|r| r.over_1ms).collect();
    let frame_over_50ms: Vec<u64> = valid_runs.iter().map(|r| r.frame_over_50ms).collect();
    let frame_over_33ms: Vec<u64> = valid_runs.iter().map(|r| r.frame_over_33ms).collect();
    let frame_over_16ms: Vec<u64> = valid_runs.iter().map(|r| r.frame_over_16ms).collect();
    let frame_p99s: Vec<u64> = valid_runs
        .iter()
        .map(|r| (r.frame_p99_ms * 1000.0) as u64)
        .collect();
    let frame_maxes: Vec<u64> = valid_runs
        .iter()
        .map(|r| (r.frame_max_ms * 1000.0) as u64)
        .collect();
    let max_latencies: Vec<u64> = valid_runs.iter().map(|r| r.max_latency_ns).collect();

    (
        invalid_run_count,
        median_u64(score_totals.clone()),
        median_u64(over_5ms),
        median_u64(over_2ms),
        median_u64(over_1ms),
        median_u64(frame_over_50ms),
        median_u64(frame_over_33ms),
        median_u64(frame_over_16ms),
        worst_u64(score_totals),
        worst_u64(frame_p99s),
        worst_u64(frame_maxes),
        worst_u64(max_latencies),
    )
}

pub fn median_u64(mut values: Vec<u64>) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    values[values.len() / 2]
}

pub fn worst_u64(values: Vec<u64>) -> u64 {
    values.into_iter().max().unwrap_or(0)
}

pub fn tune_scored_record_counts(records: &[IntervalRecord]) -> (usize, u64) {
    let mut elapsed = BTreeSet::new();
    let mut samples = 0u64;

    for record in records
        .iter()
        .filter(|record| scorer::class_contributes_to_score(record.class))
    {
        elapsed.insert(record.elapsed_ms);
        samples = samples.saturating_add(record.samples);
    }

    (elapsed.len(), samples)
}

pub fn tune_coverage_metrics(
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
            session.active_target_pids_count as usize,
            session.active_target_pids_count as usize,
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
        drop_counter_total: if session.block_io_correlation_basis == "request-pointer" {
            session.drop_counters.total()
        } else {
            session.drop_counters.total_excluding_block_io()
        },
        scored_identity_counts,
    }
}

pub fn check_tune_coverage_comparability(
    grouped: &BTreeMap<String, Vec<TuneCandidateSummary>>,
) -> anyhow::Result<()> {
    check_tune_metric_ratio(
        "unique tracked tasks",
        grouped.values().map(|runs| {
            median_u64(
                runs.iter()
                    .map(|result| result.coverage.unique_tracked_tasks as u64)
                    .collect(),
            ) as usize
        }),
    )?;
    check_tune_metric_ratio(
        "unique scored tasks",
        grouped.values().map(|runs| {
            median_u64(
                runs.iter()
                    .map(|result| result.coverage.unique_scored_tasks as u64)
                    .collect(),
            ) as usize
        }),
    )?;

    let representatives: Vec<&TuneCandidateSummary> =
        grouped.values().filter_map(|runs| runs.first()).collect();

    if let Some(first) = representatives.first() {
        for other in representatives.iter().skip(1) {
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
        grouped.values().map(|runs| {
            median_u64(
                runs.iter()
                    .map(|result| result.coverage.active_target_min as u64)
                    .collect(),
            ) as usize
        }),
    )?;
    check_tune_metric_ratio(
        "active target maximum",
        grouped.values().map(|runs| {
            median_u64(
                runs.iter()
                    .map(|result| result.coverage.active_target_max as u64)
                    .collect(),
            ) as usize
        }),
    )?;

    let profile_removed_medians: Vec<_> = grouped
        .values()
        .map(|runs| {
            median_u64(
                runs.iter()
                    .map(|result| result.coverage.removed_task_count as u64)
                    .collect(),
            )
        })
        .collect();

    let min_removed = profile_removed_medians.iter().copied().min().unwrap_or(0);
    let max_removed = profile_removed_medians.iter().copied().max().unwrap_or(0);
    if max_removed > min_removed {
        warn!(
            "tune_candidates_removed_task_counts_differ min_median={} max_median={}",
            min_removed, max_removed
        );
    }

    let max_drops = grouped
        .values()
        .flat_map(|runs| runs.iter().map(|r| r.coverage.drop_counter_total))
        .max()
        .unwrap_or(0);
    if max_drops > 0 {
        warn!("tune_candidates_drop_counters_nonzero max_drops={max_drops}");
    }

    Ok(())
}

pub fn scored_identity_overlap(
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

pub fn check_tune_metric_ratio(
    label: &str,
    values: impl Iterator<Item = usize>,
) -> anyhow::Result<()> {
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

pub fn restore_tune_on_error() {
    let path = affinity::default_restore_path();
    if path.exists()
        && let Err(err) = affinity::restore_saved(&path)
    {
        warn!("tune_restore_after_error_failed err={err:#}");
    }
}

pub fn restore_tune_after_candidate(profile_name: &str) -> anyhow::Result<()> {
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

pub fn default_tune_output_dir() -> anyhow::Result<PathBuf> {
    let mut path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    path.push(".local");
    path.push("state");
    path.push("stutter");
    cleanup_stale_tune_run_dirs(&path)?;
    path.push(format!("tune-{}", unix_nanos_now()));
    Ok(path)
}

pub fn unix_nanos_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
