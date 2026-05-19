#![allow(dead_code)] // Transitional command-runner façade.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub status_success: bool,
}
