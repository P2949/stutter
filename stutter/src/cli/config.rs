use super::*;

#[derive(Args, Debug, Clone)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConfigCommand {
    Check(ConfigCheckArgs),
    Explain(ConfigExplainArgs),
}

#[derive(Args, Debug, Clone)]
pub struct ConfigCheckArgs {
    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct ConfigExplainArgs {
    #[arg(long = "json")]
    pub json: bool,

    #[arg(long = "preset", value_name = "NAME")]
    pub preset: Option<String>,
}
