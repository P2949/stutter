use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use log::{debug, info, warn};
use tokio::time::sleep;

use super::{
    TUNE_PROFILE_REFRESH_MS, TUNE_RUN_STALE_AFTER, comparability,
    model::{
        TuneCandidateSummary, TuneControl, TuneIterationOrder, TuneMeasureResult,
        TuneProfileRefreshInput, TuneSummary,
    },
    recommendation, retain_after_warmup, unix_nanos_now,
};
use crate::{
    artifacts::ArtifactSelection, config::model::MonitorConfig, hwmon, profiles,
    recorder::IntervalRecord, scorer, session::run_monitor, session_io,
};

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

pub(super) fn tune_candidate_order(
    profiles: &[profiles::Profile],
    runs: u32,
) -> Vec<TuneIterationOrder> {
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

pub(super) struct TuneCollectionInput<'a> {
    pub(super) profiles: &'a [profiles::Profile],
    pub(super) tree_pid: u32,
    pub(super) epoch_seconds: u64,
    pub(super) warmup_seconds: u64,
    pub(super) runs: u32,
    pub(super) mangohud_log: Option<PathBuf>,
    pub(super) enforce: bool,
    pub(super) hwmon: bool,
    pub(super) tune_output_dir: &'a Path,
}

pub(super) async fn collect_tune_results(
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

pub(super) async fn write_tune_summary(
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
