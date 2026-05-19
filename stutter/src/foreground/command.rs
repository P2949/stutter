#![allow(dead_code)] // Transitional command-runner injection target.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ForegroundCommand {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

pub(crate) trait ForegroundCommandRunner {
    fn run(&self, command: &ForegroundCommand) -> anyhow::Result<CommandOutput>;
}
