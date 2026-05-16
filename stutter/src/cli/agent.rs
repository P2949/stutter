use clap::Args;
use serde::Serialize;

#[derive(Args, Clone, Debug, Serialize)]
pub struct AgentArgs {
    #[arg(long, default_value = "127.0.0.1:4242")]
    pub listen: String,

    #[arg(long)]
    pub allow_remote_apply: bool,
}
