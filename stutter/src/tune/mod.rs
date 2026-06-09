use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use log::{info, warn};

use crate::profiles;

pub mod comparability;
mod order;
mod profile_plan;
mod ranking;
pub mod recommendation;
mod recommendation_formal;
pub(crate) mod recommendation_html;
pub mod run;
pub mod statistics;
pub(crate) mod uncertainty_html;

#[cfg(test)]
pub(crate) use comparability::TuneCoverageMetrics;
#[cfg(test)]
pub(crate) use order::candidate_order_for_iteration;
use order::tune_candidate_order;
use ranking::select_best_profile;
pub(crate) use ranking::{assess_ranking_confidence, profile_stats_from_grouped};
pub use run::*;
pub const TUNE_RUN_STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
pub const TUNE_PROFILE_REFRESH_MS: u64 = 1_000;

pub mod model;
pub use model::*;

pub async fn tune_command(input: TuneCommandInput) -> anyhow::Result<()> {
    let TuneCommandInput {
        tree_pid,
        profiles_path,
        epoch_seconds,
        warmup_seconds,
        runs,
        keep_best,
        baseline_profile,
        scenario_name,
        workload_label,
        route_label,
        out_dir,
        mangohud_log,
        enforce,
        hwmon,
        order_strategy,
    } = input;
    let scenario_name = crate::scenario::normalize_identity_label(scenario_name.as_deref());
    if let Some(scenario_name) = scenario_name.as_deref() {
        crate::scenario::validate_scenario_name(scenario_name)?;
    }
    let workload_label = crate::scenario::normalize_identity_label(workload_label.as_deref());
    let route_label = crate::scenario::normalize_identity_label(route_label.as_deref());
    let scenario_hash = crate::scenario::scenario_identity_hash(
        scenario_name.as_deref(),
        workload_label.as_deref(),
        route_label.as_deref(),
    );

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

    let candidate_order = tune_candidate_order(&profiles, runs, &order_strategy);
    let results = collect_tune_results(TuneCollectionInput {
        profiles: &profiles,
        tree_pid,
        epoch_seconds,
        warmup_seconds,
        runs,
        mangohud_log,
        enforce,
        hwmon,
        scenario_name: scenario_name.clone(),
        scenario_hash: scenario_hash.clone(),
        workload_label: workload_label.clone(),
        route_label: route_label.clone(),
        tune_output_dir: &tune_output_dir,
        order_strategy: &order_strategy,
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
    let mut comparability_warnings = comparability::tune_comparability_warnings(&grouped);
    let order_warnings = comparability::tune_candidate_order_warnings(&candidate_order);
    if !order_warnings.is_empty() {
        comparability_warnings.extend(order_warnings.clone());
    }

    let profile_stats = profile_stats_from_grouped(&grouped);
    let selected_best_profile = select_best_profile(&grouped);
    let (mut ranking_confidence, mut ranking_notes) =
        assess_ranking_confidence(&profile_stats, &grouped, &selected_best_profile, runs);
    let adjusted_ranking_confidence = comparability::ranking_confidence_after_comparability_warnings(
        ranking_confidence,
        &comparability_warnings,
    );
    if adjusted_ranking_confidence != ranking_confidence {
        ranking_notes
            .push("candidate order was not counterbalanced; ranking confidence lowered".to_owned());
        ranking_confidence = adjusted_ranking_confidence;
    }
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

    let order_balance_warning = {
        let msgs: Vec<String> = order_warnings.iter().map(|w| w.message.clone()).collect();
        if msgs.is_empty() {
            None
        } else {
            Some(msgs.join("; "))
        }
    };

    let summary = TuneSummary {
        schema_version: 1,
        tree_pid,
        scenario_name,
        scenario_hash,
        workload_label,
        route_label,
        profiles_path,
        runs,
        epoch_seconds,
        warmup_seconds,
        restore_policy: restore_policy.to_owned(),
        best_profile,
        order_strategy: order_strategy.clone(),
        order_balanced: order_warnings.is_empty(),
        order_balance_warning,
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
