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

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::{
    artifacts::ArtifactSelection,
    cli::{self, Config},
    hwmon,
    process_tree::TaskFilters,
    profiles,
    recorder::IntervalRecord,
    scorer,
    session::run_monitor,
    session_io,
};

pub mod comparability;
pub mod recommendation;

pub use comparability::TuneCoverageMetrics;

pub const TUNE_RUN_STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
pub const TUNE_PROFILE_REFRESH_MS: u64 = 1_000;

#[derive(Serialize, Deserialize)]
pub struct TuneSummary {
    pub schema_version: u32,
    pub tree_pid: u32,
    pub profiles_path: PathBuf,
    pub runs: u32,
    pub epoch_seconds: u64,
    pub warmup_seconds: u64,
    pub restore_policy: String,
    pub best_profile: String,
    pub candidate_order: Vec<TuneIterationOrder>,
    pub profile_stats: Vec<TuneProfileStats>,
    pub ranking_confidence: RankingConfidence,
    pub ranking_notes: Vec<String>,
    #[serde(default)]
    pub comparability_warnings: Vec<comparability::TuneComparabilityWarning>,
    pub candidates: Vec<TuneCandidateSummary>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TuneIterationOrder {
    pub iteration: u32,
    pub profiles: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TuneProfileStats {
    pub profile: String,
    pub valid_runs: usize,
    pub invalid_runs: usize,
    pub median_score_total: u64,
    pub iqr_score_total: u64,
    pub worst_score_total: u64,
    pub median_over_5ms: u64,
    pub iqr_over_5ms: u64,
    pub median_frame_p99_us: u64,
    pub iqr_frame_p99_us: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RankingConfidence {
    High,
    Medium,
    Low,
    Unstable,
}

#[derive(Clone, Serialize, Deserialize)]
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

pub struct TuneCommandInput {
    pub tree_pid: u32,
    pub profiles_path: PathBuf,
    pub epoch_seconds: u64,
    pub warmup_seconds: u64,
    pub runs: u32,
    pub keep_best: bool,
    pub baseline_profile: Option<String>,
    pub out_dir: Option<PathBuf>,
    pub mangohud_log: Option<PathBuf>,
    pub enforce: bool,
    pub hwmon: bool,
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
        baseline_profile,
        out_dir,
        mangohud_log,
        enforce,
        hwmon,
    } = input;

    let profiles = profiles::load_profiles(&profiles_path)?;
    if profiles.is_empty() {
        anyhow::bail!(
            "profile file {} did not contain [[profile]]",
            profiles_path.display()
        );
    }
    if let Some(baseline_profile) = &baseline_profile
        && !profiles
            .iter()
            .any(|profile| profile.name == *baseline_profile)
    {
        anyhow::bail!("--baseline-profile {baseline_profile} was not found in profiles file");
    }
    let tune_output_dir = match out_dir {
        Some(path) => {
            ensure_tune_output_dir_available(&path)?;
            path
        }
        None => default_tune_output_dir()?,
    };

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

    let candidate_order = tune_candidate_order(&profiles, runs);
    let results = collect_tune_results(
        &profiles,
        tree_pid,
        epoch_seconds,
        warmup_seconds,
        runs,
        mangohud_log,
        enforce,
        hwmon,
        &tune_output_dir,
    )
    .await?;

    let mut grouped: BTreeMap<String, Vec<TuneCandidateSummary>> = BTreeMap::new();
    for r in &results {
        grouped
            .entry(r.profile.clone())
            .or_default()
            .push(r.clone());
    }

    let any_valid = results.iter().any(|r| r.valid);
    if any_valid {
        comparability::check_tune_coverage_comparability(&grouped)?;
    }
    let comparability_warnings = comparability::tune_comparability_warnings(&grouped);

    let profile_stats = profile_stats_from_grouped(&grouped);
    let selected_best_profile = select_best_profile(&grouped);
    let (ranking_confidence, ranking_notes) =
        assess_ranking_confidence(&profile_stats, &grouped, &selected_best_profile, runs);
    let best_profile = if ranking_confidence == RankingConfidence::Unstable {
        String::new()
    } else {
        selected_best_profile
    };
    let keep_best = keep_best && ranking_confidence != RankingConfidence::Unstable;

    let restore_policy = if keep_best {
        "restore-after-each-then-keep-best"
    } else {
        "restore-after-each"
    };

    let summary = TuneSummary {
        schema_version: 1,
        tree_pid,
        profiles_path,
        runs,
        epoch_seconds,
        warmup_seconds,
        restore_policy: restore_policy.to_owned(),
        best_profile,
        candidate_order,
        profile_stats,
        ranking_confidence,
        ranking_notes,
        comparability_warnings,
        candidates: results,
    };

    write_tune_summary(
        &summary,
        &tune_output_dir,
        keep_best,
        enforce,
        baseline_profile.as_deref(),
    )
    .await?;

    if summary.ranking_confidence == RankingConfidence::Unstable {
        restore_tune_on_error();
        anyhow::bail!(
            "tune ranking unstable; no best profile selected; inspect tuning_summary.json"
        );
    }

    Ok(())
}

pub fn candidate_order_for_iteration(profile_count: usize, iteration: u32) -> Vec<usize> {
    let mut order: Vec<usize> = (0..profile_count).collect();

    if profile_count <= 1 {
        return order;
    }

    let rotation = ((iteration - 1) as usize) % profile_count;
    order.rotate_left(rotation);

    if iteration.is_multiple_of(2) {
        order.reverse();
    }

    order
}

fn tune_candidate_order(profiles: &[profiles::Profile], runs: u32) -> Vec<TuneIterationOrder> {
    (1..=runs)
        .map(|iteration| TuneIterationOrder {
            iteration,
            profiles: candidate_order_for_iteration(profiles.len(), iteration)
                .into_iter()
                .map(|profile_idx| profiles[profile_idx].name.clone())
                .collect(),
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
async fn collect_tune_results(
    profiles: &[profiles::Profile],
    tree_pid: u32,
    epoch_seconds: u64,
    warmup_seconds: u64,
    runs: u32,
    mangohud_log: Option<PathBuf>,
    enforce: bool,
    hwmon: bool,
    tune_output_dir: &Path,
) -> anyhow::Result<Vec<TuneCandidateSummary>> {
    let measure_seconds = epoch_seconds.saturating_sub(warmup_seconds);
    let mut results = Vec::new();
    let shared_hwmon = if hwmon {
        hwmon::HwmonReader::discover_with_options(None, None, None)
            .map(|r| std::sync::Arc::new(std::sync::Mutex::new(r)))
    } else {
        None
    };

    for iteration in 1..=runs {
        if runs > 1 {
            println!("tune iteration={} status=Starting", iteration);
        }

        let order = candidate_order_for_iteration(profiles.len(), iteration);
        for profile_idx in order {
            let profile = &profiles[profile_idx];
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
                Arc::new(Config {
                    monitor_config_layer: None,
                    preset: None,
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
                    csv_stream: None,
                    irq_latency: false,
                    irqs: Vec::new(),
                    hwmon,
                    hwmon_root: None,
                    hwmon_drm_card: None,
                    hwmon_render_node: None,
                    mangohud_log: mangohud_log.clone(),
                    tui: false,
                    retain_intervals: None,
                    recording: Some(cli::RecordingConfig {
                        run_name: Some(format!("tune-{}", profile.name)),
                        out_dir: Some(tune_run_dir(tune_output_dir, &profile.name, iteration)),
                    }),
                    max_duration: Some(Duration::from_secs(epoch_seconds)),
                    cgroupv2: None,
                    native_cgroup_filter: false,
                    follow_exec: true,
                    exclude_tree_pids: Vec::new(),
                    cpu_freq: true,
                    faults: false,
                    cpu_perf: false,
                    cpu_perf_kernel: false,
                    cpu_perf_max_tasks: 128,
                    cpu_perf_cache_refs: false,
                    block_io: false,
                    stat_wait: false,
                    runtime_slices: false,
                    runtime_slices_max_tasks: 256,
                    json_stream: false,
                    mangohud_log_live: false,
                    metrics_port: None,
                    ringbuf_size_kb: None,
                    wakeup_map_factor: None,
                    otlp_endpoint: None,
                    otel_service_name: "stutter".to_owned(),
                    auto_focus: false,
                    focus_source: cli::FocusSource::Heuristic,
                    foreground_window: false,
                    foreground_source: cli::ForegroundSourceArg::Auto,
                    foreground_poll_ms: 1000,
                    foreground_max_stale_ms: 2500,
                    foreground_include_title: false,
                    auto_focus_poll_ms: 1000,
                    auto_focus_min_confidence: 0.60,
                    auto_focus_switch_cooldown_ms: 5000,
                    auto_focus_switch_margin: 0.20,
                    auto_focus_required_polls: 2,
                    auto_focus_max_roots: 4,
                    remote: None,
                }),
                profile.clone(),
                enforce,
                shared_hwmon.clone(),
                results.is_empty(),
                tune_output_dir.to_owned(),
                warmup_seconds,
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

    Ok(results)
}

fn select_best_profile(grouped: &BTreeMap<String, Vec<TuneCandidateSummary>>) -> String {
    grouped
        .iter()
        .filter(|(_, runs)| runs.iter().any(|r| r.valid))
        .min_by_key(|(_, runs)| aggregate_profile_rank(runs))
        .map(|(name, _)| name.clone())
        .unwrap_or_default()
}

async fn write_tune_summary(
    summary: &TuneSummary,
    tune_output_dir: &Path,
    keep_best: bool,
    enforce: bool,
    baseline_profile: Option<&str>,
) -> anyhow::Result<()> {
    if keep_best && !summary.best_profile.is_empty() {
        let profiles = profiles::load_profiles(&summary.profiles_path)?;
        if let Some(profile) = profiles
            .iter()
            .find(|profile| profile.name == summary.best_profile)
        {
            let records = match crate::watch::apply_profile_to_tree_blocking(
                summary.tree_pid,
                profile.clone(),
                false,
                false,
                enforce,
            )
            .await
            {
                Ok(records) => records,
                Err(err) => {
                    crate::audit::audit_or_warn(&crate::audit::AuditEvent {
                        schema_version: 1,
                        unix_nanos: crate::audit::unix_nanos_now(),
                        command: "tune --keep-best".to_owned(),
                        action_id: Some(format!("cpu-affinity-profile:{}", profile.name)),
                        safety_class: Some(if profiles::profile_uses_priority_actions(profile) {
                            crate::actions::SafetyClass::ReversibleMediumRisk
                        } else {
                            crate::actions::SafetyClass::ReversibleLowRisk
                        }),
                        dry_run: false,
                        success: false,
                        affected_tasks: 0,
                        restore_path: Some(crate::profile_restore::default_restore_path()),
                        message: format!(
                            "failed to apply best tune profile '{}': {err:#}",
                            profile.name
                        ),
                    });
                    return Err(err);
                }
            };
            crate::audit::audit_or_warn(&crate::audit::AuditEvent {
                schema_version: 1,
                unix_nanos: crate::audit::unix_nanos_now(),
                command: "tune --keep-best".to_owned(),
                action_id: Some(format!("cpu-affinity-profile:{}", profile.name)),
                safety_class: Some(if profiles::profile_uses_priority_actions(profile) {
                    crate::actions::SafetyClass::ReversibleMediumRisk
                } else {
                    crate::actions::SafetyClass::ReversibleLowRisk
                }),
                dry_run: false,
                success: true,
                affected_tasks: records.len(),
                restore_path: Some(crate::profile_restore::default_restore_path()),
                message: "kept best tune profile applied".to_owned(),
            });
        }
    }

    let summary_path = tune_output_dir.join("tuning_summary.json");
    if let Some(parent) = summary_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(&summary_path, serde_json::to_vec_pretty(&summary)?)?;
    let recommendation = recommendation::build_tune_recommendation(summary, baseline_profile);
    let recommendation_json_path = tune_output_dir.join("tuning_recommendation.json");
    fs::write(
        &recommendation_json_path,
        serde_json::to_vec_pretty(&recommendation)?,
    )?;
    let recommendation_markdown_path = tune_output_dir.join("tuning_recommendation.md");
    fs::write(
        &recommendation_markdown_path,
        recommendation::render_tune_recommendation_markdown(&recommendation),
    )?;

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
    println!("recommendation={}", recommendation_markdown_path.display());

    warn!(
        "tune_ranking_is_workload_sensitive: decisions are not final truth and depend on comparable workload; repeated runs showing low variance are recommended."
    );

    Ok(())
}

pub async fn measure_tune_candidate(
    config: Arc<Config>,
    profile: profiles::Profile,
    enforce: bool,
    shared_hwmon: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
    force_restore_overwrite: bool,
    _tune_output_dir: PathBuf,
    warmup_seconds: u64,
) -> anyhow::Result<TuneMeasureResult> {
    let tree_pid = config.tree_pids[0];
    let run_dir = config
        .recording
        .as_ref()
        .unwrap()
        .out_dir
        .as_ref()
        .unwrap()
        .clone();

    let cache = profiles::ProfileApplyCache::default();
    let (initial_apply, cache) = match crate::watch::apply_profile_to_tree_cached_blocking(
        tree_pid,
        profile.clone(),
        force_restore_overwrite,
        false,
        cache,
    )
    .await
    {
        Ok(res) => res,
        Err(err) => {
            crate::audit::audit_or_warn(&crate::audit::AuditEvent {
                schema_version: 1,
                unix_nanos: crate::audit::unix_nanos_now(),
                command: "tune candidate".to_owned(),
                action_id: Some(format!("cpu-affinity-profile:{}", profile.name)),
                safety_class: Some(crate::actions::SafetyClass::ReversibleLowRisk),
                dry_run: false,
                success: false,
                affected_tasks: 0,
                restore_path: Some(crate::profile_restore::default_restore_path()),
                message: format!(
                    "failed to apply tune candidate profile '{}': {err:#}",
                    profile.name
                ),
            });
            return Err(err);
        }
    };
    let initial_applied_tasks = initial_apply.affected_tasks();
    crate::audit::audit_or_warn(&crate::audit::AuditEvent {
        schema_version: 1,
        unix_nanos: crate::audit::unix_nanos_now(),
        command: "tune candidate".to_owned(),
        action_id: Some(format!("cpu-affinity-profile:{}", profile.name)),
        safety_class: Some(if profiles::profile_uses_priority_actions(&profile) {
            crate::actions::SafetyClass::ReversibleMediumRisk
        } else {
            crate::actions::SafetyClass::ReversibleLowRisk
        }),
        dry_run: false,
        success: true,
        affected_tasks: initial_applied_tasks,
        restore_path: Some(crate::profile_restore::default_restore_path()),
        message: format!("applied tune candidate profile '{}'", profile.name),
    });
    let should_force_refresh = force_restore_overwrite && initial_apply.affected_tasks() == 0;

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
        result = run_monitor(config.clone(), shared_hwmon, None, None) => result,
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

    let artifacts = session_io::load_run_artifacts(&run_dir, ArtifactSelection::tune())?;
    let mut interval_records = artifacts.intervals;
    retain_after_warmup(&mut interval_records, warmup_seconds, |r| r.elapsed_ms);

    let mut frame_events = artifacts.frame_events;
    retain_after_warmup(&mut frame_events, warmup_seconds, |f| f.elapsed_ms);

    let coverage = comparability::tune_coverage_metrics(&artifacts.session, &interval_records);

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

        let (apply_result, updated_cache) = crate::watch::apply_profile_to_tree_cached_blocking(
            tree_pid,
            profile.clone(),
            should_force,
            false,
            cache,
        )
        .await?;
        cache = updated_cache;

        if apply_result.affected_tasks() > 0 {
            control
                .applied_tasks
                .fetch_add(apply_result.affected_tasks(), Ordering::Relaxed);
        }

        should_force = false;
        sleep(refresh_interval).await;
    }
}

pub fn tune_run_dir(tune_output_dir: &Path, profile_name: &str, iteration: u32) -> PathBuf {
    tune_output_dir.join(format!(
        "iter-{iteration:03}-{}",
        sanitize_profile_name(profile_name)
    ))
}

fn sanitize_profile_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
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

pub fn percentile_nearest_rank_u64(values: &mut [u64], percentile: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }

    values.sort_unstable();
    let percentile = percentile.clamp(0.0, 100.0);
    let rank = ((percentile / 100.0) * values.len() as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(values.len() - 1);
    values[idx]
}

pub fn iqr_u64(values: Vec<u64>) -> u64 {
    if values.is_empty() {
        return 0;
    }

    let mut q25_values = values.clone();
    let q25 = percentile_nearest_rank_u64(&mut q25_values, 25.0);
    let mut q75_values = values;
    let q75 = percentile_nearest_rank_u64(&mut q75_values, 75.0);
    q75.saturating_sub(q25)
}

pub fn profile_stats_from_grouped(
    grouped: &BTreeMap<String, Vec<TuneCandidateSummary>>,
) -> Vec<TuneProfileStats> {
    grouped
        .iter()
        .map(|(profile, runs)| {
            let valid_runs = runs.iter().filter(|run| run.valid).collect::<Vec<_>>();
            let score_totals = valid_runs
                .iter()
                .map(|run| run.score_total)
                .collect::<Vec<_>>();
            let over_5ms = valid_runs
                .iter()
                .map(|run| run.over_5ms)
                .collect::<Vec<_>>();
            let frame_p99_us = valid_runs
                .iter()
                .map(|run| (run.frame_p99_ms * 1000.0) as u64)
                .collect::<Vec<_>>();

            TuneProfileStats {
                profile: profile.clone(),
                valid_runs: valid_runs.len(),
                invalid_runs: runs.len().saturating_sub(valid_runs.len()),
                median_score_total: median_u64(score_totals.clone()),
                iqr_score_total: iqr_u64(score_totals.clone()),
                worst_score_total: worst_u64(score_totals),
                median_over_5ms: median_u64(over_5ms.clone()),
                iqr_over_5ms: iqr_u64(over_5ms),
                median_frame_p99_us: median_u64(frame_p99_us.clone()),
                iqr_frame_p99_us: iqr_u64(frame_p99_us),
            }
        })
        .collect()
}

pub fn assess_ranking_confidence(
    profile_stats: &[TuneProfileStats],
    _grouped: &BTreeMap<String, Vec<TuneCandidateSummary>>,
    best_profile: &str,
    runs: u32,
) -> (RankingConfidence, Vec<String>) {
    let mut notes = Vec::new();
    let mut valid_stats = profile_stats
        .iter()
        .filter(|stat| stat.valid_runs > 0)
        .collect::<Vec<_>>();
    valid_stats.sort_by_key(|stat| stat.median_score_total);

    if valid_stats.len() < 2 {
        notes.push("fewer than two profiles produced valid runs".to_owned());
        return (RankingConfidence::Unstable, notes);
    }

    let Some(best) = valid_stats
        .iter()
        .copied()
        .find(|stat| stat.profile == best_profile)
    else {
        notes.push("best profile did not produce valid runs".to_owned());
        return (RankingConfidence::Unstable, notes);
    };

    if runs >= 3 && best.valid_runs < 2 {
        notes.push("best profile has fewer than two valid runs".to_owned());
        return (RankingConfidence::Unstable, notes);
    }

    let Some(second) = valid_stats
        .iter()
        .copied()
        .find(|stat| stat.profile != best.profile)
    else {
        notes.push("no second valid profile is available for comparison".to_owned());
        return (RankingConfidence::Unstable, notes);
    };

    let diff = second.median_score_total.abs_diff(best.median_score_total);
    let max_iqr = best.iqr_score_total.max(second.iqr_score_total);
    if diff <= max_iqr && max_iqr > 0 {
        notes.push(format!(
            "best and second-best median scores are close relative to variance (diff={diff}, max_iqr={max_iqr})"
        ));
        return (RankingConfidence::Unstable, notes);
    }

    let five_percent_second = second.median_score_total / 20;
    let close_to_second = diff <= five_percent_second;

    if runs < 3 {
        notes.push("--runs is less than 3; ranking confidence is limited".to_owned());
    }
    if best.invalid_runs > 0 {
        notes.push("best profile has invalid runs".to_owned());
    }
    if close_to_second {
        notes.push(format!(
            "best median score is within 5% of second-best (diff={diff})"
        ));
    }
    if best.iqr_score_total > 0 {
        notes.push("best profile score IQR is non-zero".to_owned());
    }

    if runs < 3 || best.invalid_runs > 0 || close_to_second {
        (RankingConfidence::Low, notes)
    } else if best.iqr_score_total > 0 || second.iqr_score_total > 0 || !notes.is_empty() {
        (RankingConfidence::Medium, notes)
    } else {
        (RankingConfidence::High, notes)
    }
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

pub fn restore_tune_on_error() {
    let path = crate::profile_restore::default_restore_path();
    if path.exists()
        && let Err(err) = crate::profile_restore::restore_saved(&path)
    {
        warn!("tune_restore_after_error_failed err={err:#}");
    }
}

pub fn restore_tune_after_candidate(profile_name: &str) -> anyhow::Result<()> {
    let path = crate::profile_restore::default_restore_path();
    if !path.exists() {
        return Ok(());
    }

    let summary = crate::profile_restore::restore_saved(&path)?;
    info!(
        "tune_candidate_restored profile={} affinity={} nice={} ionice={} skipped_dead={} skipped_identity_mismatch={}",
        profile_name,
        summary.affinity,
        summary.nice,
        summary.ionice,
        summary.skipped_dead,
        summary.skipped_identity_mismatch
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

pub fn ensure_tune_output_dir_available(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if !path.is_dir() {
        anyhow::bail!("--out-dir {} exists but is not a directory", path.display());
    }
    if fs::read_dir(path)?.next().is_some() {
        anyhow::bail!(
            "--out-dir {} already exists and is not empty",
            path.display()
        );
    }
    Ok(())
}

pub fn unix_nanos_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

pub fn retain_after_warmup<T>(
    records: &mut Vec<T>,
    warmup_seconds: u64,
    elapsed: impl Fn(&T) -> u64,
) {
    let warmup_ms = warmup_seconds * 1000;
    records.retain(|r| elapsed(r) >= warmup_ms);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tune_candidate(
        profile: &str,
        iteration: u32,
        score_total: u64,
        valid: bool,
    ) -> TuneCandidateSummary {
        TuneCandidateSummary {
            profile: profile.to_owned(),
            iteration,
            run_dir: PathBuf::from(format!("/tmp/{profile}-{iteration}")),
            applied_tasks: 1,
            warmup_seconds: 1,
            measure_seconds: 1,
            interval_count: 2,
            samples: 100,
            scored_samples: 100,
            score_total,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: 0,
            max_latency_ns: 0,
            frame_count: 0,
            frame_max_ms: 0.0,
            frame_p99_ms: 0.0,
            frame_over_16ms: 0,
            frame_over_33ms: 0,
            frame_over_50ms: 0,
            coverage: TuneCoverageMetrics::default(),
            valid,
        }
    }

    fn grouped_candidates(
        candidates: Vec<TuneCandidateSummary>,
    ) -> BTreeMap<String, Vec<TuneCandidateSummary>> {
        let mut grouped = BTreeMap::new();
        for candidate in candidates {
            grouped
                .entry(candidate.profile.clone())
                .or_insert_with(Vec::new)
                .push(candidate);
        }
        grouped
    }

    #[test]
    fn candidate_order_counterbalances_iterations() {
        assert_eq!(candidate_order_for_iteration(3, 1), vec![0, 1, 2]);
        assert_eq!(candidate_order_for_iteration(3, 2), vec![0, 2, 1]);
        assert_eq!(candidate_order_for_iteration(3, 3), vec![2, 0, 1]);
        assert_eq!(candidate_order_for_iteration(1, 4), vec![0]);
    }

    #[test]
    fn percentile_nearest_rank_and_iqr_work_on_u64_values() {
        let mut values = vec![40, 10, 30, 20];
        assert_eq!(percentile_nearest_rank_u64(&mut values, 25.0), 10);
        let mut values = vec![40, 10, 30, 20];
        assert_eq!(percentile_nearest_rank_u64(&mut values, 50.0), 20);
        let mut values = vec![40, 10, 30, 20];
        assert_eq!(percentile_nearest_rank_u64(&mut values, 100.0), 40);
        assert_eq!(iqr_u64(vec![10, 20, 30, 40]), 20);
    }

    #[test]
    fn ranking_confidence_is_unstable_for_close_results() {
        let grouped = grouped_candidates(vec![
            tune_candidate("a", 1, 100, true),
            tune_candidate("a", 2, 100, true),
            tune_candidate("a", 3, 120, true),
            tune_candidate("b", 1, 110, true),
            tune_candidate("b", 2, 110, true),
            tune_candidate("b", 3, 110, true),
        ]);
        let stats = profile_stats_from_grouped(&grouped);
        let (confidence, notes) = assess_ranking_confidence(&stats, &grouped, "a", 3);

        assert_eq!(confidence, RankingConfidence::Unstable);
        assert!(notes.iter().any(|note| note.contains("variance")));
    }

    #[test]
    fn ranking_confidence_distinguishes_high_medium_and_low() {
        let high_grouped = grouped_candidates(vec![
            tune_candidate("a", 1, 90, true),
            tune_candidate("a", 2, 90, true),
            tune_candidate("a", 3, 90, true),
            tune_candidate("b", 1, 120, true),
            tune_candidate("b", 2, 120, true),
            tune_candidate("b", 3, 120, true),
        ]);
        let high_stats = profile_stats_from_grouped(&high_grouped);
        let (confidence, _) = assess_ranking_confidence(&high_stats, &high_grouped, "a", 3);
        assert_eq!(confidence, RankingConfidence::High);

        let medium_grouped = grouped_candidates(vec![
            tune_candidate("a", 1, 90, true),
            tune_candidate("a", 2, 90, true),
            tune_candidate("a", 3, 100, true),
            tune_candidate("b", 1, 150, true),
            tune_candidate("b", 2, 150, true),
            tune_candidate("b", 3, 150, true),
        ]);
        let medium_stats = profile_stats_from_grouped(&medium_grouped);
        let (confidence, _) = assess_ranking_confidence(&medium_stats, &medium_grouped, "a", 3);
        assert_eq!(confidence, RankingConfidence::Medium);

        let low_grouped = grouped_candidates(vec![
            tune_candidate("a", 1, 90, true),
            tune_candidate("a", 2, 90, true),
            tune_candidate("b", 1, 120, true),
            tune_candidate("b", 2, 120, true),
        ]);
        let low_stats = profile_stats_from_grouped(&low_grouped);
        let (confidence, _) = assess_ranking_confidence(&low_stats, &low_grouped, "a", 2);
        assert_eq!(confidence, RankingConfidence::Low);
    }

    #[test]
    fn test_retain_after_warmup() {
        struct TestRecord {
            elapsed_ms: u64,
        }
        let mut records = vec![
            TestRecord { elapsed_ms: 0 },
            TestRecord { elapsed_ms: 500 },
            TestRecord { elapsed_ms: 1000 },
            TestRecord { elapsed_ms: 2000 },
        ];

        retain_after_warmup(&mut records, 1, |r| r.elapsed_ms);

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].elapsed_ms, 1000);
        assert_eq!(records[1].elapsed_ms, 2000);
    }

    #[test]
    fn test_tune_run_dir_iteration() {
        let base = Path::new("/tmp/tune");
        assert_ne!(tune_run_dir(base, "kcd", 1), tune_run_dir(base, "kcd", 2));
        assert_eq!(tune_run_dir(base, "kcd", 1), base.join("iter-001-kcd"));
    }

    #[test]
    fn test_sanitize_profile_name() {
        let base = Path::new("/tmp/tune");
        assert_eq!(
            tune_run_dir(base, "my profile/name", 1),
            base.join("iter-001-my_profile_name")
        );
        assert_eq!(
            tune_run_dir(base, "hot-path#123", 1),
            base.join("iter-001-hot-path_123")
        );
        assert_eq!(
            tune_run_dir(base, "../traversal", 1),
            base.join("iter-001-___traversal")
        );
    }
}
