#![allow(dead_code)] // Transitional command-runner injection target.

use std::path::{Path, PathBuf};

const TRUSTED_FOREGROUND_HELPER_DIRS: &[&str] = &["/usr/bin", "/bin"];

pub(crate) fn resolve_trusted_foreground_helper(program: &str) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.is_absolute() {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }

    if program.contains('/') {
        return None;
    }

    TRUSTED_FOREGROUND_HELPER_DIRS
        .iter()
        .map(|dir| Path::new(dir).join(program))
        .find(|candidate| candidate.is_file())
}

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
