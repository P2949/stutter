use std::path::PathBuf;

use clap::Parser;

use super::{super::*, helpers::parse_report_command};
use crate::{
    cli::{Cli, Command},
    commands::input::{
        AppCommand, RulesCommand as RulesCommandDto, ScenarioCommand as ScenarioCommandDto,
    },
    process_tree::TaskClass,
    release::ReleaseChannel,
};

#[test]
fn rules_check_accepts_source_path() {
    let cli = Cli::try_parse_from([
        "stutter",
        "rules",
        "check",
        "--source",
        "/tmp/ananicy-rules",
    ])
    .unwrap();

    let command = cli.command.unwrap();
    match command {
        Command::Rules(args) => match args.command {
            RulesCommand::Check(check) => {
                assert_eq!(check.source, Some(PathBuf::from("/tmp/ananicy-rules")));
                assert_eq!(check.generated, None);
            }
            other => panic!("expected rules check command, got {other:?}"),
        },
        other => panic!("expected rules command, got {other:?}"),
    }
}

#[test]
fn rules_check_accepts_generated_path() {
    let cli = Cli::try_parse_from([
        "stutter",
        "rules",
        "check",
        "--generated",
        "/tmp/ananicy.generated.json",
    ])
    .unwrap();

    let command = cli.command.unwrap();
    match command {
        Command::Rules(args) => match args.command {
            RulesCommand::Check(check) => {
                assert_eq!(check.source, None);
                assert_eq!(
                    check.generated,
                    Some(PathBuf::from("/tmp/ananicy.generated.json"))
                );
            }
            other => panic!("expected rules check command, got {other:?}"),
        },
        other => panic!("expected rules command, got {other:?}"),
    }
}

#[test]
fn rules_import_accepts_out_path() {
    let cli = Cli::try_parse_from([
        "stutter",
        "rules",
        "import",
        "--source",
        "/tmp/ananicy-rules",
        "--out",
        "/tmp/ananicy.generated.json",
    ])
    .unwrap();

    let command = cli.command.unwrap();
    match command {
        Command::Rules(args) => match args.command {
            RulesCommand::Import(import) => {
                assert_eq!(import.source, PathBuf::from("/tmp/ananicy-rules"));
                assert_eq!(
                    import.out,
                    Some(PathBuf::from("/tmp/ananicy.generated.json"))
                );
            }
            other => panic!("expected rules import command, got {other:?}"),
        },
        other => panic!("expected rules command, got {other:?}"),
    }
}

#[test]
fn rules_import_default_name_is_ananicy() {
    let cli = Cli::try_parse_from([
        "stutter",
        "rules",
        "import",
        "--source",
        "/tmp/ananicy-rules",
    ])
    .unwrap();

    let command = cli.command.unwrap();
    match command {
        Command::Rules(args) => match args.command {
            RulesCommand::Import(import) => {
                assert_eq!(import.name, "ananicy");
                assert_eq!(import.license, "GPL-3.0-only");
                assert!(!import.dry_run);
            }
            other => panic!("expected rules import command, got {other:?}"),
        },
        other => panic!("expected rules command, got {other:?}"),
    }
}

#[test]
fn parses_report_cluster_window_and_top() {
    let command = parse_report_command([
        "stutter",
        "report",
        "--html",
        "/tmp/report.html",
        "--cluster-ms",
        "5",
        "--top",
        "25",
        "/tmp/run",
    ])
    .unwrap();

    let AppCommand::Report(input) = command else {
        panic!("expected report command");
    };

    assert_eq!(input.top, 25);
    assert_eq!(input.html, Some(PathBuf::from("/tmp/report.html")));
    assert_eq!(input.cluster_window_ms, 5);
    assert_eq!(input.path, Some(PathBuf::from("/tmp/run")));
}

#[test]
fn parses_report_batch_json_summary() {
    let command = parse_report_command([
        "stutter",
        "report",
        "--batch",
        "/tmp/runs",
        "--json-summary",
        "--top",
        "4",
    ])
    .unwrap();

    let AppCommand::Report(input) = command else {
        panic!("expected report command");
    };

    assert_eq!(input.batch, Some(PathBuf::from("/tmp/runs")));
    assert!(input.json_summary);
    assert_eq!(input.top, 4);
    assert_eq!(input.path, None);
}

#[test]
fn parses_summary_command() {
    let command = parse_report_command([
        "stutter",
        "summary",
        "--json",
        "--top",
        "3",
        "--filter-class",
        "Game",
        "/tmp/run",
    ])
    .unwrap();

    let AppCommand::Summary(input) = command else {
        panic!("expected summary command");
    };

    assert_eq!(input.path, PathBuf::from("/tmp/run"));
    assert!(input.json);
    assert_eq!(input.top, 3);
    assert_eq!(input.filter_class, Some(TaskClass::Game));
}

