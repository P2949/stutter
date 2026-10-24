use std::sync::Arc;

use anyhow::{Context, bail};
use clap::Parser;
use stutter_validation_runner::{
    boot::{AdapterControl, BootControl, SystemShell, UnsupportedBoot, detect_adapter},
    cli::{Cli, Command},
    install::{Layout, apply_boot_discovery, execute, plan_install},
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
                match detect_adapter(Arc::new(SystemShell)) {
                    Some(adapter) => Arc::new(AdapterControl(adapter)) as Arc<dyn BootControl>,
                    None => Arc::new(UnsupportedBoot),
                },
            ));
            server.serve(&socket)?;
            Ok(())
        }
        Command::Resume {
            all,
            state_root,
            wait_for_runner,
            socket,
        } => {
            if !all {
                bail!("resume requires --all");
            }
            if wait_for_runner > 0
                && !stutter_validation_runner::resume::wait_for_runner(
                    &socket,
                    wait_for_runner,
                    &std::thread::sleep,
                )
            {
                bail!("the runner did not start serving on {}", socket.display());
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
                Some(root) => Layout {
                    root: root.clone(),
                    trusted_uid: effective_uid(),
                },
                None => Layout::system(),
            };
            // SUDO_UID is set by sudo to the real invoking user; a direct root run has none.
            let invoker = std::env::var("SUDO_UID")
                .ok()
                .and_then(|uid| uid.parse::<u32>().ok())
                .unwrap_or_else(effective_uid);
            let runner = std::env::current_exe().context("locate the runner binary")?;
            let mut plan = plan_install(&staged, &repository, &runner, invoker, &layout)
                .map_err(anyhow::Error::msg)
                .context("install refused")?;
            // Boot candidates are discovered here, as root; an unknown bootloader leaves the
            // list empty and the kernel stage reports it instead of guessing.
            match detect_adapter(Arc::new(SystemShell)) {
                Some(adapter) => apply_boot_discovery(&mut plan.authorization, adapter.as_ref())
                    .map_err(anyhow::Error::msg)
                    .context("boot discovery")?,
                None => {
                    println!("no supported bootloader detected; boot control stays unavailable")
                }
            }
            execute(&plan, root.is_none()).map_err(anyhow::Error::msg)?;
            println!("install_mode={:?}", plan.mode);
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
