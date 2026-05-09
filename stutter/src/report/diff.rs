use super::{text::*, *};

pub fn print_diff_report(
    path_a: &Path,
    path_b: &Path,
    top: usize,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<()> {
    print!("{}", render_diff_report(path_a, path_b, top, filter_class)?);
    Ok(())
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
