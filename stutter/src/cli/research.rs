use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::commands::research::{DispatchTarget, ResearchClientAction, ResearchClientInput};

#[derive(Args, Debug)]
pub(super) struct ResearchArgs {
    /// Private directory configured on the resident controller.
    #[arg(long)]
    socket_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: ResearchCommand,
}

#[derive(Subcommand, Debug)]
enum ResearchCommand {
    /// Inspect registered Steam libraries without launching Steam or the game.
    Inventory {
        /// Primary Steam client's SteamApps directory; other libraries are discovered from it.
        steamapps: PathBuf,
        /// Root-owned proton-lab manifest emitted by the pinned Nix package.
        #[arg(long)]
        proton_manifest: Option<PathBuf>,
    },
    /// Submit one bounded intent from a private JSON file.
    Submit {
        intent: PathBuf,
    },
    /// Steam compatibility-tool entry; called by the dispatcher wrapper, not by hand.
    Dispatch {
        /// One campaign's dispatch directory.
        #[arg(
            long,
            conflicts_with = "study_root",
            required_unless_present = "study_root"
        )]
        context_dir: Option<PathBuf>,
        /// The private study root; the armed attempt is selected by its unused ticket, never by time.
        #[arg(long)]
        study_root: Option<PathBuf>,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        args: Vec<String>,
    },
    /// Check GPU health against the kernel journal and record any unacknowledged GPU reset as an
    /// incident in the study directory. Local only; arms no campaign and launches nothing.
    HealthCheck {
        /// The private study directory (the same one the controller uses).
        #[arg(long)]
        study_root: PathBuf,
        /// The exact, root-owned `journalctl` to query.
        #[arg(long)]
        journalctl: PathBuf,
    },
    Status,
    Pause,
    Resume,
    Cancel,
    Report,
    Verify,
}

impl From<ResearchArgs> for ResearchClientInput {
    fn from(args: ResearchArgs) -> Self {
        let action = match args.command {
            ResearchCommand::Inventory {
                steamapps,
                proton_manifest,
            } => ResearchClientAction::Inventory {
                steamapps,
                proton_manifest,
            },
            ResearchCommand::Submit { intent } => ResearchClientAction::Submit(intent),
            ResearchCommand::Dispatch {
                context_dir,
                study_root,
                args,
            } => ResearchClientAction::Dispatch {
                // clap guarantees exactly one; an empty path cannot open and fails the dispatch.
                target: match (context_dir, study_root) {
                    (Some(dir), _) => DispatchTarget::ContextDir(dir),
                    (None, Some(root)) => DispatchTarget::StudyRoot(root),
                    (None, None) => DispatchTarget::ContextDir(PathBuf::new()),
                },
                args,
            },
            ResearchCommand::HealthCheck {
                study_root,
                journalctl,
            } => ResearchClientAction::HealthCheck {
                study_root,
                journalctl,
            },
            ResearchCommand::Status => ResearchClientAction::Status,
            ResearchCommand::Pause => ResearchClientAction::Pause,
            ResearchCommand::Resume => ResearchClientAction::Resume,
            ResearchCommand::Cancel => ResearchClientAction::Cancel,
            ResearchCommand::Report => ResearchClientAction::Report,
            ResearchCommand::Verify => ResearchClientAction::Verify,
        };
        Self {
            socket_dir: args.socket_dir,
            action,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::commands::{input::AppCommand, research::ResearchClientAction};

    #[test]
    fn research_cli_accepts_only_named_management_commands() {
        let command = crate::cli::parse_app_command_from([
            "stutter",
            "research",
            "--socket-dir",
            "/tmp/private",
            "status",
        ])
        .unwrap();
        assert!(
            matches!(command, AppCommand::Research(input) if matches!(input.action, ResearchClientAction::Status))
        );
        assert!(
            crate::cli::parse_app_command_from([
                "stutter",
                "research",
                "--socket-dir",
                "/tmp/private",
                "execute"
            ])
            .is_err()
        );
        assert!(matches!(
            crate::cli::parse_app_command_from([
                "stutter",
                "research",
                "inventory",
                "/home/user/steamapps",
                "--proton-manifest",
                "/nix/store/abc-proton-lab.json",
            ]),
            Ok(AppCommand::Research(_))
        ));
    }
}

#[cfg(test)]
mod health_check_tests {
    use crate::commands::{input::AppCommand, research::ResearchClientAction};

    #[test]
    fn the_health_check_is_local_only_and_needs_both_paths() {
        let parsed = crate::cli::parse_app_command_from([
            "stutter",
            "research",
            "health-check",
            "--study-root",
            "/home/u/Development/Studies",
            "--journalctl",
            "/nix/store/abc-systemd/bin/journalctl",
        ])
        .unwrap();
        assert!(matches!(
            parsed,
            AppCommand::Research(input)
                if matches!(input.action, ResearchClientAction::HealthCheck { .. })
        ));
        for missing in [
            vec!["stutter", "research", "health-check"],
            vec!["stutter", "research", "health-check", "--study-root", "/x"],
            vec!["stutter", "research", "health-check", "--journalctl", "/y"],
        ] {
            assert!(crate::cli::parse_app_command_from(missing).is_err());
        }
    }
}
