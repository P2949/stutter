use std::{ffi::OsString, path::PathBuf, sync::Arc, time::Duration};

use clap::{ArgAction, Args, CommandFactory, Parser, Subcommand, parser::ValueSource};

mod agent;
mod autotune;
mod config;
mod daemon;
mod map;
mod monitor;
mod parse;
mod report;
mod service;
mod validate;
mod version_parse;

use agent::{AgentArgs, PrivilegedWorkerArgs, agent_listen_args};
use autotune::{AutotuneArgs, AutotuneCommand, AutotuneStatusArgs, validate_autotune_mode};
use config::{ConfigArgs, ConfigCommand};
use daemon::{DaemonArgs, DaemonCommand, DaemonPolicyCommand, DaemonProfilesCommand};
use monitor::{
    BenchArgs, MonitorArgPresence, MonitorArgs, RecordArgs, RecordingMode,
    monitor_arg_presence_from_matches, monitor_config_from_monitor_args_with_presence,
};
use report::{
    AdvisorArgs, ApplyProfileArgs, AuditArgs, CheckArgs, CompareArgs, CompareCommand,
    CompletionsArgs, DoctorArgs, InspectDrmTracepointsArgs, InspectIrqsArgs, InspectTreeArgs,
    ManArgs, ProbesArgs, ProfileTemplateArgs, RecommendArgs, ReleaseArgs, ReleaseCommand,
    ReportArgs, RestoreArgs, RulesArgs, RulesCommand, ScenarioArgs, ScenarioCommand, SummaryArgs,
    TuneArgs, WaylandProbeArgs,
};
use service::{
    ServiceArgs, ServiceCommand, ServiceCommandRequestInput, build_service_command_request,
};
use validate::{ValidateArgs, parse_optional_task_class, validate_comm_patterns, validate_pids};

#[cfg(test)]
pub(crate) use crate::commands::input::RulesImportCommandInput;
use crate::{
    commands::input::{
        AdvisorCommandInput, AgentCommandInput, AppCommand, ApplyProfileCommandInput,
        AuditCommandInput, AutotuneApplyCandidateCommandInput,
        AutotuneCommandInput as AutotuneCommandDto, AutotuneGenerateProfilesCommandInput,
        AutotuneReplayCommandInput, AutotuneReplayHistoryCommandInput, AutotuneRestoreCommandInput,
        AutotuneStatusCommandInput, BenchCommandInput, CheckCommandInput, CompletionsCommandInput,
        ConfigCheckCommandInput, DaemonAcceptanceCommandInput, DaemonBenchOverheadCommandInput,
        DaemonConfigExplainCommandInput, DaemonDoctorCommandInput, DaemonExplainCommandInput,
        DaemonPauseCommandInput, DaemonPolicyExplainCommandInput, DaemonPolicyLintCommandInput,
        DaemonProfilesCommandInput, DaemonProfilesExplainCommandInput,
        DaemonProfilesForgetCommandInput, DaemonProfilesListCommandInput,
        DaemonResetStateCommandInput, DaemonRestoreCommandInput, DaemonResumeCommandInput,
        DaemonResyncStateCommandInput, DaemonSoakCommandInput, DaemonStatusCommandInput,
        DaemonWatchCommandInput, DaemonWhatChangedCommandInput, DaemonWhyNotOptimizeCommandInput,
        DisplayPathCompareCommandInput, DoctorCommandInput, InspectIrqsCommandInput,
        InspectTreeCommandInput, ManCommandInput, MonitorCommandInput,
        PrivilegedWorkerCommandInput, ProbesCommandInput, ProfileTemplateCommandInput,
        RecommendCommandInput, ReleaseCheckCommandInput, ReportCommandInput, RestoreCommandInput,
        RulesCommandInput, ScenarioCompareCommandInput, ScenarioCreateCommandInput,
        ScenarioPathCommandInput, ScenarioRunCommandInput, ServiceCommandInput,
        SummaryCommandInput, TuneCommandInput, ValidateCommandInput, VersionCommandInput,
        WaylandProbeCommandInput,
    },
    config::{
        CsvStreamTarget, FocusSource, ForegroundSource, WaylandPresentationSource,
        effective::resolve_monitor_config_sources,
        layer::MonitorConfigLayer,
        merge::{CliOverrides, ConfigSources, DefaultConfig, PresetConfig},
        model::MonitorConfig,
    },
    daemon::testing::{DaemonSoakBudget, DaemonSoakConfig, DaemonSoakProfile},
    process_tree::TaskClass,
    release::{ReleaseChannel, ReleaseReadinessInputs},
    service::{
        ServiceAction, ServiceCommandRequest, ServiceManager, ServiceMode,
        default_service_binary_path,
    },
};

