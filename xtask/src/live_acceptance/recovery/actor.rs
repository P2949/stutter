use std::path::PathBuf;

use clap::Subcommand;
use stutter::api::autotune::{
    AcceptanceMutation, RecoveryCheckpoint, RecoveryControllerInput, SuspendResumeControllerInput,
    hold_recovery_controller, hold_suspend_resume_controller, recover_acceptance_controller,
};

fn checkpoint(value: &str) -> Result<RecoveryCheckpoint, String> {
    value
        .parse()
        .map_err(|error: anyhow::Error| error.to_string())
}
#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum ControllerCommand {
    Hold {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        tree_pid: u32,
        #[arg(long)]
        tree_ticks: u64,
        #[arg(long, default_value = "affinity", value_parser = super::parse_mutation)]
        mutation: AcceptanceMutation,
        /// A CPU number (affinity) or a `uclamp.min` (uclamp).
        #[arg(long, alias = "cpu")]
        value: u32,
        #[arg(long)]
        owner_pid: u32,
        #[arg(long)]
        owner_ticks: u64,
        #[arg(long, value_parser = checkpoint)]
        checkpoint: RecoveryCheckpoint,
    },
    Recover {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        expected_tasks: usize,
    },
    SuspendResume {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        tree_pid: u32,
        #[arg(long)]
        tree_ticks: u64,
        #[arg(long, default_value = "affinity", value_parser = super::parse_mutation)]
        mutation: AcceptanceMutation,
        #[arg(long, alias = "cpu")]
        value: u32,
        #[arg(long)]
        owner_pid: u32,
        #[arg(long)]
        owner_ticks: u64,
        #[arg(long)]
        resume_timeout_seconds: u64,
    },
}
pub(crate) fn controller(mode: ControllerCommand) -> anyhow::Result<()> {
    match mode {
        ControllerCommand::Hold {
            out,
            tree_pid,
            tree_ticks,
            mutation,
            value,
            owner_pid,
            owner_ticks,
            checkpoint,
        } => hold_recovery_controller(RecoveryControllerInput {
            tree_pid,
            tree_ticks,
            mutation,
            value,
            owner_pid,
            owner_ticks,
            checkpoint,
            out: &out,
        }),
        ControllerCommand::SuspendResume {
            out,
            tree_pid,
            tree_ticks,
            mutation,
            value,
            owner_pid,
            owner_ticks,
            resume_timeout_seconds,
        } => hold_suspend_resume_controller(SuspendResumeControllerInput {
            tree_pid,
            tree_ticks,
            mutation,
            value,
            out: &out,
            owner_pid,
            owner_ticks,
            resume_timeout_seconds,
        }),
        ControllerCommand::Recover {
            out,
            expected_tasks,
        } => {
            println!("{}", recover_acceptance_controller(&out, expected_tasks)?);
            Ok(())
        }
    }
}
