use std::path::PathBuf;

use clap::Subcommand;
use stutter::api::autotune::{
    RecoveryCheckpoint, RecoveryControllerInput, SuspendResumeControllerInput,
    hold_affinity_recovery_controller, hold_suspend_resume_controller,
    recover_affinity_acceptance_controller,
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
        #[arg(long)]
        cpu: u32,
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
        #[arg(long)]
        cpu: u32,
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
            cpu,
            owner_pid,
            owner_ticks,
            checkpoint,
        } => hold_affinity_recovery_controller(RecoveryControllerInput {
            tree_pid,
            tree_ticks,
            desired_cpu: cpu,
            owner_pid,
            owner_ticks,
            checkpoint,
            out: &out,
        }),
        ControllerCommand::SuspendResume {
            out,
            tree_pid,
            tree_ticks,
            cpu,
            owner_pid,
            owner_ticks,
            resume_timeout_seconds,
        } => hold_suspend_resume_controller(SuspendResumeControllerInput {
            tree_pid,
            tree_ticks,
            desired_cpu: cpu,
            out: &out,
            owner_pid,
            owner_ticks,
            resume_timeout_seconds,
        }),
        ControllerCommand::Recover {
            out,
            expected_tasks,
        } => {
            println!(
                "{}",
                recover_affinity_acceptance_controller(&out, expected_tasks)?
            );
            Ok(())
        }
    }
}