#[derive(Parser, Debug)]
#[command(
    version = crate::metadata::build_version(),
    about = "Profile scheduler runnable latency for selected tasks"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    legacy_monitor: MonitorArgs,
}

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn clap_version_uses_build_version_metadata() {
        assert_eq!(
            Cli::command().get_version(),
            Some(crate::metadata::build_version())
        );
        assert_eq!(crate::metadata::build_git_rev(), env!("STUTTER_GIT_REV"));
    }

    #[test]
    fn parse_version_features_request_bypasses_clap_version_exit() {
        let command = parse_app_command_from(["stutter", "--version", "--features"]).unwrap();

        let AppCommand::Version(input) = command else {
            panic!("expected version command");
        };

        assert!(input.features);
    }

    #[test]
    fn clap_top_level_command_tree_matches_snapshot() {
        let mut rendered = String::from("stutter\n");
        for subcommand in Cli::command().get_subcommands() {
            rendered.push_str(&format!("  {}\n", subcommand.get_name()));
        }

        assert_eq!(
            rendered,
            include_str!("../../tests/snapshots/clap_top_level_commands.txt")
        );
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    Monitor(MonitorArgs),
    Record(RecordArgs),
    Bench(BenchArgs),
    InspectTree(InspectTreeArgs),
    Report(ReportArgs),
    Summary(SummaryArgs),
    Validate(ValidateArgs),
    Restore(RestoreArgs),
    ApplyProfile(ApplyProfileArgs),
    Tune(TuneArgs),
    Recommend(RecommendArgs),
    Release(ReleaseArgs),
    Check(CheckArgs),
    Compare(CompareArgs),
    Config(ConfigArgs),
    Audit(AuditArgs),
    Advisor(AdvisorArgs),
    Doctor(DoctorArgs),
    ProfileTemplate(ProfileTemplateArgs),
    #[command(name = "inspect-irqs")]
    InspectIrqs(InspectIrqsArgs),
    #[command(name = "inspect-drm-tracepoints")]
    InspectDrmTracepoints(InspectDrmTracepointsArgs),
    #[command(name = "wayland-probe")]
    WaylandProbe(WaylandProbeArgs),
    Autotune(AutotuneArgs),
    #[command(name = "autotune-status")]
    AutotuneStatus(AutotuneStatusArgs),
    Agent(AgentArgs),
    #[command(name = "privileged-worker")]
    PrivilegedWorker(PrivilegedWorkerArgs),
    Daemon(DaemonArgs),
    Service(ServiceArgs),
    #[command(name = "completions")]
    Completions(CompletionsArgs),
    #[command(name = "man")]
    Man(ManArgs),
    Probes(ProbesArgs),
    Rules(RulesArgs),
    Scenario(ScenarioArgs),
}

pub(crate) fn autotune_monitor_config(
    input: &crate::autotune::commands::live::AutotuneCommandInput,
) -> anyhow::Result<Arc<MonitorConfig>> {
    let has_target = input.tree_pid.is_some() || input.watch_process.is_some();
    if !has_target && !input.auto_focus {
        anyhow::bail!("autotune requires --tree-pid, --watch-process, or --auto-focus");
    }

    let mut monitor = MonitorArgs {
        watch_process: input.watch_process.clone(),
        tree_pids: input.tree_pid.map_or(Vec::new(), |pid| vec![pid]),
        persistent: input.watch_process.is_some(),
        summary_period_ms: Some(input.summary_ms),
        preset: Some(input.preset.clone()),
        hwmon: input.hwmon,
        no_hwmon: !input.hwmon,
        mangohud_log: input.mangohud_log.clone(),
        no_record: true,
        run_name: Some("autotune-observe".to_owned()),
        auto_focus: input.auto_focus,
        focus_source: input.focus_source,
        foreground_window: input.foreground_window
            || input.auto_focus
            || matches!(
                input.focus_source,
                FocusSource::Foreground | FocusSource::Hybrid
            ),
        foreground_source: input.foreground_source,
        foreground_poll_ms: input.foreground_poll_ms,
        foreground_max_stale_ms: input.foreground_max_stale_ms,
        foreground_include_title: false,
        auto_focus_min_confidence: 0.70,
        auto_focus_required_polls: 3,
        auto_focus_switch_cooldown_ms: 5_000,
        auto_focus_switch_margin: 0.20,
        auto_focus_max_roots: 1,
        ..Default::default()
    };

    monitor.no_record = true;

    Ok(Arc::new(monitor_config_from_monitor_args_with_presence(
        monitor,
        RecordingMode::ForceRecording {
            max_duration: input.duration_seconds.map(Duration::from_secs),
        },
        MonitorArgPresence::autotune_monitor_defaults(),
    )?))
}

pub(crate) fn parse_app_command() -> anyhow::Result<AppCommand> {
    parse_app_command_from(std::env::args_os())
}

