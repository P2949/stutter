mod analysis;
mod model;
pub(crate) mod render;

use std::path::Path;

pub use analysis::{build_run_diff_summary, run_diff_summary_from_sessions};
pub use model::{RunDiffSummary, TaskDeltaSummary};

use crate::{process_tree::TaskClass, summary};

pub fn print_diff_report(
    path_a: &Path,
    path_b: &Path,
    top: usize,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<()> {
    print!("{}", render_diff_report(path_a, path_b, top, filter_class)?);
    Ok(())
}

pub fn render_diff_report(
    path_a: &Path,
    path_b: &Path,
    top: usize,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<String> {
    let diff = build_run_diff_summary(path_a, path_b, filter_class)?;
    Ok(render::render_run_diff_summary(&diff, top))
}

pub fn print_batch_report(
    batch_dir: &Path,
    baseline_path: Option<&Path>,
    json_summary: bool,
    top: usize,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<()> {
    let summary = summary::build_batch_run_summary(batch_dir, baseline_path, top, filter_class)?;
    if json_summary {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        print!("{}", summary::render_batch_run_summary(&summary, top));
    }
    Ok(())
}
