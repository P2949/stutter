//! Release-candidate tasks: freeze the candidate, accept it offline and live, package, verify,
//! smoke-install and tag. Every command goes through a [`Shell`] so the whole sequence is tested
//! against a scripted machine; nothing here can move or overwrite an existing tag.
pub mod freeze;
pub mod offline;
pub mod package;
pub mod tag;

use super::{
    executor::RunnerOutcome,
    model::{BlockReason, FailureClass},
};

pub fn failed(class: FailureClass, stage: &str, summary: impl Into<String>) -> RunnerOutcome {
    RunnerOutcome::Failed {
        class,
        stage: stage.to_owned(),
        summary: summary.into(),
        artifacts: None,
    }
}

pub fn blocked(class: FailureClass, detail: impl Into<String>) -> RunnerOutcome {
    RunnerOutcome::Blocked(BlockReason {
        class,
        detail: detail.into(),
    })
}

/// The campaign's `release/` directory, created on demand.
pub fn release_dir(campaign: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let dir = campaign.join("release");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
