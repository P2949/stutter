use super::*;

#[derive(Args, Debug, Clone)]
pub(super) struct ConfigArgs {
    #[command(subcommand)]
    pub(super) command: ConfigCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub(super) enum ConfigCommand {
    Check(ConfigCheckArgs),
    Explain(ConfigExplainArgs),
}

#[derive(Args, Debug, Clone)]
pub(super) struct ConfigCheckArgs {
    #[arg(long = "json")]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct ConfigExplainArgs {
    #[arg(long = "json")]
    pub(super) json: bool,

    #[arg(long = "preset", value_name = "NAME")]
    pub(super) preset: Option<String>,
}

#[cfg(test)]
#[path = "tests/config.rs"]
mod tests;