pub(crate) fn parse_app_command_from<I, T>(args: I) -> anyhow::Result<AppCommand>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let argv: Vec<OsString> = args.into_iter().map(Into::into).collect();
    if requested_version_features(&argv) {
        return Ok(AppCommand::Version(VersionCommandInput { features: true }));
    }

    let matches = Cli::command().try_get_matches_from(argv.clone())?;
    let cli = Cli::try_parse_from(argv)?;

    match cli.command {
        Some(Command::Monitor(args)) => Ok(AppCommand::Monitor(MonitorCommandInput {
            config: Arc::new(monitor_config_from_monitor_args_with_presence(
                args,
                RecordingMode::Monitor,
                monitor_arg_presence_from_matches(&matches, Some("monitor")),
            )?),
        })),
        Some(Command::Record(args)) => {
            if matches!(args.duration, Some(0)) {
                anyhow::bail!("--duration must be greater than zero");
            }
            if args.monitor.no_record {
                anyhow::bail!(
                    "record --no-record is contradictory; use 'monitor' for non-recording runs"
                );
            }
            let max_duration = args.duration.map(Duration::from_secs);
            Ok(AppCommand::Monitor(MonitorCommandInput {
                config: Arc::new(monitor_config_from_monitor_args_with_presence(
                    args.monitor,
                    RecordingMode::ForceRecording { max_duration },
                    monitor_arg_presence_from_matches(&matches, Some("record")),
                )?),
            }))
        }
        Some(Command::Bench(mut args)) => {
            if args.duration == 0 {
                anyhow::bail!("--duration must be greater than zero");
            }
            if args.scenario.trim().is_empty() {
                anyhow::bail!("--scenario must not be empty");
            }
            if !matches!(args.role.as_str(), "baseline" | "current") {
                anyhow::bail!("--role must be baseline or current");
            }
            if args.monitor.no_record {
                anyhow::bail!("bench --no-record is contradictory");
            }
            let run_name = format!("bench-{}-{}", args.role, args.scenario);
            args.monitor.run_name = Some(run_name.clone());
            let config = Arc::new(monitor_config_from_monitor_args_with_presence(
                args.monitor,
                RecordingMode::ForceRecording {
                    max_duration: Some(Duration::from_secs(args.duration)),
                },
                monitor_arg_presence_from_matches(&matches, Some("bench")),
            )?);
            Ok(AppCommand::Bench(BenchCommandInput {
                config,
                role: args.role,
                run_name,
            }))
        }
        Some(Command::InspectTree(args)) => {
            if args.tree_pid == 0 {
                anyhow::bail!("--tree-pid must be greater than zero");
            }
            Ok(AppCommand::InspectTree(InspectTreeCommandInput {
                tree_pid: args.tree_pid,
            }))
        }
        Some(Command::Report(args)) => {
            if args.top == 0 {
                anyhow::bail!("--top must be greater than zero");
            }
            if args.cluster_window_ms == 0 {
                anyhow::bail!("--cluster-ms must be greater than zero");
            }
            if args.batch.is_none() && args.path.is_none() {
                anyhow::bail!("report requires PATH unless --batch is set");
            }
            Ok(AppCommand::Report(ReportCommandInput {
                path: args.path,
                json: args.json,
                analysis_json: args.analysis_json,
                json_summary: args.json_summary,
                html: args.html,
                top: args.top,
                cluster_window_ms: args.cluster_window_ms,
                batch: args.batch,
                diff: args.diff,
                filter_class: parse_optional_task_class(args.filter_class.as_deref())?,
                flamegraph: args.flamegraph,
            }))
        }
        Some(Command::Summary(args)) => {
            if args.top == 0 {
                anyhow::bail!("--top must be greater than zero");
            }
            Ok(AppCommand::Summary(SummaryCommandInput {
                path: args.path,
                json: args.json,
                top: args.top,
                filter_class: parse_optional_task_class(args.filter_class.as_deref())?,
            }))
        }
        Some(Command::Validate(args)) => Ok(AppCommand::Validate(ValidateCommandInput {
            path: args.path,
            json: args.json,
            strict: args.strict,
        })),
        Some(Command::Restore(args)) => Ok(AppCommand::Restore(RestoreCommandInput {
            dry_run: args.dry_run,
        })),
        Some(Command::ApplyProfile(args)) => {
            if args.tree_pid == 0 {
                anyhow::bail!("--tree-pid must be greater than zero");
            }
            if args.refresh_ms == 0 {
                anyhow::bail!("--refresh-ms must be greater than zero");
            }
            if args.keep_applied && !args.watch {
                anyhow::bail!("--keep-applied requires --watch");
            }
            Ok(AppCommand::ApplyProfile(ApplyProfileCommandInput {
                tree_pid: args.tree_pid,
                profile: args.profile,
                force: args.force,
                dry_run: args.dry_run,
                allow_medium_risk: args.allow_medium_risk,
                watch: args.watch,
                keep_applied: args.keep_applied,
                refresh_ms: args.refresh_ms,
                enforce: args.enforce,
            }))
        }
        Some(Command::Tune(args)) => {
            if args.tree_pid == 0 {
                anyhow::bail!("--tree-pid must be greater than zero");
            }
            if args.epoch_seconds == 0 {
                anyhow::bail!("--epoch-seconds must be greater than zero");
            }
            if args.warmup_seconds >= args.epoch_seconds {
                anyhow::bail!("--warmup-seconds must be less than --epoch-seconds");
            }
            if args.runs == 0 {
                anyhow::bail!("--runs must be greater than zero");
            }
            Ok(AppCommand::Tune(TuneCommandInput {
                tree_pid: args.tree_pid,
                profiles: args.profiles,
                epoch_seconds: args.epoch_seconds,
                warmup_seconds: args.warmup_seconds,
                runs: args.runs,
                keep_best: args.keep_best,
                baseline_profile: args.baseline_profile,
                out_dir: args.out_dir,
                mangohud_log: args.mangohud_log,
                enforce: args.enforce,
                hwmon: args.hwmon,
            }))
        }
        Some(Command::Recommend(args)) => Ok(AppCommand::Recommend(RecommendCommandInput {
            baseline: args.baseline,
            tune: args.tune,
            json: args.json,
            markdown: args.markdown,
        })),
        Some(Command::Release(args)) => match args.command {
            ReleaseCommand::Check(args) => {
                let inputs = ReleaseReadinessInputs {
                    apply_actions_enabled: args.apply_actions_enabled,
                    soak_tests: args.soak_tests,
                    stronger_tests: args.stronger_tests,
                    ..ReleaseReadinessInputs::default()
                };
                Ok(AppCommand::ReleaseCheck(ReleaseCheckCommandInput {
                    channel: args.channel.parse::<ReleaseChannel>()?,
                    inputs,
                    json: args.json,
                    enforce: args.enforce,
                }))
            }
        },
        Some(Command::Check(args)) => {
            if args.max_regression_p99_ms.is_none() && args.max_max_regression_ms.is_none() {
                anyhow::bail!(
                    "check requires at least one threshold: --max-regression-p99-ms or --max-max-regression-ms"
                );
            }
            if let Some(value) = args.max_regression_p99_ms
                && (!value.is_finite() || value < 0.0)
            {
                anyhow::bail!("--max-regression-p99-ms must be a finite non-negative value");
            }
            if let Some(value) = args.max_max_regression_ms
                && (!value.is_finite() || value < 0.0)
            {
                anyhow::bail!("--max-max-regression-ms must be a finite non-negative value");
            }
            if args.top == 0 {
                anyhow::bail!("--top must be greater than zero");
            }
            Ok(AppCommand::Check(CheckCommandInput {
                baseline: args.baseline,
                current: args.current,
                max_regression_p99_ms: args.max_regression_p99_ms,
                max_max_regression_ms: args.max_max_regression_ms,
                json: args.json,
                top: args.top,
                filter_class: parse_optional_task_class(args.filter_class.as_deref())?,
            }))
        }
        Some(Command::Config(args)) => match args.command {
            ConfigCommand::Check(check_args) => {
                Ok(AppCommand::ConfigCheck(ConfigCheckCommandInput {
                    json: check_args.json,
                }))
            }
            ConfigCommand::Explain(explain_args) => {
                Ok(AppCommand::ConfigExplain(DaemonConfigExplainCommandInput {
                    json: explain_args.json,
                    preset: explain_args.preset,
                }))
            }
        },
        Some(Command::Autotune(args)) => {
            if let Some(cmd) = args.command {
                match cmd {
                    AutotuneCommand::GenerateProfiles(args) => {
                        Ok(AppCommand::AutotuneGenerateProfiles(
                            AutotuneGenerateProfilesCommandInput {
                                watch_process: args.watch_process,
                                out: args.out,
                                allow_cpus: args.allow_cpus,
                                deny_cpus: args.deny_cpus,
                                min_render_cpus: args.min_render_cpus,
                                min_game_cpus: args.min_game_cpus,
                                min_compositor_cpus: args.min_compositor_cpus,
                                min_background_cpus: args.min_background_cpus,
                            },
                        ))
                    }
                    AutotuneCommand::ApplyCandidate(args) => Ok(
                        AppCommand::AutotuneApplyCandidate(AutotuneApplyCandidateCommandInput {
                            candidate_json: args.candidate_json,
                            dry_run: args.dry_run,
                        }),
                    ),
                    AutotuneCommand::Replay(replay) => {
                        Ok(AppCommand::AutotuneReplay(AutotuneReplayCommandInput {
                            run: replay.run,
                            config: replay.config,
                        }))
                    }
                    AutotuneCommand::ReplayHistory(replay_args) => Ok(
                        AppCommand::AutotuneReplayHistory(AutotuneReplayHistoryCommandInput {
                            history: replay_args.history,
                        }),
                    ),
                    AutotuneCommand::Restore(args) => {
                        Ok(AppCommand::AutotuneRestore(AutotuneRestoreCommandInput {
                            journal: args.journal,
                            audit: args.audit,
                            history: args.history,
                            dry_run: args.dry_run,
                        }))
                    }
                }
            } else {
                validate_autotune_mode(&args.mode, args.allow_medium_risk)?;
                Ok(AppCommand::Autotune(AutotuneCommandDto {
                    input: crate::autotune::commands::live::AutotuneCommandInput {
                        config: args.config,
                        watch_process: args.watch_process,
                        tree_pid: args.tree_pid,
                        profiles: args.profiles,
                        mode: args.mode,
                        decision_log: args.decision_log,
                        duration_seconds: args.duration_seconds,
                        washout_seconds: args.washout_seconds,
                        washout_verify_interval_ms: args.washout_verify_interval_ms,
                        summary_ms: args.summary_ms,
                        preset: args.preset,
                        hwmon: args.hwmon,
                        mangohud_log: args.mangohud_log,
                        auto_focus: args.auto_focus,
                        min_focus_confidence: args.min_focus_confidence,
                        focus_source: args.focus_source,
                        foreground_window: args.foreground_window,
                        foreground_source: args.foreground_source,
                        foreground_poll_ms: args.foreground_poll_ms,
                        foreground_max_stale_ms: args.foreground_max_stale_ms,
                        allow_system_wide_suggestions: args.allow_system_wide_suggestions,
                        allow_medium_risk: args.allow_medium_risk,
                        high_risk_dry_run: args.high_risk_dry_run,
                    },
                }))
            }
        }
        Some(Command::AutotuneStatus(args)) => {
            Ok(AppCommand::AutotuneStatus(AutotuneStatusCommandInput {
                json: args.json,
            }))
        }
        Some(Command::Audit(args)) => Ok(AppCommand::Audit(AuditCommandInput {
            path: args.path,
            tail: args.tail,
            json: args.json,
        })),
        Some(Command::Advisor(args)) => {
            if args.watch_runs && args.run.is_some() {
                anyhow::bail!("--watch-runs conflicts with --run");
            }
            if !args.watch_runs && args.run.is_none() {
                anyhow::bail!("advisor requires --run unless --watch-runs is set");
            }
            if args.poll_seconds == 0 {
                anyhow::bail!("--poll-seconds must be greater than zero");
            }
            Ok(AppCommand::Advisor(AdvisorCommandInput {
                run: args.run,
                profiles: args.profiles,
                json: args.json,
                watch_runs: args.watch_runs,
                runs_dir: args.runs_dir,
                poll_seconds: args.poll_seconds,
                once: args.once,
            }))
        }
        Some(Command::Doctor(args)) => Ok(AppCommand::Doctor(DoctorCommandInput {
            input: crate::doctor::DoctorInput {
                json: args.json,
                hwmon: args.hwmon,
                hwmon_root: args.hwmon_root,
                hwmon_drm_card: args.hwmon_drm_card,
                hwmon_render_node: args.hwmon_render_node,
                irq_latency: args.irq_latency,
                irqs: args.irqs,
                block_io: args.block_io,
                kms_timing: args.kms_timing,
                faults: args.faults,
                cpu_perf: args.cpu_perf,
                mangohud_log: args.mangohud_log,
            },
        })),
        Some(Command::ProfileTemplate(args)) => {
            Ok(AppCommand::ProfileTemplate(ProfileTemplateCommandInput {
                topology: args.topology,
            }))
        }
        Some(Command::InspectIrqs(args)) => {
            if args.top == 0 {
                anyhow::bail!("--top must be greater than zero");
            }
            Ok(AppCommand::InspectIrqs(InspectIrqsCommandInput {
                json: args.json,
                filter: args.filter.clone(),
                top: args.top,
            }))
        }
        Some(Command::InspectDrmTracepoints(args)) => Ok(AppCommand::InspectDrmTracepoints(
            crate::commands::input::InspectDrmTracepointsCommandInput {
                json: args.json,
                events_root: args.events_root,
            },
        )),
        Some(Command::Compare(args)) => match args.command {
            CompareCommand::DisplayPath(display) => Ok(AppCommand::DisplayPathCompare(
                DisplayPathCompareCommandInput {
                    baseline: display.baseline.clone(),
                    test: display.test.clone(),
                    json: display.json,
                },
            )),
        },
        Some(Command::WaylandProbe(args)) => {
            if args.duration_secs == 0 {
                anyhow::bail!("--duration must be greater than zero");
            }
            Ok(AppCommand::WaylandProbe(WaylandProbeCommandInput {
                duration: Duration::from_secs(args.duration_secs),
                output: args.output.clone(),
                fullscreen: args.fullscreen,
                out_dir: args.out_dir.clone(),
            }))
        }
        None => Ok(AppCommand::Monitor(MonitorCommandInput {
            config: Arc::new(monitor_config_from_monitor_args_with_presence(
                cli.legacy_monitor,
                RecordingMode::Monitor,
                monitor_arg_presence_from_matches(&matches, None),
            )?),
        })),
        Some(Command::Agent(args)) => {
            if args.max_duration_seconds == 0 {
                anyhow::bail!("--max-duration-seconds must be greater than zero");
            }
            if args.max_targets == 0 {
                anyhow::bail!("--max-targets must be greater than zero");
            }
            if args.max_concurrent_recordings == 0 {
                anyhow::bail!("--max-concurrent-recordings must be greater than zero");
            }
            if args.max_concurrent_recordings > 1 {
                anyhow::bail!("agent currently supports at most 1 concurrent recording");
            }
            let (bind, unix_socket) = agent_listen_args(args.bind, args.port, args.unix_socket)?;
            Ok(AppCommand::Agent(AgentCommandInput {
                bind,
                unix_socket,
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
            }))
        }
        Some(Command::PrivilegedWorker(args)) => {
            let socket = match args.socket {
                Some(socket) => socket,
                None => crate::daemon::privilege::default_privileged_worker_socket_path()?,
            };
            Ok(AppCommand::PrivilegedWorker(PrivilegedWorkerCommandInput {
                socket,
            }))
        }
        Some(Command::Daemon(args)) => match args.command {
            DaemonCommand::Config(config_args) => {
                if !config_args.explain {
                    anyhow::bail!("daemon config requires --explain");
                }
                Ok(AppCommand::DaemonConfigExplain(
                    DaemonConfigExplainCommandInput {
                        json: config_args.json,
                        preset: config_args.preset,
                    },
                ))
            }
            DaemonCommand::Policy(policy_args) => match policy_args.command {
                DaemonPolicyCommand::Explain(explain_args) => Ok(AppCommand::DaemonPolicyExplain(
                    DaemonPolicyExplainCommandInput {
                        json: explain_args.json,
                        preset: explain_args.preset,
                    },
                )),
            },
            DaemonCommand::PolicyLint(lint_args) => {
                Ok(AppCommand::DaemonPolicyLint(DaemonPolicyLintCommandInput {
                    json: lint_args.json,
                    preset: lint_args.preset,
                }))
            }
            DaemonCommand::Profiles(profiles_args) => match profiles_args.command {
                DaemonProfilesCommand::List(list_args) => Ok(AppCommand::DaemonProfiles(
                    DaemonProfilesCommandInput::List(DaemonProfilesListCommandInput {
                        json: list_args.json,
                    }),
                )),
                DaemonProfilesCommand::Forget(forget_args) => {
                    if !forget_args.all && forget_args.workload_identity_hash.is_none() {
                        anyhow::bail!("daemon profiles forget requires --workload-hash or --all");
                    }
                    if forget_args.all && forget_args.workload_identity_hash.is_some() {
                        anyhow::bail!("--all conflicts with --workload-hash");
                    }
                    Ok(AppCommand::DaemonProfiles(
                        DaemonProfilesCommandInput::Forget(DaemonProfilesForgetCommandInput {
                            workload_identity_hash: forget_args.workload_identity_hash,
                            candidate: forget_args.candidate,
                            all: forget_args.all,
                            dry_run: forget_args.dry_run,
                            json: forget_args.json,
                        }),
                    ))
                }
                DaemonProfilesCommand::Explain(explain_args) => Ok(AppCommand::DaemonProfiles(
                    DaemonProfilesCommandInput::Explain(DaemonProfilesExplainCommandInput {
                        workload_identity_hash: explain_args.workload_identity_hash,
                        json: explain_args.json,
                    }),
                )),
            },
            DaemonCommand::Explain(explain_args) => {
                Ok(AppCommand::DaemonExplain(DaemonExplainCommandInput {
                    json: explain_args.json,
                    explain_last: explain_args.explain_last,
                }))
            }
            DaemonCommand::WhyNotOptimize(args) => Ok(AppCommand::DaemonWhyNotOptimize(
                DaemonWhyNotOptimizeCommandInput {
                    json: args.json,
                    explain_last: args.explain_last,
                },
            )),
            DaemonCommand::WhatChanged(args) => Ok(AppCommand::DaemonWhatChanged(
                DaemonWhatChangedCommandInput {
                    json: args.json,
                    explain_last: args.explain_last,
                },
            )),
            DaemonCommand::Status(status_args) => {
                Ok(AppCommand::DaemonStatus(DaemonStatusCommandInput {
                    json: status_args.json,
                    explain_last: status_args.explain_last,
                }))
            }
            DaemonCommand::Watch(watch_args) => {
                if watch_args.interval_ms == 0 {
                    anyhow::bail!("--interval-ms must be greater than zero");
                }
                if watch_args.iterations == Some(0) {
                    anyhow::bail!("--iterations must be greater than zero");
                }
                Ok(AppCommand::DaemonWatch(DaemonWatchCommandInput {
                    interval_ms: watch_args.interval_ms,
                    iterations: watch_args.iterations,
                    verbose: watch_args.verbose,
                    explain_last: watch_args.explain_last,
                }))
            }
            DaemonCommand::Doctor(doctor_args) => {
                Ok(AppCommand::DaemonDoctor(DaemonDoctorCommandInput {
                    json: doctor_args.json,
                }))
            }
            DaemonCommand::ResetState(reset_args) => {
                Ok(AppCommand::DaemonResetState(DaemonResetStateCommandInput {
                    dry_run: reset_args.dry_run,
                    json: reset_args.json,
                }))
            }
            DaemonCommand::BenchOverhead(bench_args) => Ok(AppCommand::DaemonBenchOverhead(
                DaemonBenchOverheadCommandInput {
                    json: bench_args.json,
                    duration_ms: bench_args.duration_ms,
                },
            )),
            DaemonCommand::Soak(soak_args) => {
                if soak_args.duration_seconds == 0 {
                    anyhow::bail!("--duration-seconds must be greater than zero");
                }
                if soak_args.tick_ms == 0 {
                    anyhow::bail!("--tick-ms must be greater than zero");
                }
                let mut budget = DaemonSoakBudget::default();
                if let Some(max_disk_growth_bytes) = soak_args.max_disk_growth_bytes {
                    budget.max_disk_growth_bytes = max_disk_growth_bytes;
                }
                Ok(AppCommand::DaemonSoak(DaemonSoakCommandInput {
                    config: DaemonSoakConfig {
                        profile: soak_args.profile.parse::<DaemonSoakProfile>()?,
                        duration_seconds: soak_args.duration_seconds,
                        tick_millis: soak_args.tick_ms,
                        budget,
                    },
                    json: soak_args.json,
                }))
            }
            DaemonCommand::Acceptance(acceptance_args) => {
                Ok(AppCommand::DaemonAcceptance(DaemonAcceptanceCommandInput {
                    json: acceptance_args.json,
                }))
            }
            DaemonCommand::Pause(_) => Ok(AppCommand::DaemonPause(DaemonPauseCommandInput)),
            DaemonCommand::Resume(_) => Ok(AppCommand::DaemonResume(DaemonResumeCommandInput)),
            DaemonCommand::ResyncState(resync_args) => Ok(AppCommand::DaemonResyncState(
                DaemonResyncStateCommandInput {
                    dry_run: resync_args.dry_run,
                    json: resync_args.json,
                },
            )),
            DaemonCommand::Restore(restore_args) => {
                Ok(AppCommand::DaemonRestore(DaemonRestoreCommandInput {
                    dry_run: restore_args.dry_run,
                    emergency: false,
                }))
            }
            DaemonCommand::EmergencyRestore(restore_args) => {
                Ok(AppCommand::DaemonRestore(DaemonRestoreCommandInput {
                    dry_run: restore_args.dry_run,
                    emergency: true,
                }))
            }
        },
        Some(Command::Service(args)) => match args.command {
            ServiceCommand::Install(args) => Ok(AppCommand::Service(ServiceCommandInput {
                request: build_service_command_request(ServiceCommandRequestInput {
                    action: ServiceAction::Install,
                    manager: args.manager,
                    mode: args.mode,
                    dry_run: args.dry_run,
                    unit_dir: args.unit_dir,
                    config_dir: args.config_dir,
                    state_dir: args.state_dir,
                    log_dir: args.log_dir,
                    binary: args.binary,
                })?,
                json: args.json,
            })),
            ServiceCommand::Uninstall(args) => Ok(AppCommand::Service(ServiceCommandInput {
                request: build_service_command_request(ServiceCommandRequestInput {
                    action: ServiceAction::Uninstall,
                    manager: args.manager,
                    mode: args.mode,
                    dry_run: args.dry_run,
                    unit_dir: args.unit_dir,
                    config_dir: args.config_dir,
                    state_dir: args.state_dir,
                    log_dir: args.log_dir,
                    binary: args.binary,
                })?,
                json: args.json,
            })),
            ServiceCommand::Doctor(args) => Ok(AppCommand::Service(ServiceCommandInput {
                request: build_service_command_request(ServiceCommandRequestInput {
                    action: ServiceAction::Doctor,
                    manager: args.manager,
                    mode: args.mode,
                    dry_run: true,
                    unit_dir: args.unit_dir,
                    config_dir: args.config_dir,
                    state_dir: args.state_dir,
                    log_dir: args.log_dir,
                    binary: args.binary,
                })?,
                json: args.json,
            })),
        },
        Some(Command::Completions(args)) => Ok(AppCommand::Completions(CompletionsCommandInput {
            shell: args.shell,
        })),
        Some(Command::Man(args)) => Ok(AppCommand::Man(ManCommandInput {
            output: args.output,
        })),
        Some(Command::Probes(args)) => {
            Ok(AppCommand::Probes(ProbesCommandInput { json: args.json }))
        }
        Some(Command::Rules(args)) => Ok(AppCommand::Rules(RulesCommandInput {
            command: match args.command {
                RulesCommand::Import(args) => {
                    crate::commands::input::RulesCommand::Import(args.into())
                }
                RulesCommand::Check(args) => {
                    crate::commands::input::RulesCommand::Check(args.into())
                }
                RulesCommand::List(_) => crate::commands::input::RulesCommand::List,
                RulesCommand::Status(_) => crate::commands::input::RulesCommand::Status,
                RulesCommand::Enable(args) => crate::commands::input::RulesCommand::Enable(
                    crate::commands::input::RulesEnableArgs { name: args.name },
                ),
                RulesCommand::Disable(_) => crate::commands::input::RulesCommand::Disable,
                RulesCommand::Remove(args) => crate::commands::input::RulesCommand::Remove(
                    crate::commands::input::RulesRemoveArgs {
                        name: args.name,
                        dry_run: args.dry_run,
                    },
                ),
            },
        })),
        Some(Command::Scenario(args)) => Ok(AppCommand::Scenario(
            crate::commands::input::ScenarioCommandInput {
                command: match args.command {
                    ScenarioCommand::Create(args) => {
                        if args.name.trim().is_empty() {
                            anyhow::bail!("scenario name must not be empty");
                        }
                        if args.duration == 0 {
                            anyhow::bail!("scenario duration must be greater than zero");
                        }
                        crate::commands::input::ScenarioCommand::Create(
                            ScenarioCreateCommandInput {
                                name: args.name,
                                force: args.force,
                                watch_process: args.watch_process,
                                duration: args.duration,
                                preset: args.preset,
                                mangohud_log: args.mangohud_log,
                                notes: args.notes,
                            },
                        )
                    }
                    ScenarioCommand::Run(args) => {
                        if args.name.trim().is_empty() {
                            anyhow::bail!("scenario name must not be empty");
                        }
                        if !matches!(args.role.as_str(), "baseline" | "current") {
                            anyhow::bail!("--role must be baseline or current");
                        }
                        crate::commands::input::ScenarioCommand::Run(ScenarioRunCommandInput {
                            name: args.name,
                            role: args.role,
                            dry_run: args.dry_run,
                            out_dir: args.out_dir,
                            mangohud_log_override: args.mangohud_log_override,
                        })
                    }
                    ScenarioCommand::Compare(args) => {
                        if args.name.trim().is_empty() {
                            anyhow::bail!("scenario name must not be empty");
                        }
                        if args.top == 0 {
                            anyhow::bail!("--top must be greater than zero");
                        }
                        crate::commands::input::ScenarioCommand::Compare(
                            ScenarioCompareCommandInput {
                                name: args.name,
                                baseline: args.baseline,
                                current: args.current,
                                top: args.top,
                                json_summary: args.json_summary,
                                validate: args.validate,
                            },
                        )
                    }
                    ScenarioCommand::Path(args) => {
                        if args.name.trim().is_empty() {
                            anyhow::bail!("scenario name must not be empty");
                        }
                        crate::commands::input::ScenarioCommand::Path(ScenarioPathCommandInput {
                            name: args.name,
                        })
                    }
                    ScenarioCommand::List => crate::commands::input::ScenarioCommand::List,
                },
            },
        )),
    }
}

