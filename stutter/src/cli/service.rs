use clap::Args;
use serde::Serialize;

#[derive(Args, Clone, Debug, Serialize)]
pub struct ServiceArgs {
    #[command(subcommand)]
    pub command: ServiceCommand,
}

#[derive(clap::Subcommand, Clone, Debug, Serialize)]
pub enum ServiceCommand {
    Install,
    Uninstall,
}
