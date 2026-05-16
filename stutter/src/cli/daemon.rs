use super::*;

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonArgs {
    #[command(subcommand)]
    pub(super) command: DaemonCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub(super) enum DaemonCommand {
    Config(DaemonConfigArgs),
    Policy(DaemonPolicyArgs),
    Profiles(DaemonProfilesArgs),
    Explain(DaemonExplainArgs),
    #[command(name = "why-not-optimize")]
    WhyNotOptimize(DaemonWhyNotOptimizeArgs),
    #[command(name = "what-changed")]
    WhatChanged(DaemonWhatChangedArgs),
    Status(DaemonStatusArgs),
    Watch(DaemonWatchArgs),
    Doctor(DaemonDoctorArgs),
    #[command(name = "reset-state")]
    ResetState(DaemonResetStateArgs),
    #[command(name = "bench-overhead")]
    BenchOverhead(DaemonBenchOverheadArgs),
    Soak(DaemonSoakArgs),
    Acceptance(DaemonAcceptanceArgs),
    Pause(DaemonPauseArgs),
    Resume(DaemonResumeArgs),
    Restore(DaemonRestoreArgs),
    #[command(name = "emergency-restore")]
    EmergencyRestore(DaemonRestoreArgs),
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonPolicyArgs {
    #[command(subcommand)]
    pub(super) command: DaemonPolicyCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub(super) enum DaemonPolicyCommand {
    Explain(DaemonPolicyExplainArgs),
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonPolicyExplainArgs {
    #[arg(long = "json")]
    pub(super) json: bool,

    #[arg(long = "preset", value_name = "NAME")]
    pub(super) preset: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonProfilesArgs {
    #[command(subcommand)]
    pub(super) command: DaemonProfilesCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub(super) enum DaemonProfilesCommand {
    List(DaemonProfilesListArgs),
    Forget(DaemonProfilesForgetArgs),
    Explain(DaemonProfilesExplainArgs),
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonProfilesListArgs {
    #[arg(long = "json")]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonProfilesForgetArgs {
    #[arg(long = "workload-hash", value_name = "HASH")]
    pub(super) workload_identity_hash: Option<String>,

    #[arg(long = "candidate", value_name = "NAME_OR_ACTION_ID")]
    pub(super) candidate: Option<String>,

    #[arg(long = "all")]
    pub(super) all: bool,

    #[arg(long = "dry-run")]
    pub(super) dry_run: bool,

    #[arg(long = "json")]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonProfilesExplainArgs {
    #[arg(long = "workload-hash", value_name = "HASH")]
    pub(super) workload_identity_hash: Option<String>,

    #[arg(long = "json")]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonExplainArgs {
    #[arg(long = "json")]
    pub(super) json: bool,

    #[arg(long = "explain-last", default_value_t = 10)]
    pub(super) explain_last: usize,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonWhyNotOptimizeArgs {
    #[arg(long = "json")]
    pub(super) json: bool,

    #[arg(long = "explain-last", default_value_t = 10)]
    pub(super) explain_last: usize,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonWhatChangedArgs {
    #[arg(long = "json")]
    pub(super) json: bool,

    #[arg(long = "explain-last", default_value_t = 10)]
    pub(super) explain_last: usize,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonConfigArgs {
    #[arg(long = "explain")]
    pub(super) explain: bool,

    #[arg(long = "json", requires = "explain")]
    pub(super) json: bool,

    #[arg(long = "preset", value_name = "NAME")]
    pub(super) preset: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonStatusArgs {
    #[arg(long = "json")]
    pub(super) json: bool,

    #[arg(long = "explain-last", default_value_t = 10)]
    pub(super) explain_last: usize,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonWatchArgs {
    #[arg(long = "interval-ms", default_value_t = 1_000)]
    pub(super) interval_ms: u64,

    #[arg(long = "iterations", value_name = "N")]
    pub(super) iterations: Option<u64>,

    #[arg(long = "verbose")]
    pub(super) verbose: bool,

    #[arg(long = "explain-last", default_value_t = 10)]
    pub(super) explain_last: usize,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonDoctorArgs {
    #[arg(long = "json")]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonResetStateArgs {
    #[arg(long = "dry-run")]
    pub(super) dry_run: bool,

    #[arg(long = "json")]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonBenchOverheadArgs {
    #[arg(long = "json")]
    pub(super) json: bool,

    #[arg(long = "duration-ms", default_value_t = 1_000)]
    pub(super) duration_ms: u64,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonSoakArgs {
    #[arg(long = "json")]
    pub(super) json: bool,

    #[arg(long = "duration-seconds", default_value_t = 60)]
    pub(super) duration_seconds: u64,

    #[arg(long = "tick-ms", default_value_t = 1_000)]
    pub(super) tick_ms: u64,

    #[arg(long = "profile", default_value = "observe-only")]
    pub(super) profile: String,

    #[arg(long = "max-disk-growth-bytes")]
    pub(super) max_disk_growth_bytes: Option<u64>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonAcceptanceArgs {
    #[arg(long = "json")]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonPauseArgs {}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonResumeArgs {}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonRestoreArgs {
    #[arg(long = "dry-run")]
    pub(super) dry_run: bool,
}

#[cfg(test)]
mod tests {
    use crate::commands::input::{AppCommand, DaemonProfilesCommandInput};

    fn parse_daemon_command<const N: usize>(args: [&str; N]) -> anyhow::Result<AppCommand> {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        crate::cli::parse_app_command_from(args)
    }

    #[test]
    fn parses_daemon_config_explain_json_command() {
        let command =
            parse_daemon_command(["stutter", "daemon", "config", "--explain", "--json"]).unwrap();

        let AppCommand::DaemonConfigExplain(input) = command else {
            panic!("expected daemon config explain command");
        };

        assert!(input.json);
        assert_eq!(input.preset, None);
    }

    #[test]
    fn parses_daemon_config_preset() {
        let command = parse_daemon_command([
            "stutter",
            "daemon",
            "config",
            "--explain",
            "--preset",
            "gaming-low-risk",
        ])
        .unwrap();

        let AppCommand::DaemonConfigExplain(input) = command else {
            panic!("expected daemon config explain command");
        };

        assert!(!input.json);
        assert_eq!(input.preset.as_deref(), Some("gaming-low-risk"));
    }

    #[test]
    fn daemon_config_command_requires_explain_flag() {
        let err = parse_daemon_command(["stutter", "daemon", "config"]).unwrap_err();

        assert!(err.to_string().contains("daemon config requires --explain"));
    }

    #[test]
    fn parses_daemon_policy_explain_json_command() {
        let command = parse_daemon_command([
            "stutter",
            "daemon",
            "policy",
            "explain",
            "--preset",
            "gaming-low-risk",
            "--json",
        ])
        .unwrap();

        let AppCommand::DaemonPolicyExplain(input) = command else {
            panic!("expected daemon policy explain command");
        };

        assert!(input.json);
        assert_eq!(input.preset.as_deref(), Some("gaming-low-risk"));
    }

    #[test]
    fn parses_daemon_profiles_list_command() {
        let command =
            parse_daemon_command(["stutter", "daemon", "profiles", "list", "--json"]).unwrap();

        let AppCommand::DaemonProfiles(DaemonProfilesCommandInput::List(input)) = command else {
            panic!("expected daemon profiles list command");
        };

        assert!(input.json);
    }

    #[test]
    fn parses_daemon_profiles_forget_command() {
        let command = parse_daemon_command([
            "stutter",
            "daemon",
            "profiles",
            "forget",
            "--workload-hash",
            "workload-a",
            "--candidate",
            "game-main",
            "--dry-run",
            "--json",
        ])
        .unwrap();

        let AppCommand::DaemonProfiles(DaemonProfilesCommandInput::Forget(input)) = command else {
            panic!("expected daemon profiles forget command");
        };

        assert_eq!(input.workload_identity_hash.as_deref(), Some("workload-a"));
        assert_eq!(input.candidate.as_deref(), Some("game-main"));
        assert!(!input.all);
        assert!(input.dry_run);
        assert!(input.json);
    }

    #[test]
    fn parses_daemon_profiles_forget_all_command() {
        let command = parse_daemon_command([
            "stutter",
            "daemon",
            "profiles",
            "forget",
            "--all",
            "--dry-run",
            "--json",
        ])
        .unwrap();

        let AppCommand::DaemonProfiles(DaemonProfilesCommandInput::Forget(input)) = command else {
            panic!("expected daemon profiles forget command");
        };

        assert_eq!(input.workload_identity_hash, None);
        assert_eq!(input.candidate, None);
        assert!(input.all);
        assert!(input.dry_run);
        assert!(input.json);
    }

    #[test]
    fn rejects_daemon_profiles_forget_without_scope() {
        let err = parse_daemon_command(["stutter", "daemon", "profiles", "forget"])
            .unwrap_err()
            .to_string();

        assert!(err.contains("requires --workload-hash or --all"));
    }

    #[test]
    fn rejects_daemon_profiles_forget_all_with_workload_hash() {
        let err = parse_daemon_command([
            "stutter",
            "daemon",
            "profiles",
            "forget",
            "--all",
            "--workload-hash",
            "workload-a",
        ])
        .unwrap_err()
        .to_string();

        assert!(err.contains("--all conflicts with --workload-hash"));
    }

    #[test]
    fn parses_daemon_profiles_explain_command() {
        let command = parse_daemon_command([
            "stutter",
            "daemon",
            "profiles",
            "explain",
            "--workload-hash",
            "workload-a",
            "--json",
        ])
        .unwrap();

        let AppCommand::DaemonProfiles(DaemonProfilesCommandInput::Explain(input)) = command else {
            panic!("expected daemon profiles explain command");
        };

        assert_eq!(input.workload_identity_hash.as_deref(), Some("workload-a"));
        assert!(input.json);
    }

    #[test]
    fn parses_daemon_explain_command() {
        let command = parse_daemon_command([
            "stutter",
            "daemon",
            "explain",
            "--json",
            "--explain-last",
            "3",
        ])
        .unwrap();

        let AppCommand::DaemonExplain(input) = command else {
            panic!("expected daemon explain command");
        };

        assert!(input.json);
        assert_eq!(input.explain_last, 3);
    }

    #[test]
    fn parses_daemon_why_not_optimize_command() {
        let command = parse_daemon_command([
            "stutter",
            "daemon",
            "why-not-optimize",
            "--json",
            "--explain-last",
            "2",
        ])
        .unwrap();

        let AppCommand::DaemonWhyNotOptimize(input) = command else {
            panic!("expected daemon why-not-optimize command");
        };

        assert!(input.json);
        assert_eq!(input.explain_last, 2);
    }

    #[test]
    fn parses_daemon_what_changed_command() {
        let command = parse_daemon_command([
            "stutter",
            "daemon",
            "what-changed",
            "--json",
            "--explain-last",
            "2",
        ])
        .unwrap();

        let AppCommand::DaemonWhatChanged(input) = command else {
            panic!("expected daemon what-changed command");
        };

        assert!(input.json);
        assert_eq!(input.explain_last, 2);
    }

    #[test]
    fn parses_daemon_status_json_command() {
        let command = parse_daemon_command(["stutter", "daemon", "status", "--json"]).unwrap();

        let AppCommand::DaemonStatus(input) = command else {
            panic!("expected daemon status command");
        };

        assert!(input.json);
        assert_eq!(input.explain_last, 10);
    }

    #[test]
    fn parses_daemon_status_explain_last_command() {
        let command = parse_daemon_command([
            "stutter",
            "daemon",
            "status",
            "--json",
            "--explain-last",
            "6",
        ])
        .unwrap();

        let AppCommand::DaemonStatus(input) = command else {
            panic!("expected daemon status command");
        };

        assert!(input.json);
        assert_eq!(input.explain_last, 6);
    }

    #[test]
    fn parses_daemon_watch_command() {
        let command = parse_daemon_command([
            "stutter",
            "daemon",
            "watch",
            "--interval-ms",
            "250",
            "--iterations",
            "2",
            "--verbose",
            "--explain-last",
            "4",
        ])
        .unwrap();

        let AppCommand::DaemonWatch(input) = command else {
            panic!("expected daemon watch command");
        };

        assert_eq!(input.interval_ms, 250);
        assert_eq!(input.iterations, Some(2));
        assert!(input.verbose);
        assert_eq!(input.explain_last, 4);
    }

    #[test]
    fn rejects_daemon_watch_zero_interval() {
        let err = parse_daemon_command(["stutter", "daemon", "watch", "--interval-ms", "0"])
            .unwrap_err()
            .to_string();

        assert!(err.contains("--interval-ms must be greater than zero"));
    }

    #[test]
    fn rejects_daemon_watch_zero_iterations() {
        let err = parse_daemon_command(["stutter", "daemon", "watch", "--iterations", "0"])
            .unwrap_err()
            .to_string();

        assert!(err.contains("--iterations must be greater than zero"));
    }

    #[test]
    fn parses_daemon_doctor_command() {
        let command = parse_daemon_command(["stutter", "daemon", "doctor", "--json"]).unwrap();

        let AppCommand::DaemonDoctor(input) = command else {
            panic!("expected daemon doctor command");
        };

        assert!(input.json);
    }

    #[test]
    fn parses_daemon_reset_state_command() {
        let command =
            parse_daemon_command(["stutter", "daemon", "reset-state", "--dry-run", "--json"])
                .unwrap();

        let AppCommand::DaemonResetState(input) = command else {
            panic!("expected daemon reset-state command");
        };

        assert!(input.dry_run);
        assert!(input.json);
    }

    #[test]
    fn parses_daemon_bench_overhead_command() {
        let command = parse_daemon_command([
            "stutter",
            "daemon",
            "bench-overhead",
            "--duration-ms",
            "25",
            "--json",
        ])
        .unwrap();

        let AppCommand::DaemonBenchOverhead(input) = command else {
            panic!("expected daemon bench-overhead command");
        };

        assert!(input.json);
        assert_eq!(input.duration_ms, 25);
    }

    #[test]
    fn parses_daemon_soak_command() {
        let command = parse_daemon_command([
            "stutter",
            "daemon",
            "soak",
            "--duration-seconds",
            "120",
            "--tick-ms",
            "500",
            "--profile",
            "apply-low-risk-fake",
            "--max-disk-growth-bytes",
            "1024",
            "--json",
        ])
        .unwrap();

        let AppCommand::DaemonSoak(input) = command else {
            panic!("expected daemon soak command");
        };

        assert!(input.json);
        assert_eq!(input.config.duration_seconds, 120);
        assert_eq!(input.config.tick_millis, 500);
        assert_eq!(
            input.config.profile,
            crate::daemon::DaemonSoakProfile::ApplyLowRiskFake
        );
        assert_eq!(input.config.budget.max_disk_growth_bytes, 1024);
    }

    #[test]
    fn rejects_daemon_soak_zero_duration() {
        let err = parse_daemon_command(["stutter", "daemon", "soak", "--duration-seconds", "0"])
            .unwrap_err()
            .to_string();

        assert!(err.contains("--duration-seconds must be greater than zero"));
    }

    #[test]
    fn rejects_daemon_soak_zero_tick() {
        let err = parse_daemon_command(["stutter", "daemon", "soak", "--tick-ms", "0"])
            .unwrap_err()
            .to_string();

        assert!(err.contains("--tick-ms must be greater than zero"));
    }

    #[test]
    fn parses_daemon_acceptance_command() {
        let command = parse_daemon_command(["stutter", "daemon", "acceptance", "--json"]).unwrap();

        let AppCommand::DaemonAcceptance(input) = command else {
            panic!("expected daemon acceptance command");
        };

        assert!(input.json);
    }

    #[test]
    fn parses_daemon_pause_command() {
        let command = parse_daemon_command(["stutter", "daemon", "pause"]).unwrap();

        assert!(matches!(command, AppCommand::DaemonPause(_)));
    }

    #[test]
    fn parses_daemon_resume_command() {
        let command = parse_daemon_command(["stutter", "daemon", "resume"]).unwrap();

        assert!(matches!(command, AppCommand::DaemonResume(_)));
    }

    #[test]
    fn parses_daemon_restore_command() {
        let command = parse_daemon_command(["stutter", "daemon", "restore", "--dry-run"]).unwrap();

        let AppCommand::DaemonRestore(input) = command else {
            panic!("expected daemon restore command");
        };

        assert!(input.dry_run);
        assert!(!input.emergency);
    }

    #[test]
    fn parses_daemon_emergency_restore_command() {
        let command = parse_daemon_command(["stutter", "daemon", "emergency-restore"]).unwrap();

        let AppCommand::DaemonRestore(input) = command else {
            panic!("expected daemon emergency restore command");
        };

        assert!(!input.dry_run);
        assert!(input.emergency);
    }

    #[test]
    fn parses_daemon_emergency_restore_dry_run_command() {
        let command =
            parse_daemon_command(["stutter", "daemon", "emergency-restore", "--dry-run"]).unwrap();

        let AppCommand::DaemonRestore(input) = command else {
            panic!("expected daemon emergency restore command");
        };

        assert!(input.dry_run);
        assert!(input.emergency);
    }
}