fn requested_version_features(argv: &[OsString]) -> bool {
    let mut version = false;
    let mut features = false;
    for arg in argv.iter().skip(1).filter_map(|arg| arg.to_str()) {
        match arg {
            "--version" | "-V" => version = true,
            "--features" => features = true,
            _ => {}
        }
    }
    version && features
}

pub fn command() -> clap::Command {
    Cli::command()
}

#[cfg(test)]
mod split_smoke_tests {
    use super::*;

    #[test]
    fn cli_split_preserves_monitor_parse_path() {
        let command = parse_app_command_from(["stutter", "monitor", "--pid", "1234"]).unwrap();
        assert!(matches!(command, AppCommand::Monitor(_)));
    }

    #[test]
    fn cli_split_preserves_autotune_parse_path() {
        let command = parse_app_command_from([
            "stutter",
            "autotune",
            "--tree-pid",
            "1234",
            "--mode",
            "observe",
        ])
        .unwrap();
        assert!(matches!(command, AppCommand::Autotune(_)));
    }

    #[test]
    fn cli_split_preserves_daemon_status_parse_path() {
        let command = parse_app_command_from(["stutter", "daemon", "status", "--json"]).unwrap();
        assert!(matches!(command, AppCommand::DaemonStatus(_)));
    }

    #[test]
    fn cli_split_preserves_service_parse_path() {
        let command =
            parse_app_command_from(["stutter", "service", "doctor", "--mode", "user-observe"])
                .unwrap();
        assert!(matches!(command, AppCommand::Service(_)));
    }