#[test]
fn parses_release_check_command() {
    let command = parse_report_command([
        "stutter",
        "release",
        "check",
        "--channel",
        "low-risk-stable",
        "--soak-tests",
        "--json",
    ])
    .unwrap();

    let AppCommand::ReleaseCheck(input) = command else {
        panic!("expected release check command");
    };

    assert_eq!(input.channel, ReleaseChannel::LowRiskStable);
    assert!(input.inputs.soak_tests);
    assert!(input.json);
    assert!(!input.enforce);

    assert!(!input.inputs.production_distro_packaging);
    assert!(!input.inputs.reproducible_packaged_ebpf_object);
    assert!(!input.inputs.packaging_install_tests);
    assert!(!input.inputs.packaging_service_smoke_tests);
    assert!(!input.inputs.versioned_release_tarball);
}

#[test]
fn parses_release_check_full_flags() {
    let command = parse_report_command([
        "stutter",
        "release",
        "check",
        "--channel",
        "experimental",
        "--apply-actions-enabled",
        "--soak-tests",
        "--stronger-tests",
        "--real-machine-validation",
        "--production-distro-packaging",
        "--reproducible-packaged-ebpf-object",
        "--packaging-install-tests",
        "--packaging-service-smoke-tests",
        "--versioned-release-tarball",
        "--json",
        "--enforce",
    ])
    .unwrap();

    let AppCommand::ReleaseCheck(input) = command else {
        panic!("expected release check command");
    };

    assert_eq!(input.channel, ReleaseChannel::Experimental);
    assert!(input.inputs.apply_actions_enabled);
    assert!(input.inputs.soak_tests);
    assert!(input.inputs.stronger_tests);
    assert!(input.inputs.real_machine_validation);
    assert!(input.inputs.production_distro_packaging);
    assert!(input.inputs.reproducible_packaged_ebpf_object);
    assert!(input.inputs.packaging_install_tests);
    assert!(input.inputs.packaging_service_smoke_tests);
    assert!(input.inputs.versioned_release_tarball);
    assert!(input.json);
    assert!(input.enforce);
}

#[test]
fn parses_restore_and_apply_profile_commands() {
    let restore = parse_report_command(["stutter", "restore"]).unwrap();
    assert!(matches!(restore, AppCommand::Restore(input) if !input.dry_run));

    let apply = parse_report_command([
        "stutter",
        "apply-profile",
        "--tree-pid",
        "42",
        "--profile",
        "/tmp/profile.toml",
    ])
    .unwrap();

    let AppCommand::ApplyProfile(input) = apply else {
        panic!("expected apply profile command");
    };

    assert_eq!(input.tree_pid, 42);
    assert_eq!(input.profile, PathBuf::from("/tmp/profile.toml"));
    assert!(!input.force);
    assert!(!input.dry_run);
    assert!(!input.allow_medium_risk);
    assert!(!input.watch);
    assert!(!input.keep_applied);
    assert_eq!(input.refresh_ms, 1_000);
    assert!(!input.enforce);
}

#[test]
fn parses_restore_dry_run() {
    let command = parse_report_command(["stutter", "restore", "--dry-run"]).unwrap();

    assert!(matches!(command, AppCommand::Restore(input) if input.dry_run));
}

#[test]
fn parses_apply_profile_force_watch_and_refresh() {
    let command = parse_report_command([
        "stutter",
        "apply-profile",
        "--tree-pid",
        "42",
        "--profile",
        "/tmp/profile.toml",
        "--force",
        "--allow-medium-risk",
        "--watch",
        "--keep-applied",
        "--refresh-ms",
        "250",
        "--dry-run",
        "--enforce",
    ])
    .unwrap();

    let AppCommand::ApplyProfile(input) = command else {
        panic!("expected apply profile command");
    };

    assert_eq!(input.tree_pid, 42);
    assert_eq!(input.profile, PathBuf::from("/tmp/profile.toml"));
    assert!(input.force);
    assert!(input.allow_medium_risk);
    assert!(input.watch);
    assert!(input.keep_applied);
    assert_eq!(input.refresh_ms, 250);
    assert!(input.dry_run);
    assert!(input.enforce);
}

