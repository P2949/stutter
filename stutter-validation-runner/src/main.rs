use std::sync::Arc;

use anyhow::{Context, bail};
use clap::Parser;
use stutter_validation_runner::{
    boot::UnsupportedBoot,
    cli::{Cli, Command},
    install::{Layout, execute, plan_install},
    model::Authorization,
    process::SystemExecutor,
    resume::resume_all,
    server::Server,
    state::StateDir,
    sys::effective_uid,
};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    // As root the authorization must be root-owned and tamper-proof; an unprivileged run (only
    // useful for development) skips the ownership rule but keeps every other check.
    let privileged = effective_uid() == 0;
    match cli.command {
        Command::Serve {
            authorization,
            state_dir,
            socket,
        } => {
            let authorization = Authorization::load(&authorization, privileged)
                .map_err(anyhow::Error::msg)
                .context("authorization rejected")?;
            if let Some(parent) = socket.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let server = Arc::new(Server::new(
                authorization,
                StateDir(state_dir),
                Arc::new(SystemExecutor {
                    drop_to_owner: privileged,
                }),
                Arc::new(UnsupportedBoot),
            ));
            server.serve(&socket)?;
            Ok(())
        }
        Command::Resume { all, state_root } => {
            if !all {
                bail!("resume requires --all");
            }
            for line in resume_all(&state_root, privileged) {
                println!("{line}");
            }
            Ok(())
        }
        Command::Install {
            staged,
            repository,
            root,
        } => {
            let layout = match &root {
                Some(root) => Layout { root: root.clone() },
                None => Layout::system(),
            };
            // SUDO_UID is set by sudo to the real invoking user; a direct root run has none.
            let invoker = std::env::var("SUDO_UID")
                .ok()
                .and_then(|uid| uid.parse::<u32>().ok())
                .unwrap_or_else(effective_uid);
            let runner = std::env::current_exe().context("locate the runner binary")?;
            let plan = plan_install(&staged, &repository, &runner, invoker, &layout)
                .map_err(anyhow::Error::msg)
                .context("install refused")?;
            execute(&plan, root.is_none()).map_err(anyhow::Error::msg)?;
            println!("installed campaign {}", plan.authorization.campaign_id);
            Ok(())
        }
        Command::Check { authorization } => {
            let authorization = Authorization::load(&authorization, privileged)
                .map_err(anyhow::Error::msg)
                .context("authorization rejected")?;
            println!(
                "campaign {} owner uid {} grants {} max reboots {}",
                authorization.campaign_id,
                authorization.owner_uid,
                authorization.grants.len(),
                authorization.max_reboots
            );
            Ok(())
        }
    }
}
