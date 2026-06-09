use std::path::Path;

use super::{build::build_compact_run_summary, render::render_compact_run_summary};
use crate::process_tree::TaskClass;

pub fn summary_command(
    path: &Path,
    json: bool,
    top: usize,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<()> {
    let summary = build_compact_run_summary(path, top, filter_class)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        print!("{}", render_compact_run_summary(&summary));
    }
    Ok(())
}
