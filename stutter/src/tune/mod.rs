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
    artifacts::ArtifactSelection, config::model::MonitorConfig, hwmon, profiles,
    recorder::IntervalRecord, scorer, session::run_monitor, session_io,
};

pub mod comparability;
mod ranking;
pub mod recommendation;

pub use comparability::TuneCoverageMetrics;
use ranking::select_best_profile;
pub(crate) use ranking::{assess_ranking_confidence, median_u64, profile_stats_from_grouped};

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
    /// Raw diagnostic totals are only compared across fixed-duration tune runs.
    /// The explicit `raw_score_total` suffix prevents these serialized fields
    /// from being confused with normalized/rate-based comparison metrics.
    #[serde(alias = "median_diagnostic_score_total")]
    pub median_diagnostic_raw_score_total: u64,
    #[serde(alias = "iqr_diagnostic_score_total")]
    pub iqr_diagnostic_raw_score_total: u64,
    #[serde(alias = "worst_diagnostic_score_total")]
    pub worst_diagnostic_raw_score_total: u64,
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
    #[serde(alias = "diagnostic_score_total")]
    pub diagnostic_raw_score_total: u64,
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

pub struct TuneProfileRefreshInput {
    pub tree_pid: u32,
    pub profile: profiles::Profile,
    pub cache: profiles::ProfileApplyCache,
    pub force_restore_overwrite: bool,
    pub refresh_ms: u64,
    pub control: TuneControl,
    pub policy: crate::daemon_policy::DaemonPolicy,
    pub persistent_effect: bool,
    pub enforce: bool,
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
    let results = collect_tune_results(TuneCollectionInput {
        profiles: &profiles,
        tree_pid,
        epoch_seconds,
        warmup_seconds,
        runs,
        mangohud_log,
        enforce,
        hwmon,
        tune_output_dir: &tune_output_dir,
    })
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

struct TuneCollectionInput<'a> {
    profiles: &'a [profiles::Profile],
    tree_pid: u32,
    epoch_seconds: u64,
    warmup_seconds: u64,
    runs: u32,
    mangohud_log: Option<PathBuf>,
    enforce: bool,
    hwmon: bool,
    tune_output_dir: &'a Path,
}

async fn collect_tune_results(
    input: TuneCollectionInput<'_>,
) -> anyhow::Result<Vec<TuneCandidateSummary>> {
    let profiles = input.profiles;
    let tree_pid = input.tree_pid;
    let epoch_seconds = input.epoch_seconds;
    let warmup_seconds = input.warmup_seconds;
    let runs = input.runs;
    let mangohud_log = input.mangohud_log;
    let enforce = input.enforce;
    let tune_output_dir = input.tune_output_dir;
    let measure_seconds = epoch_seconds.saturating_sub(warmup_seconds);
    let mut results = Vec::new();
    let shared_hwmon = if input.hwmon {
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
                Arc::new(MonitorConfig {
                    target: crate::config::model::TargetConfig {
                        tree_pids: vec![tree_pid],
                        max_tasks: 1024,
                        ..Default::default()
                    },
                    timing: crate::config::model::TimingConfig {
                        summary_period_ms: 1_000,
                        max_duration: Some(Duration::from_secs(epoch_seconds)),
                        ..Default::default()
                    },
                    probes: crate::config::model::ProbeConfig {
                        hwmon: input.hwmon,
                        cpu_freq: true,
                        ..Default::default()
                    },
                    recording: crate::config::model::RecordingConfig {
                        run_name: Some(format!("tune-{}", profile.name)),
                        output_dir: Some(tune_run_dir(tune_output_dir, &profile.name, iteration)),
                        ..Default::default()
                    },
                    mangohud: crate::config::model::MangoHudConfig {
                        log: mangohud_log.clone(),
                        ..Default::default()
                    },
                    watch: crate::config::model::WatchConfig {
                        poll_ms: 2_000,
                        ..Default::default()
                    },
                    ..Default::default()
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
                diagnostic_raw_score_total: score.total,
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
            let keep_best_policy = crate::watch::profile_apply_policy(
                false,
                profiles::profile_uses_priority_actions(profile),
                true,
                crate::daemon_policy::ActionSource::Tune,
            );
            let records = match crate::watch::apply_profile_to_tree_blocking(
                summary.tree_pid,
                profile.clone(),
                false,
                false,
                enforce,
                keep_best_policy,
                true,
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
                        action_phase: None,
                        error_category: None,
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
                action_phase: None,
                error_category: None,
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
    monitor_config: Arc<MonitorConfig>,
    profile: profiles::Profile,
    enforce: bool,
    shared_hwmon: Option<Arc<std::sync::Mutex<hwmon::HwmonReader>>>,
    force_restore_overwrite: bool,
    _tune_output_dir: PathBuf,
    warmup_seconds: u64,
) -> anyhow::Result<TuneMeasureResult> {
    let tree_pid = monitor_config.target.tree_pids[0];
    let run_dir = monitor_config
        .recording
        .output_dir
        .as_ref()
        // invariant: monitor_config.recording.output_dir is populated by collect_tune_results
        .unwrap()
        .clone();

    let cache = profiles::ProfileApplyCache::default();
    let tune_candidate_policy = crate::watch::profile_apply_policy(
        false,
        profiles::profile_uses_priority_actions(&profile),
        false,
        crate::daemon_policy::ActionSource::Tune,
    );
    let (initial_apply, cache) = match crate::watch::apply_profile_to_tree_cached_blocking(
        tree_pid,
        profile.clone(),
        force_restore_overwrite,
        false,
        cache,
        tune_candidate_policy.clone(),
        false,
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
                action_phase: None,
                error_category: None,
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
        action_phase: None,
        error_category: None,
        message: format!("applied tune candidate profile '{}'", profile.name),
    });
    let should_force_refresh = force_restore_overwrite && initial_apply.affected_tasks() == 0;

    let control = TuneControl {
        stop_refresh: Arc::new(AtomicBool::new(false)),
        applied_tasks: Arc::new(AtomicUsize::new(0)),
    };
    let stop_refresh = control.stop_refresh.clone();
    let refreshed_applied_tasks = control.applied_tasks.clone();

    let mut profile_refresh = tokio::spawn(tune_profile_refresh_loop(TuneProfileRefreshInput {
        tree_pid,
        profile,
        cache,
        force_restore_overwrite: should_force_refresh,
        refresh_ms: TUNE_PROFILE_REFRESH_MS,
        control,
        policy: tune_candidate_policy,
        persistent_effect: false,
        enforce,
    }));

    let mut profile_refresh_finished = false;
    let monitor_result = tokio::select! {
        result = run_monitor(monitor_config, shared_hwmon, None, None) => result,
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

pub async fn tune_profile_refresh_loop(input: TuneProfileRefreshInput) -> anyhow::Result<()> {
    let TuneProfileRefreshInput {
        tree_pid,
        profile,
        mut cache,
        force_restore_overwrite,
        refresh_ms,
        control,
        policy,
        persistent_effect,
        enforce,
    } = input;
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
            policy.clone(),
            persistent_effect,
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
mod tests;
