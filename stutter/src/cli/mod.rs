use std::path::PathBuf;

use clap::{Parser, Subcommand};
use serde::Serialize;

pub mod agent;
pub mod autotune;
pub mod config;
pub mod daemon;
pub mod monitor;
pub mod report;
pub mod service;
pub mod validate;

pub fn command() -> clap::Command {
    use clap::CommandFactory;
    Cli::command()
}

use crate::commands::input::AppCommand;

#[derive(Parser, Clone, Debug, Serialize)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Clone, Debug, Serialize)]
pub enum Command {
    Monitor(monitor::MonitorArgs),
    Record(monitor::MonitorArgs),
    Bench(monitor::MonitorArgs),
    #[command(hide = true)]
    LegacyMonitor(monitor::MonitorArgs),

    Daemon(daemon::DaemonArgs),
    Agent(agent::AgentArgs),

    Autotune(autotune::AutotuneArgs),
    AutotuneStatus(autotune::AutotuneStatusArgs),

    Config(config::ConfigArgs),
    Report(report::ReportArgs),
    Summary(report::SummaryArgs),
    Service(service::ServiceArgs),
    Validate(validate::ValidateArgs),

    Check(CheckArgs),

    Completions {
        shell: clap_complete::Shell,
    },
    Man,
    Probes(ProbesArgs),
    Rules(RulesArgs),
    Scenario(ScenarioArgs),
    ProfileTemplate(ProfileTemplateArgs),
    InspectIrqs(InspectIrqsArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RecordingMode {
    LiveOnly,
    RecordOnly,
    BenchOnly,
}

impl RecordingMode {
    pub fn force_recording(&self) -> bool {
        matches!(self, Self::RecordOnly | Self::BenchOnly)
    }

    pub fn max_duration(&self) -> Option<std::time::Duration> {
        match self {
            Self::BenchOnly => Some(std::time::Duration::from_secs(30)),
            _ => None,
        }
    }
}

#[derive(clap::Args, Clone, Debug, Serialize)]
pub struct CheckArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Clone, Debug, Serialize)]
pub struct ProbesArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Clone, Debug, Serialize)]
pub struct RulesArgs {
    #[command(subcommand)]
    pub command: RulesCommand,
}

#[derive(Subcommand, Clone, Debug, Serialize)]
pub enum RulesCommand {
    Import(RulesImportArgs),
    Check(RulesCheckArgs),
    List(RulesListArgs),
    Status(RulesStatusArgs),
    Enable(RulesEnableArgs),
    Disable(RulesDisableArgs),
    Remove(RulesRemoveArgs),
}

#[derive(clap::Args, Clone, Debug, Serialize)]
pub struct RulesImportArgs {
    pub source: PathBuf,
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub source_repo: Option<String>,
    #[arg(long)]
    pub source_commit: Option<String>,
    #[arg(long, default_value = "unknown")]
    pub license: String,
    #[arg(long)]
    pub out: Option<PathBuf>,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(clap::Args, Clone, Debug, Serialize)]
pub struct RulesCheckArgs {
    #[arg(long)]
    pub source: Option<PathBuf>,
    #[arg(long)]
    pub generated: Option<PathBuf>,
}

#[derive(clap::Args, Clone, Debug, Serialize)]
pub struct RulesListArgs {}

#[derive(clap::Args, Clone, Debug, Serialize)]
pub struct RulesStatusArgs {}

#[derive(clap::Args, Clone, Debug, Serialize)]
pub struct RulesEnableArgs {
    pub name: String,
}

#[derive(clap::Args, Clone, Debug, Serialize)]
pub struct RulesDisableArgs {}

#[derive(clap::Args, Clone, Debug, Serialize)]
pub struct RulesRemoveArgs {
    pub name: String,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(clap::Args, Clone, Debug, Serialize)]
pub struct ScenarioArgs {
    #[command(subcommand)]
    pub command: ScenarioCommand,
}

#[derive(Subcommand, Clone, Debug, Serialize)]
pub enum ScenarioCommand {
    Create(ScenarioCreateArgs),
    Run(ScenarioRunArgs),
    Compare(ScenarioCompareArgs),
    Path(ScenarioPathArgs),
    List,
}

