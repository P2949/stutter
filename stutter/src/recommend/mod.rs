mod builder;
mod model;
mod render;

use std::fs;

use anyhow::Context;
pub use builder::build_baseline_tune_recommendation;
pub use model::RecommendCommandInput;
pub use render::render_baseline_tune_recommendation_markdown;

pub fn recommend_command(input: RecommendCommandInput) -> anyhow::Result<()> {
    let rec = build_baseline_tune_recommendation(&input.baseline, &input.tune)?;
    if input.json {
        println!("{}", serde_json::to_string_pretty(&rec)?);
    } else if let Some(markdown_path) = input.markdown {
        let markdown = render_baseline_tune_recommendation_markdown(&rec);
        if let Some(parent) = markdown_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&markdown_path, markdown)
            .with_context(|| format!("failed to write {}", markdown_path.display()))?;
        println!("recommendation={}", markdown_path.display());
    } else {
        print!("{}", render_baseline_tune_recommendation_markdown(&rec));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