    #[test]
    fn cli_split_preserves_rules_parse_path() {
        let command = parse_app_command_from(["stutter", "rules", "list"]).unwrap();
        assert!(matches!(command, AppCommand::Rules(_)));
    }

    #[test]
    fn cli_split_review_guard_covers_all_split_cli_modules() {
        struct CliSplitCase {
            argv: &'static [&'static str],
            matches_command: fn(AppCommand) -> bool,
        }

        let cases: &[CliSplitCase] = &[
            CliSplitCase {
                argv: &["stutter", "monitor", "--pid", "1234"],
                matches_command: |command| matches!(command, AppCommand::Monitor(_)),
            },
            CliSplitCase {
                argv: &[
                    "stutter",
                    "autotune",
                    "--tree-pid",
                    "1234",
                    "--mode",
                    "observe",
                ],
                matches_command: |command| matches!(command, AppCommand::Autotune(_)),
            },
            CliSplitCase {
                argv: &["stutter", "daemon", "status", "--json"],
                matches_command: |command| matches!(command, AppCommand::DaemonStatus(_)),
            },
            CliSplitCase {
                argv: &["stutter", "agent"],
                matches_command: |command| matches!(command, AppCommand::Agent(_)),
            },
            CliSplitCase {
                argv: &["stutter", "report", "/tmp/run"],
                matches_command: |command| matches!(command, AppCommand::Report(_)),
            },
            CliSplitCase {
                argv: &["stutter", "config", "check"],
                matches_command: |command| matches!(command, AppCommand::ConfigCheck(_)),
            },
            CliSplitCase {
                argv: &["stutter", "service", "doctor", "--mode", "user-observe"],
                matches_command: |command| matches!(command, AppCommand::Service(_)),
            },
            CliSplitCase {
                argv: &["stutter", "validate", "/tmp/run"],
                matches_command: |command| matches!(command, AppCommand::Validate(_)),
            },
        ];

        for case in cases {
            Cli::try_parse_from(case.argv).unwrap_or_else(|err| {
                panic!("Cli::try_parse_from failed for {:?}: {err}", case.argv)
            });

            let command = parse_app_command_from(case.argv).unwrap_or_else(|err| {
                panic!("parse_app_command_from failed for {:?}: {err}", case.argv)
            });

            assert!(
                (case.matches_command)(command),
                "parsed command did not match expected AppCommand variant for {:?}",
                case.argv
            );
        }
    }
}