#[derive(clap::Args, Clone, Debug, Serialize)]
pub struct ScenarioCreateArgs {
    pub name: String,
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub watch_process: Option<String>,
    #[arg(long, default_value = "30")]
    pub duration: u64,
    #[arg(long, default_value = "diagnosis")]
    pub preset: String,
    #[arg(long)]
    pub mangohud_log: Option<PathBuf>,
    #[arg(long)]
    pub notes: Option<String>,
}

#[derive(clap::Args, Clone, Debug, Serialize)]
pub struct ScenarioRunArgs {
    pub name: String,
    #[arg(long, default_value = "baseline")]
    pub role: String,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub out_dir: Option<PathBuf>,
    #[arg(long)]
    pub mangohud_log_override: Option<PathBuf>,
}

#[derive(clap::Args, Clone, Debug, Serialize)]
pub struct ScenarioCompareArgs {
    pub name: String,
    #[arg(long)]
    pub baseline: Option<PathBuf>,
    #[arg(long)]
    pub current: Option<PathBuf>,
    #[arg(long, default_value = "10")]
    pub top: usize,
    #[arg(long)]
    pub json_summary: bool,
    #[arg(long)]
    pub validate: bool,
}

#[derive(clap::Args, Clone, Debug, Serialize)]
pub struct ScenarioPathArgs {
    pub name: String,
}

#[derive(clap::Args, Clone, Debug, Serialize)]
pub struct ProfileTemplateArgs {
    #[arg(long)]
    pub topology: bool,
}

#[derive(clap::Args, Clone, Debug, Serialize)]
pub struct InspectIrqsArgs {
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub filter: Vec<String>,
    #[arg(long, default_value = "10")]
    pub top: usize,
}

pub fn parse_app_command() -> anyhow::Result<AppCommand> {
    use clap::{CommandFactory, FromArgMatches};
    let matches = Cli::command().get_matches();
    let cli = Cli::from_arg_matches(&matches)?;
    parse_app_command_from(cli, &matches)
}