#[test]
fn parses_tune_command() {
    let command = parse_report_command([
        "stutter",
        "tune",
        "--tree-pid",
        "42",
        "--profiles",
        "/tmp/profiles.toml",
        "--epoch-seconds",
        "60",
        "--warmup-seconds",
        "10",
        "--keep-best",
        "--mangohud-log",
        "/tmp/tune-mango.csv",
    ])
    .unwrap();

    let AppCommand::Tune(input) = command else {
        panic!("expected tune command");
    };

    assert_eq!(input.tree_pid, 42);
    assert_eq!(input.profiles, PathBuf::from("/tmp/profiles.toml"));
    assert_eq!(input.epoch_seconds, 60);
    assert_eq!(input.warmup_seconds, 10);
    assert_eq!(input.runs, 3);
    assert!(input.keep_best);
    assert_eq!(input.baseline_profile, None);
    assert_eq!(input.out_dir, None);
    assert_eq!(
        input.mangohud_log,
        Some(PathBuf::from("/tmp/tune-mango.csv"))
    );
    assert!(!input.enforce);
    assert!(!input.hwmon);
}

#[test]
fn parses_tune_optional_flags() {
    let command = parse_report_command([
        "stutter",
        "tune",
        "--tree-pid",
        "42",
        "--profiles",
        "/tmp/profiles.toml",
        "--epoch-seconds",
        "90",
        "--warmup-seconds",
        "15",
        "--runs",
        "5",
        "--baseline-profile",
        "stock",
        "--out-dir",
        "/tmp/tune-out",
        "--enforce",
        "--hwmon",
    ])
    .unwrap();

    let AppCommand::Tune(input) = command else {
        panic!("expected tune command");
    };

    assert_eq!(input.epoch_seconds, 90);
    assert_eq!(input.warmup_seconds, 15);
    assert_eq!(input.runs, 5);
    assert_eq!(input.baseline_profile.as_deref(), Some("stock"));
    assert_eq!(input.out_dir, Some(PathBuf::from("/tmp/tune-out")));
    assert!(input.enforce);
    assert!(input.hwmon);
}

#[test]
fn parses_rules_import_command_conversion() {
    let command = parse_report_command([
        "stutter",
        "rules",
        "import",
        "--source",
        "/tmp/ananicy-rules",
        "--name",
        "custom",
        "--license",
        "MIT",
        "--source-repo",
        "example/rules",
        "--source-commit",
        "abc123",
        "--out",
        "/tmp/ananicy.generated.json",
        "--dry-run",
    ])
    .unwrap();

    let AppCommand::Rules(input) = command else {
        panic!("expected rules command");
    };
    let RulesCommandDto::Import(import) = input.command else {
        panic!("expected rules import command");
    };

    assert_eq!(import.source, PathBuf::from("/tmp/ananicy-rules"));
    assert_eq!(import.name, "custom");
    assert_eq!(import.license, "MIT");
    assert_eq!(import.source_repo.as_deref(), Some("example/rules"));
    assert_eq!(import.source_commit.as_deref(), Some("abc123"));
    assert_eq!(
        import.out,
        Some(PathBuf::from("/tmp/ananicy.generated.json"))
    );
    assert!(import.dry_run);
}

#[test]
fn parses_rules_check_command_conversion() {
    let source = parse_report_command([
        "stutter",
        "rules",
        "check",
        "--source",
        "/tmp/ananicy-rules",
    ])
    .unwrap();

    let AppCommand::Rules(input) = source else {
        panic!("expected rules command");
    };
    let RulesCommandDto::Check(check) = input.command else {
        panic!("expected rules check command");
    };

    assert_eq!(check.source, Some(PathBuf::from("/tmp/ananicy-rules")));
    assert_eq!(check.generated, None);

    let generated = parse_report_command([
        "stutter",
        "rules",
        "check",
        "--generated",
        "/tmp/ananicy.generated.json",
    ])
    .unwrap();

    let AppCommand::Rules(input) = generated else {
        panic!("expected rules command");
    };
    let RulesCommandDto::Check(check) = input.command else {
        panic!("expected rules check command");
    };

    assert_eq!(check.source, None);
    assert_eq!(
        check.generated,
        Some(PathBuf::from("/tmp/ananicy.generated.json"))
    );
}

#[test]
fn parses_rules_list_status_enable_disable_and_remove_conversion() {
    let list = parse_report_command(["stutter", "rules", "list"]).unwrap();
    assert!(matches!(
        list,
        AppCommand::Rules(crate::commands::input::RulesCommandInput {
            command: RulesCommandDto::List
        })
    ));

    let status = parse_report_command(["stutter", "rules", "status"]).unwrap();
    assert!(matches!(
        status,
        AppCommand::Rules(crate::commands::input::RulesCommandInput {
            command: RulesCommandDto::Status
        })
    ));

    let enable = parse_report_command(["stutter", "rules", "enable", "--name", "custom"]).unwrap();
    let AppCommand::Rules(input) = enable else {
        panic!("expected rules command");
    };
    let RulesCommandDto::Enable(enable) = input.command else {
        panic!("expected rules enable command");
    };
    assert_eq!(enable.name, "custom");

    let disable = parse_report_command(["stutter", "rules", "disable"]).unwrap();
    assert!(matches!(
        disable,
        AppCommand::Rules(crate::commands::input::RulesCommandInput {
            command: RulesCommandDto::Disable
        })
    ));

    let remove = parse_report_command([
        "stutter",
        "rules",
        "remove",
        "--name",
        "custom",
        "--dry-run",
    ])
    .unwrap();
    let AppCommand::Rules(input) = remove else {
        panic!("expected rules command");
    };
    let RulesCommandDto::Remove(remove) = input.command else {
        panic!("expected rules remove command");
    };
    assert_eq!(remove.name, "custom");
    assert!(remove.dry_run);
}

