#![allow(dead_code)] // Transitional command-runner façade.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub status_success: bool,
}