pub fn parse_app_command_from(cli: Cli, matches: &clap::ArgMatches) -> anyhow::Result<AppCommand> {
    match cli.command {
        Command::Monitor(args) => {
            let presence = monitor::monitor_arg_presence_from_matches(matches, Some("monitor"));
            let config = monitor::monitor_config_from_monitor_args_with_presence(
                args,
                RecordingMode::LiveOnly,
                presence,
            )?;
            Ok(AppCommand::Monitor(
                crate::commands::input::MonitorCommandInput {
                    config: std::sync::Arc::new(config),
                },
            ))
        }
        Command::LegacyMonitor(args) => {
            let presence = monitor::monitor_arg_presence_from_matches(matches, None);
            let config = monitor::monitor_config_from_monitor_args_with_presence(
                args,
                RecordingMode::LiveOnly,
                presence,
            )?;
            Ok(AppCommand::Monitor(
                crate::commands::input::MonitorCommandInput {
                    config: std::sync::Arc::new(config),
                },
            ))
        }
        Command::Record(args) => {
            let presence = monitor::monitor_arg_presence_from_matches(matches, Some("record"));
            let config = monitor::monitor_config_from_monitor_args_with_presence(
                args,
                RecordingMode::RecordOnly,
                presence,
            )?;
            Ok(AppCommand::Monitor(
                crate::commands::input::MonitorCommandInput {
                    config: std::sync::Arc::new(config),
                },
            ))
        }
        Command::Bench(args) => {
            let presence = monitor::monitor_arg_presence_from_matches(matches, Some("bench"));
            let config = monitor::monitor_config_from_monitor_args_with_presence(
                args,
                RecordingMode::BenchOnly,
                presence,
            )?;
            Ok(AppCommand::Bench(
                crate::commands::input::BenchCommandInput {
                    config: std::sync::Arc::new(config),
                    role: args.role.clone().unwrap_or_else(|| "bench".to_owned()),
                    run_name: args.run_name.clone().unwrap_or_else(|| "bench".to_owned()),
                },
            ))
        }
        Command::Daemon(args) => {
            let input = match args.command {
                Some(daemon::DaemonCommand::Config(args)) => match args.command {
                    daemon::DaemonConfigCommand::Explain(args) => AppCommand::DaemonConfigExplain(
                        crate::commands::input::DaemonConfigExplainCommandInput {
                            json: args.json,
                            preset: args.preset,
                        },
                    ),
                },
                Some(daemon::DaemonCommand::Policy(args)) => match args.command {
                    daemon::DaemonPolicyCommand::Explain(args) => AppCommand::DaemonPolicyExplain(
                        crate::commands::input::DaemonPolicyExplainCommandInput {
                            json: args.json,
                            preset: args.preset,
                        },
                    ),
                },
                Some(daemon::DaemonCommand::Profiles(args)) => {
                    let sub = match args.command {
                        Some(daemon::DaemonProfilesCommand::List(args)) => {
                            crate::commands::input::DaemonProfilesCommandInput::List(
                                crate::commands::input::DaemonProfilesListCommandInput {
                                    json: args.json,
                                },
                            )
                        }
                        Some(daemon::DaemonProfilesCommand::Forget(args)) => {
                            crate::commands::input::DaemonProfilesCommandInput::Forget(
                                crate::commands::input::DaemonProfilesForgetCommandInput {
                                    workload_identity_hash: args.workload_identity_hash,
                                    candidate: args.candidate,
                                    all: args.all,
                                    dry_run: args.dry_run,
                                    json: args.json,
                                },
                            )
                        }
                        Some(daemon::DaemonProfilesCommand::Explain(args)) => {
                            crate::commands::input::DaemonProfilesCommandInput::Explain(
                                crate::commands::input::DaemonProfilesExplainCommandInput {
                                    workload_identity_hash: args.workload_identity_hash,
                                    json: args.json,
                                },
                            )
                        }
                        None => crate::commands::input::DaemonProfilesCommandInput::List(
                            crate::commands::input::DaemonProfilesListCommandInput { json: false },
                        ),
                    };
                    AppCommand::DaemonProfiles(sub)
                }
                Some(daemon::DaemonCommand::Explain(args)) => {
                    AppCommand::DaemonExplain(crate::commands::input::DaemonExplainCommandInput {
                        json: args.json,
                        explain_last: args.explain_last,
                    })
                }
                Some(daemon::DaemonCommand::WhyNotOptimize(args)) => {
                    AppCommand::DaemonWhyNotOptimize(
                        crate::commands::input::DaemonWhyNotOptimizeCommandInput {
                            json: args.json,
                            explain_last: args.explain_last,
                        },
                    )
                }
                Some(daemon::DaemonCommand::WhatChanged(args)) => AppCommand::DaemonWhatChanged(
                    crate::commands::input::DaemonWhatChangedCommandInput {
                        json: args.json,
                        explain_last: args.explain_last,
                    },
                ),
                Some(daemon::DaemonCommand::Status(args)) => {
                    AppCommand::DaemonStatus(crate::commands::input::DaemonStatusCommandInput {
                        json: args.json,
                        explain_last: args.explain_last,
                    })
                }
                Some(daemon::DaemonCommand::Watch(args)) => {
                    AppCommand::DaemonWatch(crate::commands::input::DaemonWatchCommandInput {
                        interval_ms: args.interval_ms,
                        iterations: args.iterations,
                        verbose: args.verbose,
                        explain_last: args.explain_last,
                    })
                }
                Some(daemon::DaemonCommand::Doctor(args)) => {
                    AppCommand::DaemonDoctor(crate::commands::input::DaemonDoctorCommandInput {
                        json: args.json,
                    })
                }
                Some(daemon::DaemonCommand::ResetState(args)) => AppCommand::DaemonResetState(
                    crate::commands::input::DaemonResetStateCommandInput {
                        dry_run: args.dry_run,
                        json: args.json,
                    },
                ),
                Some(daemon::DaemonCommand::BenchOverhead(args)) => {
                    AppCommand::DaemonBenchOverhead(
                        crate::commands::input::DaemonBenchOverheadCommandInput {
                            json: args.json,
                            duration_ms: args.duration_ms,
                        },
                    )
                }
                Some(daemon::DaemonCommand::Soak(args)) => {
                    let config = daemon::daemon_config_from_soak_args(args.clone())?;
                    AppCommand::DaemonSoak(crate::commands::input::DaemonSoakCommandInput {
                        config,
                        json: args.json,
                    })
                }
                Some(daemon::DaemonCommand::Acceptance(args)) => AppCommand::DaemonAcceptance(
                    crate::commands::input::DaemonAcceptanceCommandInput { json: args.json },
                ),
                Some(daemon::DaemonCommand::Pause(_)) => {
                    AppCommand::DaemonPause(crate::commands::input::DaemonPauseCommandInput)
                }
                Some(daemon::DaemonCommand::Resume(_)) => {
                    AppCommand::DaemonResume(crate::commands::input::DaemonResumeCommandInput)
                }
                Some(daemon::DaemonCommand::EmergencyRestore(args)) => {
                    AppCommand::DaemonRestore(crate::commands::input::DaemonRestoreCommandInput {
                        dry_run: args.dry_run,
                        emergency: args.emergency,
                    })
                }
                None => {
                    AppCommand::DaemonStatus(crate::commands::input::DaemonStatusCommandInput {
                        json: false,
                        explain_last: 10,
                    })
                }
            };
            Ok(input)
        }
        Command::Agent(args) => Ok(AppCommand::Agent(
            crate::commands::input::AgentCommandInput {
                bind: args.bind,
                unix_socket: args.unix_socket,
                runs_dir: args.runs_dir,
                allow_unsafe_bind: args.allow_unsafe_bind,
                bearer_token_env: args.bearer_token_env,
                bearer_token_file: args.bearer_token_file,
                read_token_env: args.read_token_env,
                read_token_file: args.read_token_file,
                apply_token_env: args.apply_token_env,
                apply_token_file: args.apply_token_file,
                max_duration_seconds: args.max_duration_seconds,
                max_targets: args.max_targets,
                max_concurrent_recordings: args.max_concurrent_recordings,
            },
        )),
        Command::Autotune(args) => {
            let input = match args.command {
                autotune::AutotuneCommand::ApplyCandidate(args) => {
                    AppCommand::AutotuneApplyCandidate(
                        crate::commands::input::AutotuneApplyCandidateCommandInput {
                            candidate_json: args.candidate_plan_file,
                            dry_run: args.dry_run,
                        },
                    )
                }
            };
            Ok(input)
        }
        Command::AutotuneStatus(args) => Ok(AppCommand::AutotuneStatus(
            crate::commands::input::AutotuneStatusCommandInput { json: args.json },
        )),
        Command::Config(args) => Ok(AppCommand::ConfigCheck(
            crate::commands::input::ConfigCheckCommandInput { json: args.json },
        )),
        Command::Report(args) => Ok(AppCommand::Report(
            crate::commands::input::ReportCommandInput {
                path: args.path,
                json: args.json,
                analysis_json: args.analysis_json,
                json_summary: args.json_summary,
                html: args.html,
                top: args.top,
                cluster_window_ms: args.cluster_window_ms,
                batch: args.batch,
                diff: args.diff,
                filter_class: args.filter_class,
                flamegraph: args.flamegraph,
            },
        )),
        Command::Summary(args) => Ok(AppCommand::Summary(
            crate::commands::input::SummaryCommandInput {
                path: args.path,
                json: args.json,
                top: args.top,
                filter_class: args.filter_class,
            },
        )),
        Command::Service(args) => Ok(AppCommand::Service(
            crate::commands::input::ServiceCommandInput {
                request: args.request,
                json: args.json,
            },
        )),
        Command::Validate(args) => Ok(AppCommand::Validate(
            crate::commands::input::ValidateCommandInput {
                path: args.path,
                json: args.json,
                strict: args.strict,
            },
        )),
        Command::Check(args) => {
            Ok(AppCommand::Check(
                crate::commands::input::CheckCommandInput {
                    baseline: PathBuf::new(), // Placeholder
                    current: PathBuf::new(),  // Placeholder
                    max_regression_p99_ms: None,
                    max_max_regression_ms: None,
                    json: args.json,
                    top: 10,
                    filter_class: None,
                },
            ))
        }
        Command::Completions { shell } => Ok(AppCommand::Completions(
            crate::commands::input::CompletionsCommandInput { shell },
        )),
        Command::Man => Ok(AppCommand::Man(crate::commands::input::ManCommandInput {
            output: None,
        })),
        Command::Probes(args) => Ok(AppCommand::Probes(
            crate::commands::input::ProbesCommandInput { json: args.json },
        )),
        Command::Rules(args) => {
            let command = match args.command {
                RulesCommand::Import(a) => crate::commands::input::RulesCommand::Import(
                    crate::commands::input::RulesImportArgs {
                        source: a.source,
                        name: a.name,
                        source_repo: a.source_repo,
                        source_commit: a.source_commit,
                        license: a.license,
                        out: a.out,
                        dry_run: a.dry_run,
                    },
                ),
                RulesCommand::Check(a) => crate::commands::input::RulesCommand::Check(
                    crate::commands::input::RulesCheckArgs {
                        source: a.source,
                        generated: a.generated,
                    },
                ),
                RulesCommand::List(_) => crate::commands::input::RulesCommand::List,
                RulesCommand::Status(_) => crate::commands::input::RulesCommand::Status,
                RulesCommand::Enable(a) => crate::commands::input::RulesCommand::Enable(
                    crate::commands::input::RulesEnableArgs { name: a.name },
                ),
                RulesCommand::Disable(_) => crate::commands::input::RulesCommand::Disable,
                RulesCommand::Remove(a) => crate::commands::input::RulesCommand::Remove(
                    crate::commands::input::RulesRemoveArgs {
                        name: a.name,
                        dry_run: a.dry_run,
                    },
                ),
            };
            Ok(AppCommand::Rules(
                crate::commands::input::RulesCommandInput { command },
            ))
        }
        Command::Scenario(args) => {
            let command = match args.command {
                ScenarioCommand::Create(a) => crate::commands::input::ScenarioCommand::Create(
                    crate::commands::input::ScenarioCreateCommandInput {
                        name: a.name,
                        force: a.force,
                        watch_process: a.watch_process,
                        duration: a.duration,
                        preset: a.preset,
                        mangohud_log: a.mangohud_log,
                        notes: a.notes,
                    },
                ),
                ScenarioCommand::Run(a) => crate::commands::input::ScenarioCommand::Run(
                    crate::commands::input::ScenarioRunCommandInput {
                        name: a.name,
                        role: a.role,
                        dry_run: a.dry_run,
                        out_dir: a.out_dir,
                        mangohud_log_override: a.mangohud_log_override,
                    },
                ),
                ScenarioCommand::Compare(a) => crate::commands::input::ScenarioCommand::Compare(
                    crate::commands::input::ScenarioCompareCommandInput {
                        name: a.name,
                        baseline: a.baseline,
                        current: a.current,
                        top: a.top,
                        json_summary: a.json_summary,
                        validate: a.validate,
                    },
                ),
                ScenarioCommand::Path(a) => crate::commands::input::ScenarioCommand::Path(
                    crate::commands::input::ScenarioPathCommandInput { name: a.name },
                ),
                ScenarioCommand::List => crate::commands::input::ScenarioCommand::List,
            };
            Ok(AppCommand::Scenario(
                crate::commands::input::ScenarioCommandInput { command },
            ))
        }
        Command::ProfileTemplate(args) => Ok(AppCommand::ProfileTemplate(
            crate::commands::input::ProfileTemplateCommandInput {
                topology: args.topology,
            },
        )),
        Command::InspectIrqs(args) => Ok(AppCommand::InspectIrqs(
            crate::commands::input::InspectIrqsCommandInput {
                json: args.json,
                filter: args.filter,
                top: args.top,
            },
        )),
    }
}

pub fn autotune_monitor_config(
    input: &crate::autotune::AutotuneCommandInput,
) -> anyhow::Result<crate::config::model::MonitorConfig> {
    let mut args = monitor::MonitorArgs::default();
    args.watch_process = input.watch_process.clone();
    args.tree_pid = input.tree_pid.map(|p| vec![p]).unwrap_or_default();
    args.summary_period_ms = Some(input.summary_ms);
    args.hwmon = input.hwmon;
    args.mangohud_log = input.mangohud_log.clone();
    args.auto_focus = input.auto_focus;
    args.focus_source = input.focus_source;
    args.foreground_window = input.foreground_window;
    args.foreground_source = input.foreground_source;
    args.foreground_poll_ms = input.foreground_poll_ms;
    args.foreground_max_stale_ms = input.foreground_max_stale_ms;

    let presence = monitor::MonitorArgPresence {
        summary_period_ms: true,
        focus_source: true,
        foreground_source: true,
        foreground_poll_ms: true,
        foreground_max_stale_ms: true,
        ..Default::default()
    };

    monitor::monitor_config_from_monitor_args_with_presence(args, RecordingMode::LiveOnly, presence)
}
