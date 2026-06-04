use std::{collections::BTreeMap, fs, path::Path};

use super::model::{TuneProfilePlanSummary, TuneProfileRulePlanSummary};

pub(super) fn write_profile_plan_artifacts(
    run_dir: &Path,
    report: &crate::profiles::explain::ProfileExplainReport,
) -> anyhow::Result<()> {
    fs::create_dir_all(run_dir)?;
    fs::write(
        run_dir.join("profile_plan.json"),
        serde_json::to_vec_pretty(report)?,
    )?;
    let text = crate::watch::profile_explain_render::render_profile_explain_text(
        report,
        &crate::watch::profile_explain_render::ProfileExplainRenderOptions::default(),
    );
    fs::write(run_dir.join("profile_plan.txt"), text)?;
    Ok(())
}

pub(super) fn tune_profile_plan_summary(
    report: &crate::profiles::explain::ProfileExplainReport,
) -> TuneProfilePlanSummary {
    TuneProfilePlanSummary {
        snapshot_tasks: report.snapshot_tasks,
        matched_tasks: report.matched_tasks,
        pending_unique_tasks: report.pending_unique_tasks,
        pending_affinity: report.pending_affinity,
        rules: report
            .rules
            .iter()
            .map(|rule| TuneProfileRulePlanSummary {
                rule_index: rule.rule_index,
                matched_tasks: rule.matched_tasks,
                pending_affinity: rule.pending_affinity,
                top_classes: top_map(&rule.classes, 10),
                top_thread_comms: top_map(&rule.top_thread_comms, 10),
                process_comm_captures: rule.broad_process_comm_captured_thread_comms.values().sum(),
            })
            .collect(),
    }
}

fn top_map(map: &BTreeMap<String, usize>, top: usize) -> BTreeMap<String, usize> {
    let mut entries = map.iter().collect::<Vec<_>>();
    entries.sort_by(|(left_key, left_count), (right_key, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_key.cmp(right_key))
    });
    entries
        .into_iter()
        .take(top)
        .map(|(key, value)| (key.clone(), *value))
        .collect()
}
