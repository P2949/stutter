use std::path::PathBuf;

use clap::Subcommand;
use stutter::api::autotune::{
    RecoveryCheckpoint, RecoveryControllerInput, hold_affinity_recovery_controller,
    recover_affinity_acceptance_controller,
};

fn checkpoint(value: &str) -> Result<RecoveryCheckpoint, String> {
    value
        .parse()
        .map_err(|error: anyhow::Error| error.to_string())
}
#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub(crate) enum ControllerCommand {
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
            cpu,
            owner_pid,
            owner_ticks,
            checkpoint,
            out: &out,
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
