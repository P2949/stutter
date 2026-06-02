mod builder;
mod model;
mod render;

use std::fs;

use anyhow::Context;
pub use builder::{
    build_baseline_tune_recommendation, build_baseline_tune_recommendation_for_baselines,
};
pub use model::RecommendCommandInput;
pub use render::{
    render_baseline_tune_recommendation_html, render_baseline_tune_recommendation_markdown,
};

pub fn recommend_command(input: RecommendCommandInput) -> anyhow::Result<()> {
    let rec = if input.baseline.len() == 1 {
        build_baseline_tune_recommendation(&input.baseline[0], &input.tune)?
    } else {
        build_baseline_tune_recommendation_for_baselines(&input.baseline, &input.tune)?
    };

    let mut wrote_file = false;
    if let Some(markdown_path) = input.markdown {
        let markdown = render_baseline_tune_recommendation_markdown(&rec);
        if let Some(parent) = markdown_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&markdown_path, markdown)
            .with_context(|| format!("failed to write {}", markdown_path.display()))?;
        println!("recommendation={}", markdown_path.display());
        wrote_file = true;
    }
    if let Some(html_path) = input.html {
        let html = render_baseline_tune_recommendation_html(&rec);
        if let Some(parent) = html_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&html_path, html)
            .with_context(|| format!("failed to write {}", html_path.display()))?;
        println!("recommendation_html={}", html_path.display());
        wrote_file = true;
    }

    if input.json {
        println!("{}", serde_json::to_string_pretty(&rec)?);
    } else if !wrote_file {
        print!("{}", render_baseline_tune_recommendation_markdown(&rec));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
