use std::{fs, path::Path};

use anyhow::Context;
use stutter::api::tune_recommendation::{
    TuneSummary, build_tune_recommendation, render_tune_recommendation_html,
    render_tune_recommendation_markdown,
};

pub fn refresh_tune_recommendation(
    summary_path: &Path,
    baseline_profile: Option<&str>,
    out_dir: &Path,
) -> anyhow::Result<()> {
    let summary_bytes = fs::read(summary_path)
        .with_context(|| format!("failed to read {}", summary_path.display()))?;
    let summary: TuneSummary = serde_json::from_slice(&summary_bytes)
        .with_context(|| format!("failed to parse {}", summary_path.display()))?;
    let recommendation = build_tune_recommendation(&summary, baseline_profile);

    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;

    let json_path = out_dir.join("tuning_recommendation.json");
    fs::write(&json_path, serde_json::to_vec_pretty(&recommendation)?)
        .with_context(|| format!("failed to write {}", json_path.display()))?;

    let markdown_path = out_dir.join("tuning_recommendation.md");
    fs::write(
        &markdown_path,
        render_tune_recommendation_markdown(&recommendation),
    )
    .with_context(|| format!("failed to write {}", markdown_path.display()))?;

    let html_path = out_dir.join("tuning_recommendation.html");
    fs::write(&html_path, render_tune_recommendation_html(&recommendation))
        .with_context(|| format!("failed to write {}", html_path.display()))?;

    println!("recommendation_json={}", json_path.display());
    println!("recommendation_markdown={}", markdown_path.display());
    println!("recommendation_html={}", html_path.display());

    Ok(())
}
