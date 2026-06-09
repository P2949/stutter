mod builder;
mod fix_validation;
mod model;
mod render;

use std::fs;

use anyhow::Context;
pub use builder::build_baseline_tune_recommendation_for_baselines_with_options;
#[cfg(test)]
pub(crate) use builder::{
    build_baseline_tune_recommendation, build_baseline_tune_recommendation_for_baselines,
};
pub use fix_validation::validate_fix_plan_against_recommendation;
use model::BaselineTuneRecommendationOptions;
pub use model::RecommendCommandInput;
pub use render::{
    render_baseline_tune_recommendation_html, render_baseline_tune_recommendation_markdown,
    render_fix_validation_report_html, render_fix_validation_report_markdown,
};

use crate::advisor::{AdvisorFixPlan, models::AdvisorReport};

pub fn recommend_command(input: RecommendCommandInput) -> anyhow::Result<()> {
    let options = BaselineTuneRecommendationOptions {
        allow_scenario_mismatch: input.allow_scenario_mismatch,
    };
    let rec = build_baseline_tune_recommendation_for_baselines_with_options(
        &input.baseline,
        &input.tune,
        options,
    )?;
    let fix_validation = if let Some(path) = input.fix_plan.as_deref() {
        let plan = load_fix_plan(path)?;
        Some(validate_fix_plan_against_recommendation(&plan, &rec))
    } else {
        None
    };

    let mut wrote_file = false;
    if let Some(markdown_path) = input.markdown {
        let markdown = fix_validation
            .as_ref()
            .map(render_fix_validation_report_markdown)
            .unwrap_or_else(|| render_baseline_tune_recommendation_markdown(&rec));
        if let Some(parent) = markdown_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&markdown_path, markdown)
            .with_context(|| format!("failed to write {}", markdown_path.display()))?;
        println!("recommendation={}", markdown_path.display());
        wrote_file = true;
    }
    if let Some(html_path) = input.html {
        let html = fix_validation
            .as_ref()
            .map(render_fix_validation_report_html)
            .unwrap_or_else(|| render_baseline_tune_recommendation_html(&rec));
        if let Some(parent) = html_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&html_path, html)
            .with_context(|| format!("failed to write {}", html_path.display()))?;
        println!("recommendation_html={}", html_path.display());
        wrote_file = true;
    }

    if input.json {
        if let Some(report) = &fix_validation {
            println!("{}", serde_json::to_string_pretty(report)?);
        } else {
            println!("{}", serde_json::to_string_pretty(&rec)?);
        }
    } else if !wrote_file {
        if let Some(report) = &fix_validation {
            print!("{}", render_fix_validation_report_markdown(report));
        } else {
            print!("{}", render_baseline_tune_recommendation_markdown(&rec));
        }
    }
    Ok(())
}

fn load_fix_plan(path: &std::path::Path) -> anyhow::Result<AdvisorFixPlan> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if let Ok(plan) = serde_json::from_slice::<AdvisorFixPlan>(&bytes) {
        return Ok(plan);
    }
    let report: AdvisorReport = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "failed to parse fix plan or advisor report {}",
            path.display()
        )
    })?;
    report.fix_plans.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!(
            "advisor report {} did not contain fix_plans",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests;
