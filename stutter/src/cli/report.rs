use std::path::PathBuf;

use clap::Args;
use serde::Serialize;

#[derive(Args, Clone, Debug, Serialize)]
pub struct ReportArgs {
    pub dir: PathBuf,

    #[arg(long)]
    pub out_dir: Option<PathBuf>,

    #[arg(long)]
    pub run_name: Option<String>,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct SummaryArgs {
    pub dir: PathBuf,
}
