use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::commands::research::{ResearchClientAction, ResearchClientInput};

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
