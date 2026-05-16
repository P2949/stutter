use clap::Args;
use serde::Serialize;

#[derive(Args, Clone, Debug, Serialize)]
pub struct ConfigArgs {
    #[arg(long)]
    pub explain: bool,

    #[arg(long)]
    pub preset: Option<String>,
}
