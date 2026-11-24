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
    /// Print an intent with its cold arms frozen into it (`prefix_init_arms`): each authorized tool's
    /// compat directory under the study directory is read, read-only. Local only.
    FreezeColdArms {
        /// The private intent file to read.
        #[arg(long)]
        intent: PathBuf,
        /// The private study directory (its `compatdata/<arm>` prefixes are observed).
        #[arg(long)]
        study_root: PathBuf,
    },
    /// Print the definition digest and duration a long work unit must carry for a session file (the route, the key script
    /// and the save as it is now). Read-only and local; sends nothing to the game.
    SessionDefinition {
        /// The private session file (the one STUTTER_RESEARCH_LONG_SESSION names).
        #[arg(long)]
        session_file: PathBuf,
    },
    /// Record the destructive fault classes' fixture results from a `cargo test` log as evidence beside a study, and print the
    /// cases a fault-matrix receipt carries for them. Local; touches nothing but the study directory.
    FaultFixtures {
        /// The private study directory the evidence is written beside.
        #[arg(long)]
        study_root: PathBuf,
        /// The log of the fixture test run (`cargo test` output).
        #[arg(long)]
        test_log: PathBuf,
        /// SHA-256 of the Stutter binary the fixtures ran against.
        #[arg(long)]
        binary_sha256: String,
    },
    /// Record what the Stutter units actually run (binary hash, active state) against an expected
    /// pin, as a receipt in the study directory. Read-only and local.
    DeployReceipt {
        /// The private study directory.
        #[arg(long)]
        study_root: PathBuf,
        /// The exact, root-owned `systemctl` to ask.
        #[arg(long)]
        systemctl: PathBuf,
        /// A unit to inspect (repeat for the controller and the worker).
        #[arg(long = "unit", required = true)]
        units: Vec<String>,
        /// The expected SHA-256 (hex) of the pinned release binary.
        #[arg(long)]
        expect_binary_sha256: String,
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
            ResearchCommand::FreezeColdArms { intent, study_root } => {
                ResearchClientAction::FreezeColdArms { intent, study_root }
            }
            ResearchCommand::SessionDefinition { session_file } => {
                ResearchClientAction::SessionDefinition { session_file }
            }
            ResearchCommand::FaultFixtures {
                study_root,
                test_log,
                binary_sha256,
            } => ResearchClientAction::FaultFixtures {
                study_root,
                test_log,
                binary_sha256,
            },
            ResearchCommand::DeployReceipt {
                study_root,
                systemctl,
                units,
                expect_binary_sha256,
            } => ResearchClientAction::DeployReceipt {
                study_root,
                systemctl,
                units,
                expect_binary_sha256,
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
    fn deploy_receipt_takes_one_or_more_units_a_systemctl_and_a_pin() {
        let parsed = crate::cli::parse_app_command_from([
            "stutter",
            "research",
            "deploy-receipt",
            "--study-root",
            "/x",
            "--systemctl",
            "/y",
            "--unit",
            "a.service",
            "--unit",
            "b.service",
            "--expect-binary-sha256",
            "00",
        ])
        .unwrap();
        assert!(matches!(
            parsed,
            AppCommand::Research(input)
                if matches!(input.action, ResearchClientAction::DeployReceipt { ref units, .. } if units.len() == 2)
        ));
        for missing in [
            vec![
                "stutter",
                "research",
                "deploy-receipt",
                "--study-root",
                "/x",
                "--systemctl",
                "/y",
            ],
            vec![
                "stutter",
                "research",
                "deploy-receipt",
                "--unit",
                "a",
                "--systemctl",
                "/y",
                "--expect-binary-sha256",
                "0",
            ],
        ] {
            assert!(crate::cli::parse_app_command_from(missing).is_err());
        }
    }

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

#[cfg(test)]
mod session_definition_tests {
    use crate::commands::{input::AppCommand, research::ResearchClientAction};

    #[test]
    fn the_session_definition_is_local_and_needs_the_session_file() {
        let parsed = crate::cli::parse_app_command_from([
            "stutter",
            "research",
            "session-definition",
            "--session-file",
            "/home/u/long-session.json",
        ])
        .unwrap();
        assert!(matches!(
            parsed,
            AppCommand::Research(input)
                if matches!(input.action, ResearchClientAction::SessionDefinition { .. })
        ));
        assert!(
            crate::cli::parse_app_command_from(["stutter", "research", "session-definition"])
                .is_err()
        );
    }
}