#[test]
fn scenario_create_parses() {
    let command = parse_report_command([
        "stutter",
        "scenario",
        "create",
        "kcd-route",
        "--duration",
        "60",
        "--watch-process",
        "KingdomCome.exe",
        "--preset",
        "diagnosis",
        "--mangohud-log",
        "/tmp/mango.csv",
        "--notes",
        "forest route",
        "--force",
    ])
    .unwrap();

    let AppCommand::Scenario(input) = command else {
        panic!("expected scenario command");
    };
    let ScenarioCommandDto::Create(input) = input.command else {
        panic!("expected scenario create command");
    };

    assert_eq!(input.name, "kcd-route");
    assert!(input.force);
    assert_eq!(input.duration, 60);
    assert_eq!(input.watch_process, Some("KingdomCome.exe".to_owned()));
    assert_eq!(input.preset, "diagnosis");
    assert_eq!(input.mangohud_log, Some(PathBuf::from("/tmp/mango.csv")));
    assert_eq!(input.notes.as_deref(), Some("forest route"));
}

#[test]
fn scenario_run_parses_baseline() {
    let command = parse_report_command([
        "stutter",
        "scenario",
        "run",
        "kcd-route",
        "--role",
        "baseline",
        "--dry-run",
        "--out-dir",
        "/tmp/out",
        "--mangohud-log",
        "/tmp/override.csv",
    ])
    .unwrap();

    let AppCommand::Scenario(input) = command else {
        panic!("expected scenario command");
    };
    let ScenarioCommandDto::Run(input) = input.command else {
        panic!("expected scenario run command");
    };

    assert_eq!(input.name, "kcd-route");
    assert_eq!(input.role, "baseline");
    assert!(input.dry_run);
    assert_eq!(input.out_dir, Some(PathBuf::from("/tmp/out")));
    assert_eq!(
        input.mangohud_log_override,
        Some(PathBuf::from("/tmp/override.csv"))
    );
}

#[test]
fn scenario_run_parses_current() {
    let command = parse_report_command([
        "stutter",
        "scenario",
        "run",
        "kcd-route",
        "--role",
        "current",
    ])
    .unwrap();

    let AppCommand::Scenario(input) = command else {
        panic!("expected scenario command");
    };
    let ScenarioCommandDto::Run(input) = input.command else {
        panic!("expected scenario run command");
    };

    assert_eq!(input.name, "kcd-route");
    assert_eq!(input.role, "current");
}

#[test]
fn scenario_compare_parses() {
    let command = parse_report_command([
        "stutter",
        "scenario",
        "compare",
        "kcd-route",
        "--baseline",
        "/tmp/base",
        "--current",
        "/tmp/current",
        "--top",
        "5",
        "--json-summary",
        "--validate",
    ])
    .unwrap();

    let AppCommand::Scenario(input) = command else {
        panic!("expected scenario command");
    };
    let ScenarioCommandDto::Compare(input) = input.command else {
        panic!("expected scenario compare command");
    };

    assert_eq!(input.name, "kcd-route");
    assert_eq!(input.baseline, Some(PathBuf::from("/tmp/base")));
    assert_eq!(input.current, Some(PathBuf::from("/tmp/current")));
    assert_eq!(input.top, 5);
    assert!(input.json_summary);
    assert!(input.validate);
}

#[test]
fn scenario_path_and_list_parse() {
    let path = parse_report_command(["stutter", "scenario", "path", "kcd-route"]).unwrap();

    let AppCommand::Scenario(input) = path else {
        panic!("expected scenario command");
    };
    let ScenarioCommandDto::Path(input) = input.command else {
        panic!("expected scenario path command");
    };
    assert_eq!(input.name, "kcd-route");

    let list = parse_report_command(["stutter", "scenario", "list"]).unwrap();
    assert!(matches!(
        list,
        AppCommand::Scenario(crate::commands::input::ScenarioCommandInput {
            command: ScenarioCommandDto::List
        })
    ));
}
